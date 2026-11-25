//! Editing commands: moving the caret, typing, formatting, lists, tables, pictures, links,
//! page setup, headers and footers, find and replace, undo.

use office_core::drawing::{Color, ColorBase, Rgb};
use office_core::model::{Align, Block, Body, Cell, Image, Inline, Link, Origin, ParaProps, Paragraph, Row, Run, RunProps, Strike, Table};
use office_core::units::{Emu, FontSize, Percent};
use office_core::xml::{ns, Element};
use office_engine::edit::{self, Edit, Splice};
use office_engine::flow::ParaBox;
use office_engine::measure::Measurer;
use office_engine::para::{self, Ctx, LineEnd};
use office_engine::pos::{self, Path, Pos, Selection};
use office_engine::text;
use serde::Deserialize;

use super::resolve::emu_px;
use super::{StoryId, WStep, WordView};
use crate::{word, word_mut, ListKind, StoryKind};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
    Superscript,
    Subscript,
}

impl WordView {
    // ---- selection ----

    pub fn selection(&self) -> &Selection {
        &self.sel
    }

    pub fn story(&self) -> StoryId {
        self.story
    }

    /// The body's blocks.
    pub fn body(&self) -> &[Block] {
        self.blocks(StoryId::Body)
    }

    /// The blocks of the story the caret is in.
    pub fn current_blocks(&self) -> &[Block] {
        self.blocks(self.story)
    }

    pub fn set_selection(&mut self, sel: Selection) {
        self.sel = self.clamp(sel);
        self.pending = None;
        self.goal_x = None;
        self.caret_near = None;
        self.history.seal();
    }

    /// A selection with both ends on paragraphs that exist, at offsets they have.
    pub(crate) fn clamp(&self, sel: Selection) -> Selection {
        let blocks = self.blocks(self.story);
        let fix = |p: Pos| -> Pos {
            match pos::para(blocks, &p.path) {
                Some(para) => {
                    let len = text::len(para);
                    let mut off = p.offset.min(len);
                    let flat = text::flat(para);
                    while off > 0 && !flat.text.is_char_boundary(off as usize) {
                        off -= 1;
                    }
                    Pos::new(p.path, off)
                }
                None => pos::story_start(blocks).unwrap_or(Pos::start()),
            }
        };
        Selection { anchor: fix(sel.anchor), focus: fix(sel.focus) }
    }

    /// A click on a page: places the caret (or extends the selection with shift), selects a word
    /// on a double click and a paragraph on a triple click. A click on a picture selects it.
    pub fn click(&mut self, host: &mut dyn Measurer, page: usize, x: f64, y: f64, extend: bool, count: u32) -> bool {
        let Some(at) = self.hit(host, page, x, y) else { return false };
        if extend {
            let anchor = self.sel.anchor.clone();
            self.set_selection(Selection { anchor, focus: at });
            return true;
        }
        let blocks = self.blocks(self.story);
        let Some(p) = pos::para(blocks, &at.path) else { return false };
        let flat = text::flat(p);
        match count {
            2 => {
                // Double-clicking a picture keeps only the picture selected instead of expanding
                // into adjacent punctuation via word selection.
                if let Some(sel) = self.image_at(host, page, x, y) {
                    self.set_selection(sel);
                    return true;
                }
                let (a, b) = para::word_at(&flat.text, at.offset);
                self.set_selection(Selection { anchor: Pos::new(at.path.clone(), a), focus: Pos::new(at.path, b) });
            }
            n if n >= 3 => {
                let len = flat.text.len() as u32;
                self.set_selection(Selection { anchor: Pos::new(at.path.clone(), 0), focus: Pos::new(at.path, len) });
            }
            _ => {
                // A picture under the pointer is selected whole.
                if let Some(sel) = self.image_at(host, page, x, y) {
                    self.set_selection(sel);
                    return true;
                }
                self.set_selection(Selection::caret(at));
                self.caret_near = Some((page, x));
            }
        }
        true
    }

    /// The selection for a picture under a point, if there is one.
    fn image_at(&mut self, host: &mut dyn Measurer, page: usize, x: f64, y: f64) -> Option<Selection> {
        let at = self.hit(host, page, x, y)?;
        let p = pos::para(self.blocks(self.story), &at.path)?.clone();
        let p = &p;
        let (pb, dx, dy) = self.find_box(host, &at.path)?;
        let (_, py0) = self.box_page_origin(&at.path, dy + pb.y);
        let fx = x - self.page_body_x(page) - dx;
        let fy = y - py0 - pb.y;
        for line in &pb.layout.lines {
            for f in &line.frags {
                if let para::FragKind::Object { inline, height } = f.kind {
                    let top = line.top + line.baseline - height;
                    if fx >= f.x && fx <= f.x + f.width && fy >= top && fy <= top + height {
                        let (_, start) = text::inline_at(p, f.start)?;
                        let _ = inline;
                        return Some(Selection { anchor: Pos::new(at.path.clone(), start), focus: Pos::new(at.path.clone(), f.end) });
                    }
                }
            }
        }
        None
    }

    fn page_body_x(&self, page: usize) -> f64 {
        match self.story {
            StoryId::Body => self.pages.get(page).map(|p| p.body_x).unwrap_or(0.0),
            StoryId::Story(_) | StoryId::Note(_) => self.pages.get(page).map(|p| p.body_x).unwrap_or(0.0),
        }
    }

    /// The top of the flow containing `path` on the page showing flow y `y`: (page, page y of
    /// flow y 0).
    fn box_page_origin(&self, path: &Path, y: f64) -> (usize, f64) {
        match self.story {
            StoryId::Body => {
                let si = self.sections.iter().position(|s| path.0[0] >= s.first && path.0[0] <= s.last).unwrap_or(0);
                let page = self.page_at(si, y);
                let p = &self.pages[page];
                (page, p.body_y - p.top)
            }
            StoryId::Story(_) => (self.story_page, 0.0),
            StoryId::Note(note) => {
                let places =
                    self.pages.iter().enumerate().flat_map(|(i, p)| {
                        super::note_places(p).into_iter().filter(|(s, _)| s.note == note).map(move |(s, oy)| (i, s, oy))
                    });
                let mut best = (self.story_page, 0.0);
                for (i, s, oy) in places {
                    best = (i, oy);
                    if y < s.bottom - 0.01 {
                        break;
                    }
                }
                best
            }
        }
    }

    /// Drags the selection's focus to a point.
    pub fn drag(&mut self, host: &mut dyn Measurer, page: usize, x: f64, y: f64) {
        if let Some(at) = self.hit(host, page, x, y) {
            let anchor = self.sel.anchor.clone();
            self.sel = Selection { anchor, focus: at };
            self.caret_near = None;
            self.pending = None;
        }
    }

    pub fn select_all(&mut self) {
        let blocks = self.blocks(self.story);
        if let (Some(a), Some(b)) = (pos::story_start(blocks), pos::story_end(blocks)) {
            self.set_selection(Selection { anchor: a, focus: b });
        }
    }

    /// A paragraph's laid-out box in the current story, with its offset in its flow.
    pub(crate) fn find_box(&mut self, host: &mut dyn Measurer, path: &Path) -> Option<(ParaBox, f64, f64)> {
        self.layout(host);
        match self.story {
            StoryId::Body => {
                let si = self.sections.iter().position(|s| path.0[0] >= s.first && path.0[0] <= s.last)?;
                self.sections[si].flow.find(path).map(|(p, x, y)| (p.clone(), x, y))
            }
            StoryId::Story(i) => {
                let flow = self.story_flow(host, i, self.story_page)?;
                flow.find(path).map(|(p, x, y)| (p.clone(), x, y))
            }
            StoryId::Note(i) => {
                let width = self.pages.get(self.story_page).or(self.pages.first()).map(|p| self.note_width(p))?;
                let flow = self.note_flow(host, i, width)?;
                flow.find(path).map(|(p, x, y)| (p.clone(), x, y))
            }
        }
    }

    /// Moves the caret, or the selection's focus with `extend`.
    pub fn move_caret(&mut self, host: &mut dyn Measurer, m: Move, extend: bool) {
        let focus = self.sel.focus.clone();
        // In a right-to-left paragraph the text goes on to the left: the arrows swap.
        let rtl = pos::para(self.blocks(self.story), &focus.path).is_some_and(|p| self.is_rtl(&p.props));
        let m = match (m, rtl) {
            (Move::Left, true) => Move::Right,
            (Move::Right, true) => Move::Left,
            (Move::WordLeft, true) => Move::WordRight,
            (Move::WordRight, true) => Move::WordLeft,
            (m, _) => m,
        };
        let collapse = !extend && !self.sel.is_collapsed();
        let goal = self.goal_x;
        let target = match m {
            Move::Left if collapse => self.sel.start().clone(),
            Move::Right if collapse => self.sel.end().clone(),
            Move::Left => step_char(self.blocks(self.story), &focus, false),
            Move::Right => step_char(self.blocks(self.story), &focus, true),
            Move::WordLeft | Move::WordRight => {
                let forward = m == Move::WordRight;
                let blocks = self.blocks(self.story);
                let flat = pos::para(blocks, &focus.path).map(text::flat).unwrap_or_default();
                let at_edge = if forward { focus.offset as usize >= flat.text.len() } else { focus.offset == 0 };
                if at_edge {
                    step_char(blocks, &focus, forward)
                } else {
                    Pos::new(focus.path.clone(), para::word_move(&flat.text, focus.offset, forward))
                }
            }
            Move::Up | Move::Down => {
                let Some(t) = self.vertical(host, &focus, m == Move::Down, goal) else { return };
                let keep_goal = self.goal_x;
                self.apply_move(t, extend);
                self.goal_x = keep_goal;
                return;
            }
            Move::LineStart | Move::LineEnd => {
                let Some((pb, _, _)) = self.find_box(host, &focus.path) else { return };
                let li = pb.layout.line_of(focus.offset, false);
                let line = &pb.layout.lines[li];
                let off = if m == Move::LineStart {
                    line.start
                } else {
                    let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
                    if line.ends == LineEnd::Wrap {
                        pb.layout.offset_at(&mut ctx, &pb.spec, li, f64::MAX)
                    } else {
                        line.end.saturating_sub(if line.ends == LineEnd::Para { 0 } else { 1 })
                    }
                };
                Pos::new(focus.path.clone(), off)
            }
            Move::DocStart => pos::story_start(self.blocks(self.story)).unwrap_or(focus.clone()),
            Move::DocEnd => pos::story_end(self.blocks(self.story)).unwrap_or(focus.clone()),
            Move::PageUp | Move::PageDown => {
                let ov = self.overlay(host);
                let Some((page, x, y, _)) = ov.caret else { return };
                let n = self.pages.len();
                let target = if m == Move::PageDown { (page + 1).min(n.saturating_sub(1)) } else { page.saturating_sub(1) };
                match self.hit(host, target, x, y) {
                    Some(p) => p,
                    None => return,
                }
            }
        };
        self.apply_move(target, extend);
    }

    fn apply_move(&mut self, target: Pos, extend: bool) {
        let anchor = if extend { self.sel.anchor.clone() } else { target.clone() };
        self.sel = Selection { anchor, focus: target };
        self.pending = None;
        self.goal_x = None;
        self.caret_near = None;
        self.history.seal();
    }

    /// The position a line up or down from `at`, aiming at the same x.
    fn vertical(&mut self, host: &mut dyn Measurer, at: &Pos, down: bool, goal: Option<f64>) -> Option<Pos> {
        let (pb, dx, _) = self.find_box(host, &at.path)?;
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let (cx, li) = pb.layout.caret(&mut ctx, &pb.spec, at.offset, false);
        let x = goal.unwrap_or(dx + cx);
        self.goal_x = Some(x);
        let next_line = if down { li + 1 } else { li.wrapping_sub(1) };
        if next_line < pb.layout.lines.len() {
            let off = pb.layout.offset_at(&mut ctx, &pb.spec, next_line, x - dx);
            return Some(Pos::new(at.path.clone(), off));
        }
        let blocks = self.blocks(self.story);
        let other = if down { pos::next_para(blocks, &at.path) } else { pos::prev_para(blocks, &at.path) };
        let Some(other) = other else {
            return if down { pos::story_end(blocks) } else { pos::story_start(blocks) };
        };
        let (ob, odx, _) = self.find_box(host, &other)?;
        let line = if down { 0 } else { ob.layout.lines.len() - 1 };
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let off = ob.layout.offset_at(&mut ctx, &ob.spec, line, x - odx);
        Some(Pos::new(other, off))
    }

