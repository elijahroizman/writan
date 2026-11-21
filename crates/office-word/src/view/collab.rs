//! A Word document shared with other copies (see `office_engine::collab`).
//!
//! Steps go as the blocks a splice replaced (by fingerprint) and the blocks that replaced them.
//! A splice made before someone else's splice above it in the same list moves down past it,
//! the way an index in a list moves when items are added before it.
//!
//! Some changes touch the layer's own tables, outside the undo steps: a new list's numbering,
//! a built-in style first used, a new header or footer. They're sent along with the change as
//! what to do again (`extra`), and doing it again is a no-op where it's been done.

use office_core::model::{Block, Origin};
use office_core::xml::parse_element;
use office_engine::collab::{self, fingerprint, send_content, take_content, Replica, Shared};
use office_engine::edit::Splice;
use office_engine::pos::{self, Pos, Selection};
use office_engine::text;
use serde_json::{json, Value};

use super::{Dirty, StoryId, WStep, WordView};
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

    fn take_text(&mut self, step: &Value, since: Option<&[&WStep]>) -> Option<WStep> {
        let story = story_from(step["story"].as_str()?)?;
        if !self.has_story(story) {
            return None;
        }
        let mut container: Vec<u32> = serde_json::from_value(step["at"].clone()).ok()?;
        let mut index = step["index"].as_u64()? as i64;
        let n = step["n"].as_u64()? as usize;
        let before = step["before"].as_str()?;
        // Past what others changed before it, in the same list or a list above. Then what it
        // replaced must be exactly there: if someone changed those blocks first, it's dropped.
        for s in since.unwrap_or(&[]) {
            let WStep::Text(st, s) = s else { continue };
            if *st != story {
                continue;
            }
            let delta = s.inserted.len() as i64 - s.removed.len() as i64;
            let end = (s.index as usize + s.removed.len()) as i64;
            if s.container == container {
                if end <= index {
                    index += delta;
                }
            } else if container.len() > s.container.len() && container[..s.container.len()] == s.container[..] {
                let at = &mut container[s.container.len()];
                if end <= *at as i64 {
                    *at = (*at as i64 + delta).max(0) as u32;
                }
            }
        }
        let list = pos::blocks_at(self.blocks(story), &container)?;
        let fits = |i: i64| i >= 0 && (i as usize) + n <= list.len() && fingerprint(&list[i as usize..i as usize + n]) == before;
        let index = if n == 0 {
            index.clamp(0, list.len() as i64)
        } else if since.is_some() {
            Some(index).filter(|&i| fits(i))?
        } else {
            // Undo: other copies may have moved it since, by an unknown amount.
            (0..=NEAR).flat_map(|d| [index + d, index - d]).find(|&i| fits(i))?
        } as usize;
        let old = list[index..index + n].to_vec();
        let inserted: Vec<Block> = take_content(&old, &step["after"])?;
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

    fn take_step(&mut self, step: &Value, since: Option<&[&WStep]>) -> Option<WStep> {
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

        /// Sends what copy `i` changed.
        fn send(&mut self, i: usize) {
            if let Some(op) = self.views[i].collab_take() {
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

        fn sync(&mut self) {
            for i in 0..self.views.len() {
                self.send(i);
            }
            self.deliver();
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
        room.deliver();
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
        room.deliver();
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
        room.deliver();
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
}
