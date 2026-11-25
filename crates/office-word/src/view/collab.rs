//! A Word document shared with other copies (see `office_engine::collab`).
//!
//! Steps go as the blocks a splice replaced (by fingerprint) and the blocks that replaced them.
//! A splice made before someone else's splice above it in the same list moves down past it,
//! the way an index in a list moves when items are added before it.
//!
//! Some changes touch the layer's own tables, outside the undo steps: a new list's numbering,
//! a built-in style first used, a new header or footer. They're sent along with the change as
//! what to do again (`extra`), and doing it again is a no-op where it's been done.

use office_core::model::{Block, Body, Origin};
use office_core::xml::parse_element;
use office_engine::collab::{self, fingerprint, send_content, take_content, Replica, Shared};
use office_engine::display;
use office_engine::edit::Splice;
use office_engine::measure::Measurer;
use office_engine::para::Ctx;
use office_engine::pos::{self, Path, Pos, Selection};
use office_engine::text;
use serde_json::{json, Value};

use super::{to_page, Dirty, Overlay, StoryId, WStep, WordView};
use crate::{word, ListKind, StoryKind};

/// How far undo looks for a splice's blocks around where they were, when they aren't there.
const NEAR: i64 = 32;

impl WStep {
    pub(crate) fn inverse(&self) -> WStep {
        match self {
            WStep::Text(story, s) => WStep::Text(
                *story,
                Splice { container: s.container.clone(), index: s.index, removed: s.inserted.clone(), inserted: s.removed.clone() },
            ),
            WStep::Origin { block, before, after } => WStep::Origin { block: *block, before: after.clone(), after: before.clone() },
        }
    }
}

fn story_name(s: StoryId) -> String {
    match s {
        StoryId::Body => "b".into(),
        StoryId::Story(i) => format!("s{i}"),
        StoryId::Note(i) => format!("n{i}"),
    }
}

fn story_from(s: &str) -> Option<StoryId> {
    if s == "b" {
        return Some(StoryId::Body);
    }
    let n = s.get(1..)?.parse().ok()?;
    match s.as_bytes()[0] {
        b's' => Some(StoryId::Story(n)),
        b'n' => Some(StoryId::Note(n)),
        _ => None,
    }
}

fn origin_xml(o: &Origin) -> Value {
    o.get().map(|e| Value::String(e.to_xml_string())).unwrap_or(Value::Null)
}

impl WordView {
    /// Starts sharing: from now on changes are kept to send, and undo goes through the checks.
    pub fn collab_start(&mut self, client: &str) {
        self.replica = Some(Replica::new(client, &self.doc));
        self.history = Default::default();
    }

    pub fn collab_stop(&mut self) {
        self.replica = None;
        self.collab_extra.clear();
    }

    pub fn collab_take(&mut self) -> Option<Value> {
        collab::take(self)
    }

    pub fn collab_receive(&mut self, op: &Value) -> bool {
        let changed = collab::receive(self, op);
        if self.replica.as_mut().is_some_and(|r| r.take_changed_by_others()) {
            self.others_changed = true;
        }
        if changed {
            self.stale = true;
            self.revision += 1;
            self.goal_x = None;
            self.caret_near = None;
        }
        changed
    }

    pub fn collab_pending(&self) -> bool {
        self.replica.as_ref().is_some_and(|r| r.has_pending())
    }

    /// Where the caret is, for the others to draw: the story, and the selection's ends as
    /// `[path, byte offset]`.
    pub fn collab_presence(&self) -> Value {
        let end = |p: &Pos| json!([p.path.0, p.offset]);
        json!({ "s": story_name(self.story), "a": end(&self.sel.anchor), "f": end(&self.sel.focus) })
    }