    // ---- applying changes ----

    pub(crate) fn record(&mut self, splices: Vec<Splice>, selection: Option<Selection>, kind: Option<&'static str>) {
        let before = (self.story, self.sel.clone());
        if let Some(sel) = selection {
            self.sel = sel;
        }
        let story = self.story;
        if story == StoryId::Body {
            for s in &splices {
                self.dirty.note(s, false);
            }
            self.unrecorded = false;
        }
        let steps: Vec<WStep> = splices.into_iter().map(|s| WStep::Text(story, s)).collect();
        self.shared_made(&steps);
        self.history.record(steps, before, (self.story, self.sel.clone()), kind);
        self.stale = true;
        self.goal_x = None;
        self.caret_near = None;
    }

    /// Removes the body's blocks `from..=to` from the list at `container` (the body, `[]`, or a
    /// table cell's) as one change, leaving the blocks around them as they were. Refuses, saying
    /// why: `missing` (the blocks aren't there, or the caret isn't in the body), `section` (one
    /// ends a section, so deleting it would join two), or `all` (they're the whole list, which is
    /// never emptied).
    pub fn delete_blocks(&mut self, container: &[u32], from: u32, to: u32) -> Result<(), &'static str> {
        if self.story != StoryId::Body || from > to {
            return Err("missing");
        }
        let Some(list) = pos::blocks_at_mut(self.blocks_mut(StoryId::Body), container) else { return Err("missing") };
        if to as usize >= list.len() {
            return Err("missing");
        }
        // A paragraph that ends a section holds that section's page setup.
        let ends_section =
            |b: &Block| matches!(b, Block::Paragraph(p) if p.origin.get().and_then(|o| o.path(ns::W, &["pPr", "sectPr"])).is_some());
        if list[from as usize..=to as usize].iter().any(ends_section) {
            return Err("section");
        }
        if list.len() == (to - from + 1) as usize {
            return Err("all");
        }
        let removed: Vec<Block> = list.drain(from as usize..=to as usize).collect();
        let splice = Splice { container: container.to_vec(), index: from, removed, inserted: Vec::new() };
        // The caret goes where the blocks were.
        let mut at = container.to_vec();
        at.push(from.min(list.len() as u32 - 1));
        let sel = self.clamp(Selection::caret(Pos::new(Path(at), 0)));
        self.record(vec![splice], Some(sel), None);
        Ok(())
    }

    /// Puts new paragraphs in at `index` of the body's block list `container`, each with its text
    /// and style id, replacing nothing, so two people adding paragraphs at the same place both
    /// keep theirs. Each has the paragraph properties of the paragraph at `like`, and is in its
    /// list, as Enter after it would make it; one with a style of its own isn't in the list.
    /// `index` past the list's end puts them at its end.
    pub fn insert_paragraphs(
        &mut self,
        container: &[u32],
        index: u32,
        like: &[u32],
        paras: &[(String, Option<String>)],
    ) -> Result<(), &'static str> {
        if self.story != StoryId::Body || paras.is_empty() {
            return Err("missing");
        }
        let mut styled = false;
        for id in paras.iter().filter_map(|(_, s)| s.as_deref()) {
            if crate::ensure_style(&mut self.doc, id) {
                self.shared_extra(serde_json::json!({ "style": id }));
                styled = true;
            }
        }
        if styled {
            self.extras = super::read_extras(&self.doc);
            self.specs.clear();
            self.dirty = super::Dirty::Full;
        }
        let template = pos::para(self.blocks(StoryId::Body), &Path(like.to_vec())).ok_or("missing")?;
        let base = template.props.clone();
        let num_pr = template.origin.get().and_then(|o| o.path(ns::W, &["pPr", "numPr"])).cloned();
        let inserted: Vec<Block> = paras
            .iter()
            .map(|(t, style)| {
                let mut p = Paragraph { props: base.clone(), ..Default::default() };
                match (style, &num_pr) {
                    (Some(s), _) => {
                        p.props.style = Some(s.clone());
                        p.props.level = None;
                    }
                    (None, Some(n)) => {
                        p.origin = Origin::new(Element::new(ns::W, "p").with_child(Element::new(ns::W, "pPr").with_child(n.clone())))
                    }
                    (None, None) => {}
                }
                // Right-to-left text in a paragraph with no direction of its own makes it a
                // right-to-left paragraph, as typing it would.
                if p.props.rtl.is_none() && !self.is_rtl(&p.props) && para::first_strong_rtl(t) == Some(true) {
                    p.props.rtl = Some(true);
                }
                text::insert_text(&mut p, 0, t, None);
                Block::Paragraph(p)
            })
            .collect();
        let list = pos::blocks_at_mut(self.blocks_mut(StoryId::Body), container).ok_or("missing")?;
        let index = index.min(list.len() as u32);
        let n = inserted.len() as u32;
        list.splice(index as usize..index as usize, inserted.iter().cloned());
        let splice = Splice { container: container.to_vec(), index, removed: Vec::new(), inserted };
        let mut at = container.to_vec();
        at.push(index + n - 1);
        let len = paras.last().map(|(t, _)| t.len() as u32).unwrap_or(0);
        let sel = self.clamp(Selection::caret(Pos::new(Path(at), len)));
        self.record(vec![splice], Some(sel), None);
        Ok(())
    }

    fn apply_edit(&mut self, e: Edit, kind: Option<&'static str>) {
        self.record(e.splices, e.selection, kind);
    }

    pub fn undo(&mut self) -> bool {
        self.caret_near = None;
        let Some(e) = self.history.undo() else { return false };
        if self.replica.is_some() {
            if !self.shared_replay(e.steps.iter().rev().map(WStep::inverse).collect()) {
                return false;
            }
        } else {
            for step in e.steps.iter().rev() {
                self.undo_step(step);
                self.note_step(step, true);
            }
        }
        self.unrecorded = false;
        self.story = e.before.0;
        self.sel = self.clamp(e.before.1);
        self.stale = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        self.caret_near = None;
        let Some(e) = self.history.redo() else { return false };
        if self.replica.is_some() {
            if !self.shared_replay(e.steps.clone()) {
                return false;
            }
        } else {
            for step in &e.steps {
                self.redo_step(step);
                self.note_step(step, false);
            }
        }
        self.unrecorded = false;
        self.story = e.after.0;
        self.sel = self.clamp(e.after.1);
        self.stale = true;
        true
    }

    pub(crate) fn undo_step(&mut self, step: &WStep) {
        match step {
            WStep::Text(story, s) => s.undo(self.blocks_mut(*story)),
            WStep::Origin { block, before, .. } => self.set_origin(*block, before.clone()),
        }
    }

    pub(crate) fn redo_step(&mut self, step: &WStep) {
        match step {
            WStep::Text(story, s) => s.apply(self.blocks_mut(*story)),
            WStep::Origin { block, after, .. } => self.set_origin(*block, after.clone()),
        }
    }

    /// Notes what an undone or redone step changed in the body.
    pub(crate) fn note_step(&mut self, step: &WStep, undo: bool) {
        match step {
            WStep::Text(StoryId::Body, s) => self.dirty.note(s, undo),
            WStep::Text(..) => {}
            WStep::Origin { .. } => self.dirty = super::Dirty::Full,
        }
    }

    fn set_origin(&mut self, block: Option<u32>, origin: Origin) {
        self.stale = true;
        self.dirty = super::Dirty::Full;
        match block {
            None => self.doc.origin = origin,
            Some(i) => {
                if let Body::Flow { blocks } = &mut self.doc.body {
                    if let Some(Block::Paragraph(p)) = blocks.get_mut(i as usize) {
                        p.origin = origin;
                    }
                }
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Starts and ends a group of commands undone as one.
    pub fn begin_group(&mut self) {
        self.history.begin_group((self.story, self.sel.clone()));
    }

    pub fn end_group(&mut self) {
        self.history.end_group();
    }

    // ---- typing ----

    pub fn insert_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let pending = self.pending.take();
        let sel = self.sel.clone();
        let props = pending.or_else(|| {
            // Replacing a selection: the new text looks like the start of what it replaces.
            (!sel.is_collapsed()).then(|| {
                let (from, _) = sel.ordered();
                pos::para(self.blocks(self.story), &from.path).map(|p| text::props_at(p, from.offset + 1)).unwrap_or_default()
            })
        });
        // Right-to-left text typed into an empty paragraph with no direction of its own makes it
        // a right-to-left paragraph.
        let turn_rtl = sel.is_collapsed()
            && !s.contains('\n')
            && para::first_strong_rtl(s) == Some(true)
            && pos::para(self.blocks(self.story), &sel.focus.path)
                .is_some_and(|p| text::len(p) == 0 && !self.is_rtl(&p.props) && p.props.rtl.is_none());
        if turn_rtl {
            self.history.seal();
            self.begin_group();
            self.para_change(&|p| p.props.rtl = Some(true));
        }
        let e = self.replace_sel(&sel, |root, at| edit::insert_text(root, at, s, props.as_ref()));
        let typing = s.chars().count() == 1 && sel.is_collapsed() && !s.contains('\n') && !turn_rtl;
        if s == " " {
            self.history.seal();
        }
        self.apply_edit(e, if typing { Some("typing") } else { None });
        if turn_rtl {
            self.end_group();
            self.history.seal();
        }
        if s == " " && sel.is_collapsed() {
            self.list_autoformat();
        }
    }

    /// `- `, `* `, `1. ` or `1) ` typed at the start of a paragraph starts a list, as in Word.
    /// Undo takes it back to what was typed.
    fn list_autoformat(&mut self) {
        if self.story != StoryId::Body {
            return;
        }
        let at = self.sel.focus.clone();
        let Some(p) = pos::para(self.blocks(StoryId::Body), &at.path) else { return };
        let kind = match text::flat(p).text.get(..at.offset as usize) {
            Some("- " | "* ") => ListKind::Bullets,
            Some("1. " | "1) ") => ListKind::Numbers,
            _ => return,
        };
        if crate::list_of(&self.doc, p).is_some_and(|(n, _)| n != "0") {
            return;
        }
        // One step for removing the marker and making the list (list's own group takes it in).
        self.history.seal();
        self.begin_group();
        let marker = Selection { anchor: Pos::new(at.path.clone(), 0), focus: at };
        let e = edit::delete(self.blocks_mut(StoryId::Body), &marker);
        self.apply_edit(e, None);
        self.list(Some(kind));
    }

    /// The flat offsets of the current note's own marks (`w:footnoteRef`/`w:endnoteRef`): one
    /// object character each, at the start of the note's text. Empty outside a note.
    fn own_marks(&self) -> Vec<(Path, u32, u32)> {
        if !matches!(self.story, StoryId::Note(_)) {
            return Vec::new();
        }
        let mut out = Vec::new();
        fn walk(blocks: &[Block], base: &mut Vec<u32>, out: &mut Vec<(Path, u32, u32)>) {
            for (i, b) in blocks.iter().enumerate() {
                match b {
                    Block::Paragraph(p) => {
                        base.push(i as u32);
                        let flat = text::flat(p);
                        for (k, inline) in p.content.iter().enumerate() {
                            let mark = matches!(inline, Inline::Raw { xml }
                                if xml.is(ns::W, "r")
                                    && (xml.child(ns::W, "footnoteRef").is_some() || xml.child(ns::W, "endnoteRef").is_some()));
                            if mark {
                                out.push((Path(base.clone()), flat.spans[k].0, flat.spans[k].1));
                            }
                        }
                        base.pop();
                    }
                    Block::Table(t) => {
                        base.push(i as u32);
                        for (r, row) in t.rows.iter().enumerate() {
                            for (c, cell) in row.cells.iter().enumerate() {
                                base.extend([r as u32, c as u32]);
                                walk(&cell.blocks, base, out);
                                base.truncate(base.len() - 2);
                            }
                        }
                        base.pop();
                    }
                    Block::Raw { .. } => {}
                }
            }
        }
        walk(self.blocks(self.story), &mut Vec::new(), &mut out);
        out
    }

    /// Deletes `sel` in the current story. In a note its own mark can't be deleted, so the
    /// range is cut around each covered mark and the caret lands after any kept at the cut.
    fn delete_sel(&mut self, sel: &Selection) -> Edit {
        let marks = self.own_marks();
        if marks.is_empty() || sel.is_collapsed() {
            let root = self.blocks_mut(self.story);
            return edit::delete(root, sel);
        }
        let (from, to) = sel.ordered();
        // What's left to delete once each covered mark is carved out, in order.
        let mut pieces = Vec::new();
        let mut cursor = from.clone();
        let mut kept_at_cut = 0;
        for (path, start, end) in marks {
            let ms = Pos::new(path.clone(), start);
            let me = Pos::new(path.clone(), end);
            if me <= from || ms >= to {
                continue;
            }
            if cursor < ms {
                pieces.push((cursor, ms));
            }
            cursor = me;
            if path == from.path {
                kept_at_cut += end - start;
            }
        }
        if cursor < to {
            pieces.push((cursor, to));
        }
        if pieces.is_empty() {
            // Only the mark was in the range: nothing to delete, and the caret stays put.
            return Edit::default();
        }
        // Back to front, so positions earlier in the story still hold.
        let mut splices = Vec::new();
        let root = self.blocks_mut(self.story);
        for (a, b) in pieces.into_iter().rev() {
            let e = edit::delete(root, &Selection { anchor: a.clone(), focus: b });
            splices.extend(e.splices);
        }
        let mut caret = from;
        caret.offset += kept_at_cut;
        Edit { splices, selection: Some(Selection::caret(caret)) }
    }

    /// Deletes `sel` with [`Self::delete_sel`], then runs `op` on the story's blocks at the
    /// resulting caret and folds its edit in.
    fn replace_sel(&mut self, sel: &Selection, op: impl FnOnce(&mut Vec<Block>, &Selection) -> Edit) -> Edit {
        let mut e = self.delete_sel(sel);
        let at = e.selection.as_ref().map(|s| s.focus.clone()).unwrap_or_else(|| sel.focus.clone());
        let mut e2 = op(self.blocks_mut(self.story), &Selection::caret(at));
        e.splices.append(&mut e2.splices);
        e.selection = e2.selection;
        e
    }

    /// Applies `e` unless it changed nothing (a delete that only covered a note's mark).
    fn apply_nonempty(&mut self, e: Edit, kind: Option<&'static str>) {
        if e.splices.is_empty() && e.selection.is_none() {
            return;
        }
        self.apply_edit(e, kind);
    }

    pub fn backspace(&mut self, word: bool) {
        self.delete_dir(false, word);
    }

    pub fn delete_forward(&mut self, word: bool) {
        self.delete_dir(true, word);
    }

    fn delete_dir(&mut self, forward: bool, word: bool) {
        let sel = self.sel.clone();
        if !sel.is_collapsed() {
            let e = self.delete_sel(&sel);
            self.apply_nonempty(e, None);
            return;
        }
        let at = sel.focus.clone();
        // What to delete, worked out from the story before changing it.
        enum Plan {
            Unlist,
            Range(Pos),
            Nothing,
        }
        let plan = {
            let blocks = self.blocks(self.story);
            match pos::para(blocks, &at.path) {
                None => Plan::Nothing,
                Some(p) => {
                    let flat = text::flat(p);
                    let len = flat.text.len() as u32;
                    if !forward
                        && at.offset == 0
                        && self.story == StoryId::Body
                        && crate::list_of(&self.doc, p).is_some_and(|(n, _)| n != "0")
                    {
                        // Backspace at the start of a list item takes it out of the list first, as Word does.
                        Plan::Unlist
                    } else if forward && at.offset >= len {
                        match pos::next_para(blocks, &at.path)
                            .filter(|n| n.container() == at.path.container() && n.block() == at.path.block() + 1)
                        {
                            Some(n) => Plan::Range(Pos::new(n, 0)),
                            None => Plan::Nothing,
                        }
                    } else if !forward && at.offset == 0 {
                        match pos::prev_para(blocks, &at.path)
                            .filter(|n| n.container() == at.path.container() && n.block() + 1 == at.path.block())
                        {
                            Some(n) => {
                                let l = pos::para(blocks, &n).map(text::len).unwrap_or(0);
                                Plan::Range(Pos::new(n, l))
                            }
                            None => Plan::Nothing,
                        }
                    } else {
                        let off = match (forward, word) {
                            (true, true) => para::word_move(&flat.text, at.offset, true),
                            (false, true) => para::word_move(&flat.text, at.offset, false),
                            (true, false) => para::next_boundary(&flat.text, at.offset),
                            (false, false) => para::prev_boundary(&flat.text, at.offset),
                        };
                        Plan::Range(Pos::new(at.path.clone(), off))
                    }
                }
            }
        };
        match plan {
            Plan::Unlist => self.list(None),
            Plan::Range(other) if other != at => {
                let range = Selection { anchor: at, focus: other };
                let e = self.delete_sel(&range);
                self.apply_nonempty(e, if word { None } else { Some("deleting") });
            }
            _ => {}
        }
    }

    /// Enter: a new paragraph. After a heading the new one is the heading's "next" style; in an
    /// empty list item, Enter ends the list instead.
    pub fn enter(&mut self) {
        let blocks = self.blocks(self.story);
        if let Some(p) = pos::para(blocks, &self.sel.focus.path) {
            if self.sel.is_collapsed() && text::len(p) == 0 && crate::list_of(&self.doc, p).is_some_and(|(n, _)| n != "0") {
                self.list(None);
                return;
            }
        }
        let in_list = pos::para(blocks, &self.sel.focus.path).is_some_and(|p| crate::list_of(&self.doc, p).is_some_and(|(n, _)| n != "0"));
        self.history.seal();
        let next_style = {
            let styles = word(&self.doc).map(|w| w.styles.clone());
            move |props: &office_core::model::ParaProps, at_end: bool| {
                let mut next = props.clone();
                if at_end && !in_list {
                    if let (Some(styles), Some(id)) = (&styles, &props.style) {
                        if let Some(n) = styles.get(id).and_then(|s| s.next.clone()) {
                            if n != *id {
                                next.style = Some(n);
                                next.level = None;
                            }
                        }
                    }
                }
                next
            }
        };
        let sel = self.sel.clone();
        let mut e = self.delete_sel(&sel);
        let at = e.selection.clone().map(|s| s.focus).unwrap_or(sel.focus);
        let root = self.blocks_mut(self.story);
        let num_pr = pos::para(root, &at.path).and_then(|p| p.origin.get()).and_then(|o| o.path(ns::W, &["pPr", "numPr"])).cloned();
        let s = edit::split_para(root, &at, Some(&next_style));
        e.splices.extend(s.splices);
        e.selection = s.selection;
        if let (Some(num_pr), Some(sel)) = (num_pr, &e.selection) {
            if let Some(s) = edit::change_para(root, &sel.focus.path, |p| {
                p.origin = Origin::new(Element::new(ns::W, "p").with_child(Element::new(ns::W, "pPr").with_child(num_pr)));
            }) {
                e.splices.push(s);
            }
        }
        // A paragraph that takes its section's end keeps it: the section break stays at the end.
        self.apply_edit(e, None);
    }

    /// Shift-Enter: a line break in the paragraph.
    pub fn line_break(&mut self) {
        let sel = self.sel.clone();
        let props = self.pending.take();
        let mut e = self.delete_sel(&sel);
        let at = e.selection.clone().map(|s| s.focus).unwrap_or(sel.focus);
        let root = self.blocks_mut(self.story);
        if let Some(s) = edit::change_para(root, &at.path, |p| text::insert_text(p, at.offset, "\n", props.as_ref())) {
            edit::push(&mut e.splices, s);
        }
        e.selection = Some(Selection::caret(Pos::new(at.path, at.offset + 1)));
        self.apply_edit(e, None);
    }

    pub fn page_break(&mut self) {
        if self.story != StoryId::Body {
            return;
        }
        let sel = self.sel.clone();
        let e = self.replace_sel(&sel, |root, at| {
            edit::insert_inline(root, at, Inline::Break { kind: office_core::model::BreakKind::Page, origin: Default::default() })
        });
        self.apply_edit(e, None);
    }

    /// Tab: the next cell in a table (a new row at the end), deeper in a list, or a tab character.
    pub fn tab(&mut self, shift: bool) {
        let path = self.sel.focus.path.clone();
        if let Some((table, row, col)) = path.cell() {
            self.next_cell(&table, row, col, shift);
            return;
        }
        let blocks = self.blocks(self.story);
        let in_list = pos::para(blocks, &path).is_some_and(|p| crate::list_of(&self.doc, p).is_some_and(|(n, _)| n != "0"));
        if in_list && (self.sel.focus.offset == 0 || !self.sel.is_collapsed()) {
            self.indent(if shift { -1 } else { 1 });
            return;
        }
        if !shift {
            self.insert_text("\t");
        }
    }

    fn next_cell(&mut self, table: &Path, row: u32, col: u32, back: bool) {
        let blocks = self.blocks(self.story).to_vec();
        let Some(Block::Table(t)) = pos::blocks_at(&blocks, table.container()).and_then(|b| b.get(table.block() as usize)) else { return };
        let mut slots: Vec<(u32, u32)> = Vec::new();
        for (r, rw) in t.rows.iter().enumerate() {
            for (c, cell) in rw.cells.iter().enumerate() {
                if !cell.covered {
                    slots.push((r as u32, c as u32));
                }
            }
        }
        let Some(i) = slots.iter().position(|s| *s == (row, col)) else { return };
        let target = if back {
            if i == 0 {
                return;
            }
            slots[i - 1]
        } else if i + 1 < slots.len() {
            slots[i + 1]
        } else {
            // Tab in the last cell adds a row.
            self.table_op(TableOp::InsertRow { below: true });
            let row = t.rows.len() as u32;
            (row, 0)
        };
        let mut container = table.0.clone();
        container.extend([target.0, target.1]);
        let blocks = self.blocks(self.story);
        if let Some(p) = pos::first_para_from(blocks, &container, 0) {
            let len = pos::para(blocks, &p).map(text::len).unwrap_or(0);
            self.set_selection(Selection { anchor: Pos::new(p.clone(), 0), focus: Pos::new(p, len) });
        }
    }

    // ---- formatting ----

    /// Changes the selection's run formatting; with a caret, the formatting of what's typed next.
    pub fn format(&mut self, f: &dyn Fn(&mut RunProps)) {
        if self.sel.is_collapsed() {
            let blocks = self.blocks(self.story);
            let p = pos::para(blocks, &self.sel.focus.path);
            let mut props = self.pending.clone().or_else(|| p.map(|p| text::props_at(p, self.sel.focus.offset))).unwrap_or_default();
            f(&mut props);
            // An empty paragraph also shows the change on its mark.
            if p.is_some_and(|p| text::len(p) == 0) {
                let sel = self.sel.clone();
                let root = self.blocks_mut(self.story);
                let s = edit::format_runs(root, &sel, f);
                self.record(s, None, None);
            }
            self.pending = Some(props);
            return;
        }
        let sel = self.sel.clone();
        let root = self.blocks_mut(self.story);
        let s = edit::format_runs(root, &sel, f);
        self.record(s, None, None);
    }

    pub fn toggle(&mut self, t: Toggle) {
        let state = self.state_now();
        let on = !match t {
            Toggle::Bold => state.bold,
            Toggle::Italic => state.italic,
            Toggle::Underline => state.underline,
            Toggle::Strike => state.strike,
            Toggle::Superscript => state.superscript,
            Toggle::Subscript => state.subscript,
        };
        // Turning something off that a style turns on needs an explicit "off".
        let styled = self.styled_props();
        self.format(&move |r: &mut RunProps| match t {
            // Word sets complex-script bold and italic with the others.
            Toggle::Bold => {
                r.bold = if on {
                    Some(true)
                } else if styled.bold.or(styled.bold_complex) == Some(true) {
                    Some(false)
                } else {
                    None
                };
                r.bold_complex = r.bold;
            }
            Toggle::Italic => {
                r.italic = if on {
                    Some(true)
                } else if styled.italic.or(styled.italic_complex) == Some(true) {
                    Some(false)
                } else {
                    None
                };
                r.italic_complex = r.italic;
            }
            Toggle::Underline => {
                r.underline = if on {
                    Some("single".into())
                } else if styled.underline.as_deref().is_some_and(|u| u != "none") {
                    Some("none".into())
                } else {
                    None
                }
            }
            Toggle::Strike => r.strike = if on { Some(Strike::Single) } else { None },
            Toggle::Superscript => r.baseline = if on { Some(office_core::model::Baseline::Superscript) } else { None },
            Toggle::Subscript => r.baseline = if on { Some(office_core::model::Baseline::Subscript) } else { None },
        });
    }

    /// What the paragraph's style alone makes the selected text look like.
    fn styled_props(&self) -> RunProps {
        let blocks = self.blocks(self.story);
        let Some(p) = pos::para(blocks, &self.sel.start().path) else { return RunProps::default() };
        match word(&self.doc) {
            Some(w) => w.styles.resolve_run(&p.props, &RunProps::default(), self.doc.theme.as_ref()),
            None => RunProps::default(),
        }
    }

    pub fn set_font(&mut self, family: &str) {
        let f = family.to_string();
        // The font for complex-script text too (Hebrew, Arabic…), which Word keeps apart.
        self.format(&move |r| {
            r.font = Some(f.clone());
            r.font_complex = Some(f.clone());
        });
    }

    /// Size in half-points (Word's unit: 22 is 11 pt).
    pub fn set_size(&mut self, half_points: u32) {
        let hp = half_points.clamp(2, 3276);
        self.format(&move |r| {
            r.size = Some(FontSize::from_half_points(hp));
            r.size_complex = Some(FontSize::from_half_points(hp));
        });
    }

    /// `RRGGBB`, or `auto` for the automatic color.
    pub fn set_color(&mut self, hex: &str) {
        let c = Rgb::parse(hex).map(Color::rgb);
        self.format(&move |r| r.color = c.clone());
    }

    /// One of Word's highlight names (`yellow`, `green`…), or `none`.
    pub fn set_highlight(&mut self, name: &str) {
        let c = (name != "none").then(|| Color { base: ColorBase::Preset { name: name.to_string() }, transforms: Vec::new() });
        self.format(&move |r| r.highlight = c.clone());
    }

    pub fn clear_formatting(&mut self) {
        self.format(&|r| {
            let style = r.style.take().filter(|s| s == "Hyperlink");
            *r = RunProps { style, ..Default::default() };
        });
    }

    fn para_change(&mut self, f: &dyn Fn(&mut Paragraph)) {
        let sel = self.sel.clone();
        let root = self.blocks_mut(self.story);
        let s = edit::format_paras(root, &sel, f);
        self.record(s, None, None);
    }

    /// Aligns the selected paragraphs: `left`, `center`, `right` or `justify`, as on the page.
    /// Word's left and right are the start and end, so a right-to-left paragraph aligned right
    /// is aligned to its start ("left").
    pub fn align(&mut self, a: &str) {
        use office_core::model::Align;
        let align = match a {
            "center" => Align::Center,
            "right" => Align::Right,
            "justify" => Align::Justify,
            _ => Align::Left,
        };
        let rtl = self.rtl_resolver();
        self.para_change(&move |p| {
            p.props.align = Some(match (align, rtl(&p.props)) {
                (Align::Left, true) => Align::Right,
                (Align::Right, true) => Align::Left,
                (a, _) => a,
            })
        });
    }

    /// Makes the selected paragraphs right to left (Word's `w:bidi`), or left to right.
    pub fn set_direction(&mut self, rtl: bool) {
        let resolve = self.rtl_resolver();
        self.para_change(&move |p| {
            let styled = resolve(&office_core::model::ParaProps { rtl: None, ..p.props.clone() });
            p.props.rtl = if styled == rtl { None } else { Some(rtl) };
        });
    }

    /// Whether a paragraph with these properties is right to left, its style included.
    pub(crate) fn is_rtl(&self, props: &office_core::model::ParaProps) -> bool {
        props.rtl.unwrap_or_else(|| word(&self.doc).is_some_and(|w| w.styles.resolve_para(props).rtl == Some(true)))
    }

    /// [`WordView::is_rtl`] to keep, for changing paragraphs.
    pub(crate) fn rtl_resolver(&self) -> impl Fn(&office_core::model::ParaProps) -> bool + 'static {
        let styles = word(&self.doc).map(|w| w.styles.clone());
        move |props| match props.rtl {
            Some(rtl) => rtl,
            None => styles.as_ref().is_some_and(|s| s.resolve_para(props).rtl == Some(true)),
        }
    }

    pub fn set_style(&mut self, id: &str) {
        if crate::ensure_style(&mut self.doc, id) {
            self.shared_extra(serde_json::json!({ "style": id }));
            self.extras = super::read_extras(&self.doc);
            self.specs.clear();
            self.dirty = super::Dirty::Full;
        }
        let id = id.to_string();
        self.para_change(&move |p| p.props.style = Some(id.clone()));
    }

    /// Line spacing as a multiple of single spacing.
    pub fn line_spacing(&mut self, multiple: f64) {
        let pct = Percent((multiple * Percent::WHOLE as f64).round() as i32);
        self.para_change(&move |p| p.props.line_spacing = Some(office_core::model::LineSpacing::Multiple(pct)));
    }

    /// Puts the selected paragraphs in a bulleted or numbered list, or takes them out (`None`).
    /// Paragraphs already in a list of that kind come out of it, like Word's toggle.
    pub fn list(&mut self, kind: Option<ListKind>) {
        if self.story != StoryId::Body {
            return;
        }
        let state = self.state_now();
        let kind = match (kind, state.list.as_deref()) {
            (Some(ListKind::Bullets), Some("bullet")) | (Some(ListKind::Numbers), Some("numbered")) => None,
            (k, _) => k,
        };
        let blocks = self.blocks(StoryId::Body).to_vec();
        let (from, to) = self.sel.ordered();
        let paths = pos::paras_between(&blocks, &from.path, &to.path);
        // A new list goes on from the list just before, when it's the same kind.
        let num = kind.map(|k| {
            let prev =
                pos::prev_para(&blocks, &from.path).and_then(|pp| pos::para(&blocks, &pp)).and_then(|p| crate::list_of(&self.doc, p));
            match prev {
                Some((n, _)) if n != "0" && self.list_kind(&n) == Some(k) => n,
                _ => {
                    let id = crate::add_list(&mut self.doc, k);
                    let kind = if k == ListKind::Numbers { "numbers" } else { "bullets" };
                    self.shared_extra(serde_json::json!({ "list": id, "kind": kind }));
                    // The new list's indents are in the numbering part.
                    self.extras = super::read_extras(&self.doc);
                    id
                }
            }
        });
        // Lists change paragraphs' markup (numPr), which the layer writes into their origins.
        self.begin_group();
        for path in paths {
            let Some(index) = body_index(&blocks, &path) else { continue };
            let Some(before) = pos::para(self.blocks(StoryId::Body), &path).cloned() else { continue };
            let level = before.props.level.unwrap_or(0);
            crate::set_list(&mut self.doc, index, num.as_deref().map(|n| (n, level)));
            if let Some(after) = pos::para(self.blocks(StoryId::Body), &path).cloned() {
                let s = Splice {
                    container: path.container().to_vec(),
                    index: path.block(),
                    removed: vec![Block::Paragraph(before)],
                    inserted: vec![Block::Paragraph(after)],
                };
                self.record(vec![s], None, None);
            }
        }
        self.end_group();
        self.stale = true;
    }

    fn list_kind(&self, num: &str) -> Option<ListKind> {
        let levels = word(&self.doc)?.numbering.levels(num)?;
        Some(if levels.first()?.format == "bullet" { ListKind::Bullets } else { ListKind::Numbers })
    }

    /// Deeper (`1`) or shallower (`-1`) in a list; outside lists, the indent moves by half an inch.
    pub fn indent(&mut self, delta: i32) {
        let blocks = self.blocks(self.story).to_vec();
        let doc = &self.doc;
        let in_list = pos::para(&blocks, &self.sel.focus.path).is_some_and(|p| crate::list_of(doc, p).is_some_and(|(n, _)| n != "0"));
        if in_list {
            self.para_change(&move |p| {
                let l = p.props.level.unwrap_or(0) as i32 + delta;
                p.props.level = Some(l.clamp(0, 8) as u8);
            });
            // The level lives in numPr too.
            let (from, to) = self.sel.ordered();
            let paths = pos::paras_between(self.blocks(self.story), &from.path, &to.path);
            let blocks = self.blocks(StoryId::Body).to_vec();
            for path in paths {
                if let (Some(i), Some(p)) = (body_index(&blocks, &path), pos::para(&blocks, &path)) {
                    if let Some((n, _)) = crate::list_of(&self.doc, p) {
                        crate::set_list(&mut self.doc, i, Some((&n, p.props.level.unwrap_or(0))));
                    }
                }
            }
            self.dirty = super::Dirty::Full;
            self.stale = true;
        } else {
            let step = Emu::from_inches(0.5).0;
            self.para_change(&move |p| {
                let cur = p.props.indent_start.map(|e| e.0).unwrap_or(0);
                p.props.indent_start = Some(Emu((cur + step * delta as i64).max(0)));
            });
        }
    }

    // ---- clipboard ----

    /// The selection as plain text and as model blocks (JSON) for pasting back into Writan.
    pub fn copy(&self) -> (String, String) {
        let blocks = edit::copy(self.blocks(self.story), &self.sel);
        let text = office_core::model::blocks_text(&blocks);
        let json = serde_json::to_string(&blocks).unwrap_or_default();
        (text, json)
    }

    pub fn cut(&mut self) -> (String, String) {
        let out = self.copy();
        if !self.sel.is_collapsed() {
            let sel = self.sel.clone();
            let e = self.delete_sel(&sel);
            self.apply_nonempty(e, None);
        }
        out
    }

    /// Pastes blocks copied from Writan (JSON), or plain text if they can't be read.
    pub fn paste(&mut self, text: &str, json: Option<&str>) {
        let blocks: Option<Vec<Block>> = json.and_then(|j| serde_json::from_str(j).ok());
        match blocks {
            Some(mut blocks) if !blocks.is_empty() => {
                blocks = blocks.into_iter().map(edit::strip_origins).collect();
                let sel = self.sel.clone();
                // One paragraph pastes into the text; more become paragraphs of their own.
                if let [Block::Paragraph(p)] = blocks.as_slice() {
                    let p = p.clone();
                    let mut e = self.delete_sel(&sel);
                    let at = e.selection.clone().map(|s| s.focus).unwrap_or(sel.focus);
                    let root = self.blocks_mut(self.story);
                    let n = text::len(&p);
                    if let Some(s) = edit::change_para(root, &at.path, |target| {
                        let i = text::boundary(target, at.offset);
                        for (k, inline) in p.content.into_iter().enumerate() {
                            target.content.insert(i + k, inline);
                        }
                    }) {
                        e.splices.push(s);
                    }
                    e.selection = Some(Selection::caret(Pos::new(at.path, at.offset + n)));
                    self.apply_edit(e, None);
                } else {
                    let e = self.replace_sel(&sel, |root, at| edit::insert_blocks(root, at, blocks));
                    self.apply_edit(e, None);
                }
            }
            _ => {
                let t = text.replace("\r\n", "\n").replace('\r', "\n");
                self.insert_text(&t);
            }
        }
    }

    // ---- links ----

    pub fn set_link(&mut self, url: &str) {
        if self.sel.is_collapsed() {
            return;
        }
        let url = url.trim().to_string();
        let sel = self.sel.clone();
        self.begin_group();
        let root = self.blocks_mut(self.story);
        let s = edit::set_link(root, &sel, Some(Link { url: Some(url), ..Default::default() }));
        self.record(s, None, None);
        // Word's Hyperlink style: blue and underlined.
        let root = self.blocks_mut(self.story);
        let s = edit::format_runs(root, &sel, &|r| {
            r.color = Rgb::parse("0563C1").map(Color::rgb);
            r.underline = Some("single".into());
        });
        self.record(s, None, None);
        self.end_group();
        let end = self.sel.end().clone();
        self.set_selection(Selection::caret(end));
    }

    pub fn remove_link(&mut self) {
        let mut sel = self.sel.clone();
        if sel.is_collapsed() {
            // The whole link around the caret.
            let blocks = self.blocks(self.story);
            if let Some(p) = pos::para(blocks, &sel.focus.path) {
                let flat = text::flat(p);
                if let Some((i, _)) = text::inline_at(p, sel.focus.offset.saturating_sub(1)) {
                    if let Inline::Text(r) = &p.content[i] {
                        let link = r.link.clone();
                        if link.is_some() {
                            let mut a = i;
                            while a > 0 && matches!(&p.content[a - 1], Inline::Text(x) if x.link == link) {
                                a -= 1;
                            }
                            let mut b = i;
                            while b + 1 < p.content.len() && matches!(&p.content[b + 1], Inline::Text(x) if x.link == link) {
                                b += 1;
                            }
                            sel = Selection {
                                anchor: Pos::new(sel.focus.path.clone(), flat.spans[a].0),
                                focus: Pos::new(sel.focus.path.clone(), flat.spans[b].1),
                            };
                        }
                    }
                }
            }
        }
        let root = self.blocks_mut(self.story);
        let s = edit::set_link(root, &sel, None);
        self.record(s, None, None);
    }

    /// The link the caret or selection is in.
    pub fn link(&self) -> Option<String> {
        let blocks = self.blocks(self.story);
        let at = self.sel.start();
        let p = pos::para(blocks, &at.path)?;
        let probe = if self.sel.is_collapsed() { at.offset.saturating_sub(1) } else { at.offset };
        let (i, _) = text::inline_at(p, probe)?;
        match &p.content[i] {
            Inline::Text(r) => r.link.as_ref()?.url.clone(),
            _ => None,
        }
    }

    // ---- find and replace ----

    /// Every match of `query` in the current story: (path, start, end).
    pub fn find(&self, query: &str, match_case: bool, whole_word: bool) -> Vec<(Path, u32, u32)> {
        let mut out = Vec::new();
        if query.is_empty() {
            return out;
        }
        let blocks = self.blocks(self.story);
        let mut at = pos::story_start(blocks).map(|p| p.path);
        let q = query.as_bytes();
        while let Some(path) = at {
            if let Some(p) = pos::para(blocks, &path) {
                let flat = text::flat(p);
                let t = flat.text.as_bytes();
                let mut i = 0;
                while i + q.len() <= t.len() {
                    let hit = if match_case { &t[i..i + q.len()] == q } else { t[i..i + q.len()].eq_ignore_ascii_case(q) };
                    if hit && flat.text.is_char_boundary(i) && flat.text.is_char_boundary(i + q.len()) {
                        let word_ok = !whole_word || {
                            let before = flat.text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
                            let after = flat.text[i + q.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
                            before && after
                        };
                        if word_ok {
                            out.push((path.clone(), i as u32, (i + q.len()) as u32));
                            i += q.len();
                            continue;
                        }
                    }
                    i += 1;
                }
            }
            at = pos::next_para(blocks, &path);
        }
        out
    }

    /// Replaces each `(path, start, end)` with `by`, last first so earlier offsets hold.
    pub fn replace(&mut self, matches: &[(Path, u32, u32)], by: &str) {
        // Matches are found in the body.
        if self.story != StoryId::Body {
            self.exit_header_footer();
        }
        self.begin_group();
        for (path, a, b) in matches.iter().rev() {
            let sel = Selection { anchor: Pos::new(path.clone(), *a), focus: Pos::new(path.clone(), *b) };
            self.sel = self.clamp(sel);
            if by.is_empty() {
                let sel = self.sel.clone();
                let e = self.delete_sel(&sel);
                self.apply_nonempty(e, None);
            } else {
                self.insert_text(by);
            }
        }
        self.end_group();
    }

    // ---- tracked changes ----

    /// The tracked changes in the body.
    pub fn revisions(&self) -> Vec<crate::Revision> {
        crate::revisions(&self.doc)
    }

    /// Accepts (`true`) or rejects (`false`) every tracked change in the body, undone as one
    /// change; returns how many there were.
    pub fn resolve_revisions(&mut self, accept: bool) -> usize {
        if self.story != StoryId::Body {
            self.exit_header_footer();
        }
        let before = self.body().to_vec();
        let count = crate::revisions::resolve_all(&mut self.doc, accept).unwrap_or_default();
        if count == 0 {
            self.doc.body = Body::Flow { blocks: before };
            return 0;
        }
        if let Body::Flow { blocks } = &mut self.doc.body {
            if !blocks.iter().any(|b| matches!(b, Block::Paragraph(_))) {
                blocks.push(Block::Paragraph(Paragraph::default()));
            }
        }
        let after = self.body().to_vec();
        let sel = self.clamp(self.sel.clone());
        self.record(vec![Splice { container: Vec::new(), index: 0, removed: before, inserted: after }], Some(sel), None);
        count
    }

    // ---- headings ----

    /// The body's headings: (text, depth, path).
    pub fn outline(&self) -> Vec<(String, u8, Path)> {
        let Some(w) = word(&self.doc) else { return Vec::new() };
        let blocks = self.blocks(StoryId::Body);
        let mut out = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            let Block::Paragraph(p) = b else { continue };
            let Some(id) = &p.props.style else { continue };
            let Some(style) = w.styles.get(id) else { continue };
            let name = style.name.to_ascii_lowercase();
            let level = name.strip_prefix("heading ").and_then(|n| n.trim().parse::<u8>().ok());
            if let Some(level) = level {
                out.push((p.plain_text().trim().to_string(), level.saturating_sub(1), Path::top(i as u32)));
            }
        }
        out
    }

    pub fn word_count(&self) -> usize {
        let blocks = self.blocks(StoryId::Body);
        let mut n = 0;
        let mut at = pos::story_start(blocks).map(|p| p.path);
        while let Some(path) = at {
            if let Some(p) = pos::para(blocks, &path) {
                n += p.plain_text().split_whitespace().filter(|w| w.chars().any(|c| c.is_alphanumeric())).count();
            }
            at = pos::next_para(blocks, &path);
        }
        n
    }

    // ---- tables ----

    pub fn insert_table(&mut self, rows: u32, cols: u32) {
        let rows = rows.clamp(1, 200) as usize;
        let cols = cols.clamp(1, 63) as usize;
        let width = self
            .sections
            .iter()
            .find(|s| self.sel.focus.path.0[0] >= s.first && self.sel.focus.path.0[0] <= s.last)
            .map(|s| s.section.text_width())
            .unwrap_or(Emu::from_inches(6.27));
        let col = Emu(width.0 / cols as i64);
        let border = |side: &str| {
            Element::new(ns::W, side)
                .with_attr(ns::W, "val", "single")
                .with_attr(ns::W, "sz", "4")
                .with_attr(ns::W, "space", "0")
                .with_attr(ns::W, "color", "auto")
        };
        let mut borders = Element::new(ns::W, "tblBorders");
        for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            borders.push(border(side));
        }
        let tbl_pr = Element::new(ns::W, "tblPr")
            .with_child(Element::new(ns::W, "tblW").with_attr(ns::W, "w", "0").with_attr(ns::W, "type", "auto"))
            .with_child(borders)
            .with_child(Element::new(ns::W, "tblLook").with_attr(ns::W, "val", "04A0"));
        let table = Table {
            columns: vec![col; cols],
            rows: (0..rows)
                .map(|_| Row {
                    cells: (0..cols).map(|_| Cell { blocks: vec![Block::Paragraph(Paragraph::default())], ..Default::default() }).collect(),
                    ..Default::default()
                })
                .collect(),
            style: None,
            origin: Origin::new(Element::new(ns::W, "tbl").with_child(tbl_pr)),
        };
        let sel = self.sel.clone();
        let mut e = self.replace_sel(&sel, |root, at| edit::insert_blocks(root, at, vec![Block::Table(table)]));
        // The caret goes into the first cell.
        if let Some(Splice { container, index, inserted, .. }) = e.splices.last() {
            if let Some(off) = inserted.iter().position(|b| matches!(b, Block::Table(_))) {
                let mut c = container.clone();
                c.extend([index + off as u32, 0, 0, 0]);
                e.selection = Some(Selection::caret(Pos::new(Path(c), 0)));
            }
        }
        self.apply_edit(e, None);
    }

    /// The table the caret is in: its path, and the caret's cell.
    pub fn table_at_caret(&self) -> Option<(Path, u32, u32, &Table)> {
        let (table, row, col) = self.sel.focus.path.cell()?;
        let blocks = self.blocks(self.story);
        match pos::blocks_at(blocks, table.container())?.get(table.block() as usize)? {
            Block::Table(t) => Some((table, row, col, t)),
            _ => None,
        }
    }

    /// Changes the caret's table.
    pub fn table_op(&mut self, op: TableOp) {
        let Some((path, row, col, t)) = self.table_at_caret() else { return };
        let mut t = t.clone();
        let (row, col) = (row as usize, col as usize);
        let sel_cells = self.selected_cells();
        let keep = matches!(op, TableOp::Fill(_) | TableOp::Borders(_));
        let mut caret = (row, col);
        match op {
            TableOp::InsertRow { below } => {
                let at = if below { row + t.rows[row].cells[col].row_span.max(1) as usize } else { row };
                let cols = t.columns.len().max(t.rows[row].cells.len());
                let mut new = Row {
                    cells: (0..cols).map(|_| Cell { blocks: vec![Block::Paragraph(Paragraph::default())], ..Default::default() }).collect(),
                    ..Default::default()
                };
                // Inside a cell spanning down, the new row is covered there and the span grows.
                if at > 0 && at < t.rows.len() {
                    for (c, cell) in new.cells.iter_mut().enumerate() {
                        if t.rows[at].cells.get(c).is_some_and(|x| x.covered) {
                            if let Some((r0, c0)) = t.owner(at, c) {
                                if r0 < at && c0 == c {
                                    t.rows[r0].cells[c0].row_span += 1;
                                }
                                *cell = Cell { covered: true, ..Default::default() };
                            }
                        }
                    }
                }
                t.rows.insert(at, new);
                caret = (at, col);
            }
            TableOp::DeleteRow => {
                if t.rows.len() <= 1 {
                    return self.table_op(TableOp::Delete);
                }
                for c in 0..t.rows[row].cells.len() {
                    let cell = &t.rows[row].cells[c];
                    if cell.covered {
                        if let Some((r0, c0)) = t.owner(row, c) {
                            if r0 < row && c0 == c {
                                t.rows[r0].cells[c0].row_span = t.rows[r0].cells[c0].row_span.saturating_sub(1).max(1);
                            }
                        }
                    } else if cell.row_span > 1 && row + 1 < t.rows.len() {
                        // The cell below takes over the span.
                        let mut moved = cell.clone();
                        moved.row_span -= 1;
                        t.rows[row + 1].cells[c] = moved;
                    }
                }
                t.rows.remove(row);
                caret = (row.min(t.rows.len() - 1), col);
            }
            TableOp::InsertColumn { right } => {
                let at = if right { col + t.rows[row].cells[col].col_span.max(1) as usize } else { col };
                let width = t.columns.get(col).copied().unwrap_or(Emu::from_inches(1.0));
                t.columns.insert(at.min(t.columns.len()), width);
                for r in 0..t.rows.len() {
                    let covered = at > 0
                        && at < t.rows[r].cells.len()
                        && t.rows[r].cells[at].covered
                        && t.owner(r, at).is_some_and(|(r0, c0)| r0 == r && c0 < at);
                    if covered {
                        if let Some((r0, c0)) = t.owner(r, at) {
                            t.rows[r0].cells[c0].col_span += 1;
                        }
                        t.rows[r].cells.insert(at, Cell { covered: true, ..Default::default() });
                    } else {
                        let pos = at.min(t.rows[r].cells.len());
                        t.rows[r].cells.insert(pos, Cell { blocks: vec![Block::Paragraph(Paragraph::default())], ..Default::default() });
                    }
                }
                caret = (row, at);
            }
            TableOp::DeleteColumn => {
                if t.columns.len() <= 1 {
                    return self.table_op(TableOp::Delete);
                }
                for r in 0..t.rows.len() {
                    if col >= t.rows[r].cells.len() {
                        continue;
                    }
                    let cell = t.rows[r].cells[col].clone();
                    if cell.covered {
                        if let Some((r0, c0)) = t.owner(r, col) {
                            if c0 < col && r0 == r {
                                t.rows[r0].cells[c0].col_span = t.rows[r0].cells[c0].col_span.saturating_sub(1).max(1);
                            }
                        }
                    } else if cell.col_span > 1 && col + 1 < t.rows[r].cells.len() {
                        let mut moved = cell.clone();
                        moved.col_span -= 1;
                        t.rows[r].cells[col + 1] = moved;
                    }
                    t.rows[r].cells.remove(col);
                }
                if col < t.columns.len() {
                    t.columns.remove(col);
                }
                caret = (row, col.min(t.columns.len() - 1));
            }
            TableOp::Fill(color) => {
                for (r, c) in &sel_cells {
                    if let Some(cell) = t.rows.get_mut(*r).and_then(|x| x.cells.get_mut(*c)) {
                        cell.fill = color.as_deref().and_then(Rgb::parse).map(Color::rgb);
                    }
                }
            }
            TableOp::Borders(scope) => {
                let (r0, r1) = (sel_cells.iter().map(|x| x.0).min().unwrap_or(row), sel_cells.iter().map(|x| x.0).max().unwrap_or(row));
                let (c0, c1) = (sel_cells.iter().map(|x| x.1).min().unwrap_or(col), sel_cells.iter().map(|x| x.1).max().unwrap_or(col));
                for (r, c) in &sel_cells {
                    let Some(cell) = t.rows.get_mut(*r).and_then(|x| x.cells.get_mut(*c)) else { continue };
                    let (rs, cs) = (cell.row_span.max(1) as usize, cell.col_span.max(1) as usize);
                    let sides: Vec<(&str, bool)> = match scope {
                        BorderScope::All => vec![("top", true), ("left", true), ("bottom", true), ("right", true)],
                        BorderScope::None => vec![("top", false), ("left", false), ("bottom", false), ("right", false)],
                        BorderScope::Outside => {
                            let mut v = Vec::new();
                            if *r == r0 {
                                v.push(("top", true));
                            }
                            if *r + rs > r1 {
                                v.push(("bottom", true));
                            }
                            if *c == c0 {
                                v.push(("left", true));
                            }
                            if *c + cs > c1 {
                                v.push(("right", true));
                            }
                            v
                        }
                    };
                    set_cell_borders(cell, &sides);
                }
            }
            TableOp::Delete => {
                let blocks = self.blocks(self.story).to_vec();
                let len = pos::blocks_at(&blocks, path.container()).map(|b| b.len()).unwrap_or(0);
                let root = self.blocks_mut(self.story);
                // A story keeps at least one paragraph.
                let replacement = if len == 1 { vec![Block::Paragraph(Paragraph::default())] } else { Vec::new() };
                let s = edit::replace(root, path.container(), path.block(), 1, replacement);
                let root = self.blocks(self.story);
                let caret = pos::first_para_from(root, path.container(), path.block())
                    .or_else(|| pos::last_para_upto(root, path.container(), path.block().saturating_sub(1)))
                    .map(|p| Pos::new(p, 0))
                    .unwrap_or(Pos::start());
                self.record(vec![s], Some(Selection::caret(caret)), None);
                return;
            }
        }
        let root = self.blocks_mut(self.story);
        let s = edit::replace(root, path.container(), path.block(), 1, vec![Block::Table(t)]);
        let mut c = path.0.clone();
        c.extend([caret.0 as u32, caret.1 as u32]);
        let blocks = self.blocks(self.story);
        let target = pos::first_para_from(blocks, &c, 0).map(|p| Pos::new(p, 0)).unwrap_or_else(|| self.sel.focus.clone());
        self.record(vec![s], Some(if keep { self.clamp(self.sel.clone()) } else { Selection::caret(target) }), None);
    }

    /// The cells the selection covers in the caret's table: a rectangle between the anchor's
    /// cell and the focus's, or the caret's cell.
    fn selected_cells(&self) -> Vec<(usize, usize)> {
        let Some((table, row, col, t)) = self.table_at_caret() else { return Vec::new() };
        let other = self.sel.anchor.path.cell().filter(|(tp, _, _)| *tp == table).map(|(_, r, c)| (r, c)).unwrap_or((row, col));
        let (r0, r1) = (row.min(other.0) as usize, row.max(other.0) as usize);
        let (c0, c1) = (col.min(other.1) as usize, col.max(other.1) as usize);
        let mut out = Vec::new();
        let row_end = r1.min(t.rows.len() - 1) + 1;
        for (r, row) in t.rows.iter().enumerate().take(row_end).skip(r0) {
            let col_end = c1.min(row.cells.len() - 1) + 1;
            for (c, cell) in row.cells.iter().enumerate().take(col_end).skip(c0) {
                if !cell.covered {
                    out.push((r, c));
                }
            }
        }
        out
    }

    // ---- pictures ----

    /// Inserts a picture (its bytes and media type), `width` by `height` pixels, shrunk to fit
    /// the text width.
    pub fn insert_image(&mut self, bytes: Vec<u8>, content_type: &str, width: f64, height: f64) {
        let max_w = self.sections.first().map(|s| emu_px(s.section.text_width())).unwrap_or(600.0);
        let k = if width > max_w && width > 0.0 { max_w / width } else { 1.0 };
        let media = self.add_media(content_type, bytes);
        let img = Image {
            media,
            width: Emu::from_pixels(width * k),
            height: Emu::from_pixels(height * k),
            name: "Picture".into(),
            description: None,
            origin: Origin::default(),
        };
        let sel = self.sel.clone();
        let e = self.replace_sel(&sel, |root, at| edit::insert_inline(root, at, Inline::Image(img)));
        self.apply_edit(e, None);
    }

    /// The selected picture and where it is.
    pub fn selected_image(&self) -> Option<(Path, usize, &Image)> {
        let (from, to) = self.sel.ordered();
        if from.path != to.path {
            return None;
        }
        let p = pos::para(self.blocks(self.story), &from.path)?;
        let (i, at) = text::inline_at(p, from.offset)?;
        match &p.content[i] {
            Inline::Image(img) if at == from.offset && at + text::inline_len(&p.content[i]) == to.offset => {
                Some((from.path.clone(), i, img))
            }
            _ => None,
        }
    }

    fn change_image(&mut self, f: &dyn Fn(&mut Image)) {
        let Some((path, i, _)) = self.selected_image() else { return };
        let root = self.blocks_mut(self.story);
        if let Some(s) = edit::change_para(root, &path, |p| {
            if let Some(Inline::Image(img)) = p.content.get_mut(i) {
                f(img);
            }
        }) {
            self.record(vec![s], None, None);
        }
    }

    pub fn set_image_size(&mut self, width_emu: i64, height_emu: i64) {
        let (w, h) = (Emu(width_emu.max(9525)), Emu(height_emu.max(9525)));
        self.change_image(&move |img| {
            img.width = w;
            img.height = h;
        });
    }

    /// Crops the selected picture: shares of its full size cut from each side, in percent, and
    /// its new size.
    pub fn crop_image(&mut self, crop: [f64; 4], width_emu: i64, height_emu: i64) {
        let (w, h) = (Emu(width_emu.max(9525)), Emu(height_emu.max(9525)));
        self.change_image(&move |img| {
            img.width = w;
            img.height = h;
            let mut r = img.origin.get().cloned().unwrap_or_else(|| Element::new(ns::W, "r"));
            // A picture put in here has no XML of its own until it's saved: its crop waits in a
            // bare `pic:blipFill`, which the writer carries into the picture it makes.
            if find_mut(&mut r, ns::PIC, "blipFill").is_none() {
                r.push(Element::new(ns::PIC, "blipFill"));
            }
            if let Some(fill) = find_mut(&mut r, ns::PIC, "blipFill") {
                let rect = fill.child_or_insert(ns::A, "srcRect");
                for (k, v) in ["l", "t", "r", "b"].iter().zip(crop) {
                    let v = (v.clamp(0.0, 100.0) * 1000.0).round() as i64;
                    if v == 0 {
                        rect.remove_attr("", k);
                    } else {
                        rect.set_attr("", k, v.to_string());
                    }
                }
                // srcRect goes before the stretch fill.
                if let Some(i) =
                    fill.children.iter().position(|n| matches!(n, office_core::xml::Node::Element(e) if e.is(ns::A, "srcRect")))
                {
                    let node = fill.children.remove(i);
                    let at = fill
                        .children
                        .iter()
                        .position(|n| matches!(n, office_core::xml::Node::Element(e) if e.is(ns::A, "stretch") || e.is(ns::A, "tile")))
                        .unwrap_or(fill.children.len());
                    fill.children.insert(at, node);
                }
                img.origin = Origin::new(r);
            }
        });
    }

    /// Back to the picture's own size in pixels (at 96 per inch), shrunk to fit the text width.
    pub fn reset_image_size(&mut self) {
        let Some((_, _, img)) = self.selected_image() else { return };
        let Ok(bytes) = self.doc.media_bytes(&img.media) else { return };
        let Some((w, h)) = image_size(&bytes) else { return };
        let max_w = self.sections.first().map(|s| emu_px(s.section.text_width())).unwrap_or(600.0);
        let k = if w as f64 > max_w { max_w / w as f64 } else { 1.0 };
        self.set_image_size(Emu::from_pixels(w as f64 * k).0, Emu::from_pixels(h as f64 * k).0);
    }

    // ---- linked Excel ranges & charts ----

    /// Inserts a Word table populated from a linked Excel range, tagged in `w:tblPr/w:tblDescription`.
    pub fn insert_linked_table(&mut self, tag: &str, rows: &[Vec<LinkedCell>]) {
        if rows.is_empty() {
            return;
        }
        let width = self
            .sections
            .iter()
            .find(|s| self.sel.focus.path.0[0] >= s.first && self.sel.focus.path.0[0] <= s.last)
            .map(|s| s.section.text_width())
            .unwrap_or(Emu::from_inches(6.27));
        let table = build_linked_table(tag, rows, width);
        let sel = self.sel.clone();
        let mut e = self
            .replace_sel(&sel, |root, at| edit::insert_blocks(root, at, vec![Block::Table(table), Block::Paragraph(Paragraph::default())]));
        if let Some(Splice { container, index, inserted, .. }) = e.splices.last() {
            if let Some(off) = inserted.iter().position(|b| matches!(b, Block::Table(_))) {
                let mut c = container.clone();
                c.push(index + off as u32 + 1);
                e.selection = Some(Selection::caret(Pos::new(Path(c), 0)));
            }
        }
        self.apply_edit(e, None);
    }

    /// Updates every Word table carrying `tag` in place; returns whether any was found.
    pub fn update_linked_table(&mut self, tag: &str, rows: &[Vec<LinkedCell>]) -> bool {
        if rows.is_empty() {
            return false;
        }
        let matches = find_all_linked_tables(self.blocks(StoryId::Body), tag);
        if matches.is_empty() {
            return false;
        }
        let mut splices = Vec::with_capacity(matches.len());
        let mut sel_after = None;
        for (path, width) in matches {
            let table = build_linked_table(tag, rows, width);
            let root = self.blocks_mut(StoryId::Body);
            splices.push(edit::replace(root, path.container(), path.block(), 1, vec![Block::Table(table)]));
            if self.sel.focus.path.0.starts_with(&path.0) {
                let mut c = path.0.clone();
                c.extend([0, 0, 0]);
                sel_after = Some(Selection::caret(Pos::new(Path(c), 0)));
            }
        }
        let prev_story = self.story;
        self.story = StoryId::Body;
        self.record(splices, sel_after, None);
        self.story = prev_story;
        true
    }

    /// Inserts a picture tagged with `tag` in its `description` (for linked Excel charts).
    pub fn insert_linked_image(&mut self, tag: &str, bytes: Vec<u8>, content_type: &str, width: f64, height: f64, name: &str) {
        let max_w = self.sections.first().map(|s| emu_px(s.section.text_width())).unwrap_or(600.0);
        let k = if width > max_w && width > 0.0 { max_w / width } else { 1.0 };
        let media = self.add_media(content_type, bytes);
        let img = Image {
            media,
            width: Emu::from_pixels(width * k),
            height: Emu::from_pixels(height * k),
            name: if name.is_empty() { "Linked Chart".into() } else { name.into() },
            description: Some(tag.into()),
            origin: Origin::default(),
        };
        let sel = self.sel.clone();
        let e = self.replace_sel(&sel, |root, at| edit::insert_inline(root, at, Inline::Image(img)));
        self.apply_edit(e, None);
    }

    /// Replaces the image bytes of every inline picture carrying `tag` in place, keeping its display size.
    pub fn update_linked_image(&mut self, tag: &str, bytes: Vec<u8>, content_type: &str) -> bool {
        let matches = find_all_linked_images(self.blocks(StoryId::Body), tag);
        if matches.is_empty() {
            return false;
        }
        let media = self.add_media(content_type, bytes);
        let mut splices = Vec::new();
        for (path, inline_index) in matches {
            let root = self.blocks_mut(StoryId::Body);
            if let Some(s) = edit::change_para(root, &path, |p| {
                if let Some(Inline::Image(img)) = p.content.get_mut(inline_index) {
                    img.media = media.clone();
                }
            }) {
                splices.push(s);
            }
        }
        if splices.is_empty() {
            return false;
        }
        let prev_story = self.story;
        self.story = StoryId::Body;
        self.record(splices, None, None);
        self.story = prev_story;
        true
    }

    /// Removes `tag` from every table or image carrying it so the objects stay in the document as normal content.
    pub fn clear_link_tag(&mut self, tag: &str) -> bool {
        let mut splices = Vec::new();
        for (path, _) in find_all_linked_tables(self.blocks(StoryId::Body), tag) {
            let Some(Block::Table(mut table)) =
                pos::blocks_at(self.blocks(StoryId::Body), path.container()).and_then(|b| b.get(path.block() as usize)).cloned()
            else {
                continue;
            };
            strip_link_tag_in_table(&mut table, tag);
            let root = self.blocks_mut(StoryId::Body);
            splices.push(edit::replace(root, path.container(), path.block(), 1, vec![Block::Table(table)]));
        }
        for (path, inline_index) in find_all_linked_images(self.blocks(StoryId::Body), tag) {
            let root = self.blocks_mut(StoryId::Body);
            if let Some(s) = edit::change_para(root, &path, |p| {
                if let Some(Inline::Image(img)) = p.content.get_mut(inline_index) {
                    img.description = None;
                }
            }) {
                splices.push(s);
            }
        }
        if splices.is_empty() {
            return false;
        }
        let prev_story = self.story;
        self.story = StoryId::Body;
        self.record(splices, None, None);
        self.story = prev_story;
        true
    }

    /// Every `writan:excel-link:*` tag currently present in the body (tables and images).
    pub fn linked_tags(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_linked_tags(self.blocks(StoryId::Body), &mut out);
        out
    }

    // ---- inline LaTeX math (real Office Math / OMML) ----

    /// The selected inline LaTeX Office Math equation (`<w:sdt>` wrapping `<m:oMath>` / `<m:oMathPara>`).
    pub fn selected_math_raw(&self) -> Option<(Path, usize, &str)> {
        let (from, to) = self.sel.ordered();
        if from.path != to.path {
            return None;
        }
        let p = pos::para(self.blocks(self.story), &from.path)?;
        let (i, at) = text::inline_at(p, from.offset)?;
        match p.content.get(i)? {
            Inline::Raw { xml } if at == from.offset && to.offset == from.offset + text::inline_len(&p.content[i]) => {
                let tag = super::resolve::inline_latex_math_tag(xml)?;
                Some((from.path, i, tag))
            }
            _ => None,
        }
    }

    /// Inserts an inline Office Math (`m:oMath` / `m:oMathPara`) equation wrapped in a `<w:sdt>`
    /// content control carrying `tag` in `<w:sdtPr><w:tag w:val="..."/>` so it opens as a native
    /// editable equation in Microsoft Word while preserving the LaTeX source for Writan.
    pub fn insert_math(&mut self, tag: &str, omml_xml: &str) -> bool {
        let Some(sdt) = build_math_sdt(tag, omml_xml) else {
            return false;
        };
        let sel = self.sel.clone();
        let e = self.replace_sel(&sel, |root, at| edit::insert_inline(root, at, Inline::Raw { xml: sdt.clone() }));
        self.apply_edit(e, None);
        true
    }

    /// Replaces the currently selected LaTeX equation (OMML `<w:sdt>` or legacy tagged image) in place.
    pub fn update_selected_math(&mut self, tag: &str, omml_xml: &str) -> bool {
        let target = if let Some((path, i, _)) = self.selected_math_raw() {
            Some((path, i))
        } else {
            self.selected_image()
                .filter(|(_, _, img)| img.description.as_deref().is_some_and(|t| t.starts_with(LATEX_MATH_PREFIX)))
                .map(|(path, i, _)| (path, i))
        };
        let Some((path, i)) = target else {
            return false;
        };
        let Some(sdt) = build_math_sdt(tag, omml_xml) else {
            return false;
        };
        let root = self.blocks_mut(self.story);
        let Some(s) = edit::change_para(root, &path, |p| {
            if let Some(slot) = p.content.get_mut(i) {
                *slot = Inline::Raw { xml: sdt.clone() };
            }
        }) else {
            return false;
        };
        self.record(vec![s], None, None);
        true
    }

    // ---- page setup ----

    /// The page setup of the caret's section.
    pub fn page_setup(&self) -> crate::sections::Section {
        let i = self.sections.iter().position(|s| self.sel.focus.path.0[0] >= s.first && self.sel.focus.path.0[0] <= s.last).unwrap_or(0);
        self.sections.get(i).map(|s| s.section.clone()).unwrap_or_default()
    }

    /// Changes every section's page setup through `f`.
    pub fn set_page_setup(&mut self, f: &dyn Fn(&mut crate::sections::Section)) {
        let places = super::section_ends(&self.doc);
        let sections = crate::sections(&self.doc);
        let mut steps = Vec::new();
        for (i, (mut s, place)) in sections.into_iter().zip(places).enumerate() {
            f(&mut s);
            let block = place.map(|b| b as u32);
            let before = self.origin_of(block);
            crate::set_section(&mut self.doc, i, &s);
            let after = self.origin_of(block);
            steps.push(WStep::Origin { block, before, after });
        }
        let before = (self.story, self.sel.clone());
        self.shared_made(&steps);
        self.history.record(steps, before.clone(), before, None);
        self.dirty = super::Dirty::Full;
        self.stale = true;
    }

    pub(crate) fn origin_of(&self, block: Option<u32>) -> Origin {
        match block {
            None => self.doc.origin.clone(),
            Some(i) => match &self.doc.body {
                Body::Flow { blocks } => match blocks.get(i as usize) {
                    Some(Block::Paragraph(p)) => p.origin.clone(),
                    _ => Origin::default(),
                },
                _ => Origin::default(),
            },
        }
    }

    // ---- headers and footers ----

    /// Starts editing the header or footer on `page`, making one if the page has none.
    pub fn edit_header_footer(&mut self, page: usize, kind: StoryKind) -> bool {
        let Some(p) = self.pages.get(page).cloned() else { return false };
        let existing = if kind == StoryKind::Header { p.header } else { p.footer };
        let story = match existing {
            Some(i) => i,
            None => match self.add_story(p.section, kind) {
                Some(i) => {
                    let name = if kind == StoryKind::Footer { "footer" } else { "header" };
                    self.shared_extra(serde_json::json!({ "story": i, "section": p.section, "kind": name }));
                    i
                }
                None => return false,
            },
        };
        self.story = StoryId::Story(story);
        self.story_page = page;
        let start = pos::story_start(self.blocks(self.story)).unwrap_or(Pos::start());
        self.sel = Selection::caret(start);
        self.history.seal();
        self.stale = true;
        true
    }

    pub fn exit_header_footer(&mut self) {
        if self.story == StoryId::Body {
            return;
        }
        self.story = StoryId::Body;
        let start = pos::story_start(self.blocks(StoryId::Body)).unwrap_or(Pos::start());
        self.sel = Selection::caret(start);
        self.history.seal();
        self.stale = true;
    }

    /// Which kind of story is being edited, if not the body.
    pub fn editing_header_footer(&self) -> Option<StoryKind> {
        match self.story {
            StoryId::Story(i) => word(&self.doc).and_then(|w| w.headers.get(i)).map(|s| s.kind),
            StoryId::Body | StoryId::Note(_) => None,
        }
    }

    /// Starts editing the footnote or endnote at a point on a page, with the caret there.
    pub fn edit_note(&mut self, host: &mut dyn Measurer, page: usize, x: f64, y: f64) -> bool {
        self.layout(host);
        let Some(note) = self.note_at(page, y) else { return false };
        self.story = StoryId::Note(note);
        self.story_page = page;
        let at = self.hit(host, page, x, y).or_else(|| pos::story_start(self.blocks(self.story))).unwrap_or(Pos::start());
        self.sel = Selection::caret(at);
        self.caret_near = None;
        self.pending = None;
        self.history.seal();
        self.stale = true;
        true
    }

    /// The footnote or endnote being edited, by its index in the layer's notes.
    pub fn editing_note(&self) -> Option<usize> {
        match self.story {
            StoryId::Note(i) => Some(i),
            _ => None,
        }
    }

    /// Empties the header or footer being edited and goes back to the body.
    pub fn remove_header_footer(&mut self) {
        let StoryId::Story(_) = self.story else { return };
        let blocks = self.blocks(self.story).len();
        let root = self.blocks_mut(self.story);
        let s = edit::replace(root, &[], 0, blocks, vec![Block::Paragraph(Paragraph::default())]);
        self.record(vec![s], Some(Selection::caret(Pos::start())), None);
        self.exit_header_footer();
    }

    /// Inserts a page number field: `PAGE`, or `PAGE_X_OF_Y` for "1 of 3".
    pub fn insert_page_field(&mut self, field: &str) {
        let simple = |instr: &str| Inline::Raw {
            xml: Element::new(ns::W, "fldSimple")
                .with_attr(ns::W, "instr", format!(" {instr} "))
                .with_child(Element::new(ns::W, "r").with_child(Element::new(ns::W, "t").with_text("1"))),
        };
        self.begin_group();
        let sel = self.sel.clone();
        let e = self.replace_sel(&sel, |root, at| edit::insert_inline(root, at, simple("PAGE")));
        self.apply_edit(e, None);
        if field == "PAGE_X_OF_Y" {
            self.insert_text(" of ");
            let sel = self.sel.clone();
            let e = self.replace_sel(&sel, |root, at| edit::insert_inline(root, at, simple("NUMPAGES")));
            self.apply_edit(e, None);
        }
        self.end_group();
    }

    /// Makes a header or footer for section `si`, referenced as its default one.
    pub(crate) fn add_story(&mut self, si: usize, kind: StoryKind) -> Option<usize> {
        let (local, rel_kind, ty) = match kind {
            StoryKind::Header => (
                "hdr",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml",
            ),
            StoryKind::Footer => (
                "ftr",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml",
            ),
        };
        let pkg = self.doc.package.as_mut()?;
        let main = pkg.main_part().ok()?;
        let base = if kind == StoryKind::Header { "word/header" } else { "word/footer" };
        let part = (1..).map(|n| format!("{base}{n}.xml")).find(|p| !pkg.has(p))?;
        let root = Element::new(ns::W, local).with_child(Element::new(ns::W, "p"));
        let mut root = root;
        root.declare(Some("w"), ns::W);
        root.declare(Some("r"), ns::R);
        pkg.put_xml(&part, &office_core::xml::XmlDoc::new(root), ty);
        let rid = pkg.relate(&main, rel_kind, &part).ok()?;
        // The section's w:sectPr references the new part as its default header or footer.
        let places = super::section_ends(&self.doc);
        let block = places.get(si).copied().flatten();
        let reference = Element::new(ns::W, if kind == StoryKind::Header { "headerReference" } else { "footerReference" })
            .with_attr(ns::W, "type", "default")
            .with_attr(ns::R, "id", rid);
        let with_ref = |sect: &mut Element| {
            let name = reference.local.clone();
            sect.children.retain(|n| !matches!(n, office_core::xml::Node::Element(e) if e.is(ns::W, &name) && e.attr_ns(ns::W, "type").unwrap_or("default") == "default"));
            sect.children.insert(0, office_core::xml::Node::Element(reference.clone()));
        };
        match block {
            None => {
                let mut o =
                    self.doc.origin.get().cloned().filter(|o| o.is(ns::W, "document")).unwrap_or_else(|| Element::new(ns::W, "document"));
                with_ref(o.child_or_insert(ns::W, "body").child_or_insert(ns::W, "sectPr"));
                self.doc.origin = Origin::new(o);
            }
            Some(i) => {
                if let Body::Flow { blocks } = &mut self.doc.body {
                    if let Some(Block::Paragraph(p)) = blocks.get_mut(i) {
                        let mut o = p.origin.get().cloned().unwrap_or_else(|| Element::new(ns::W, "p"));
                        with_ref(o.child_or_insert(ns::W, "pPr").child_or_insert(ns::W, "sectPr"));
                        p.origin = Origin::new(o);
                    }
                }
            }
        }
        let w = word_mut(&mut self.doc);
        let story = crate::Story { kind, part, blocks: vec![Block::Paragraph(Paragraph::default())] };
        w.read.0.push(story.clone());
        w.headers.push(story);
        self.dirty = super::Dirty::Full;
        self.stale = true;
        Some(w.headers.len() - 1)
    }
}

