//! Editing presentations: picking and arranging frames, formatting them, typing in them, slides
//! and their notes and transitions, and undo.

use std::collections::{BTreeMap, BTreeSet};

use office_core::drawing::{Color, Fill, Line, Rgb, Xfrm};
use office_core::model::{Block, Cell, Frame, FrameContent, Image, Origin, Paragraph, Row, RunProps, Slide, Strike, Table, TextBody};
use office_core::units::{Angle, Emu, FontSize};
use office_core::xml::{ns, Element};
use office_engine::display::{self, Rect};
use office_engine::edit::{self, Edit};
use office_engine::flow::{self, FlowLayout};
use office_engine::measure::Measurer;
use office_engine::para::{self, Ctx, LineEnd};
use office_engine::pos::{self, Path, Pos, Selection};
use office_engine::text;
use serde::{Deserialize, Serialize};

use super::draw::{Boxed, Drawer, Map};
use super::{find_frame, find_frame_mut, free_id, from_blocks, text_blocks, Editing, SStep, Sel, SlidesView};

fn px_emu(v: f64) -> Emu {
    Emu((v * 9525.0).round() as i64)
}

/// Where the caret and selection show while text is edited: in the frame's own coordinates,
/// with the frame's box (which may be rotated).
#[derive(Clone, Debug, Default, Serialize)]
pub struct TextOverlay {
    pub frame: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret: Option<(f64, f64, f64)>,
    pub rects: Vec<Rect>,
}

/// Where ranges of a frame's text are: the frame's box, and rectangles inside it tagged with
/// the index of the range they underline (see `SlidesView::text_ranges`).
#[derive(Clone, Debug, Serialize)]
pub struct RangeBoxes {
    pub frame: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot: f64,
    pub rects: Vec<(usize, f64, f64, f64, f64)>,
}

/// A change to a frame's text formatting, from the inspector.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPatch {
    pub font_family: Option<String>,
    /// Points.
    pub font_size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    /// `#rrggbb` or `rrggbb`.
    pub color: Option<String>,
    pub align: Option<String>,
}

/// Frame media ids belong to the source deck. Carry the referenced bytes so paste can
/// allocate ids in a different deck, including when that deck already uses the same ids.
#[derive(Serialize, Deserialize)]
struct FrameClipboard {
    frames: Vec<Frame>,
    #[serde(default)]
    media: BTreeMap<String, ClipboardMedia>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardMedia {
    content_type: String,
    bytes: Vec<u8>,
}

impl TextPatch {
    fn apply(&self, r: &mut RunProps) {
        if let Some(f) = &self.font_family {
            r.font = Some(f.clone());
        }
        if let Some(s) = self.font_size {
            r.size = Some(FontSize::from_points(s.clamp(1.0, 4000.0)));
        }
        if let Some(b) = self.bold {
            r.bold = Some(b);
        }
        if let Some(i) = self.italic {
            r.italic = Some(i);
        }
        if let Some(u) = self.underline {
            r.underline = Some(if u { "single".into() } else { "none".into() });
        }
        if let Some(s) = self.strikethrough {
            r.strike = Some(if s { Strike::Single } else { Strike::None });
        }
        if let Some(c) = &self.color {
            r.color = Rgb::parse(c).map(Color::rgb);
        }
    }

    fn align(&self) -> Option<office_core::model::Align> {
        Some(match self.align.as_deref()? {
            "center" => office_core::model::Align::Center,
            "right" => office_core::model::Align::Right,
            "justify" => office_core::model::Align::Justify,
            _ => office_core::model::Align::Left,
        })
    }
}

impl SlidesView {
    // ---- recording changes ----

    fn record(&mut self, steps: Vec<SStep>, before: Sel, kind: Option<&'static str>) {
        let after = self.sel.clone();
        self.history.record(steps, before, after, kind);
        self.goal_x = None;
    }

    /// Changes slide `index` through `f`, as one undo step.
    pub(crate) fn change_slide(&mut self, index: usize, kind: Option<&'static str>, f: impl FnOnce(&mut Slide)) {
        let Some(before) = self.slides().get(index).cloned() else { return };
        let sel = self.sel.clone();
        let mut after = before.clone();
        f(&mut after);
        if after == before && after.origin.id() == before.origin.id() {
            return;
        }
        self.slides_mut()[index] = after.clone();
        self.touch(index);
        self.record(vec![SStep::Slide { index, before: Box::new(before), after: Box::new(after) }], sel, kind);
    }

    /// Changes the list of slides, as one undo step.
    pub(crate) fn change_deck(&mut self, f: impl FnOnce(&mut Vec<Slide>)) {
        let before = self.slides().to_vec();
        let sel = self.sel.clone();
        f(self.slides_mut());
        let after = self.slides().to_vec();
        self.touch_all();
        let n = after.len();
        self.sel.active = self.sel.active.min(n.saturating_sub(1));
        self.record(vec![SStep::Deck { before, after }], sel, None);
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.history.undo() else { return false };
        for step in e.steps.iter().rev() {
            self.apply(step, true);
        }
        self.sel = e.before;
        self.clamp_sel();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.history.redo() else { return false };
        for step in &e.steps {
            self.apply(step, false);
        }
        self.sel = e.after;
        self.clamp_sel();
        true
    }

    fn apply(&mut self, step: &SStep, undo: bool) {
        match step {
            SStep::Slide { index, before, after } => {
                if let Some(s) = self.slides_mut().get_mut(*index) {
                    *s = if undo { (**before).clone() } else { (**after).clone() };
                }
                self.touch(*index);
            }
            SStep::Deck { before, after } => {
                *self.slides_mut() = if undo { before.clone() } else { after.clone() };
                self.touch_all();
            }
            SStep::Notes { index, before, after } => {
                crate::set_notes(&mut self.doc, *index, if undo { before.clone() } else { after.clone() });
                self.touch(*index);
            }
        }
    }