    /// Where another copy's caret and selection (its `collab_presence`) show on the pages, when
    /// it's in the body. An end on a paragraph this copy doesn't have hides it: that copy made
    /// changes that haven't arrived yet, and says where it is again when it sends them.
    pub fn remote_overlay(&mut self, host: &mut dyn Measurer, presence: &Value) -> Overlay {
        let mut out = Overlay::default();
        if presence["s"] != "b" {
            return out;
        }
        self.layout(host);
        let WordView { doc, sections, pages, measure, styles, .. } = self;
        let blocks: &[Block] = match &doc.body {
            Body::Flow { blocks } => blocks,
            _ => &[],
        };
        let end = |v: &Value| -> Option<Pos> {
            let path = Path(v.get(0)?.as_array()?.iter().map(|n| n.as_u64().map(|n| n as u32)).collect::<Option<_>>()?);
            let para = pos::para(blocks, &path)?;
            let flat = text::flat(para);
            let mut offset = (v.get(1)?.as_u64()? as u32).min(flat.text.len() as u32);
            while offset > 0 && !flat.text.is_char_boundary(offset as usize) {
                offset -= 1;
            }
            Some(Pos::new(path, offset))
        };
        let (Some(anchor), Some(focus)) = (end(&presence["a"]), end(&presence["f"])) else { return out };
        let sel = Selection { anchor, focus };
        let section_of = |path: &Path| sections.iter().position(|s| path.0[0] >= s.first && path.0[0] <= s.last);
        let mut ctx = Ctx { host, measure, styles };
        let (from, to) = sel.ordered();
        if !sel.is_collapsed() {
            for path in pos::paras_between(blocks, &from.path, &to.path) {
                let Some(si) = section_of(&path) else { continue };
                let Some((pb, dx, dy)) = sections[si].flow.find(&path) else { continue };
                let len = pos::para(blocks, &path).map(text::len).unwrap_or(0);
                let a = if path == from.path { from.offset } else { 0 };
                let b = if path == to.path { to.offset } else { len };
                for (x, y, w, h) in display::selection_rects(&mut ctx, pb, dx, dy, a, b, path != to.path) {
                    let (page, px, py) = to_page(pages, si, x, y);
                    out.rects.push((page, px, py, w, h));
                }
            }
        }
        if let Some(si) = section_of(&sel.focus.path) {
            if let Some((pb, dx, dy)) = sections[si].flow.find(&sel.focus.path) {
                let before = sel.focus.offset > 0 && from != to && sel.focus == to;
                let (x, y, h) = display::caret_rect(&mut ctx, pb, dx, dy, sel.focus.offset, before);
                let (page, px, py) = to_page(pages, si, x, y);
                out.caret = Some((page, px, py, h));
            }
        }
        out
    }

    /// Notes steps made here for sending, when shared.
    pub(crate) fn shared_made(&mut self, steps: &[WStep]) {
        if let Some(r) = &mut self.replica {
            r.made(steps.iter().cloned());
        }
    }

    /// Notes a change to the layer's own tables for sending, when shared.
    pub(crate) fn shared_extra(&mut self, extra: Value) {
        if self.replica.is_some() {
            self.collab_extra.push(extra);
        }
    }

    /// Adds a picture, under an id unique among the copies when shared.
    pub(crate) fn add_media(&mut self, content_type: &str, bytes: Vec<u8>) -> String {
        collab::add_media(&mut self.doc, self.replica.as_ref(), content_type, bytes)
    }

    /// Undoes or redoes an entry's steps where the document is now; `false` if someone else's
    /// change is in the way.
    pub(crate) fn shared_replay(&mut self, steps: Vec<WStep>) -> bool {
        match collab::replay(self, &steps) {
            Some(done) => {
                self.shared_made(&done);
                true
            }
            None => false,
        }
    }

    fn has_story(&self, story: StoryId) -> bool {
        match story {
            StoryId::Body => true,
            StoryId::Story(i) => word(&self.doc).is_some_and(|w| i < w.headers.len()),
            StoryId::Note(i) => word(&self.doc).is_some_and(|w| i < w.notes.len()),
        }
    }