/// A table change at the caret.
#[derive(Clone, Debug, PartialEq)]
pub enum TableOp {
    InsertRow {
        below: bool,
    },
    DeleteRow,
    InsertColumn {
        right: bool,
    },
    DeleteColumn,
    /// A cell fill (`RRGGBB`), or none.
    Fill(Option<String>),
    Borders(BorderScope),
    Delete,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BorderScope {
    All,
    Outside,
    None,
}

/// The character before or after a position, across paragraphs.
fn step_char(blocks: &[Block], at: &Pos, forward: bool) -> Pos {
    let Some(p) = pos::para(blocks, &at.path) else { return at.clone() };
    let flat = text::flat(p);
    if forward {
        if (at.offset as usize) < flat.text.len() {
            return Pos::new(at.path.clone(), para::next_boundary(&flat.text, at.offset));
        }
        pos::next_para(blocks, &at.path).map(|n| Pos::new(n, 0)).unwrap_or_else(|| at.clone())
    } else {
        if at.offset > 0 {
            return Pos::new(at.path.clone(), para::prev_boundary(&flat.text, at.offset));
        }
        pos::prev_para(blocks, &at.path)
            .map(|n| {
                let len = pos::para(blocks, &n).map(text::len).unwrap_or(0);
                Pos::new(n, len)
            })
            .unwrap_or_else(|| at.clone())
    }
}

/// The index of a body paragraph among all body paragraphs in reading order (what the layer's
/// list functions number them by).
fn body_index(blocks: &[Block], path: &Path) -> Option<usize> {
    let mut at = pos::story_start(blocks).map(|p| p.path);
    let mut i = 0;
    while let Some(p) = at {
        if &p == path {
            return Some(i);
        }
        i += 1;
        at = pos::next_para(blocks, &p);
    }
    None
}

fn find_mut<'a>(el: &'a mut Element, ns_: &str, local: &str) -> Option<&'a mut Element> {
    if el.is(ns_, local) {
        return Some(el);
    }
    for c in el.elements_mut() {
        if let Some(f) = find_mut(c, ns_, local) {
            return Some(f);
        }
    }
    None
}