    fn clamp_sel(&mut self) {
        let n = self.slides().len();
        self.sel.active = self.sel.active.min(n.saturating_sub(1));
        let frames: Vec<u32> = self.slides().get(self.sel.active).map(|s| s.frames.iter().map(|f| f.id).collect()).unwrap_or_default();
        self.sel.frames.retain(|id| frames.contains(id));
        if self.sel.editing.as_ref().is_some_and(|e| !frames.contains(&e.frame)) {
            self.sel.editing = None;
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn begin_group(&mut self) {
        self.history.begin_group(self.sel.clone());
    }

    pub fn end_group(&mut self) {
        self.history.end_group();
    }

    pub fn selection(&self) -> &Sel {
        &self.sel
    }

    // ---- slides ----

    pub fn set_active(&mut self, index: usize) {
        if index < self.slides().len() && index != self.sel.active {
            self.sel = Sel { active: index, frames: Vec::new(), editing: None };
            self.history.seal();
        }
    }

    /// Adds a slide after `after` on layout `layout` (a layout's part), or on the layout of the
    /// slide it follows. Becomes the active slide.
    pub fn add_slide(&mut self, after: usize, layout: Option<&str>) {
        let layout = layout
            .map(str::to_string)
            .or_else(|| self.slides().get(after).and_then(|s| s.layout.clone()))
            .or_else(|| crate::layouts(&self.doc).first().map(|t| t.part.clone()));
        let before = self.slides().to_vec();
        let sel = self.sel.clone();
        let added = match &layout {
            Some(l) => crate::add_slide(&mut self.doc, l),
            None => {
                self.slides_mut().push(Slide::default());
                Some(self.slides().len() - 1)
            }
        };
        let Some(i) = added else { return };
        let to = (after + 1).min(self.slides().len() - 1);
        let s = self.slides_mut().remove(i);
        self.slides_mut().insert(to, s);
        self.touch_all();
        self.sel = Sel { active: to, frames: Vec::new(), editing: None };
        let after_slides = self.slides().to_vec();
        self.record(vec![SStep::Deck { before, after: after_slides }], sel, None);
    }

    pub fn duplicate_slides(&mut self, indexes: &[usize]) {
        let mut idx: Vec<usize> = indexes.to_vec();
        idx.sort_unstable();
        idx.dedup();
        let last = idx.last().copied().unwrap_or(0);
        self.change_deck(|slides| {
            let copies: Vec<Slide> = idx
                .iter()
                .filter_map(|&i| slides.get(i).cloned())
                .map(|mut s| {
                    // A copy is a new slide: its own part when saved, written from the model.
                    s.part = None;
                    s
                })
                .collect();
            let at = (last + 1).min(slides.len());
            for (k, s) in copies.into_iter().enumerate() {
                slides.insert(at + k, s);
            }
        });
        self.sel.active = last + 1;
        self.sel.frames.clear();
    }

    /// Deletes slides; a deck keeps at least one (deleting them all leaves a blank one).
    pub fn delete_slides(&mut self, indexes: &[usize]) {
        let mut idx: Vec<usize> = indexes.to_vec();
        idx.sort_unstable();
        idx.dedup();
        let blank_layout = crate::layout(&self.doc, "blank").map(|t| t.part.clone());
        self.change_deck(|slides| {
            for &i in idx.iter().rev() {
                if i < slides.len() {
                    slides.remove(i);
                }
            }
            if slides.is_empty() {
                slides.push(Slide { layout: blank_layout, ..Default::default() });
            }
        });
        self.sel = Sel { active: idx.first().copied().unwrap_or(0).min(self.slides().len() - 1), frames: Vec::new(), editing: None };
    }

    pub fn move_slide(&mut self, from: usize, to: usize) {
        let n = self.slides().len();
        if from >= n || to >= n || from == to {
            return;
        }
        self.change_deck(|slides| {
            let s = slides.remove(from);
            slides.insert(to, s);
        });
        self.sel.active = to;
    }

    pub fn toggle_hidden(&mut self, indexes: &[usize]) {
        let all_hidden = indexes.iter().all(|&i| self.slides().get(i).is_some_and(|s| s.hidden));
        self.change_deck(|slides| {
            for &i in indexes {
                if let Some(s) = slides.get_mut(i) {
                    s.hidden = !all_hidden;
                }
            }
        });
    }

    /// Puts the active slide on another layout.
    pub fn apply_layout(&mut self, layout: &str) {
        let active = self.sel.active;
        let l = layout.to_string();
        self.change_slide(active, None, |s| s.layout = Some(l));
    }

    pub fn set_notes(&mut self, index: usize, text_: &str) {
        let before = crate::notes(&self.doc, index);
        let after: Vec<Paragraph> = text_.split('\n').map(Paragraph::text).collect();
        if before.iter().map(|p| p.plain_text()).collect::<Vec<_>>() == after.iter().map(|p| p.plain_text()).collect::<Vec<_>>() {
            return;
        }
        let sel = self.sel.clone();
        crate::set_notes(&mut self.doc, index, after.clone());
        self.record(vec![SStep::Notes { index, before, after }], sel, Some("notes"));
    }

    pub fn set_transition(&mut self, indexes: &[usize], t: Option<crate::Transition>) {
        let before_sel = self.sel.clone();
        let mut steps = Vec::new();
        for &index in indexes {
            let Some(before) = self.slides().get(index).cloned() else { continue };
            let mut after = before.clone();
            crate::set_transition(&mut after, t.as_ref());
            self.slides_mut()[index] = after.clone();
            self.touch(index);
            steps.push(SStep::Slide { index, before: Box::new(before), after: Box::new(after) });
        }
        if !steps.is_empty() {
            self.record(steps, before_sel, None);
        }
    }

    // ---- picking frames ----

    fn drawer_boxes(&mut self, host: &mut dyn Measurer, index: usize) -> Vec<(u32, Boxed)> {
        let Some(slide) = self.slides().get(index).cloned() else { return Vec::new() };
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let d = Drawer { doc: &self.doc, art: &self.art, tables: &self.tables, ctx: &mut ctx, cache: &mut self.cache };
        slide.frames.iter().filter(|f| !f.hidden).map(|f| (f.id, d.boxed(&slide, f, Map::IDENTITY))).collect()
    }

    /// The topmost frame at a point on the active slide.
    pub fn frame_at(&mut self, host: &mut dyn Measurer, x: f64, y: f64) -> Option<u32> {
        let active = self.sel.active;
        self.drawer_boxes(host, active).into_iter().rev().find(|(_, b)| b.contains(x, y, 3.0)).map(|(id, _)| id)
    }

    /// A press on the slide: picks the frame under it (adding to or taking from the selection
    /// with `additive`), or nothing. Answers the frame picked.
    pub fn press(&mut self, host: &mut dyn Measurer, x: f64, y: f64, additive: bool) -> Option<u32> {
        let hit = self.frame_at(host, x, y);
        if let Some(e) = &self.sel.editing {
            if Some(e.frame) == hit {
                return hit;
            }
        }
        self.sel.editing = None;
        match (hit, additive) {
            (Some(id), true) => {
                if let Some(i) = self.sel.frames.iter().position(|f| *f == id) {
                    self.sel.frames.remove(i);
                } else {
                    self.sel.frames.push(id);
                }
            }
            (Some(id), false) => {
                if !self.sel.frames.contains(&id) {
                    self.sel.frames = vec![id];
                }
            }
            (None, false) => self.sel.frames.clear(),
            (None, true) => {}
        }
        self.history.seal();
        hit
    }

    pub fn select_frames(&mut self, ids: Vec<u32>) {
        self.sel.frames = ids;
        self.sel.editing = None;
        self.history.seal();
    }

    /// Picks every frame inside a rectangle (a marquee drag).
    pub fn select_in(&mut self, host: &mut dyn Measurer, x0: f64, y0: f64, x1: f64, y1: f64, additive: bool) {
        let (l, t, r, b) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
        let active = self.sel.active;
        let inside: Vec<u32> = self
            .drawer_boxes(host, active)
            .into_iter()
            .filter(|(_, bx)| bx.x >= l && bx.y >= t && bx.x + bx.w <= r && bx.y + bx.h <= b)
            .map(|(id, _)| id)
            .collect();
        if additive {
            for id in inside {
                if !self.sel.frames.contains(&id) {
                    self.sel.frames.push(id);
                }
            }
        } else {
            self.sel.frames = inside;
        }
        self.sel.editing = None;
    }

    /// The selected frames' boxes, for drawing their handles.
    pub fn selected_boxes(&mut self, host: &mut dyn Measurer) -> Vec<(u32, Boxed)> {
        let active = self.sel.active;
        let ids = self.sel.frames.clone();
        self.drawer_boxes(host, active).into_iter().filter(|(id, _)| ids.contains(id)).collect()
    }

    // ---- arranging ----

    /// Frames on the active slide get their resolved positions written down, so moving a
    /// placeholder that inherits its place from the layout moves it from where it shows.
    fn with_frames(&mut self, host: &mut dyn Measurer, ids: &[u32], kind: Option<&'static str>, f: impl Fn(&mut Frame, Boxed)) {
        let active = self.sel.active;
        let boxes = self.drawer_boxes(host, active);
        self.change_slide(active, kind, |s| {
            for fr in &mut s.frames {
                if !ids.contains(&fr.id) {
                    continue;
                }
                let Some((_, b)) = boxes.iter().find(|(id, _)| *id == fr.id) else { continue };
                f(fr, *b);
            }
        });
    }

    /// Moves frames by (`dx`, `dy`) pixels.
    pub fn move_frames(&mut self, host: &mut dyn Measurer, ids: &[u32], dx: f64, dy: f64) {
        self.with_frames(host, ids, None, |fr, b| {
            let mut x = fr.shape.xfrm.clone().unwrap_or_else(|| xfrm_of(&b));
            x.x = px_emu(b.x + dx);
            x.y = px_emu(b.y + dy);
            if fr.shape.xfrm.is_none() {
                x.width = px_emu(b.w);
                x.height = px_emu(b.h);
            }
            fr.shape.xfrm = Some(x);
        });
    }

    /// Sets a frame's box (pixels) and rotation (degrees); its flips stay. A group's children
    /// move with it.
    pub fn set_box(&mut self, host: &mut dyn Measurer, id: u32, to: Boxed) {
        let Boxed { x, y, w, h, rot, .. } = to;
        self.with_frames(host, &[id], None, |fr, b| {
            let mut xf = fr.shape.xfrm.clone().unwrap_or_else(|| xfrm_of(&b));
            xf.x = px_emu(x);
            xf.y = px_emu(y);
            xf.width = px_emu(w.max(1.0));
            xf.height = px_emu(h.max(1.0));
            xf.rotation = Angle::from_degrees(((rot % 360.0) + 360.0) % 360.0);
            if let FrameContent::Group { .. } = fr.content {
                // The children's space stays; the group stretches it over the new box.
                if xf.child.is_none() {
                    xf.child = Some((px_emu(b.x), px_emu(b.y), px_emu(b.w), px_emu(b.h)));
                }
            }
            if let FrameContent::Table { table } = &mut fr.content {
                // A table stretches its columns to the new width and its rows to the new height.
                let tw: i64 = table.columns.iter().map(|c| c.0).sum();
                if tw > 0 {
                    let k = xf.width.0 as f64 / tw as f64;
                    table.columns.iter_mut().for_each(|c| *c = Emu((c.0 as f64 * k).round() as i64));
                }
                let th: i64 = table.rows.iter().filter_map(|r| r.height.map(|h| h.0)).sum();
                if th > 0 {
                    let k = xf.height.0 as f64 / th as f64;
                    table.rows.iter_mut().for_each(|r| r.height = r.height.map(|h| Emu((h.0 as f64 * k).round() as i64)));
                }
            }
            if let FrameContent::Picture { image } = &mut fr.content {
                image.width = xf.width;
                image.height = xf.height;
            }
            fr.shape.xfrm = Some(xf);
        });
    }

    pub fn flip(&mut self, host: &mut dyn Measurer, horizontal: bool) {
        let ids = self.sel.frames.clone();
        self.with_frames(host, &ids, None, |fr, b| {
            let mut x = fr.shape.xfrm.clone().unwrap_or_else(|| xfrm_of(&b));
            if horizontal {
                x.flip_h = !x.flip_h;
            } else {
                x.flip_v = !x.flip_v;
            }
            fr.shape.xfrm = Some(x);
        });
    }

    pub fn delete_frames(&mut self) {
        let ids = self.sel.frames.clone();
        if ids.is_empty() {
            return;
        }
        let active = self.sel.active;
        self.sel.frames.clear();
        self.sel.editing = None;
        self.change_slide(active, None, |s| s.frames.retain(|f| !ids.contains(&f.id)));
    }

    /// Copies of the selected frames, a little down and to the right, picked instead.
    pub fn duplicate_frames(&mut self) {
        let ids = self.sel.frames.clone();
        let active = self.sel.active;
        let mut new_ids = Vec::new();
        self.change_slide(active, None, |s| {
            let mut next = free_id(&s.frames);
            let copies: Vec<Frame> = s
                .frames
                .iter()
                .filter(|f| ids.contains(&f.id))
                .cloned()
                .map(|mut f| {
                    renumber(&mut f, &mut next);
                    if let Some(x) = &mut f.shape.xfrm {
                        x.x = Emu(x.x.0 + 182_880);
                        x.y = Emu(x.y.0 + 182_880);
                    }
                    f.placeholder = None;
                    new_ids.push(f.id);
                    f
                })
                .collect();
            s.frames.extend(copies);
        });
        self.sel.frames = new_ids;
    }

    /// The selected frames, for the clipboard (JSON).
    pub fn copy_frames(&self) -> String {
        let Some(s) = self.slides().get(self.sel.active) else { return "[]".into() };
        let mut frames: Vec<Frame> = s.frames.iter().filter(|f| self.sel.frames.contains(&f.id)).cloned().collect();
        let mut ids = BTreeSet::new();
        for f in &mut frames {
            for_each_picture(f, &mut |image| {
                ids.insert(image.media.clone());
            });
        }
        let media: Option<BTreeMap<_, _>> = ids
            .into_iter()
            .map(|id| {
                let bytes = self.media(&id)?;
                let content_type = self.media_type(&id)?;
                Some((id, ClipboardMedia { content_type, bytes }))
            })
            .collect();
        let Some(media) = media else { return "[]".into() };
        serde_json::to_string(&FrameClipboard { frames, media }).unwrap_or_else(|_| "[]".into())
    }

    /// Frames from the clipboard, put on the active slide and picked.
    pub fn paste_frames(&mut self, json: &str) -> bool {
        let Ok(clipboard) = serde_json::from_str::<FrameClipboard>(json).or_else(|_| {
            // Older clipboards can still paste shapes and text. Pictures need carried media:
            // an id present in this deck may refer to an unrelated picture.
            serde_json::from_str::<Vec<Frame>>(json).map(|frames| FrameClipboard { frames, media: BTreeMap::new() })
        }) else {
            return false;
        };
        let FrameClipboard { mut frames, mut media } = clipboard;
        let active = self.sel.active;
        if frames.is_empty() || self.slides().get(active).is_none() {
            return false;
        }
        let mut ids = BTreeSet::new();
        for f in &mut frames {
            for_each_picture(f, &mut |image| {
                ids.insert(image.media.clone());
            });
        }
        // Validate the whole payload before adding media or changing the slide.
        if ids.iter().any(|id| !media.contains_key(id)) {
            return false;
        }
        let remap: BTreeMap<_, _> = ids
            .into_iter()
            .map(|id| {
                let picture = media.remove(&id).unwrap();
                let imported = self.doc.add_media(&picture.content_type, picture.bytes);
                (id, imported)
            })
            .collect();
        for f in &mut frames {
            for_each_picture(f, &mut |image| image.media = remap[&image.media].clone());
        }
        let mut new_ids = Vec::new();
        self.change_slide(active, None, |s| {
            let mut next = free_id(&s.frames);
            for mut f in frames {
                renumber(&mut f, &mut next);
                f.placeholder = None;
                new_ids.push(f.id);
                s.frames.push(f);
            }
        });
        self.sel.frames = new_ids;
        self.sel.editing = None;
        true
    }

    /// Restacks the selected frames: `front`, `forward`, `backward` or `back`.
    pub fn restack(&mut self, how: &str) {
        let ids = self.sel.frames.clone();
        let active = self.sel.active;
        let how = how.to_string();
        self.change_slide(active, None, |s| {
            let picked: Vec<Frame> = s.frames.iter().filter(|f| ids.contains(&f.id)).cloned().collect();
            let rest: Vec<Frame> = s.frames.iter().filter(|f| !ids.contains(&f.id)).cloned().collect();
            match how.as_str() {
                "front" => s.frames = [rest, picked].concat(),
                "back" => s.frames = [picked, rest].concat(),
                "forward" => {
                    for i in (0..s.frames.len().saturating_sub(1)).rev() {
                        if ids.contains(&s.frames[i].id) && !ids.contains(&s.frames[i + 1].id) {
                            s.frames.swap(i, i + 1);
                        }
                    }
                }
                _ => {
                    for i in 1..s.frames.len() {
                        if ids.contains(&s.frames[i].id) && !ids.contains(&s.frames[i - 1].id) {
                            s.frames.swap(i, i - 1);
                        }
                    }
                }
            }
        });
    }

    /// Whether a restack would change anything.
    pub fn can_restack(&self, how: &str) -> bool {
        let Some(s) = self.slides().get(self.sel.active) else { return false };
        let ids = &self.sel.frames;
        if ids.is_empty() {
            return false;
        }
        let picked: Vec<bool> = s.frames.iter().map(|f| ids.contains(&f.id)).collect();
        match how {
            "front" | "forward" => picked.iter().rposition(|p| !p).is_some_and(|last_unpicked| picked[..last_unpicked].iter().any(|p| *p)),
            _ => picked.iter().position(|p| !p).is_some_and(|first_unpicked| picked[first_unpicked..].iter().any(|p| *p)),
        }
    }

    /// Lines up the selected frames: `left`, `center`, `right`, `top`, `middle` or `bottom`.
    pub fn align_frames(&mut self, host: &mut dyn Measurer, how: &str) {
        let boxes = self.selected_boxes(host);
        if boxes.len() < 2 {
            return;
        }
        let l = boxes.iter().map(|(_, b)| b.x).fold(f64::MAX, f64::min);
        let t = boxes.iter().map(|(_, b)| b.y).fold(f64::MAX, f64::min);
        let r = boxes.iter().map(|(_, b)| b.x + b.w).fold(f64::MIN, f64::max);
        let bt = boxes.iter().map(|(_, b)| b.y + b.h).fold(f64::MIN, f64::max);
        let how = how.to_string();
        let ids: Vec<u32> = boxes.iter().map(|(id, _)| *id).collect();
        self.with_frames(host, &ids, None, |fr, b| {
            let (nx, ny) = match how.as_str() {
                "left" => (l, b.y),
                "center" => ((l + r) / 2.0 - b.w / 2.0, b.y),
                "right" => (r - b.w, b.y),
                "top" => (b.x, t),
                "middle" => (b.x, (t + bt) / 2.0 - b.h / 2.0),
                _ => (b.x, bt - b.h),
            };
            let mut x = fr.shape.xfrm.clone().unwrap_or_else(|| xfrm_of(&b));
            x.x = px_emu(nx);
            x.y = px_emu(ny);
            fr.shape.xfrm = Some(x);
        });
    }

    /// Spaces three or more selected frames evenly across (`horizontal`) or down.
    pub fn distribute(&mut self, host: &mut dyn Measurer, horizontal: bool) {
        let mut boxes = self.selected_boxes(host);
        if boxes.len() < 3 {
            return;
        }
        boxes.sort_by(|a, b| if horizontal { a.1.x.total_cmp(&b.1.x) } else { a.1.y.total_cmp(&b.1.y) });
        let first = boxes[0].1;
        let last = boxes[boxes.len() - 1].1;
        let total: f64 = boxes.iter().map(|(_, b)| if horizontal { b.w } else { b.h }).sum();
        let span = if horizontal { last.x + last.w - first.x } else { last.y + last.h - first.y };
        let gap = (span - total) / (boxes.len() - 1) as f64;
        let mut at = if horizontal { first.x } else { first.y };
        let mut places = Vec::new();
        for (id, b) in &boxes {
            places.push((*id, at));
            at += (if horizontal { b.w } else { b.h }) + gap;
        }
        let ids: Vec<u32> = boxes.iter().map(|(id, _)| *id).collect();
        self.with_frames(host, &ids, None, |fr, b| {
            let Some((_, p)) = places.iter().find(|(id, _)| *id == fr.id) else { return };
            let mut x = fr.shape.xfrm.clone().unwrap_or_else(|| xfrm_of(&b));
            if horizontal {
                x.x = px_emu(*p);
            } else {
                x.y = px_emu(*p);
            }
            fr.shape.xfrm = Some(x);
        });
    }

    /// Groups the selected frames.
    pub fn group(&mut self, host: &mut dyn Measurer) {
        let boxes = self.selected_boxes(host);
        if boxes.len() < 2 {
            return;
        }
        let ids: Vec<u32> = boxes.iter().map(|(id, _)| *id).collect();
        let l = boxes.iter().map(|(_, b)| b.x).fold(f64::MAX, f64::min);
        let t = boxes.iter().map(|(_, b)| b.y).fold(f64::MAX, f64::min);
        let r = boxes.iter().map(|(_, b)| b.x + b.w).fold(f64::MIN, f64::max);
        let bt = boxes.iter().map(|(_, b)| b.y + b.h).fold(f64::MIN, f64::max);
        let active = self.sel.active;
        let mut gid = 0;
        self.change_slide(active, None, |s| {
            gid = free_id(&s.frames);
            let at = s.frames.iter().position(|f| ids.contains(&f.id)).unwrap_or(s.frames.len());
            let mut members = Vec::new();
            for f in s.frames.iter_mut().filter(|f| ids.contains(&f.id)) {
                if let Some((_, b)) = boxes.iter().find(|(id, _)| *id == f.id) {
                    let mut x = f.shape.xfrm.clone().unwrap_or_else(|| xfrm_of(b));
                    x.x = px_emu(b.x);
                    x.y = px_emu(b.y);
                    x.width = px_emu(b.w);
                    x.height = px_emu(b.h);
                    f.shape.xfrm = Some(x);
                }
                f.placeholder = None;
                members.push(f.clone());
            }
            s.frames.retain(|f| !ids.contains(&f.id));
            let rect = (px_emu(l), px_emu(t), px_emu(r - l), px_emu(bt - t));
            let group = Frame {
                id: gid,
                name: format!("Group {gid}"),
                description: None,
                hidden: false,
                shape: office_core::drawing::ShapeProps {
                    xfrm: Some(Xfrm { x: rect.0, y: rect.1, width: rect.2, height: rect.3, child: Some(rect), ..Default::default() }),
                    ..Default::default()
                },
                placeholder: None,
                link: None,
                content: FrameContent::Group { frames: members },
                origin: Origin::default(),
            };
            s.frames.insert(at.min(s.frames.len()), group);
        });
        self.sel.frames = vec![gid];
    }

    /// Takes the selected group's frames out of it, in the group's place.
    pub fn ungroup(&mut self) {
        let Some(&id) = self.sel.frames.first() else { return };
        let active = self.sel.active;
        let mut out_ids = Vec::new();
        self.change_slide(active, None, |s| {
            let Some(i) = s.frames.iter().position(|f| f.id == id) else { return };
            let FrameContent::Group { frames } = s.frames[i].content.clone() else { return };
            let gx = s.frames[i].shape.xfrm.clone().unwrap_or_default();
            let map = Map::IDENTITY.child(&gx);
            let kids: Vec<Frame> = frames
                .into_iter()
                .map(|mut f| {
                    if let Some(x) = &f.shape.xfrm {
                        let b = map.apply(Boxed::of(x));
                        let mut nx = x.clone();
                        nx.x = px_emu(b.x);
                        nx.y = px_emu(b.y);
                        nx.width = px_emu(b.w);
                        nx.height = px_emu(b.h);
                        f.shape.xfrm = Some(nx);
                    }
                    out_ids.push(f.id);
                    f
                })
                .collect();
            s.frames.splice(i..=i, kids);
        });
        self.sel.frames = out_ids;
    }

    // ---- formatting frames ----

    /// Restyles all the text in the selected frames (or, while editing, the selected text).
    pub fn text_style(&mut self, patch: &TextPatch) {
        if self.sel.editing.is_some() {
            let p = patch.clone();
            let align = patch.align();
            let sel = self.sel.editing.as_ref().map(|e| e.sel.clone()).unwrap();
            if sel.is_collapsed() && align.is_none() {
                let mut props = self.pending.clone().unwrap_or_default();
                p.apply(&mut props);
                self.pending = Some(props);
                return;
            }
            self.edit_text(None, |blocks, sel| {
                let mut splices = edit::format_runs(blocks, sel, &|r| p.apply(r));
                if let Some(a) = align {
                    splices.extend(edit::format_paras(blocks, sel, &|q| q.props.align = Some(a)));
                }
                Edit { splices, selection: None }
            });
            return;
        }
        let ids = self.sel.frames.clone();
        let active = self.sel.active;
        let p = patch.clone();
        let align = patch.align();
        self.change_slide(active, None, |s| {
            for f in s.frames.iter_mut().filter(|f| ids.contains(&f.id)) {
                for_each_paragraph(f, &mut |q| {
                    for r in q.runs_mut() {
                        p.apply(&mut r.props);
                    }
                    let mut mark = q.mark.clone().unwrap_or_default();
                    p.apply(&mut mark);
                    q.mark = Some(mark);
                    if let Some(a) = align {
                        q.props.align = Some(a);
                    }
                });
            }
        });
    }

    /// Applies text formatting to every cell, even while a cell is being edited.
    pub fn table_text_style(&mut self, patch: &TextPatch) {
        let ids = self.sel.frames.clone();
        self.change_slide(self.sel.active, None, |s| {
            for f in s.frames.iter_mut().filter(|f| ids.contains(&f.id) && matches!(f.content, FrameContent::Table { .. })) {
                for_each_paragraph(f, &mut |q| {
                    for r in q.runs_mut() {
                        patch.apply(&mut r.props);
                    }
                    let mark = q.mark.get_or_insert_with(RunProps::default);
                    patch.apply(mark);
                    if let Some(a) = patch.align() {
                        q.props.align = Some(a);
                    }
                });
            }
        });
    }

    /// The URL opened when a frame is clicked during a slideshow.
    pub fn set_link(&mut self, id: u32, url: Option<&str>) {
        let link = url.filter(|u| !u.is_empty()).map(|u| office_core::model::Link { url: Some(u.into()), ..Default::default() });
        self.change_slide(self.sel.active, None, |s| {
            if let Some(f) = find_frame_mut(&mut s.frames, id) {
                f.link = link;
            }
        });
    }

    /// Bullets (`bullet`) or numbers (`numbered`) on the selected frames' paragraphs, or off when
    /// they already have them.
    pub fn toggle_bullets(&mut self, kind: &str) {
        let ids = self.sel.frames.clone();
        let editing = self.sel.editing.clone();
        let active = self.sel.active;
        let numbered = kind == "numbered";
        self.change_slide(active, None, |s| {
            let targets: Vec<u32> = match &editing {
                Some(e) => vec![e.frame],
                None => ids.clone(),
            };
            for f in s.frames.iter_mut().filter(|f| targets.contains(&f.id)) {
                let mut all_on = true;
                for_each_paragraph(f, &mut |q| {
                    let has = q
                        .origin
                        .get()
                        .and_then(|o| o.child(ns::A, "pPr"))
                        .is_some_and(|p| p.child(ns::A, if numbered { "buAutoNum" } else { "buChar" }).is_some());
                    all_on &= has;
                });
                for_each_paragraph(f, &mut |q| set_bullet(q, if all_on { None } else { Some(numbered) }));
            }
        });
    }

    /// The selected shapes' fill: a color (`rrggbb`), or none.
    pub fn set_fill(&mut self, hex: Option<&str>) {
        let ids = self.sel.frames.clone();
        let active = self.sel.active;
        let fill = match hex {
            Some(h) => Rgb::parse(h).map(|c| Fill::Solid { color: Color::rgb(c) }),
            None => Some(Fill::None),
        };
        self.change_slide(active, None, |s| {
            for f in s.frames.iter_mut().filter(|f| ids.contains(&f.id)) {
                for_each_shape(f, &mut |f| f.shape.fill = fill.clone());
            }
        });
    }

    /// The selected shapes' outline: a color and a width in points (0 for none).
    pub fn set_line(&mut self, hex: Option<&str>, width_pt: Option<f64>) {
        let ids = self.sel.frames.clone();
        let active = self.sel.active;
        let color = hex.and_then(Rgb::parse);
        self.change_slide(active, None, |s| {
            for f in s.frames.iter_mut().filter(|f| ids.contains(&f.id)) {
                for_each_shape(f, &mut |f| {
                    let mut line = f.shape.line.clone().unwrap_or(Line { width: None, fill: None, dash: None });
                    if let Some(c) = color {
                        line.fill = Some(Fill::Solid { color: Color::rgb(c) });
                    }
                    if let Some(w) = width_pt {
                        if w <= 0.0 {
                            line.fill = Some(Fill::None);
                        } else {
                            line.width = Some(Emu::from_points(w));
                            if matches!(line.fill, Some(Fill::None)) || line.fill.is_none() && color.is_none() {
                                line.fill = Some(Fill::Solid { color: Color::rgb(Rgb::BLACK) });
                            }
                        }
                    }
                    f.shape.line = Some(line);
                });
            }
        });
    }

    /// Crops a picture frame: shares of the whole picture cut from each side (0 to 1), and the
    /// frame's new box in pixels.
    pub fn crop_picture(&mut self, id: u32, crop: [f64; 4], to: Boxed) {
        let active = self.sel.active;
        self.change_slide(active, None, |s| {
            let Some(f) = s.frames.iter_mut().find(|f| f.id == id) else { return };
            if !matches!(f.content, FrameContent::Picture { .. }) {
                return;
            }
            let mut origin = f.origin.get().cloned().unwrap_or_else(|| Element::new(ns::P, "pic"));
            let fill = origin.child_or_insert(ns::P, "blipFill");
            if fill.child(ns::A, "srcRect").is_none() {
                let at = fill
                    .children
                    .iter()
                    .position(|n| matches!(n, office_core::xml::Node::Element(e) if e.is(ns::A, "stretch") || e.is(ns::A, "tile")))
                    .unwrap_or(fill.children.len());
                fill.children.insert(at, office_core::xml::Node::Element(Element::new(ns::A, "srcRect")));
            }
            let rect = fill.child_mut(ns::A, "srcRect").expect("srcRect was just made");
            for (k, v) in ["l", "t", "r", "b"].iter().zip(crop) {
                let v = (v.clamp(0.0, 1.0) * 100_000.0).round() as i64;
                if v == 0 {
                    rect.remove_attr("", k);
                } else {
                    rect.set_attr("", k, v.to_string());
                }
            }
            f.origin = Origin::new(origin);
            let mut x = f.shape.xfrm.clone().unwrap_or_default();
            x.x = px_emu(to.x);
            x.y = px_emu(to.y);
            x.width = px_emu(to.w.max(1.0));
            x.height = px_emu(to.h.max(1.0));
            if let FrameContent::Picture { image } = &mut f.content {
                image.width = x.width;
                image.height = x.height;
            }
            f.shape.xfrm = Some(x);
        });
    }

    // ---- inserting ----

    fn insert_frame(&mut self, mut frame: Frame) -> u32 {
        let active = self.sel.active;
        let mut id = 0;
        self.change_slide(active, None, |s| {
            id = free_id(&s.frames);
            frame.id = id;
            if frame.name.is_empty() {
                frame.name = format!("Shape {id}");
            }
            s.frames.push(frame);
        });
        self.sel.frames = vec![id];
        self.sel.editing = None;
        id
    }

    /// A text box in the middle of the slide, ready to type in.
    pub fn insert_text_box(&mut self) -> u32 {
        let (w, h) = self.size();
        let (bw, bh) = (w * 0.4, 60.0);
        let xf = Xfrm { x: px_emu((w - bw) / 2.0), y: px_emu((h - bh) / 2.0), width: px_emu(bw), height: px_emu(bh), ..Default::default() };
        let mut f = Frame::text_box(0, xf, vec![Paragraph::default()]);
        f.name = String::new();
        let id = self.insert_frame(f);
        self.sel.editing = Some(Editing { frame: id, cell: None, sel: Selection::caret(Pos::start()) });
        id
    }

    /// A shape (a DrawingML preset: `rect`, `ellipse`, `star5`…) in the middle of the slide.
    pub fn insert_shape(&mut self, preset: &str) -> u32 {
        let (w, h) = self.size();
        let line = preset == "line";
        let (bw, bh) = if line { (w * 0.25, 0.0) } else { (w * 0.2, h * 0.2) };
        let accent = Color::scheme("accent1");
        let f = Frame {
            id: 0,
            name: String::new(),
            description: None,
            hidden: false,
            shape: office_core::drawing::ShapeProps {
                xfrm: Some(Xfrm {
                    x: px_emu((w - bw) / 2.0),
                    y: px_emu((h - bh) / 2.0),
                    width: px_emu(bw),
                    height: px_emu(bh),
                    ..Default::default()
                }),
                preset: Some(preset.to_string()),
                fill: (!line).then(|| Fill::Solid { color: accent.clone() }),
                line: Some(Line {
                    width: Some(Emu::from_points(if line { 2.0 } else { 1.0 })),
                    fill: Some(Fill::Solid { color: if line { accent } else { Color::scheme("accent1").with("shade", 50000) } }),
                    dash: None,
                }),
            },
            placeholder: None,
            link: None,
            content: FrameContent::Shape {
                text: (!line).then(|| TextBody { paragraphs: vec![Paragraph::default()], origin: Origin::default() }),
                connector: line,
            },
            origin: Origin::default(),
        };
        self.insert_frame(f)
    }

    /// A picture (its bytes and media type), `width` by `height` pixels, fitted inside most of
    /// the slide and centered.
    pub fn insert_image(&mut self, bytes: Vec<u8>, content_type: &str, width: f64, height: f64, name: &str) -> u32 {
        let (w, h) = self.size();
        let k = (w * 0.6 / width.max(1.0)).min(h * 0.6 / height.max(1.0)).min(1.0);
        let (bw, bh) = (width * k, height * k);
        let media = self.doc.add_media(content_type, bytes);
        let xf = Xfrm { x: px_emu((w - bw) / 2.0), y: px_emu((h - bh) / 2.0), width: px_emu(bw), height: px_emu(bh), ..Default::default() };
        let f = Frame {
            id: 0,
            name: if name.is_empty() { "Picture".into() } else { name.into() },
            description: None,
            hidden: false,
            shape: office_core::drawing::ShapeProps { xfrm: Some(xf.clone()), preset: Some("rect".into()), ..Default::default() },
            placeholder: None,
            link: None,
            content: FrameContent::Picture {
                image: Image {
                    media,
                    width: xf.width,
                    height: xf.height,
                    name: String::new(),
                    description: None,
                    origin: Origin::default(),
                },
            },
            origin: Origin::default(),
        };
        self.insert_frame(f)
    }

    /// A table of `rows` by `cols`, in PowerPoint's default style with a header row.
    pub fn insert_table(&mut self, rows: u32, cols: u32) -> u32 {
        let (w, h) = self.size();
        let (rows, cols) = (rows.clamp(1, 50) as usize, cols.clamp(1, 20) as usize);
        let bw = w * 0.8;
        let row_h = 37.0;
        let col = px_emu(bw / cols as f64);
        let tbl_pr = Element::new(ns::A, "tblPr").with_attr("", "firstRow", "1").with_attr("", "bandRow", "1");
        let table = Table {
            columns: vec![col; cols],
            rows: (0..rows)
                .map(|_| Row {
                    cells: (0..cols).map(|_| Cell { blocks: vec![Block::Paragraph(Paragraph::default())], ..Default::default() }).collect(),
                    height: Some(px_emu(row_h)),
                    header: false,
                    origin: Origin::default(),
                })
                .collect(),
            style: Some("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}".into()),
            origin: Origin::new(Element::new(ns::A, "tbl").with_child(tbl_pr)),
        };
        let th = row_h * rows as f64;
        let xf = Xfrm {
            x: px_emu((w - bw) / 2.0),
            y: px_emu(((h - th) / 2.0).max(0.0)),
            width: px_emu(bw),
            height: px_emu(th),
            ..Default::default()
        };
        let f = Frame {
            id: 0,
            name: "Table".into(),
            description: None,
            hidden: false,
            shape: office_core::drawing::ShapeProps { xfrm: Some(xf), ..Default::default() },
            placeholder: None,
            link: None,
            content: FrameContent::Table { table },
            origin: Origin::default(),
        };
        self.insert_frame(f)
    }

    /// Changes the selected table: `header`/`banded` turn those looks on or off, `rows`/`cols`
    /// set the counts, `style` sets its table style.
    pub fn table_op(&mut self, op: &str, value: Option<&str>) {
        let Some(&id) = self.sel.frames.first() else { return };
        let active = self.sel.active;
        let op = op.to_string();
        let value = value.map(str::to_string);
        self.change_slide(active, None, |s| {
            let Some(f) = s.frames.iter_mut().find(|f| f.id == id) else { return };
            let FrameContent::Table { table } = &mut f.content else { return };
            match op.as_str() {
                "header" | "banded" => {
                    let key = if op == "header" { "firstRow" } else { "bandRow" };
                    let mut origin = table.origin.get().cloned().unwrap_or_else(|| Element::new(ns::A, "tbl"));
                    let pr = origin.child_or_insert(ns::A, "tblPr");
                    let on = pr.attr(key).and_then(office_core::units::parse_bool).unwrap_or(false);
                    pr.set_attr("", key, if on { "0" } else { "1" });
                    table.origin = Origin::new(origin);
                }
                "rows" => {
                    let n = value.as_deref().and_then(|v| v.parse::<usize>().ok()).unwrap_or(table.rows.len()).clamp(1, 50);
                    let cols = table.columns.len();
                    let h = table.rows.last().and_then(|r| r.height);
                    while table.rows.len() < n {
                        table.rows.push(Row {
                            cells: (0..cols)
                                .map(|_| Cell { blocks: vec![Block::Paragraph(Paragraph::default())], ..Default::default() })
                                .collect(),
                            height: h,
                            header: false,
                            origin: Origin::default(),
                        });
                    }
                    table.rows.truncate(n);
                }
                "cols" => {
                    let n = value.as_deref().and_then(|v| v.parse::<usize>().ok()).unwrap_or(table.columns.len()).clamp(1, 20);
                    let total: i64 = table.columns.iter().map(|c| c.0).sum();
                    let each = Emu(total / n as i64);
                    table.columns = vec![each; n];
                    for row in &mut table.rows {
                        while row.cells.len() < n {
                            row.cells.push(Cell { blocks: vec![Block::Paragraph(Paragraph::default())], ..Default::default() });
                        }
                        row.cells.truncate(n);
                    }
                }
                "style" => {
                    table.style = value.clone();
                    // Direct colors and borders would otherwise conceal the chosen look.
                    for cell in table.rows.iter_mut().flat_map(|r| &mut r.cells) {
                        cell.fill = None;
                        if let Some(o) = cell.origin.get() {
                            let mut o = o.clone();
                            if let Some(pr) = o.child_mut(ns::A, "tcPr") {
                                for local in [
                                    "solidFill",
                                    "gradFill",
                                    "noFill",
                                    "pattFill",
                                    "blipFill",
                                    "lnL",
                                    "lnR",
                                    "lnT",
                                    "lnB",
                                    "lnTlToBr",
                                    "lnBlToTr",
                                ] {
                                    pr.remove_children(ns::A, local);
                                }
                            }
                            cell.origin = Origin::new(o);
                        }
                        for block in &mut cell.blocks {
                            if let Block::Paragraph(p) = block {
                                for r in p.runs_mut() {
                                    r.props.color = None;
                                }
                                if let Some(mark) = &mut p.mark {
                                    mark.color = None;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            if let Some(x) = &mut f.shape.xfrm {
                let rows_h: i64 = table.rows.iter().filter_map(|r| r.height.map(|h| h.0)).sum();
                if rows_h > 0 {
                    x.height = Emu(rows_h);
                }
            }
        });
    }

    // ---- editing text ----

    /// Starts editing a frame's text: at a point on the slide (or with all of it selected).
    pub fn begin_edit(&mut self, host: &mut dyn Measurer, id: u32, at: Option<(f64, f64)>) -> bool {
        let Some(slide) = self.slides().get(self.sel.active).cloned() else { return false };
        let Some(f) = slide.frames.iter().find(|f| f.id == id) else { return false };
        if !matches!(&f.content, FrameContent::Shape { .. } | FrameContent::Table { .. }) {
            return false;
        }
        let blocks = frame_blocks(f);
        let is_table = matches!(f.content, FrameContent::Table { .. });
        let sel = match at.and_then(|(x, y)| self.text_hit(host, id, x, y)) {
            Some(p) => Selection::caret(p),
            // A table starts in its first cell; a shape's text is all selected.
            None if is_table => Selection::caret(pos::story_start(&blocks).unwrap_or(Pos::start())),
            None => match (pos::story_start(&blocks), pos::story_end(&blocks)) {
                (Some(a), Some(b)) => Selection { anchor: a, focus: b },
                _ => Selection::caret(Pos::start()),
            },
        };
        let cell = sel.focus.path.cell().map(|(_, r, c)| (r, c));
        self.sel.frames = vec![id];
        self.sel.editing = Some(Editing { frame: id, cell, sel });
        self.pending = None;
        self.history.seal();
        true
    }

    pub fn end_edit(&mut self) {
        if let Some(e) = self.sel.editing.take() {
            self.sel.frames = vec![e.frame];
        }
        self.pending = None;
        self.history.seal();
    }

    /// The editing frame's laid-out text, where it sits in the frame, and the frame's box.
    fn edit_flow(&mut self, host: &mut dyn Measurer, id: u32) -> Option<(FlowLayout, f64, f64, Boxed)> {
        let slide = self.slides().get(self.sel.active)?.clone();
        let f = slide.frames.iter().find(|f| f.id == id)?.clone();
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let mut d = Drawer { doc: &self.doc, art: &self.art, tables: &self.tables, ctx: &mut ctx, cache: &mut self.cache };
        let b = d.boxed(&slide, &f, Map::IDENTITY);
        match &f.content {
            FrameContent::Shape { text: Some(body), .. } => {
                let look = d.look(&slide, &f);
                let (x, y, _, flow) = d.text_flow(&slide, &f, body, &b, &look);
                Some((flow, x, y, b))
            }
            FrameContent::Table { table } => {
                let paint = super::text::Paint { doc: &self.doc, slide: &slide };
                let style = super::table_style::style_for(&self.tables, table);
                let mut flowed = Vec::new();
                let mut ctx2 = Ctx { host: d.ctx.host, measure: d.ctx.measure, styles: d.ctx.styles };
                let mut r = TableText { paint: &paint, style: &style, table };
                let flow = flow::layout(&mut ctx2, d.cache, &mut r, &[Block::Table(table.clone())], &[], b.w);
                flowed.push(());
                Some((flow, 0.0, 0.0, b))
            }
            _ => None,
        }
    }

    /// The text position at a slide point inside a frame.
    fn text_hit(&mut self, host: &mut dyn Measurer, id: u32, x: f64, y: f64) -> Option<Pos> {
        let (flow, ox, oy, b) = self.edit_flow(host, id)?;
        let (lx, ly) = b.to_local(x, y);
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        flow::hit(&mut ctx, &flow, lx - ox, (ly - oy).max(0.0))
    }

    /// A click (or double click, `count` 2, for a word; 3 for a paragraph) in the editing frame.
    pub fn text_click(&mut self, host: &mut dyn Measurer, x: f64, y: f64, extend: bool, count: u32) {
        let Some(e) = self.sel.editing.clone() else { return };
        let Some(at) = self.text_hit(host, e.frame, x, y) else { return };
        let blocks = self.editing_blocks();
        let flat = pos::para(&blocks, &at.path).map(text::flat).unwrap_or_default();
        let sel = if extend {
            Selection { anchor: e.sel.anchor.clone(), focus: at }
        } else if count == 2 {
            let (a, b) = para::word_at(&flat.text, at.offset);
            Selection { anchor: Pos::new(at.path.clone(), a), focus: Pos::new(at.path, b) }
        } else if count >= 3 {
            Selection { anchor: Pos::new(at.path.clone(), 0), focus: Pos::new(at.path, flat.text.len() as u32) }
        } else {
            Selection::caret(at)
        };
        self.set_text_sel(sel);
    }

    fn set_text_sel(&mut self, sel: Selection) {
        if let Some(e) = &mut self.sel.editing {
            e.cell = sel.focus.path.cell().map(|(_, r, c)| (r, c));
            e.sel = sel;
        }
        self.pending = None;
        self.goal_x = None;
        self.history.seal();
    }

    /// The editing frame's text as blocks (a copy).
    fn editing_blocks(&self) -> Vec<Block> {
        let Some(e) = &self.sel.editing else { return Vec::new() };
        self.slides().get(self.sel.active).and_then(|s| s.frames.iter().find(|f| f.id == e.frame)).map(frame_blocks).unwrap_or_default()
    }

    /// Runs a text edit on the editing frame's blocks, as an undo step (joining a run of typing).
    fn edit_text(&mut self, kind: Option<&'static str>, f: impl FnOnce(&mut Vec<Block>, &Selection) -> Edit) {
        let Some(e) = self.sel.editing.clone() else { return };
        let active = self.sel.active;
        let mut new_sel = None;
        self.change_slide(active, kind, |s| {
            let Some(fr) = s.frames.iter_mut().find(|fr| fr.id == e.frame) else { return };
            with_frame_blocks(fr, |blocks| {
                let edit = f(blocks, &e.sel);
                new_sel = edit.selection;
            });
        });
        if let Some(sel) = new_sel {
            if let Some(ed) = &mut self.sel.editing {
                ed.cell = sel.focus.path.cell().map(|(_, r, c)| (r, c));
                ed.sel = sel;
            }
        }
    }

    pub fn insert_text(&mut self, s: &str) {
        if self.sel.editing.is_none() {
            return;
        }
        let props = self.pending.take();
        let typing = s.chars().count() == 1;
        let s = s.to_string();
        self.edit_text(if typing { Some("typing") } else { None }, |blocks, sel| edit::insert_text(blocks, sel, &s, props.as_ref()));
    }

    pub fn backspace(&mut self, word: bool) {
        self.delete_dir(false, word);
    }

    pub fn delete_forward(&mut self, word: bool) {
        self.delete_dir(true, word);
    }

    fn delete_dir(&mut self, forward: bool, word: bool) {
        let Some(e) = self.sel.editing.clone() else { return };
        let blocks = self.editing_blocks();
        let range = if e.sel.is_collapsed() {
            let at = e.sel.focus.clone();
            let Some(p) = pos::para(&blocks, &at.path) else { return };
            let flat = text::flat(p);
            let other = if forward && at.offset as usize >= flat.text.len() {
                pos::next_para(&blocks, &at.path).filter(|n| n.container() == at.path.container()).map(|n| Pos::new(n, 0))
            } else if !forward && at.offset == 0 {
                pos::prev_para(&blocks, &at.path).filter(|n| n.container() == at.path.container()).map(|n| {
                    let l = pos::para(&blocks, &n).map(text::len).unwrap_or(0);
                    Pos::new(n, l)
                })
            } else {
                let off = match (forward, word) {
                    (true, true) => para::word_move(&flat.text, at.offset, true),
                    (false, true) => para::word_move(&flat.text, at.offset, false),
                    (true, false) => para::next_boundary(&flat.text, at.offset),
                    (false, false) => para::prev_boundary(&flat.text, at.offset),
                };
                Some(Pos::new(at.path.clone(), off))
            };
            let Some(other) = other else { return };
            Selection { anchor: at, focus: other }
        } else {
            e.sel.clone()
        };
        self.edit_text(if word { None } else { Some("deleting") }, |blocks, _| edit::delete(blocks, &range));
    }

    pub fn enter(&mut self) {
        // In a table cell, Enter is a new paragraph in the cell, like anywhere else.
        self.history.seal();
        self.edit_text(None, |blocks, sel| {
            let mut e = edit::delete(blocks, sel);
            let at = e.selection.clone().map(|s| s.focus).unwrap_or_else(|| sel.focus.clone());
            let s = edit::split_para(blocks, &at, None);
            e.splices.extend(s.splices);
            e.selection = s.selection;
            e
        });
    }

    pub fn line_break(&mut self) {
        let props = self.pending.take();
        self.edit_text(None, |blocks, sel| {
            let mut e = edit::delete(blocks, sel);
            let at = e.selection.clone().map(|s| s.focus).unwrap_or_else(|| sel.focus.clone());
            if let Some(s) = edit::change_para(blocks, &at.path, |p| text::insert_text(p, at.offset, "\n", props.as_ref())) {
                e.splices.push(s);
            }
            e.selection = Some(Selection::caret(Pos::new(at.path, at.offset + 1)));
            e
        });
    }

    /// Tab: the next table cell, or a tab character.
    pub fn tab(&mut self, shift: bool) {
        let Some(e) = self.sel.editing.clone() else { return };
        if let Some((_, r, c)) = e.sel.focus.path.cell() {
            let blocks = self.editing_blocks();
            let Some(Block::Table(t)) = blocks.first() else { return };
            let mut slots = Vec::new();
            for (ri, row) in t.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    if !cell.covered {
                        slots.push((ri as u32, ci as u32));
                    }
                }
            }
            let i = slots.iter().position(|s| *s == (r, c)).unwrap_or(0);
            let next = if shift { i.checked_sub(1) } else { (i + 1 < slots.len()).then_some(i + 1) };
            if let Some(n) = next {
                let (nr, nc) = slots[n];
                if let Some(p) = pos::first_para_from(&blocks, &[0, nr, nc], 0) {
                    let len = pos::para(&blocks, &p).map(text::len).unwrap_or(0);
                    self.set_text_sel(Selection { anchor: Pos::new(p.clone(), 0), focus: Pos::new(p, len) });
                }
            }
            return;
        }
        if !shift {
            self.insert_text("\t");
        }
    }

    pub fn select_all_text(&mut self) {
        let blocks = self.editing_blocks();
        if let (Some(a), Some(b)) = (pos::story_start(&blocks), pos::story_end(&blocks)) {
            self.set_text_sel(Selection { anchor: a, focus: b });
        }
    }

    /// Moves the caret in the editing frame: `left`, `right`, `up`, `down`, `wordLeft`,
    /// `wordRight`, `lineStart`, `lineEnd`, `start`, `end`.
    pub fn move_caret(&mut self, host: &mut dyn Measurer, to: &str, extend: bool) {
        let Some(e) = self.sel.editing.clone() else { return };
        let blocks = self.editing_blocks();
        let focus = e.sel.focus.clone();
        let collapse = !extend && !e.sel.is_collapsed();
        let flat = pos::para(&blocks, &focus.path).map(text::flat).unwrap_or_default();
        let step = |forward: bool| -> Pos {
            if forward {
                if (focus.offset as usize) < flat.text.len() {
                    return Pos::new(focus.path.clone(), para::next_boundary(&flat.text, focus.offset));
                }
                pos::next_para(&blocks, &focus.path).map(|n| Pos::new(n, 0)).unwrap_or(focus.clone())
            } else {
                if focus.offset > 0 {
                    return Pos::new(focus.path.clone(), para::prev_boundary(&flat.text, focus.offset));
                }
                pos::prev_para(&blocks, &focus.path)
                    .map(|n| Pos::new(n.clone(), pos::para(&blocks, &n).map(text::len).unwrap_or(0)))
                    .unwrap_or(focus.clone())
            }
        };
        let target = match to {
            "left" if collapse => e.sel.start().clone(),
            "right" if collapse => e.sel.end().clone(),
            "left" => step(false),
            "right" => step(true),
            "wordLeft" => Pos::new(focus.path.clone(), para::word_move(&flat.text, focus.offset, false)),
            "wordRight" => Pos::new(focus.path.clone(), para::word_move(&flat.text, focus.offset, true)),
            "start" => pos::story_start(&blocks).unwrap_or(focus.clone()),
            "end" => pos::story_end(&blocks).unwrap_or(focus.clone()),
            "up" | "down" | "lineStart" | "lineEnd" => {
                let Some((flow, _, _, _)) = self.edit_flow(host, e.frame) else { return };
                let Some((pb, dx, _)) = flow.find(&focus.path) else { return };
                let pb = pb.clone();
                let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
                let (cx, li) = pb.layout.caret(&mut ctx, &pb.spec, focus.offset, false);
                match to {
                    "lineStart" => Pos::new(focus.path.clone(), pb.layout.lines[li].start),
                    "lineEnd" => {
                        let line = &pb.layout.lines[li];
                        let off = if line.ends == LineEnd::Wrap {
                            pb.layout.offset_at(&mut ctx, &pb.spec, li, f64::MAX)
                        } else {
                            line.end.saturating_sub(if line.ends == LineEnd::Para { 0 } else { 1 })
                        };
                        Pos::new(focus.path.clone(), off)
                    }
                    _ => {
                        let x = self.goal_x.unwrap_or(dx + cx);
                        let down = to == "down";
                        let next = if down { li + 1 } else { li.wrapping_sub(1) };
                        let t = if next < pb.layout.lines.len() {
                            Pos::new(focus.path.clone(), pb.layout.offset_at(&mut ctx, &pb.spec, next, x - dx))
                        } else {
                            let other = if down { pos::next_para(&blocks, &focus.path) } else { pos::prev_para(&blocks, &focus.path) };
                            match other.and_then(|o| flow.find(&o).map(|(b, odx, _)| (o, b.clone(), odx))) {
                                Some((o, ob, odx)) => {
                                    let line = if down { 0 } else { ob.layout.lines.len() - 1 };
                                    Pos::new(o, ob.layout.offset_at(&mut ctx, &ob.spec, line, x - odx))
                                }
                                None => {
                                    if down {
                                        pos::story_end(&blocks).unwrap_or(focus.clone())
                                    } else {
                                        pos::story_start(&blocks).unwrap_or(focus.clone())
                                    }
                                }
                            }
                        };
                        let anchor = if extend { e.sel.anchor.clone() } else { t.clone() };
                        if let Some(ed) = &mut self.sel.editing {
                            ed.sel = Selection { anchor, focus: t };
                        }
                        self.goal_x = Some(x);
                        self.history.seal();
                        return;
                    }
                }
            }
            _ => return,
        };
        let anchor = if extend { e.sel.anchor.clone() } else { target.clone() };
        self.set_text_sel(Selection { anchor, focus: target });
    }

    /// The selected text, plain.
    pub fn copy_text(&self) -> String {
        let Some(e) = &self.sel.editing else { return String::new() };
        let blocks = self.editing_blocks();
        office_core::model::blocks_text(&edit::copy(&blocks, &e.sel))
    }

    /// Where the caret and selection show in the editing frame.
    pub fn text_overlay(&mut self, host: &mut dyn Measurer) -> Option<TextOverlay> {
        let e = self.sel.editing.clone()?;
        let (flow, ox, oy, b) = self.edit_flow(host, e.frame)?;
        let blocks = self.editing_blocks();
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let mut out =
            TextOverlay { frame: e.frame.to_string(), x: b.x, y: b.y, w: b.w, h: b.h, rot: b.rot, caret: None, rects: Vec::new() };
        let (from, to) = e.sel.ordered();
        if !e.sel.is_collapsed() {
            for path in pos::paras_between(&blocks, &from.path, &to.path) {
                let Some((pb, dx, dy)) = flow.find(&path) else { continue };
                let len = pos::para(&blocks, &path).map(text::len).unwrap_or(0);
                let a = if path == from.path { from.offset } else { 0 };
                let bb = if path == to.path { to.offset } else { len };
                out.rects.extend(display::selection_rects(&mut ctx, pb, ox + dx, oy + dy, a, bb, path != to.path));
            }
        } else if let Some((pb, dx, dy)) = flow.find(&e.sel.focus.path) {
            out.caret = Some(display::caret_rect(&mut ctx, pb, ox + dx, oy + dy, e.sel.focus.offset, false));
        }
        Some(out)
    }

    // ---- spelling ----

    /// The text of each frame on the current slide that holds text, paragraph by paragraph:
    /// (frame id, paragraph path, text). Spelling checks it.
    pub fn slide_text(&self) -> Vec<(u32, Path, String)> {
        let Some(slide) = self.slides().get(self.sel.active) else { return Vec::new() };
        let mut out = Vec::new();
        for f in slide.frames.iter().filter(|f| has_text(f)) {
            let blocks = frame_blocks(f);
            let mut at = pos::story_start(&blocks).map(|p| p.path);
            while let Some(path) = at {
                if let Some(p) = pos::para(&blocks, &path) {
                    out.push((f.id, path.clone(), text::flat(p).text));
                }
                at = pos::next_para(&blocks, &path);
            }
        }
        out
    }

    /// Where ranges of text in frames on the current slide are, for underlining misspelled
    /// words: per frame, its box and the rectangles under each range (one per line), tagged with
    /// the range's index in `ranges` and placed like `text_overlay`'s selection. A range stays
    /// within one paragraph.
    pub fn text_ranges(&mut self, host: &mut dyn Measurer, ranges: &[(u32, Pos, Pos)]) -> Vec<RangeBoxes> {
        let mut ids: Vec<u32> = ranges.iter().map(|r| r.0).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut out = Vec::new();
        for id in ids {
            let Some((flow, ox, oy, b)) = self.edit_flow(host, id) else { continue };
            let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
            let mut boxes = RangeBoxes { frame: id.to_string(), x: b.x, y: b.y, w: b.w, h: b.h, rot: b.rot, rects: Vec::new() };
            for (i, (_, from, to)) in ranges.iter().enumerate().filter(|(_, r)| r.0 == id) {
                if from.path != to.path {
                    continue;
                }
                let Some((pb, dx, dy)) = flow.find(&from.path) else { continue };
                for (x, y, w, h) in display::selection_rects(&mut ctx, pb, ox + dx, oy + dy, from.offset, to.offset, false) {
                    boxes.rects.push((i, x, y, w, h));
                }
            }
            out.push(boxes);
        }
        out
    }

    /// Starts editing frame `id` with `sel` selected (a misspelled word, to replace it).
    pub fn select_text(&mut self, id: u32, sel: Selection) -> bool {
        let Some(slide) = self.slides().get(self.sel.active) else { return false };
        if !slide.frames.iter().any(|f| f.id == id && has_text(f)) {
            return false;
        }
        let cell = sel.focus.path.cell().map(|(_, r, c)| (r, c));
        self.sel.frames = vec![id];
        self.sel.editing = Some(Editing { frame: id, cell, sel });
        self.pending = None;
        self.history.seal();
        true
    }

    // ---- find and replace ----

    /// Frames whose text holds `query` (any case): (slide, frame id).
    pub fn find(&self, query: &str) -> Vec<(usize, u32)> {
        let q = query.trim().to_lowercase();
        let mut out = Vec::new();
        if q.is_empty() {
            return out;
        }
        for (i, s) in self.slides().iter().enumerate() {
            for f in &s.frames {
                let t: String = f.paragraphs().iter().map(|p| p.plain_text()).collect::<Vec<_>>().join("\n").to_lowercase();
                if t.contains(&q) {
                    out.push((i, f.id));
                }
            }
        }
        out
    }

    /// Replaces `query` (any case) with `by` in the frames given, one undo step.
    pub fn replace(&mut self, matches: &[(usize, u32)], query: &str, by: &str) {
        let q = query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.begin_group();
        for &(slide, id) in matches {
            let (q, by) = (q.clone(), by.to_string());
            self.change_slide(slide, None, |s| {
                if let Some(f) = find_frame_mut(&mut s.frames, id) {
                    for_each_paragraph(f, &mut |p| {
                        for r in p.runs_mut() {
                            r.text = replace_ci(&r.text, &q, &by);
                        }
                    });
                }
            });
        }
        self.end_group();
    }

    /// The frame with `id` on the active slide.
    pub fn frame(&self, id: u32) -> Option<&Frame> {
        find_frame(&self.slides().get(self.sel.active)?.frames, id)
    }
}

/// A box as an `a:xfrm`.
fn xfrm_of(b: &Boxed) -> Xfrm {
    Xfrm {
        x: px_emu(b.x),
        y: px_emu(b.y),
        width: px_emu(b.w),
        height: px_emu(b.h),
        rotation: Angle::from_degrees(b.rot),
        flip_h: b.flip_h,
        flip_v: b.flip_v,
        child: None,
    }
}

/// Gives a frame (and a group's frames) new ids from `next` on.
fn renumber(f: &mut Frame, next: &mut u32) {
    f.id = *next;
    *next += 1;
    f.origin = strip_ids(&f.origin);
    if let FrameContent::Group { frames } = &mut f.content {
        for c in frames {
            renumber(c, next);
        }
    }
}

/// A frame's markup with its non-visual id removed, so the writer gives the copy its own.
fn strip_ids(o: &Origin) -> Origin {
    match o.get() {
        Some(el) => {
            let mut el = el.clone();
            if let Some(nv) = el.elements_mut().find(|e| e.local.starts_with("nv")) {
                if let Some(c) = nv.child_mut(ns::P, "cNvPr") {
                    c.remove_attr("", "id");
                }
            }
            Origin::new(el)
        }
        None => Origin::default(),
    }
}

/// Every picture in a frame, including pictures inside nested groups.
fn for_each_picture(f: &mut Frame, cb: &mut dyn FnMut(&mut Image)) {
    match &mut f.content {
        FrameContent::Picture { image } => cb(image),
        FrameContent::Group { frames } => frames.iter_mut().for_each(|f| for_each_picture(f, cb)),
        _ => {}
    }
}

/// Every paragraph in a frame: a shape's text, a table's cells, a group's frames.
fn for_each_paragraph(f: &mut Frame, cb: &mut dyn FnMut(&mut Paragraph)) {
    match &mut f.content {
        FrameContent::Shape { text: Some(body), .. } => body.paragraphs.iter_mut().for_each(cb),
        FrameContent::Table { table } => {
            for cell in table.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                for b in &mut cell.blocks {
                    if let Block::Paragraph(p) = b {
                        cb(p);
                    }
                }
            }
        }
        FrameContent::Group { frames } => frames.iter_mut().for_each(|c| for_each_paragraph(c, cb)),
        _ => {}
    }
}

/// A frame's text as engine blocks: a shape's paragraphs (paths `[i]`), or the table itself
/// (cells' paragraphs at `[0, row, col, i]`).
/// A frame whose text can be edited: a shape with text, or a table.
fn has_text(f: &Frame) -> bool {
    matches!(&f.content, FrameContent::Shape { text: Some(_), .. } | FrameContent::Table { .. })
}

fn frame_blocks(f: &Frame) -> Vec<Block> {
    match &f.content {
        FrameContent::Shape { text: Some(body), .. } => body.paragraphs.iter().cloned().map(Block::Paragraph).collect(),
        FrameContent::Shape { text: None, .. } => vec![Block::Paragraph(Paragraph::default())],
        FrameContent::Table { table } => vec![Block::Table(table.clone())],
        _ => Vec::new(),
    }
}

/// Runs `f` on a frame's text as engine blocks and puts the result back.
fn with_frame_blocks(f: &mut Frame, edit: impl FnOnce(&mut Vec<Block>)) {
    match &mut f.content {
        FrameContent::Shape { text, .. } => {
            let body = text.get_or_insert_with(|| TextBody { paragraphs: vec![Paragraph::default()], origin: Origin::default() });
            let mut blocks = text_blocks(body);
            edit(&mut blocks);
            from_blocks(body, blocks);
        }
        FrameContent::Table { table } => {
            let mut blocks = vec![Block::Table(std::mem::take(table))];
            edit(&mut blocks);
            if let Some(Block::Table(t)) = blocks.into_iter().next() {
                *table = t;
            }
        }
        _ => {}
    }
}

/// Sets a paragraph's bullet: a character, a number, or none.
fn set_bullet(p: &mut Paragraph, numbered: Option<bool>) {
    let mut origin = p.origin.get().cloned().unwrap_or_else(|| Element::new(ns::A, "p"));
    let has_ppr = origin.child(ns::A, "pPr").is_some();
    if !has_ppr {
        origin.children.insert(0, office_core::xml::Node::Element(Element::new(ns::A, "pPr")));
    }
    let ppr = origin.child_mut(ns::A, "pPr").expect("pPr was just made");
    for k in ["buNone", "buChar", "buAutoNum", "buFont", "buBlip"] {
        ppr.remove_children(ns::A, k);
    }
    match numbered {
        Some(n) => {
            ppr.set_attr("", "marL", "342900");
            ppr.set_attr("", "indent", "-342900");
            if n {
                ppr.push(Element::new(ns::A, "buFont").with_attr("", "typeface", "+mj-lt"));
                ppr.push(Element::new(ns::A, "buAutoNum").with_attr("", "type", "arabicPeriod"));
            } else {
                ppr.push(Element::new(ns::A, "buFont").with_attr("", "typeface", "Arial"));
                ppr.push(Element::new(ns::A, "buChar").with_attr("", "char", "•"));
            }
            p.props.indent_start = Some(Emu(342_900));
            p.props.indent_first_line = Some(Emu(-342_900));
        }
        None => {
            ppr.push(Element::new(ns::A, "buNone"));
            p.props.indent_start = Some(Emu(0));
            p.props.indent_first_line = Some(Emu(0));
        }
    }
    p.origin = Origin::new(origin);
}

/// `text` with every `query` (any case) replaced by `by`.
fn replace_ci(text: &str, query: &str, by: &str) -> String {
    if query.is_empty() {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let q = query.to_lowercase();
    if lower.len() != text.len() {
        return text.replace(query, by);
    }
    let mut out = String::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&q) {
        out.push_str(&text[from..from + at]);
        out.push_str(by);
        from += at + q.len();
    }
    out.push_str(&text[from..]);
    out
}

/// Text in an editing table's cells, as it's drawn (see `draw::CellText`).
struct TableText<'a> {
    paint: &'a super::text::Paint<'a>,
    style: &'a super::table_style::TableStyle,
    table: &'a Table,
}

impl flow::Resolver for TableText<'_> {
    fn para(&mut self, ctx: &mut Ctx, path: &Path, p: &Paragraph, _: f64) -> (std::sync::Arc<para::ParaSpec>, flow::Keep) {
        let (row, col) = path.cell().map(|(_, r, c)| (r as usize, c as usize)).unwrap_or((0, 0));
        let part = super::table_style::text_for(self.table, self.style, row, col);
        let mut base = RunProps { size: Some(FontSize::from_points(18.0)), ..Default::default() };
        if let Some(b) = part.bold {
            base.bold = Some(b);
        }
        let runs: Vec<RunProps> = p
            .runs()
            .map(|r| {
                let mut x = base.clone();
                x.overlay(&r.props);
                x
            })
            .collect();
        (std::sync::Arc::new(super::text::cell_spec(ctx, self.paint, p, &runs, part.color.as_ref())), flow::Keep::default())
    }

    fn table(&mut self, _: &Path, t: &Table, width: f64) -> flow::TableLook {
        super::table_style::look(self.paint, t, self.style, width)
    }
}

/// Fill and line controls affect shapes inside selected groups as well.
fn for_each_shape(frame: &mut Frame, cb: &mut dyn FnMut(&mut Frame)) {
    match &mut frame.content {
        FrameContent::Group { frames } => {
            for frame in frames {
                for_each_shape(frame, cb);
            }
        }
        FrameContent::Shape { .. } => cb(frame),
        _ => {}
    }
}
