//! A presentation shared with other copies (see `office_engine::collab`).
//!
//! Steps go as what a change replaced — a slide, a slide's notes, or the whole list of slides —
//! by fingerprint, and what replaced it. A slide changed while another copy added or moved
//! slides is found by looking for the nearest slide with the fingerprint; the list itself needs
//! an exact match.
//!
//! Some changes touch the layer's own tables, outside the undo steps: duplicating a slide also
//! copies its speaker notes, which live by slide part name. They're sent along with the change
//! (`extra`), and doing it again is a no-op where it's been done.

use office_core::model::{Document, Paragraph, Slide};
use office_engine::collab::{self, fingerprint, send_content, take_content, Replica, Shared};
use serde_json::{json, Value};

use super::{SStep, Sel, SlidesView};

impl SStep {
    /// The step that takes the change back.
    pub(crate) fn inverse(&self) -> SStep {
        match self {
            SStep::Slide { index, before, after } => SStep::Slide { index: *index, before: after.clone(), after: before.clone() },
            SStep::Deck { before, after } => SStep::Deck { before: after.clone(), after: before.clone() },
            SStep::Notes { index, before, after } => SStep::Notes { index: *index, before: after.clone(), after: before.clone() },
            SStep::Design { before, after } => SStep::Design { before: after.clone(), after: before.clone() },
        }
    }
}

impl SlidesView {
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
            self.goal_x = None;
            self.sync_caret();
        }
        changed
    }

    pub fn collab_pending(&self) -> bool {
        self.replica.as_ref().is_some_and(|r| r.has_pending())
    }

    /// Notes steps made here for sending, when shared.
    pub(crate) fn shared_made(&mut self, steps: &[SStep]) {
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

    /// Sends the speaker notes copied slides take with them, when shared (see `take_extra`).
    pub(crate) fn share_copied_notes(&mut self, parts: &[Option<String>]) {
        if self.replica.is_none() {
            return;
        }
        for part in parts.iter().flatten() {
            let paras = crate::notes_for_part(&self.doc, part);
            self.shared_extra(json!({ "notes": part, "after": send_content(&Vec::<Paragraph>::new(), &paras) }));
        }
    }

    /// Adds a picture, under an id unique among the copies when shared.
    pub(crate) fn add_media(&mut self, content_type: &str, bytes: Vec<u8>) -> String {
        collab::add_media(&mut self.doc, self.replica.as_ref(), content_type, bytes)
    }

    /// Undoes or redoes an entry's steps where the document is now; `false` if someone else's
    /// change is in the way.
    pub(crate) fn shared_replay(&mut self, steps: Vec<SStep>) -> bool {
        match collab::replay(self, &steps) {
            Some(done) => {
                self.shared_made(&done);
                true
            }
            None => false,
        }
    }

    /// A seed for naming new slide parts, when shared: this copy's name, so another copy's part
    /// names can't collide with it (see `crate::free_part_in`).
    pub(crate) fn new_part_seed(&self) -> Option<String> {
        self.replica.as_ref().map(|r| format!("new-{}", r.tag()))
    }

    /// A seed for naming a partless slide's part, when shared: the slide itself, which is the
    /// same on every copy the change lands on, so every copy picks the same name.
    pub(crate) fn notes_part_seed(&self, index: usize) -> Option<String> {
        self.replica.as_ref()?;
        self.slides().get(index).map(fingerprint)
    }

    /// The slide a step points at: `index`, or the nearest slide holding its `before`
    /// fingerprint when slides were added or moved since.
    fn slide_at(&self, index: usize, before: &str) -> Option<usize> {
        let n = self.slides().len();
        (0..n)
            .flat_map(|d| [index.saturating_add(d), index.wrapping_sub(d)])
            .find(|&i| self.slides().get(i).is_some_and(|s| fingerprint(s) == before))
    }

    /// The slide whose notes a step points at, located like `slide_at` but by their fingerprint.
    fn notes_at(&self, index: usize, before: &str) -> Option<usize> {
        let n = self.slides().len();
        (0..n)
            .flat_map(|d| [index.saturating_add(d), index.wrapping_sub(d)])
            .find(|&i| self.slides().get(i).is_some() && fingerprint(&crate::notes(&self.doc, i)) == before)
    }

    fn take_slide(&mut self, step: &Value) -> Option<SStep> {
        let index = step["index"].as_u64()? as usize;
        let at = self.slide_at(index, step["before"].as_str()?)?;
        let live = self.slides()[at].clone();
        let after: Slide = take_content(&live, &step["after"])?;
        self.slides_mut()[at] = after.clone();
        self.touch(at);
        Some(SStep::Slide { index: at, before: Box::new(live), after: Box::new(after) })
    }

    fn take_deck(&mut self, step: &Value) -> Option<SStep> {
        if fingerprint(self.slides()) != step["before"].as_str()? {
            return None;
        }
        let before = self.slides().to_vec();
        let after: Vec<Slide> = take_content(&before, &step["after"])?;
        *self.slides_mut() = after.clone();
        self.touch_all();
        Some(SStep::Deck { before, after })
    }

    fn take_notes(&mut self, step: &Value) -> Option<SStep> {
        let index = step["index"].as_u64()? as usize;
        let at = self.notes_at(index, step["before"].as_str()?)?;
        let live = crate::notes(&self.doc, at);
        let after: Vec<Paragraph> = take_content(&live, &step["after"])?;
        // The replica is out of the view while a change applies, so seed from the slide itself:
        // partless slides agree on every copy, so every copy names the part the same.
        let seed = self.slides().get(at).map(fingerprint);
        crate::set_notes_in(&mut self.doc, at, after.clone(), seed.as_deref());
        self.touch(at);
        Some(SStep::Notes { index: at, before: live, after })
    }
}