    fn take_text(&mut self, step: &Value, since: Option<&[(bool, &WStep)]>) -> Option<WStep> {
        let story = story_from(step["story"].as_str()?)?;
        if !self.has_story(story) {
            return None;
        }
        let mut container: Vec<u32> = serde_json::from_value(step["at"].clone()).ok()?;
        let mut index = step["index"].as_u64()? as i64;
        let n = step["n"].as_u64()? as usize;
        let before = step["before"].as_str()?;
        // Past what others changed before it, in the same list or a list above. A splice over the
        // same blocks, or over a table they're in, means this one comes second and is dropped.
        let known = since.is_some();
        let since = since.unwrap_or(&[]);
        for (i, (own, s)) in since.iter().enumerate() {
            let WStep::Text(st, s) = s else { continue };
            if *own || *st != story {
                continue;
            }
            let Some((start, end)) = sender_frame(s, story, &since[i + 1..]) else { continue };
            let delta = s.inserted.len() as i64 - s.removed.len() as i64;
            if s.container == container {
                if end <= index {
                    index += delta;
                } else if start < index + n as i64 || (n == 0 && start < index) {
                    return None;
                }
            } else if container.len() > s.container.len() && container[..s.container.len()] == s.container[..] {
                let at = &mut container[s.container.len()];
                if end <= *at as i64 {
                    *at = (*at as i64 + delta).max(0) as u32;
                } else if start <= *at as i64 {
                    return None;
                }
            }
        }
        let list = pos::blocks_at(self.blocks(story), &container)?;
        let fits = |i: i64| i >= 0 && (i as usize) + n <= list.len() && fingerprint(&list[i as usize..i as usize + n]) == before;
        let index = if n == 0 {
            index.clamp(0, list.len() as i64)
        } else if known {
            Some(index).filter(|&i| fits(i))?
        } else {
            // Undo: other copies may have moved it since, by an unknown amount.
            (0..=NEAR).flat_map(|d| [index + d, index - d]).find(|&i| fits(i))?
        } as usize;
        let old = list[index..index + n].to_vec();
        let inserted: Vec<Block> = take_content(&old, &step["after"])?;
        // Two copies deleting a list's last blocks at once (each leaving one) would leave it empty.
        if inserted.is_empty() && n > 0 && n == list.len() {
            return None;
        }
        let root = self.blocks_mut(story);
        let list = pos::blocks_at_mut(root, &container)?;
        let removed: Vec<Block> = list.splice(index..index + n, inserted.iter().cloned()).collect();
        let s = Splice { container, index: index as u32, removed, inserted };
        if story == StoryId::Body {
            self.dirty.note(&s, false);
        }
        self.unrecorded = false;
        Some(WStep::Text(story, s))
    }

    fn take_origin(&mut self, step: &Value) -> Option<WStep> {
        let block = step["block"].as_u64().map(|b| b as u32);
        let now = self.origin_of(block);
        if fingerprint(&origin_xml(&now)) != step["before"].as_str()? {
            return None;
        }
        let after = match step["after"].as_str() {
            Some(x) => Origin::new(parse_element(x).ok()?),
            None => Origin::default(),
        };
        let s = WStep::Origin { block, before: now, after };
        self.redo_step(&s);
        self.note_step(&s, false);
        Some(s)
    }
}

impl Shared for WordView {
    type Step = WStep;
    type Mark = (StoryId, Selection);

    fn replica(&mut self) -> &mut Option<Replica<WStep>> {
        &mut self.replica
    }

    fn doc(&mut self) -> &mut office_core::model::Document {
        &mut self.doc
    }

    fn send_step(&self, step: &WStep) -> Value {
        match step {
            WStep::Text(story, s) => json!({
                "k": "text",
                "story": story_name(*story),
                "at": s.container,
                "index": s.index,
                "n": s.removed.len(),
                "before": fingerprint(&s.removed),
                "after": send_content(&s.removed, &s.inserted),
            }),
            WStep::Origin { block, before, after } => json!({
                "k": "origin",
                "block": block,
                "before": fingerprint(&origin_xml(before)),
                "after": origin_xml(after),
            }),
        }
    }

    fn take_step(&mut self, step: &Value, since: Option<&[(bool, &WStep)]>) -> Option<WStep> {
        let s = match step["k"].as_str()? {
            "text" => self.take_text(step, since),
            "origin" => self.take_origin(step),
            _ => None,
        };
        if s.is_some() {
            self.stale = true;
        }
        s
    }

    fn revert_step(&mut self, step: &WStep) {
        self.undo_step(step);
        self.note_step(step, true);
        self.unrecorded = false;
        self.stale = true;
    }

    fn mark(&self) -> (StoryId, Selection) {
        (self.story, self.sel.clone())
    }

    fn restore(&mut self, (story, sel): (StoryId, Selection), through: &[WStep]) {
        let mut sel = sel;
        for step in through {
            if let WStep::Text(st, s) = step {
                if *st == story {
                    map_pos(&mut sel.anchor, s);
                    map_pos(&mut sel.focus, s);
                }
            }
        }
        self.story = if self.has_story(story) { story } else { StoryId::Body };
        self.sel = self.clamp(sel);
    }