/// The children of `w:tcPr`, in schema order.
const TC_PR: &[&[&str]] = &[
    &["cnfStyle"],
    &["tcW"],
    &["gridSpan"],
    &["hMerge"],
    &["vMerge"],
    &["tcBorders"],
    &["shd"],
    &["noWrap"],
    &["tcMar"],
    &["textDirection"],
    &["tcFitText"],
    &["vAlign"],
    &["hideMark"],
];

fn set_cell_borders(cell: &mut Cell, sides: &[(&str, bool)]) {
    let mut tc = cell.origin.get().cloned().unwrap_or_else(|| Element::new(ns::W, "tc"));
    if tc.child(ns::W, "tcPr").is_none() {
        tc.children.insert(0, office_core::xml::Node::Element(Element::new(ns::W, "tcPr")));
    }
    let pr = tc.child_mut(ns::W, "tcPr").expect("tcPr was just made");
    if pr.child(ns::W, "tcBorders").is_none() {
        pr.place(Element::new(ns::W, "tcBorders"), TC_PR);
    }
    let borders = pr.child_mut(ns::W, "tcBorders").expect("tcBorders was just made");
    for (side, on) in sides {
        borders.remove_children(ns::W, side);
        let el = if *on {
            Element::new(ns::W, side)
                .with_attr(ns::W, "val", "single")
                .with_attr(ns::W, "sz", "4")
                .with_attr(ns::W, "space", "0")
                .with_attr(ns::W, "color", "000000")
        } else {
            Element::new(ns::W, side).with_attr(ns::W, "val", "nil")
        };
        borders
            .place(el, &[&["top"], &["start", "left"], &["bottom"], &["end", "right"], &["insideH"], &["insideV"], &["tl2br"], &["tr2bl"]]);
    }
    cell.origin = Origin::new(tc);
}