impl Shared for SlidesView {
    type Step = SStep;
    type Mark = Sel;

    fn replica(&mut self) -> &mut Option<Replica<SStep>> {
        &mut self.replica
    }

    fn doc(&mut self) -> &mut Document {
        &mut self.doc
    }

    fn send_step(&self, step: &SStep) -> Value {
        match step {
            SStep::Slide { index, before, after } => json!({
                "k": "slide",
                "index": index,
                "before": fingerprint(&**before),
                "after": send_content(&**before, &**after),
            }),
            SStep::Deck { before, after } => json!({
                "k": "deck",
                "before": fingerprint(before),
                "after": send_content(before, after),
            }),
            SStep::Notes { index, before, after } => json!({
                "k": "notes",
                "index": index,
                "before": fingerprint(before),
                "after": send_content(before, after),
            }),
            SStep::Design { .. } => Value::Null,
        }
    }

    fn take_step(&mut self, step: &Value, _since: Option<&[(bool, &SStep)]>) -> Option<SStep> {
        match step["k"].as_str()? {
            "slide" => self.take_slide(step),
            "deck" => self.take_deck(step),
            "notes" => self.take_notes(step),
            _ => None,
        }
    }

    fn revert_step(&mut self, step: &SStep) {
        self.apply(step, true);
    }

    fn mark(&self) -> Sel {
        self.sel.clone()
    }

    fn restore(&mut self, mark: Sel, through: &[SStep]) {
        let mut sel = mark;
        for step in through {
            // Keep the selection on the same slide when slides were added or removed before it.
            if let SStep::Deck { before, after } = step {
                if let Some(was) = before.get(sel.active) {
                    sel.active =
                        after.iter().position(|s| if was.part.is_some() { s.part == was.part } else { s == was }).unwrap_or(sel.active);
                }
            }
        }
        self.sel = sel;
        self.clamp_sel();
    }

    fn send_extra(&mut self) -> Value {
        if self.collab_extra.is_empty() {
            return Value::Null;
        }
        Value::Array(std::mem::take(&mut self.collab_extra))
    }