    fn send_extra(&mut self) -> Value {
        if self.collab_extra.is_empty() {
            return Value::Null;
        }
        Value::Array(std::mem::take(&mut self.collab_extra))
    }

    fn take_extra(&mut self, extra: &Value) {
        let mut changed = false;
        for e in extra.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if let (Some(id), Some(kind)) = (e["list"].as_str(), e["kind"].as_str()) {
                let has = word(&self.doc).is_some_and(|w| w.numbering.levels(id).is_some());
                if !has {
                    crate::add_list(&mut self.doc, if kind == "numbers" { ListKind::Numbers } else { ListKind::Bullets });
                    changed = true;
                }
            }
            if let Some(id) = e["style"].as_str() {
                changed |= crate::ensure_style(&mut self.doc, id);
            }
            if let (Some(index), Some(section), Some(kind)) = (e["story"].as_u64(), e["section"].as_u64(), e["kind"].as_str()) {
                if word(&self.doc).is_some_and(|w| w.headers.len() == index as usize) {
                    self.add_story(section as usize, if kind == "footer" { StoryKind::Footer } else { StoryKind::Header });
                    changed = true;
                }
            }
        }
        if changed {
            // Only another copy's change does anything here: this copy's own was done already.
            self.others_changed = true;
            self.extras = super::read_extras(&self.doc);
            self.specs.clear();
            self.dirty = Dirty::Full;
            self.stale = true;
        }
    }
}

/// Where another copy's splice `s` replaced blocks (`start..end`), in the frame this copy's
/// own steps in `later` (`true` in `since`) were made in. Those were made one after another,
/// starting from the document `s` was applied to, so `s` moves past each in turn. `None` when one
/// of them changed the same blocks.
fn sender_frame(s: &Splice, story: StoryId, later: &[(bool, &WStep)]) -> Option<(i64, i64)> {
    let (mut start, mut end) = (s.index as i64, s.index as i64 + s.removed.len() as i64);
    for (own, p) in later {
        let WStep::Text(st, p) = p else { continue };
        if !*own || *st != story || p.container != s.container {
            continue;
        }
        let (p_start, p_end) = (p.index as i64, p.index as i64 + p.removed.len() as i64);
        if p_end <= start && p_start < start {
            let delta = p.inserted.len() as i64 - p.removed.len() as i64;
            start += delta;
            end += delta;
        } else if p_start < end || (p_start == start && p_end > start) {
            return None;
        }
    }
    Some((start, end))
}