/// A PNG's, JPEG's or GIF's size in pixels, from its header.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() > 24 && &b[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some((u32::from_be_bytes(b[16..20].try_into().ok()?), u32::from_be_bytes(b[20..24].try_into().ok()?)));
    }
    if b.len() > 10 && &b[..3] == b"GIF" {
        return Some((u16::from_le_bytes([b[6], b[7]]) as u32, u16::from_le_bytes([b[8], b[9]]) as u32));
    }
    if b.len() > 4 && b[0] == 0xFF && b[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

/// Prefix stored in `w:tblDescription` (tables) or `docPr/@descr` (images) for linked Excel objects.
pub const EXCEL_LINK_PREFIX: &str = "writan:excel-link:";

/// Prefix stored in `docPr/@descr` (images) for editable inline LaTeX math equations.
pub const LATEX_MATH_PREFIX: &str = "writan:latex:";

/// One cell of a linked Excel range, deserialized from `SheetsDoc::rangeSnapshot`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedCell {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub align: Option<String>,
}

pub(crate) fn table_link_tag(table: &Table) -> Option<String> {
    table
        .origin
        .get()
        .and_then(|o| o.path(ns::W, &["tblPr", "tblDescription"]))
        .and_then(|e| e.attr_ns(ns::W, "val").or_else(|| e.attr("val")))
        .map(str::to_string)
}