    fn take_extra(&mut self, extra: &Value) {
        for e in extra.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if let Some(part) = e["notes"].as_str() {
                let paras: Option<Vec<Paragraph>> = take_content(&Vec::<Paragraph>::new(), &e["after"]);
                if let Some(paras) = paras {
                    crate::put_notes(&mut self.doc, part.to_string(), paras);
                    if let Some(i) = self.slides().iter().position(|s| s.part.as_deref() == Some(part)) {
                        self.touch(i);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use office_core::drawing::Xfrm;
    use office_core::model::{Body, Document, Frame, FrameContent, Paragraph, Slide};
    use office_core::units::Emu;
    use office_core::{Compiler, Kind};
    use office_engine::pos::{Path, Pos, Selection};

    use super::*;
    use crate::Decks;

    fn deck(texts: &[&[&str]]) -> Vec<u8> {
        let mut doc = Document::new(Kind::Slides);
        let slides = texts
            .iter()
            .map(|frames| Slide {
                frames: frames
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        Frame::text_box(
                            i as u32 + 2,
                            Xfrm {
                                x: Emu::from_inches(1.0),
                                y: Emu::from_inches(1.0 + i as f64 * 1.5),
                                width: Emu::from_inches(6.0),
                                height: Emu::from_inches(1.0),
                                ..Default::default()
                            },
                            vec![Paragraph::text(*t)],
                        )
                    })
                    .collect(),
                ..Default::default()
            })
            .collect();
        doc.body = Body::Slides { size: Default::default(), slides };
        Compiler::new().with_layer(Decks).write(&doc, Kind::Slides).unwrap()
    }

    // A red 1x1 PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63,
        0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
        0xAE, 0x42, 0x60, 0x82,
    ];

    /// Copies of one presentation, and a relay that orders what they send.
    struct Room {
        views: Vec<SlidesView>,
        seq: u64,
        /// Sent and not yet delivered, in order.
        queue: Vec<Value>,
    }

    impl Room {
        fn new(texts: &[&[&str]], n: usize) -> Room {
            let b = deck(texts);
            let views = (0..n)
                .map(|i| {
                    let mut v = SlidesView::open(&b).unwrap();
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

        /// Every copy holds the same slides (parts included) and the same notes.
        fn agree(&self) -> Vec<String> {
            let first = state(&self.views[0]);
            for (i, v) in self.views.iter().enumerate().skip(1) {
                assert_eq!(state(v), first, "copy {i} differs");
            }
            self.texts(0)
        }
    }

    /// Each slide's frame text, paragraphs on their own lines.
    fn texts(v: &SlidesView) -> Vec<String> {
        v.slides()
            .iter()
            .map(|s| s.frames.iter().flat_map(|f| f.paragraphs()).map(|p| p.plain_text()).collect::<Vec<_>>().join("\n"))
            .collect()
    }

    /// Each slide's part, text and notes, for comparing copies.
    fn state(v: &SlidesView) -> Vec<String> {
        v.slides()
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let notes = crate::notes(&v.doc, i).iter().map(|p| p.plain_text()).collect::<Vec<_>>().join("\n");
                format!("{}|{}|{}", s.part.as_deref().unwrap_or(""), texts(v)[i], notes)
            })
            .collect()
    }

    /// Puts a caret in `slide`'s frame `frame` at `offset` in its first paragraph and types.
    fn type_in(v: &mut SlidesView, slide: usize, frame: u32, offset: u32, s: &str) {
        v.set_active(slide);
        assert!(v.select_text(frame, Selection::caret(Pos::new(Path::top(0), offset))));
        v.insert_text(s);
    }

    #[test]
    fn others_see_the_frames_picked_and_the_caret_in_text() {
        let mut host = office_engine::measure::FixedMeasurer::default();
        let mut room = Room::new(&[&["Hello world", "Below"], &["Second"]], 2);
        room.views[1].set_active(0);
        room.views[1].select_frames(vec![3]);
        let presence = room.views[1].collab_presence();
        let seen = room.views[0].remote_overlay(&mut host, &presence);
        let own = room.views[1].selected_boxes(&mut host);
        assert_eq!(seen.boxes.len(), 1);
        assert_eq!((seen.boxes[0].id.as_str(), seen.boxes[0].x, seen.boxes[0].y), ("3", own[0].1.x, own[0].1.y));
        assert!(seen.text.is_none());

        // Editing text: the caret or selection shows where it does on that copy.
        assert!(room.views[1].select_text(2, Selection { anchor: Pos::new(Path::top(0), 2), focus: Pos::new(Path::top(0), 7) }));
        let presence = room.views[1].collab_presence();
        let own = room.views[1].text_overlay(&mut host).unwrap();
        let seen = room.views[0].remote_overlay(&mut host, &serde_json::from_str(&presence.to_string()).unwrap());
        let text = seen.text.unwrap();
        assert_eq!((text.frame.as_str(), text.rects.clone()), ("2", own.rects));
        assert!(!text.rects.is_empty());

        // Someone on another slide isn't drawn on this one.
        room.views[1].set_active(1);
        let presence = room.views[1].collab_presence();
        let seen = room.views[0].remote_overlay(&mut host, &presence);
        assert!(seen.boxes.is_empty() && seen.text.is_none());
        room.views[0].set_active(1);
        assert_eq!(room.views[0].remote_overlay(&mut host, &presence).boxes.len(), 0);
        assert_eq!(presence["slide"], json!(room.views[0].slide_id(1)));
    }

    #[test]
    fn typing_in_a_text_box_reaches_every_copy() {
        let mut room = Room::new(&[&["Hello"], &["Second"]], 2);
        type_in(&mut room.views[0], 0, 2, 5, "!");
        room.sync();
        assert_eq!(room.agree(), ["Hello!", "Second"]);
        // Origins come along, so the copies save the same file.
        let a = room.views[0].save().unwrap();
        let b = room.views[1].save().unwrap();
        let part = |b: &[u8]| office_core::opc::Package::read(b).unwrap().part("ppt/slides/slide1.xml").unwrap().to_vec();
        assert_eq!(String::from_utf8(part(&a)).unwrap(), String::from_utf8(part(&b)).unwrap());
    }

    #[test]
    fn edits_to_different_slides_both_survive() {
        let mut room = Room::new(&[&["One"], &["Two"]], 2);
        type_in(&mut room.views[0], 0, 2, 3, "!");
        type_in(&mut room.views[1], 1, 2, 3, "~");
        room.sync();
        assert_eq!(room.agree(), ["One!", "Two~"]);
    }

    #[test]
    fn the_second_of_two_edits_to_one_slide_is_dropped() {
        let mut room = Room::new(&[&["Same"], &["Other"]], 2);
        type_in(&mut room.views[0], 0, 2, 4, "A");
        type_in(&mut room.views[1], 0, 2, 4, "B");
        room.sync();
        assert_eq!(room.agree(), ["SameA", "Other"]);
        assert!(!room.views[1].collab_pending());
    }

    #[test]
    fn a_slide_added_elsewhere_moves_typing_to_the_right_slide() {
        let mut room = Room::new(&[&["One"], &["Two"], &["Three"]], 2);
        // Copy 0 adds a slide after the first; copy 1 types into the second, neither having
        // seen the other's change.
        room.views[0].add_slide(0, None);
        type_in(&mut room.views[1], 1, 2, 3, "!");
        room.sync();
        let texts = room.agree();
        assert_eq!(texts.len(), 4);
        assert_eq!([texts[0].as_str(), texts[2].as_str(), texts[3].as_str()], ["One", "Two!", "Three"]);
        // Copy 1's caret followed its slide down.
        assert_eq!(room.views[1].sel.active, 2);
    }

    #[test]
    fn undo_takes_back_only_your_change_where_it_is_now() {
        let mut room = Room::new(&[&["One"], &["Two"]], 2);
        type_in(&mut room.views[0], 0, 2, 3, " more");
        room.sync();
        room.views[1].add_slide(0, None);
        room.views[1].move_slide(1, 0);
        room.sync();
        assert_eq!(room.agree()[1..], ["One more", "Two"]);
        assert!(room.views[0].undo());
        room.sync();
        assert_eq!(room.agree()[1..], ["One", "Two"]);
    }

    #[test]
    fn a_picture_inserted_on_one_copy_shows_up_on_the_other() {
        let mut room = Room::new(&[&["One"]], 2);
        let frame = room.views[0].insert_image(PNG.to_vec(), "image/png", 10.0, 10.0, "Picture");
        let media = match &room.views[0].slides()[0].frames.iter().find(|f| f.id == frame).unwrap().content {
            FrameContent::Picture { image } => image.media.clone(),
            _ => unreachable!(),
        };
        room.sync();
        assert_eq!(&*room.views[1].media(&media).unwrap(), PNG);
        assert!(matches!(room.views[1].slides()[0].frames.last().map(|f| &f.content), Some(FrameContent::Picture { .. })));
    }

    #[test]
    fn notes_reach_the_other_copy() {
        let mut room = Room::new(&[&["One"], &["Two"]], 2);
        room.views[0].set_notes(1, "Remember the demo");
        room.sync();
        room.agree();
        let notes = |v: &SlidesView| crate::notes(&v.doc, 1).iter().map(|p| p.plain_text()).collect::<Vec<_>>();
        assert_eq!(notes(&room.views[1]), ["Remember the demo"]);
    }

    #[test]
    fn a_partless_slide_names_its_part_the_same_on_every_copy() {
        let mut room = Room::new(&[&["One"]], 2);
        // Deleting the only slide leaves a blank one, which has no part name until it needs one.
        room.views[0].delete_slides(&[0]);
        room.sync();
        assert!(room.views[0].slides()[0].part.is_none());
        room.views[0].set_notes(0, "First notes");
        room.sync();
        room.agree();
        assert!(room.views[1].slides()[0].part.is_some());
    }

    #[test]
    fn a_duplicated_slide_keeps_its_notes() {
        let mut room = Room::new(&[&["One"], &["Two"]], 2);
        room.views[0].set_notes(0, "Copy me");
        room.sync();
        room.views[0].duplicate_slides(&[0]);
        room.sync();
        assert_eq!(room.agree().len(), 3);
        let notes = |v: &SlidesView| crate::notes(&v.doc, 1).iter().map(|p| p.plain_text()).collect::<Vec<_>>();
        assert_eq!(notes(&room.views[1]), ["Copy me"]);
    }
}