/// Moves a position past a splice: down when blocks were added above it, along when text was
/// typed before it in its own paragraph.
fn map_pos(p: &mut Pos, s: &Splice) {
    let level = s.container.len();
    if p.path.0.len() <= level || p.path.0[..level] != s.container[..] {
        return;
    }
    let b = p.path.0[level];
    let end = s.index + s.removed.len() as u32;
    if b >= end {
        p.path.0[level] = (b as i64 + s.inserted.len() as i64 - s.removed.len() as i64).max(0) as u32;
    } else if b >= s.index && p.path.0.len() == level + 1 && s.removed.len() == 1 && s.inserted.len() == 1 {
        if let (Block::Paragraph(old), Block::Paragraph(new)) = (&s.removed[0], &s.inserted[0]) {
            let (was, now) = (text::flat(old).text, text::flat(new).text);
            let same = was.bytes().zip(now.bytes()).take_while(|(x, y)| x == y).count() as u32;
            if p.offset > same {
                p.offset = (p.offset as i64 + now.len() as i64 - was.len() as i64).max(same as i64) as u32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use office_core::model::{Body, Document, Paragraph};
    use office_core::{Compiler, Kind};
    use office_engine::pos::Path;

    use super::*;
    use crate::Words;

    fn bytes(paras: &[&str]) -> Vec<u8> {
        let mut doc = Document::new(Kind::Word);
        doc.body = Body::Flow { blocks: paras.iter().map(|t| Block::Paragraph(Paragraph::text(*t))).collect() };
        Compiler::new().with_layer(Words).write(&doc, Kind::Word).unwrap()
    }

    /// Copies of one document, and a relay that orders what they send.
    struct Room {
        views: Vec<WordView>,
        seq: u64,
        /// Sent and not yet delivered, in order.
        queue: Vec<Value>,
    }

    impl Room {
        fn new(paras: &[&str], n: usize) -> Room {
            let b = bytes(paras);
            let views = (0..n)
                .map(|i| {
                    let mut v = WordView::open(&b).unwrap();
                    v.collab_start(&format!("c{i}"));
                    v
                })
                .collect();
            Room { views, seq: 0, queue: Vec::new() }
        }

        /// Sends what copy `i` has to send now: its next change, unless one is still out.
        fn send(&mut self, i: usize) {
            while let Some(op) = self.views[i].collab_take() {
                self.seq += 1;
                self.queue
                    .push(json!({ "seq": self.seq, "client": format!("c{i}"), "id": op["id"], "base": op["base"], "steps": op["steps"] }));
            }
        }

        fn deliver(&mut self) {
            for op in std::mem::take(&mut self.queue) {
                for v in &mut self.views {
                    v.collab_receive(&op);
                }
            }
        }

        /// Puts the last change sent first, as the relay would if it had arrived first.
        fn reorder(&mut self) {
            if let Some(last) = self.queue.pop() {
                self.queue.insert(0, last);
            }
            let first = self.seq + 1 - self.queue.len() as u64;
            for (n, op) in self.queue.iter_mut().enumerate() {
                op["seq"] = json!(first + n as u64);
            }
        }

        /// Delivers what's queued, then sends and delivers until no copy has anything left.
        fn sync(&mut self) {
            loop {
                self.deliver();
                for i in 0..self.views.len() {
                    self.send(i);
                }
                if self.queue.is_empty() {
                    break;
                }
            }
        }

        fn texts(&self, i: usize) -> Vec<String> {
            texts(&self.views[i])
        }

        fn agree(&self) -> Vec<String> {
            let first = self.texts(0);
            for i in 1..self.views.len() {
                assert_eq!(self.texts(i), first, "copy {i} differs");
            }
            first
        }
    }

    fn texts(v: &WordView) -> Vec<String> {
        v.blocks(StoryId::Body)
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => text::flat(p).text,
                _ => "[block]".into(),
            })
            .collect()
    }

    fn caret(v: &mut WordView, block: u32, offset: u32) {
        v.set_selection(Selection::caret(Pos::new(Path::top(block), offset)));
    }

    #[test]
    fn others_see_the_caret_and_selection_where_they_are() {
        let mut host = office_engine::measure::FixedMeasurer::default();
        let mut room = Room::new(&["One", "Two words"], 2);
        room.views[1].set_selection(Selection { anchor: Pos::new(Path::top(1), 1), focus: Pos::new(Path::top(1), 6) });
        let presence = room.views[1].collab_presence();
        let own = room.views[1].overlay(&mut host);
        let seen = room.views[0].remote_overlay(&mut host, &presence);
        assert_eq!(seen.caret, own.caret);
        assert!(!seen.rects.is_empty());
        assert_eq!(seen.rects, own.rects);
        // It travels as JSON.
        let seen = room.views[0].remote_overlay(&mut host, &serde_json::from_str(&presence.to_string()).unwrap());
        assert_eq!(seen.caret, own.caret);

        // A paragraph this copy doesn't have yet hides it; an offset past the end stops there.
        let ahead = json!({ "s": "b", "a": [[5], 0], "f": [[5], 0] });
        assert!(room.views[0].remote_overlay(&mut host, &ahead).caret.is_none());
        let past = json!({ "s": "b", "a": [[0], 99], "f": [[0], 99] });
        room.views[1].set_selection(Selection::caret(Pos::new(Path::top(0), 3)));
        assert_eq!(room.views[0].remote_overlay(&mut host, &past).caret, room.views[1].overlay(&mut host).caret);
        // Only the body is drawn.
        assert!(room.views[0].remote_overlay(&mut host, &json!({ "s": "s0", "a": [[0], 0], "f": [[0], 0] })).caret.is_none());
    }

    #[test]
    fn deleting_blocks_leaves_the_ones_around_them_as_they_were() {
        let mut room = Room::new(&["Title", "Body", "Next"], 2);
        // The first paragraph looks different from the second, on both copies.
        for v in &mut room.views {
            let Body::Flow { blocks } = &mut v.doc.body else { panic!() };
            let Block::Paragraph(p) = &mut blocks[0] else { panic!() };
            p.props.align = Some(office_core::model::Align::Center);
        }
        let second = room.views[1].body()[1].clone();
        assert_eq!(room.views[0].delete_blocks(&[], 0, 0), Ok(()));
        room.sync();
        assert_eq!(room.agree(), ["Body", "Next"]);
        assert_eq!(room.views[1].body()[0], second);
        // Undo puts it back, on every copy.
        assert!(room.views[0].undo());
        room.sync();
        assert_eq!(room.agree(), ["Title", "Body", "Next"]);
        // A list is never emptied, and blocks that aren't there aren't deleted.
        assert_eq!(room.views[0].delete_blocks(&[], 0, 2), Err("all"));
        assert_eq!(room.views[0].delete_blocks(&[], 2, 5), Err("missing"));
        // Nor is a paragraph that ends a section, which holds its page setup.
        let Body::Flow { blocks } = &mut room.views[0].doc.body else { panic!() };
        let Block::Paragraph(p) = &mut blocks[1] else { panic!() };
        let origin = office_core::xml::parse_element(
            r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:pPr><w:sectPr/></w:pPr></w:p>"#,
        )
        .unwrap();
        p.origin = Origin::new(origin);
        assert_eq!(room.views[0].delete_blocks(&[], 1, 1), Err("section"));
        assert_eq!(room.views[0].delete_blocks(&[], 0, 2), Err("section"));
    }

    #[test]
    fn two_copies_deleting_the_last_blocks_at_once_leave_one() {
        for first in [0, 1] {
            let mut room = Room::new(&["One", "Two"], 2);
            assert_eq!(room.views[0].delete_blocks(&[], 0, 0), Ok(()));
            assert_eq!(room.views[1].delete_blocks(&[], 1, 1), Ok(()));
            room.send(first);
            room.send(1 - first);
            room.sync();
            let left = room.agree();
            assert_eq!(left.len(), 1, "{left:?}");
        }
    }

    #[test]
    fn two_copies_adding_paragraphs_at_the_end_at_once_keep_both() {
        for first in [0, 1] {
            let mut room = Room::new(&["Title", "Body"], 2);
            let mine = [("Ada's".to_string(), None)];
            let theirs = [("Bo's".to_string(), Some("Heading1".to_string())), ("More".to_string(), None)];
            assert_eq!(room.views[0].insert_paragraphs(&[], 2, &[1], &mine), Ok(()));
            assert_eq!(room.views[1].insert_paragraphs(&[], 2, &[1], &theirs), Ok(()));
            room.send(first);
            room.send(1 - first);
            room.sync();
            let all = room.agree();
            assert_eq!(all.len(), 5, "{all:?}");
            assert_eq!(&all[..2], ["Title", "Body"]);
            for t in ["Ada's", "Bo's", "More"] {
                assert!(all.iter().any(|x| x == t), "{all:?}");
            }
            // Each keeps its own style, and the one without takes the paragraph before's.
            let style = |t: &str| {
                room.views[0].body().iter().find_map(|b| match b {
                    Block::Paragraph(p) if text::flat(p).text == t => Some(p.props.style.clone()),
                    _ => None,
                })
            };
            assert_eq!(style("Bo's"), Some(Some("Heading1".into())));
            assert_eq!(style("More"), style("Body"));
        }
    }

    #[test]
    fn typing_reaches_every_copy() {
        let mut room = Room::new(&["One", "Two"], 2);
        caret(&mut room.views[0], 0, 3);
        room.views[0].insert_text("!");
        room.sync();
        assert_eq!(room.agree(), ["One!", "Two"]);
        // Origins come along, so the copies save the same file.
        let a = room.views[0].save().unwrap();
        let b = room.views[1].save().unwrap();
        let part = |b: &[u8]| office_core::opc::Package::read(b).unwrap().part("word/document.xml").unwrap().to_vec();
        assert_eq!(String::from_utf8(part(&a)).unwrap(), String::from_utf8(part(&b)).unwrap());
    }

    #[test]
    fn a_paragraph_added_above_moves_typing_down() {
        let mut room = Room::new(&["", "", "Third"], 2);
        // Copy 1 adds a paragraph at the top, copy 0 types into the second empty paragraph,
        // neither having seen the other's change.
        caret(&mut room.views[1], 0, 0);
        room.views[1].enter();
        caret(&mut room.views[0], 1, 0);
        room.views[0].insert_text("typed");
        room.send(1);
        room.send(0);
        room.sync();
        assert_eq!(room.agree(), ["", "", "typed", "Third"]);
        // Copy 0's caret followed its paragraph down.
        assert_eq!(room.views[0].sel.focus, Pos::new(Path::top(2), 5));
    }

    #[test]
    fn the_second_of_two_edits_to_one_paragraph_is_dropped() {
        let mut room = Room::new(&["Same"], 2);
        caret(&mut room.views[0], 0, 4);
        room.views[0].insert_text("A");
        caret(&mut room.views[1], 0, 4);
        room.views[1].insert_text("B");
        room.sync();
        assert_eq!(room.agree(), ["SameA"]);
        assert!(!room.views[1].collab_pending());
    }

    #[test]
    fn pending_changes_survive_others_arriving() {
        let mut room = Room::new(&["One", "Two"], 2);
        caret(&mut room.views[0], 1, 3);
        room.views[0].insert_text("x");
        room.send(0);
        // Copy 0 keeps typing before its first change comes back.
        room.views[0].insert_text("y");
        caret(&mut room.views[1], 0, 0);
        room.views[1].insert_text(">");
        room.send(1);
        room.send(0);
        room.sync();
        assert_eq!(room.agree(), [">One", "Twoxy"]);
    }

    #[test]
    fn undo_takes_back_only_your_change_where_it_is_now() {
        let mut room = Room::new(&["One", "Two"], 2);
        caret(&mut room.views[0], 1, 3);
        room.views[0].insert_text(" more");
        room.sync();
        caret(&mut room.views[1], 0, 0);
        room.views[1].enter();
        room.sync();
        assert_eq!(room.agree(), ["", "One", "Two more"]);
        assert!(room.views[0].undo());
        room.sync();
        assert_eq!(room.agree(), ["", "One", "Two"]);
    }

    #[test]
    fn a_new_list_brings_its_numbering() {
        let mut room = Room::new(&["Item"], 2);
        caret(&mut room.views[0], 0, 0);
        room.views[0].list(Some(ListKind::Numbers));
        room.sync();
        let num = |v: &WordView| match &v.blocks(StoryId::Body)[0] {
            Block::Paragraph(p) => crate::list_of(&v.doc, p).map(|(n, _)| n),
            _ => None,
        };
        let n = num(&room.views[0]).expect("a list");
        assert_eq!(num(&room.views[1]).as_deref(), Some(n.as_str()));
        assert!(word(&room.views[1].doc).unwrap().numbering.levels(&n).is_some());
    }

    #[test]
    fn others_changes_are_unsaved_changes() {
        let mut room = Room::new(&["One"], 2);
        caret(&mut room.views[1], 0, 3);
        room.views[1].insert_text("!");
        room.sync();
        assert!(room.views[0].is_dirty());
        room.views[0].mark_saved();
        assert!(!room.views[0].is_dirty());
    }

    #[test]
    fn the_losing_edit_to_an_empty_paragraph_doesnt_land_on_its_neighbor() {
        let mut room = Room::new(&["", "", "Third"], 2);
        caret(&mut room.views[0], 1, 0);
        room.views[0].insert_text("A");
        caret(&mut room.views[1], 1, 0);
        room.views[1].insert_text("B");
        room.sync();
        assert_eq!(room.agree(), ["", "A", "Third"]);
    }

    #[test]
    fn typing_on_a_new_line_survives_an_edit_below_it() {
        let mut room = Room::new(&["One", "Two"], 2);
        caret(&mut room.views[1], 1, 3);
        room.views[1].insert_text("!");
        room.send(1);
        // Copy 0 splits the first paragraph and types on the new line, both before copy 1's
        // edit arrives; its typing has the same index copy 1's edit had.
        caret(&mut room.views[0], 0, 3);
        room.views[0].enter();
        room.send(0);
        room.views[0].insert_text("x");
        room.send(0);
        room.sync();
        assert_eq!(room.agree(), ["One", "x", "Two!"]);
    }

    #[test]
    fn others_changes_stay_unsaved_after_sharing_stops() {
        let mut room = Room::new(&["One"], 2);
        caret(&mut room.views[1], 0, 3);
        room.views[1].insert_text("!");
        room.sync();
        room.views[0].collab_stop();
        assert!(room.views[0].is_dirty());
    }

    #[test]
    fn typing_in_a_paragraph_someone_joined_is_dropped() {
        let mut room = Room::new(&["One", "Same", "Same"], 2);
        // Copy 0 joins the first "Same" onto "One" while copy 1 types in it.
        caret(&mut room.views[0], 1, 0);
        room.views[0].backspace(false);
        caret(&mut room.views[1], 1, 4);
        room.views[1].insert_text("!");
        room.sync();
        assert_eq!(room.agree(), ["OneSame", "Same"]);
    }

    /// Three copies typing, splitting and joining paragraphs at random, sending and receiving
    /// in a random order: they always end up the same.
    #[test]
    fn random_edits_converge() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n.max(1)
        };
        for _ in 0..40 {
            let mut room = Room::new(&["Alpha", "", "Beta", "", "Gamma"], 3);
            for _ in 0..60 {
                let i = next(3) as usize;
                match next(6) {
                    0..=2 => {
                        let v = &mut room.views[i];
                        let paras = v.blocks(StoryId::Body).len() as u64;
                        let p = next(paras) as u32;
                        let len = match &v.blocks(StoryId::Body)[p as usize] {
                            Block::Paragraph(para) => text::len(para) as u64,
                            _ => 0,
                        };
                        caret(v, p, next(len + 1) as u32);
                        match next(4) {
                            0 => v.enter(),
                            1 => v.backspace(false),
                            _ => v.insert_text(["a", "b", "c"][next(3) as usize]),
                        }
                    }
                    3 => room.send(i),
                    4 => {
                        if !room.queue.is_empty() {
                            let op = room.queue.remove(0);
                            for v in &mut room.views {
                                v.collab_receive(&op);
                            }
                        }
                    }
                    _ => {
                        let _ = room.views[i].undo();
                    }
                }
            }
            room.sync();
            room.sync();
            room.agree();
        }
    }

    #[test]
    fn typing_lands_where_it_was_made_past_several_changes() {
        let mut room = Room::new(&["Alpha", "Bravo", "Charlie", "Delta", "Same", "Same"], 2);
        // Copy 0 joins Charlie and Delta, then splits Alpha.
        caret(&mut room.views[0], 3, 0);
        room.views[0].backspace(false);
        room.send(0);
        caret(&mut room.views[0], 0, 2);
        room.views[0].enter();
        room.send(0);
        // Before either arrives, copy 1 splits Bravo and types in the first Same.
        caret(&mut room.views[1], 1, 2);
        room.views[1].enter();
        room.send(1);
        caret(&mut room.views[1], 5, 4);
        room.views[1].insert_text("!");
        room.send(1);
        room.sync();
        room.sync();
        let texts = room.agree();
        assert_eq!(texts[texts.len() - 2..], ["Same!", "Same"]);
    }

    #[test]
    fn typing_queued_after_a_lost_edit_goes_with_it() {
        let mut room = Room::new(&["Same", "Same"], 2);
        // Copy 0's edit to the first paragraph is out; its typing in the second waits.
        caret(&mut room.views[0], 0, 4);
        room.views[0].insert_text("a");
        room.send(0);
        caret(&mut room.views[0], 1, 4);
        room.views[0].insert_text("b");
        // Copy 1 splits the first paragraph at its start, and the relay puts that first.
        caret(&mut room.views[1], 0, 0);
        room.views[1].enter();
        room.send(1);
        room.reorder();
        room.sync();
        // Copy 0's edits are dropped, not put on the paragraph that looks like the first.
        assert_eq!(room.agree(), ["", "Same", "Same"]);
    }

    #[test]
    fn a_dropped_changes_new_style_still_reaches_the_others() {
        let mut room = Room::new(&["One", "Two"], 2);
        caret(&mut room.views[0], 0, 3);
        room.views[0].insert_text("a");
        room.send(0);
        // Waiting behind it: a style used for the first time, on the same paragraph.
        room.views[0].set_style("Title");
        // Copy 1's edit to that paragraph is ordered first, so both of copy 0's are dropped.
        caret(&mut room.views[1], 0, 0);
        room.views[1].insert_text(">");
        room.send(1);
        room.reorder();
        room.sync();
        room.agree();
        assert!(word(&room.views[1].doc).unwrap().styles.get("Title").is_some());
    }
}