fn parse_align(a: Option<&str>) -> Option<Align> {
    match a {
        Some("center") => Some(Align::Center),
        Some("right") => Some(Align::Right),
        Some("left") => Some(Align::Left),
        _ => None,
    }
}

fn build_linked_table(tag: &str, rows: &[Vec<LinkedCell>], width: Emu) -> Table {
    let row_count = rows.len().clamp(1, 200);
    let col_count = rows.iter().map(Vec::len).max().unwrap_or(1).clamp(1, 63);
    let col = Emu(width.0 / col_count as i64);
    let border = |side: &str| {
        Element::new(ns::W, side).with_attr(ns::W, "val", "single").with_attr(ns::W, "sz", "4").with_attr(ns::W, "space", "0").with_attr(
            ns::W,
            "color",
            "D9D9D9",
        )
    };
    let mut borders = Element::new(ns::W, "tblBorders");
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        borders.push(border(side));
    }
    let tbl_pr = Element::new(ns::W, "tblPr")
        .with_child(Element::new(ns::W, "tblW").with_attr(ns::W, "w", "0").with_attr(ns::W, "type", "auto"))
        .with_child(borders)
        .with_child(Element::new(ns::W, "tblLook").with_attr(ns::W, "val", "04A0"))
        .with_child(Element::new(ns::W, "tblDescription").with_attr(ns::W, "val", tag));
    let built_rows = rows
        .iter()
        .take(row_count)
        .enumerate()
        .map(|(ri, r)| Row {
            header: ri == 0 && row_count > 1,
            cells: (0..col_count)
                .map(|ci| {
                    let c = r.get(ci);
                    let text = c.map(|x| x.text.as_str()).unwrap_or("");
                    let props = RunProps {
                        bold: c.is_some_and(|x| x.bold).then_some(true),
                        italic: c.is_some_and(|x| x.italic).then_some(true),
                        color: c.and_then(|x| x.color.as_deref()).and_then(Rgb::parse).map(Color::rgb),
                        ..Default::default()
                    };
                    let p = Paragraph {
                        props: ParaProps { align: parse_align(c.and_then(|x| x.align.as_deref())), ..Default::default() },
                        content: if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![Inline::Text(Run { text: text.into(), props, ..Default::default() })]
                        },
                        ..Default::default()
                    };
                    let fill = c.and_then(|x| x.fill.as_deref()).and_then(Rgb::parse).map(Color::rgb);
                    Cell { blocks: vec![Block::Paragraph(p)], fill, ..Default::default() }
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    Table {
        columns: vec![col; col_count],
        rows: built_rows,
        style: None,
        origin: Origin::new(Element::new(ns::W, "tbl").with_child(tbl_pr)),
    }
}

fn strip_link_tag_in_table(table: &mut Table, tag: &str) {
    if table_link_tag(table).as_deref() == Some(tag) {
        if let Some(mut tbl_el) = table.origin.get().cloned() {
            if let Some(pr) = tbl_el.child_mut(ns::W, "tblPr") {
                pr.children.retain(|n| !matches!(n, office_core::xml::Node::Element(e) if e.is(ns::W, "tblDescription")));
            }
            table.origin = Origin::new(tbl_el);
        }
    }
    for row in &mut table.rows {
        for cell in &mut row.cells {
            for b in &mut cell.blocks {
                match b {
                    Block::Table(nested) => strip_link_tag_in_table(nested, tag),
                    Block::Paragraph(p) => {
                        for inline in &mut p.content {
                            if let Inline::Image(img) = inline {
                                if img.description.as_deref() == Some(tag) {
                                    img.description = None;
                                }
                            }
                        }
                    }
                    Block::Raw { .. } => {}
                }
            }
        }
    }
}

fn find_all_linked_tables(blocks: &[Block], tag: &str) -> Vec<(Path, Emu)> {
    fn walk(blocks: &[Block], prefix: &mut Vec<u32>, tag: &str, out: &mut Vec<(Path, Emu)>) {
        for (i, b) in blocks.iter().enumerate() {
            if let Block::Table(t) = b {
                if table_link_tag(t).as_deref() == Some(tag) {
                    let mut p = prefix.clone();
                    p.push(i as u32);
                    let total: i64 = t.columns.iter().map(|c| c.0).sum();
                    let width = if total > 0 { Emu(total) } else { Emu::from_inches(6.27) };
                    out.push((Path(p), width));
                    continue;
                }
                for (ri, row) in t.rows.iter().enumerate() {
                    for (ci, cell) in row.cells.iter().enumerate() {
                        prefix.extend([i as u32, ri as u32, ci as u32]);
                        walk(&cell.blocks, prefix, tag, out);
                        prefix.truncate(prefix.len() - 3);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut Vec::new(), tag, &mut out);
    out
}

fn find_all_linked_images(blocks: &[Block], tag: &str) -> Vec<(Path, usize)> {
    fn walk(blocks: &[Block], prefix: &mut Vec<u32>, tag: &str, out: &mut Vec<(Path, usize)>) {
        for (i, b) in blocks.iter().enumerate() {
            match b {
                Block::Paragraph(p) => {
                    for (ii, inline) in p.content.iter().enumerate() {
                        if let Inline::Image(img) = inline {
                            if img.description.as_deref() == Some(tag) {
                                let mut path = prefix.clone();
                                path.push(i as u32);
                                out.push((Path(path), ii));
                            }
                        }
                    }
                }
                Block::Table(t) => {
                    for (ri, row) in t.rows.iter().enumerate() {
                        for (ci, cell) in row.cells.iter().enumerate() {
                            prefix.extend([i as u32, ri as u32, ci as u32]);
                            walk(&cell.blocks, prefix, tag, out);
                            prefix.truncate(prefix.len() - 3);
                        }
                    }
                }
                Block::Raw { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut Vec::new(), tag, &mut out);
    out
}

fn collect_linked_tags(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        match b {
            Block::Paragraph(p) => {
                for inline in &p.content {
                    if let Inline::Image(img) = inline {
                        if let Some(tag) = img.description.as_deref().filter(|t| t.starts_with(EXCEL_LINK_PREFIX)) {
                            out.push(tag.to_string());
                        }
                    }
                }
            }
            Block::Table(t) => {
                if let Some(tag) = table_link_tag(t).filter(|t| t.starts_with(EXCEL_LINK_PREFIX)) {
                    out.push(tag);
                }
                for row in &t.rows {
                    for cell in &row.cells {
                        collect_linked_tags(&cell.blocks, out);
                    }
                }
            }
            Block::Raw { .. } => {}
        }
    }
}

/// Builds a `<w:sdt>` inline content control wrapping parsed Office Math (`<m:oMath>` or `<m:oMathPara>`),
/// storing the Writan LaTeX tag in `<w:sdtPr><w:tag w:val="..."/>`.
fn build_math_sdt(tag: &str, omml_xml: &str) -> Option<Element> {
    let omml = office_core::xml::parse_element(omml_xml).ok()?;
    if !super::math::is_math(&omml) {
        return None;
    }
    let tag_el = Element::new(ns::W, "tag").with_attr(ns::W, "val", tag);
    let sdt_pr = Element::new(ns::W, "sdtPr").with_child(tag_el);
    let sdt_content = Element::new(ns::W, "sdtContent").with_child(omml);
    Some(Element::new(ns::W, "sdt").with_child(sdt_pr).with_child(sdt_content))
}
