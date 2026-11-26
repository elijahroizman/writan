//! The Word layer's view: a document laid out on pages, drawn, and edited. It sits on the shared
//! engine (`office-engine`) the way the layer's model code sits on the compiler's core: the
//! engine breaks lines, lays out tables and edits text; this adds what only Word has: sections
//! and pages, headers and footers, styles and lists, fields and page numbers.

mod collab;
mod commands;
pub mod extras;
mod fingerprint;
pub mod math;
mod paginate;
pub mod resolve;
mod state;

use std::collections::HashMap;

use office_core::model::{Block, Body, Document, Inline, Paragraph};
use office_core::xml::ns;
use office_core::Compiler;
use office_engine::display::{self, Item, Objects};
use office_engine::edit::Splice;
use office_engine::flow::{self, BlockBox, Cache, FlowLayout};
use office_engine::history::{History, Step};
use office_engine::measure::{Measure, Measurer};
use office_engine::para::{Ctx, Styles};
use office_engine::pos::{self, Path, Pos, Selection};
use office_engine::text;
use serde::Serialize;

use crate::sections::Section;
use crate::{word, StoryKind, Words};
pub use commands::{image_size, BorderScope, LinkedCell, Move, TableOp, Toggle, EXCEL_LINK_PREFIX, LATEX_MATH_PREFIX};
use extras::Extras;
use resolve::{emu_px, PageFields, WordResolver};
pub use state::*;

/// Which story the caret is in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum StoryId {
    Body,
    /// A header or footer: its index in the layer's `headers`.
    Story(usize),
    /// A footnote or endnote: its index in the layer's `notes`.
    Note(usize),
}

/// One change, for undo.
#[derive(Clone, Debug)]
pub enum WStep {
    Text(StoryId, Splice),
    /// A body paragraph's origin (section settings live there) or, for `None`, the document's.
    Origin {
        block: Option<u32>,
        before: office_core::model::Origin,
        after: office_core::model::Origin,
    },
}

impl Step for WStep {
    fn absorb(&mut self, next: &Self) -> bool {
        match (self, next) {
            (WStep::Text(a, s), WStep::Text(b, n)) if a == b => s.absorb(n),
            _ => false,
        }
    }
}

/// A section's share of the body, laid out.
pub(crate) struct SectionFlow {
    pub section: Section,
    pub first: u32,
    pub last: u32,
    pub flow: FlowLayout,
}

/// A page: which section it's in and which stretch of the section's flow it shows.
#[derive(Clone, Debug)]
pub struct Page {
    pub section: usize,
    pub number: u32,
    pub width: f64,
    pub height: f64,
    /// The stretch of the section's flow on this page.
    pub top: f64,
    pub bottom: f64,
    /// Where the body's text starts on the page.
    pub body_x: f64,
    pub body_y: f64,
    pub header: Option<usize>,
    pub footer: Option<usize>,
    /// The footnotes on this page, in order: those continuing from the page before, then those
    /// referenced here.
    pub footnotes: Vec<NoteSlice>,
    /// Where the footnotes end: the bottom of the body's area.
    pub notes_bottom: f64,
    /// The bottom of the last line or row on the page, in the flow's coordinates.
    pub content_bottom: f64,
    /// The endnotes on this page (the document's last, and any pages added for them), and where
    /// on the page the first one starts.
    pub endnotes: Vec<NoteSlice>,
    pub endnotes_y: f64,
}

/// A stretch of a note's text on a page, in its flow's coordinates: all of it, or the part that
/// fits before it continues on the next page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteSlice {
    /// Its index in the layer's `notes`.
    pub note: usize,
    pub top: f64,
    pub bottom: f64,
}

/// The space above a page's footnotes, with the separator line across its middle.
pub(crate) const NOTE_SEPARATOR: f64 = 16.0;

pub struct WordView {
    pub doc: Document,
    pub(crate) extras: Extras,
    pub(crate) measure: Measure,
    pub(crate) styles: Styles,
    pub(crate) cache: Cache,
    pub(crate) specs: resolve::SpecCache,
    pub(crate) sections: Vec<SectionFlow>,
    pub(crate) pages: Vec<Page>,
    pub(crate) labels: HashMap<Path, String>,
    pub(crate) story: StoryId,
    /// The page whose header or footer is being edited.
    pub(crate) story_page: usize,
    pub(crate) sel: Selection,
    pub(crate) history: History<WStep, (StoryId, Selection)>,
    /// Formatting picked with a caret, for the next text typed.
    pub(crate) pending: Option<office_core::model::RunProps>,
    /// Where up and down arrows aim, kept across a run of them.
    pub(crate) goal_x: Option<f64>,
    /// Where the caret was clicked (page, x), for drawing it on that side of a change of text
    /// direction, where one offset shows in two places.
    pub(crate) caret_near: Option<(usize, f64)>,
    pub(crate) saved: u64,
    pub(crate) stale: bool,
    /// Styles the host has been sent.
    pub(crate) styles_sent: usize,
    /// Goes up whenever the document (or how it's laid out) changes, so the host knows to draw
    /// its pages again; moving the caret doesn't change it.
    pub(crate) revision: u64,
    /// Ranges of body text highlighted over the page: find's matches, comments' text.
    pub(crate) marks: Vec<Mark>,
    /// What changed in the body since the last layout (see [`Dirty`]).
    pub(crate) dirty: Dirty,
    /// The body was handed out for changing and no change was recorded: lay it all out again.
    pub(crate) unrecorded: bool,
    /// Which list each paragraph is in, by its markup, style and level (see `body_labels`).
    pub(crate) list_memo: ListMemo,
    /// Each body paragraph's list and level at the last layout.
    pub(crate) last_lists: Vec<Option<(String, u8)>>,
    /// True when the body contains only top-level paragraphs (1:1 with `last_lists`).
    pub(crate) flat_body_paras: bool,
    /// Set while the document is shared with other copies (see `collab`).
    /// Another copy changed the document since it was saved here, while shared. Their changes
    /// aren't in the undo history, which [`Self::is_dirty`] otherwise goes by.
    pub(crate) others_changed: bool,
    pub(crate) replica: Option<office_engine::collab::Replica<WStep>>,
    /// Changes to the layer's tables to send with the next shared change.
    pub(crate) collab_extra: Vec<serde_json::Value>,
}

/// What changed in the body since it was last laid out, so a layout pass can keep every other
/// block's laid-out box.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Dirty {
    Clean,
    /// Top-level blocks `first..end` (as the body is now) changed, and the body has `delta` more
    /// blocks than when it was laid out.
    Blocks {
        first: u32,
        end: u32,
        delta: i64,
    },
    /// Anything else: lay it all out.
    Full,
}

impl Dirty {
    /// Notes that `removed` top-level blocks at `index` were replaced by `inserted` ones.
    pub(crate) fn splice(&mut self, index: u32, removed: u32, inserted: u32) {
        let d = inserted as i64 - removed as i64;
        let (i, end_new) = (index, index + inserted);
        *self = match *self {
            Dirty::Full => Dirty::Full,
            Dirty::Clean => Dirty::Blocks { first: i, end: end_new, delta: d },
            Dirty::Blocks { first, end, delta } => {
                // The earlier range, in terms of the body after this change.
                let end = if end >= index + removed { (end as i64 + d).max(0) as u32 } else { end.max(end_new) };
                Dirty::Blocks { first: first.min(i), end: end.max(end_new), delta: delta + d }
            }
        }
    }

    /// Notes a change to a splice's container: the top-level block it's in, or the body itself.
    pub(crate) fn note(&mut self, splice: &Splice, undo: bool) {
        match splice.container.first() {
            Some(top) => self.splice(*top, 1, 1),
            None => {
                let (removed, inserted) = (splice.removed.len() as u32, splice.inserted.len() as u32);
                if undo {
                    self.splice(splice.index, inserted, removed)
                } else {
                    self.splice(splice.index, removed, inserted)
                }
            }
        }
    }
}

/// A highlighted range of body text, in a named set.
#[derive(Clone, Debug)]
pub struct Mark {
    pub set: String,
    /// What it's for (a comment's id), for telling which mark is under the pointer.
    pub id: String,
    pub from: Pos,
    pub to: Pos,
}

/// Whether `p` is still a place a mark may end: an existing paragraph, at a character boundary.
fn mark_pos_valid(blocks: &[Block], p: &Pos) -> bool {
    pos::para(blocks, &p.path).is_some_and(|q| {
        let flat = text::flat(q);
        let offset = p.offset as usize;
        offset <= flat.text.len() && flat.text.is_char_boundary(offset)
    })
}

/// One page drawn: its size and items.
#[derive(Clone, Debug, Serialize)]
pub struct PagePaint {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
}

/// Where the caret and selection show, by page.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Overlay {
    /// (page, x, y, height)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret: Option<(usize, f64, f64, f64)>,
    /// (page, x, y, width, height)
    pub rects: Vec<(usize, f64, f64, f64, f64)>,
    /// A selected picture: (page, x, y, width, height).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<(usize, f64, f64, f64, f64)>,
    /// Marked text: (page, x, y, width, height, set, id).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<(usize, f64, f64, f64, f64, String, String)>,
    /// The paragraph holding the caret, one box per page it's on: (page, x, y, width, height).
    /// For focus mode, which dims the others.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paragraph: Vec<(usize, f64, f64, f64, f64)>,
}

impl WordView {
    /// Opens a Word file.
    pub fn open(bytes: &[u8]) -> office_core::Result<WordView> {
        Self::open_arc(std::sync::Arc::from(bytes))
    }

    /// Opens a Word file whose bytes are already shared, without copying its zip again.
    pub fn open_arc(bytes: std::sync::Arc<[u8]>) -> office_core::Result<WordView> {
        let doc = Compiler::new().with_layer(Words).read_arc(bytes, Some("document.docx"))?;
        // The bytes say what they are, whatever the file's name: a workbook named .docx isn't a flow.
        if doc.kind != office_core::Kind::Word {
            return Err(office_core::Error::Invalid(format!("this is {}, not a Word document", doc.kind.described())));
        }
        Ok(WordView::from_document(doc))
    }

    /// A new, empty document on A4 paper.
    pub fn blank() -> WordView {
        let mut doc = Document::new(office_core::Kind::Word);
        doc.body = Body::Flow { blocks: vec![Block::Paragraph(Paragraph::default())] };
        let bytes = Compiler::new().with_layer(Words).write(&doc, office_core::Kind::Word).expect("a blank document writes");
        WordView::open(&bytes).expect("a blank document reads")
    }

    pub fn from_document(mut doc: Document) -> WordView {
        // A body always has a paragraph to put the caret in.
        if let Body::Flow { blocks } = &mut doc.body {
            if !blocks.iter().any(|b| matches!(b, Block::Paragraph(_) | Block::Table(_))) {
                blocks.push(Block::Paragraph(Paragraph::default()));
            }
        }
        let extras = read_extras(&doc);
        let mut v = WordView {
            doc,
            extras,
            measure: Measure::new(),
            styles: Styles::new(),
            cache: Cache::new(),
            specs: Default::default(),
            sections: Vec::new(),
            pages: Vec::new(),
            labels: HashMap::new(),
            story: StoryId::Body,
            story_page: 0,
            sel: Selection::caret(Pos::start()),
            history: History::new(),
            pending: None,
            goal_x: None,
            caret_near: None,
            saved: 0,
            stale: true,
            styles_sent: 0,
            revision: 0,
            marks: Vec::new(),
            dirty: Dirty::Full,
            unrecorded: false,
            list_memo: HashMap::new(),
            last_lists: Vec::new(),
            flat_body_paras: false,
            others_changed: false,
            replica: None,
            collab_extra: Vec::new(),
        };
        if let Some(start) = pos::story_start(v.blocks(StoryId::Body)) {
            v.sel = Selection::caret(start);
        }
        v
    }

    /// The document's bytes, saved as Word.
    pub fn save(&mut self) -> office_core::Result<Vec<u8>> {
        Compiler::new().with_layer(Words).write(&self.doc, office_core::Kind::Word)
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.history.version();
        self.others_changed = false;
    }

    /// Changed since it was opened or last saved.
    pub fn is_dirty(&self) -> bool {
        self.history.version() != self.saved || self.others_changed
    }

    // ---- stories ----

    pub(crate) fn blocks(&self, story: StoryId) -> &[Block] {
        match story {
            StoryId::Body => match &self.doc.body {
                Body::Flow { blocks } => blocks,
                _ => &[],
            },
            StoryId::Story(i) => word(&self.doc).and_then(|w| w.headers.get(i)).map(|s| s.blocks.as_slice()).unwrap_or(&[]),
            StoryId::Note(i) => word(&self.doc).and_then(|w| w.notes.get(i)).map(|n| n.blocks.as_slice()).unwrap_or(&[]),
        }
    }

    pub(crate) fn blocks_mut(&mut self, story: StoryId) -> &mut Vec<Block> {
        self.stale = true;
        self.revision += 1;
        if story == StoryId::Body {
            self.unrecorded = true;
        }
        match story {
            StoryId::Body => match &mut self.doc.body {
                Body::Flow { blocks } => blocks,
                _ => panic!("a Word view holds a flow"),
            },
            StoryId::Story(i) => &mut crate::word_mut(&mut self.doc).headers[i].blocks,
            StoryId::Note(i) => &mut crate::word_mut(&mut self.doc).notes[i].blocks,
        }
    }

    // ---- layout ----

    /// How many top-level body blocks the document holds.
    pub fn body_block_count(&self) -> usize {
        match &self.doc.body {
            Body::Flow { blocks } => blocks.len(),
            _ => 0,
        }
    }

    /// How many page sections the document declares.
    pub fn section_count(&self) -> usize {
        crate::sections(&self.doc).len()
    }

    /// Lays the document out again where it changed.
    pub fn layout(&mut self, host: &mut dyn Measurer) {
        self.layout_inner(host, None, false);
    }

    /// Lays only the first `max_blocks` top-level blocks. Progressive opening uses this for a
    /// quick first paint; the result is temporary and `layout_full` replaces it.
    pub fn layout_prefix(&mut self, host: &mut dyn Measurer, max_blocks: u32) {
        self.layout_inner(host, Some(max_blocks), true);
    }

    /// Lays the whole document even though nothing since the last layout is marked dirty.
    pub fn layout_full(&mut self, host: &mut dyn Measurer) {
        self.layout_inner(host, None, true);
    }

    fn layout_inner(&mut self, host: &mut dyn Measurer, max_blocks: Option<u32>, force: bool) {
        if !self.stale && !force {
            return;
        }
        self.stale = false;
        self.revision += 1;
        let t0 = timing::now();
        self.cache.next_pass();
        self.specs.next_pass();
        if self.list_memo.len() > 100_000 {
            self.list_memo.clear();
        }
        // Paragraphs moved (blocks added or removed): labels are keyed by path, so work them out again.
        let body_now = match &self.doc.body {
            Body::Flow { blocks } => blocks.as_slice(),
            _ => &[],
        };
        let dirty_snapshot = if self.unrecorded { Dirty::Full } else { self.dirty.clone() };
        let moved = !matches!(dirty_snapshot, Dirty::Clean | Dirty::Blocks { delta: 0, .. });
        let skip_labels = match &dirty_snapshot {
            Dirty::Clean if !self.last_lists.is_empty() || body_now.is_empty() => true,
            Dirty::Blocks { first, end, delta: 0 } if self.flat_body_paras && self.last_lists.len() == body_now.len() => {
                let s = (*first as usize).min(body_now.len());
                let e = (*end as usize).min(body_now.len());
                body_now[s..e].iter().enumerate().all(|(offset, b)| {
                    let Block::Paragraph(p) = b else { return false };
                    let idx = s + offset;
                    let key = list_key(p);
                    self.list_memo.get(&key) == self.last_lists.get(idx)
                })
            }
            _ => false,
        };
        let labels_changed = if skip_labels {
            false
        } else if let Some(labels) = body_labels(&self.doc, &mut self.list_memo, &mut self.last_lists, moved) {
            self.flat_body_paras = body_now.iter().all(|b| matches!(b, Block::Paragraph(_)));
            self.labels = labels;
            true
        } else {
            self.flat_body_paras = body_now.iter().all(|b| matches!(b, Block::Paragraph(_)));
            false
        };
        let t_labels = timing::now();
        let sections = crate::sections(&self.doc);
        let places = section_ends(&self.doc);
        let body = body_now;
        let mut ranges = Vec::with_capacity(sections.len());
        let mut first = 0u32;
        for i in 0..sections.len() {
            let last = places.get(i).copied().flatten().map(|b| b as u32).unwrap_or((body.len() as u32).saturating_sub(1));
            ranges.push((first, last));
            first = last + 1;
        }
        let dirty = std::mem::replace(&mut self.dirty, Dirty::Clean);
        let dirty = if self.unrecorded { Dirty::Full } else { dirty };
        self.unrecorded = false;
        // Blocks laid out last time can be kept when the sections are the same ones, moved at most
        // by the blocks added or removed.
        let (keep_from, keep_end, delta) = match dirty {
            Dirty::Clean => (u32::MAX, u32::MAX, 0),
            Dirty::Blocks { first, end, delta } => (first, end, delta),
            Dirty::Full => (0, u32::MAX, 0),
        };
        let reusable = dirty != Dirty::Full
            && self.sections.len() == sections.len()
            && self.sections.iter().zip(&sections).zip(&ranges).all(|((old, new), &(f, l))| {
                let shift = |x: u32| if x >= keep_from { (x as i64 + delta).max(0) as u32 } else { x };
                old.section == *new
                    && shift(old.first).min(f) == f
                    && (old.last >= keep_from || old.last == l)
                    && (old.last < keep_from || shift(old.last) == l)
            });
        let mut old_flows: Vec<Option<SectionFlow>> =
            if reusable { std::mem::take(&mut self.sections).into_iter().map(Some).collect() } else { Vec::new() };
        let mut flows = Vec::with_capacity(sections.len());
        // Pages can be kept only when the edit was to body blocks whose breaks all came out the
        // same. A header, footer or note edit marks nothing in the body, but moves it.
        let mut same_pagination = reusable && !self.pages.is_empty() && matches!(dirty, Dirty::Blocks { .. });
        // Progressive opening is deliberately conservative: one flowing section whose tail can
        // be appended without changes to earlier section starts. Anything more exotic lays out whole.
        let prefix = max_blocks.filter(|_| sections.len() == 1);
        for (i, section) in sections.into_iter().enumerate() {
            let (first, last) = ranges[i];
            let last = match prefix {
                Some(max) if i == 0 => (first + max.saturating_sub(1)).min(last),
                _ => last,
            };
            let width = emu_px(section.text_width()).max(24.0);
            let slice = body.get(first as usize..=(last as usize).min(body.len().saturating_sub(1))).unwrap_or(&[]);
            let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
            let mut r = WordResolver {
                doc: &self.doc,
                extras: &self.extras,
                story: body,
                labels: &self.labels,
                label_prefix: &[],
                fields: None,
                note: None,
                specs: &mut self.specs,
            };
            let flow = match old_flows.get_mut(i).and_then(Option::take) {
                Some(old) if !labels_changed && delta == 0 && old.first == first && old.flow.blocks.len() == slice.len() => {
                    let (f, unchanged) =
                        relayout_in_place(&mut ctx, &mut self.cache, &mut r, old, slice, first, width, (keep_from, keep_end));
                    same_pagination &= unchanged;
                    f
                }
                Some(old) => {
                    let (f, unchanged) =
                        relayout(&mut ctx, &mut self.cache, &mut r, old, slice, first, width, (keep_from, keep_end, delta), &self.labels);
                    same_pagination &= unchanged;
                    f
                }
                None => {
                    same_pagination = false;
                    flow::layout_from(&mut ctx, &mut self.cache, &mut r, slice, &[], first, width)
                }
            };
            flows.push(SectionFlow { section, first, last, flow });
        }
        self.sections = flows;
        let t_flow = timing::now();
        if !same_pagination {
            self.pages = paginate::paginate(self, host);
        }
        let t_pages = timing::now();
        timing::report(&[("labels", t_labels - t0), ("flow", t_flow - t_labels), ("pages", t_pages - t_flow)]);
    }

    /// See [`WordView::revision`]'s field.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces the marks in set `set`.
    pub fn set_marks(&mut self, set: &str, marks: Vec<(String, Pos, Pos)>) {
        let blocks = self.blocks(StoryId::Body);
        let kept: Vec<Mark> = marks
            .into_iter()
            .filter(|(_, from, to)| from <= to && mark_pos_valid(blocks, from) && mark_pos_valid(blocks, to))
            .map(|(id, from, to)| Mark { set: set.to_string(), id, from, to })
            .collect();
        self.marks.retain(|m| m.set != set);
        self.marks.extend(kept);
    }

    /// A font finished loading: measure everything again.
    pub fn fonts_changed(&mut self) {
        self.measure.clear();
        self.cache.clear();
        self.specs.clear();
        self.dirty = Dirty::Full;
        self.stale = true;
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page_size(&self, page: usize) -> (f64, f64) {
        self.pages.get(page).map(|p| (p.width, p.height)).unwrap_or((794.0, 1123.0))
    }

    /// Every page's size, for the host to place pages before drawing any.
    pub fn page_sizes(&self) -> Vec<(f64, f64)> {
        self.pages.iter().map(|p| (display::r(p.width), display::r(p.height))).collect()
    }

    /// A header or footer laid out for a page (its fields show that page's number).
    pub(crate) fn story_flow(&mut self, host: &mut dyn Measurer, story: usize, page: usize) -> Option<FlowLayout> {
        let p = self.pages.get(page)?;
        let width = (p.width - p.body_x - (p.width - p.body_x - emu_px(self.sections[p.section].section.text_width()))).max(24.0);
        let width = emu_px(self.sections[p.section].section.text_width()).max(24.0).min(width.max(24.0));
        let fields = PageFields { page: p.number, pages: self.pages.len() as u32 };
        let blocks = word(&self.doc)?.headers.get(story)?.blocks.clone();
        let labels = HashMap::new();
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let mut r = WordResolver {
            doc: &self.doc,
            extras: &self.extras,
            story: &blocks,
            labels: &labels,
            label_prefix: &[],
            fields: Some(fields),
            note: None,
            specs: &mut self.specs,
        };
        Some(flow::layout(&mut ctx, &mut self.cache, &mut r, &blocks, &[], width))
    }

    /// A footnote's or endnote's text laid out at `width`; its own mark shows its number.
    pub(crate) fn note_flow(&mut self, host: &mut dyn Measurer, note: usize, width: f64) -> Option<FlowLayout> {
        let n = word(&self.doc)?.notes.get(note)?;
        let (blocks, id, kind) = (n.blocks.clone(), n.id.clone(), n.kind);
        let labels = HashMap::new();
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let mut r = WordResolver {
            doc: &self.doc,
            extras: &self.extras,
            story: &blocks,
            labels: &labels,
            label_prefix: &[],
            fields: None,
            note: Some((&id, kind)),
            specs: &mut self.specs,
        };
        Some(flow::layout(&mut ctx, &mut self.cache, &mut r, &blocks, &[], width))
    }

    /// Draws a page's footnotes at the bottom of its body area, under a short separator line.
    fn paint_footnotes(&mut self, host: &mut dyn Measurer, p: &Page, out: &mut Vec<Item>) {
        if p.footnotes.is_empty() {
            return;
        }
        self.paint_notes(host, p, &p.footnotes, footnotes_y(p), out);
    }

    /// The width a page's notes are laid out to.
    pub(crate) fn note_width(&self, p: &Page) -> f64 {
        emu_px(self.sections[p.section].section.text_width()).max(24.0)
    }

    /// Draws `notes` one after another from `y` down, under a separator line: a short one, or
    /// one across the text when the first continues a note from the page before.
    fn paint_notes(&mut self, host: &mut dyn Measurer, p: &Page, notes: &[NoteSlice], mut y: f64, out: &mut Vec<Item>) {
        if notes.is_empty() {
            return;
        }
        let width = self.note_width(p);
        let flows: Vec<(NoteSlice, FlowLayout)> = notes.iter().filter_map(|&s| Some((s, self.note_flow(host, s.note, width)?))).collect();
        let line_y = display::r(y - NOTE_SEPARATOR / 2.0);
        let line_w = if notes[0].top > 0.01 { width } else { (width / 3.0).min(192.0) };
        out.push(Item::Line {
            x1: display::r(p.body_x),
            y1: line_y,
            x2: display::r(p.body_x + line_w),
            y2: line_y,
            color: "#000000".into(),
            w: 0.75,
            dash: "solid",
        });
        let links = |_: &Path, _: u32| None;
        for (slice, flow) in flows {
            let blocks = word(&self.doc).and_then(|w| w.notes.get(slice.note)).map(|n| n.blocks.clone()).unwrap_or_default();
            let mut objects = NoteObjects { blocks: &blocks };
            let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
            display::paint_flow(
                &mut ctx,
                &flow,
                p.body_x,
                y - slice.top,
                (slice.top, slice.bottom),
                &mut display::Sink { objects: &mut objects, links: &links, out },
            );
            y += slice.bottom - slice.top;
        }
    }

    /// Where a header or footer's flow sits on its page: its top left.
    pub(crate) fn story_origin(&self, page: usize, story: usize, height: f64) -> (f64, f64) {
        let p = &self.pages[page];
        let s = &self.sections[p.section].section;
        let kind = word(&self.doc).and_then(|w| w.headers.get(story)).map(|s| s.kind).unwrap_or(StoryKind::Header);
        let x = p.body_x;
        match kind {
            StoryKind::Header => (x, emu_px(s.header)),
            StoryKind::Footer => (x, p.height - emu_px(s.footer) - height),
        }
    }

    // ---- drawing ----

    /// Draws a page.
    pub fn paint(&mut self, host: &mut dyn Measurer, page: usize) -> PagePaint {
        self.layout(host);
        let Some(p) = self.pages.get(page).cloned() else { return PagePaint { width: 0.0, height: 0.0, items: Vec::new() } };
        let mut items = Vec::new();
        let editing_story = matches!(self.story, StoryId::Story(_));
        // Headers and footers, dimmed while the body is edited, as Word shows them.
        for story in [p.header, p.footer].into_iter().flatten() {
            let Some(flow) = self.story_flow(host, story, page) else { continue };
            let (x, y) = self.story_origin(page, story, flow.height);
            let mut out = Vec::new();
            let mut objects = PageObjects { doc: &self.doc, story: StoryId::Story(story) };
            let links = |_: &Path, _: u32| None;
            let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
            display::paint_flow(
                &mut ctx,
                &flow,
                x,
                y,
                (f64::MIN, f64::MAX),
                &mut display::Sink { objects: &mut objects, links: &links, out: &mut out },
            );
            items.push(Item::Group {
                x: 0.0,
                y: 0.0,
                w: p.width,
                h: p.height,
                rot: 0.0,
                fh: false,
                fv: false,
                clip: false,
                id: Some(if editing_story { "hf".into() } else { "hf-dim".into() }),
                items: out,
            });
        }
        let sf = &self.sections[p.section];
        let blocks = match &self.doc.body {
            Body::Flow { blocks } => blocks.as_slice(),
            _ => &[],
        };
        let links = |path: &Path, offset: u32| link_at(blocks, path, offset);
        let mut objects = PageObjects { doc: &self.doc, story: StoryId::Body };
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        display::paint_flow(
            &mut ctx,
            &sf.flow,
            p.body_x,
            p.body_y - p.top,
            (p.top, p.bottom),
            &mut display::Sink { objects: &mut objects, links: &links, out: &mut items },
        );
        paint_floating(&mut ctx, &self.doc, &p, blocks, &sf.flow, &mut items);
        self.paint_footnotes(host, &p, &mut items);
        self.paint_notes(host, &p, &p.endnotes, p.endnotes_y, &mut items);
        PagePaint { width: display::r(p.width), height: display::r(p.height), items }
    }

    /// Paint styles the host hasn't had yet. Call after painting.
    pub fn new_styles(&mut self) -> Vec<office_engine::style::PaintStyle> {
        let out = display::styles_since(&self.styles, self.styles_sent);
        self.styles_sent = self.styles.len();
        out
    }

    /// The host lost its styles (a new painter): send them all again.
    pub fn resend_styles(&mut self) {
        self.styles_sent = 0;
    }

    // ---- positions on pages ----

    /// The page of section `si` showing flow position `y`.
    pub(crate) fn page_at(&self, si: usize, y: f64) -> usize {
        page_at(&self.pages, si, y)
    }

    /// Where on which page a flow position of section `si` is drawn.
    pub(crate) fn to_page(&self, si: usize, x: f64, y: f64) -> (usize, f64, f64) {
        to_page(&self.pages, si, x, y)
    }

    /// The caret and selection, as drawn on the pages.
    pub fn overlay(&mut self, host: &mut dyn Measurer) -> Overlay {
        self.layout(host);
        let mut out = Overlay::default();
        if let StoryId::Story(story) = self.story {
            return self.story_overlay(host, story);
        }
        if let StoryId::Note(note) = self.story {
            return self.note_overlay(host, note);
        }
        if let Some(img) = self.selected_image_box(host) {
            out.image = Some(img);
        }
        // Borrow the parts apart: the document and layout to read, the measurer to use.
        let WordView { doc, sections, pages, measure, styles, sel, marks, .. } = self;
        let blocks: &[Block] = match &doc.body {
            Body::Flow { blocks } => blocks,
            _ => &[],
        };
        let section_of = |path: &Path| {
            let b = path.0[0];
            let i = sections.partition_point(|s| s.last < b);
            sections.get(i).filter(|s| b >= s.first).map(|_| i)
        };
        let mut ctx = Ctx { host, measure, styles };
        let mut rects_for = |from: &Pos, to: &Pos, through_marks: bool, out: &mut Vec<(usize, f64, f64, f64, f64)>| {
            for path in pos::paras_between(blocks, &from.path, &to.path) {
                let Some(si) = section_of(&path) else { continue };
                let Some((pb, dx, dy)) = sections[si].flow.find(&path) else { continue };
                let len = pos::para(blocks, &path).map(text::len).unwrap_or(0);
                let a = if path == from.path { from.offset } else { 0 };
                let b = if path == to.path { to.offset } else { len };
                for (x, y, w, h) in display::selection_rects(&mut ctx, pb, dx, dy, a, b, !through_marks && path != to.path) {
                    let (page, px, py) = to_page(pages, si, x, y);
                    out.push((page, px, py, w, h));
                }
            }
        };
        let (from, to) = sel.ordered();
        if !sel.is_collapsed() {
            rects_for(&from, &to, false, &mut out.rects);
        }
        for mark in marks.iter().filter(|m| m.from <= m.to && mark_pos_valid(blocks, &m.from) && mark_pos_valid(blocks, &m.to)) {
            let mut rects = Vec::new();
            rects_for(&mark.from, &mark.to, true, &mut rects);
            out.marks.extend(rects.into_iter().map(|(p, x, y, w, h)| (p, x, y, w, h, mark.set.clone(), mark.id.clone())));
        }
        let len = pos::para(blocks, &sel.focus.path).map(text::len).unwrap_or(0);
        let mut lines = Vec::new();
        rects_for(&Pos::new(sel.focus.path.clone(), 0), &Pos::new(sel.focus.path.clone(), len), true, &mut lines);
        for (page, x, y, w, h) in lines {
            match out.paragraph.iter_mut().find(|b| b.0 == page) {
                Some(b) => {
                    let (x1, y1) = ((b.1 + b.3).max(x + w), (b.2 + b.4).max(y + h));
                    (b.1, b.2) = (b.1.min(x), b.2.min(y));
                    (b.3, b.4) = (x1 - b.1, y1 - b.2);
                }
                None => out.paragraph.push((page, x, y, w, h)),
            }
        }
        if out.image.is_none() {
            if let Some(si) = section_of(&sel.focus.path) {
                if let Some((pb, dx, dy)) = sections[si].flow.find(&sel.focus.path) {
                    let before = sel.focus.offset > 0 && from != to && sel.focus == to;
                    let (x, y, h) = display::caret_rect(&mut ctx, pb, dx, dy, sel.focus.offset, before);
                    let (page, px, _) = to_page(pages, si, x, y);
                    let near = self.caret_near.filter(|n| n.0 == page && from == to).map(|n| n.1 - (px - x));
                    let (x, y, h) = if near.is_some() {
                        display::caret_rect_near(&mut ctx, pb, dx, dy, sel.focus.offset, before, near)
                    } else {
                        (x, y, h)
                    };
                    let (page, px, py) = to_page(pages, si, x, y);
                    out.caret = Some((page, px, py, h));
                }
            }
        }
        out
    }

    /// The caret and selection in the header or footer being edited.
    /// The note at a point on a page, if one is painted there.
    pub fn note_at(&self, page: usize, y: f64) -> Option<usize> {
        let p = self.pages.get(page)?;
        note_places(p).into_iter().find(|(s, oy)| y >= oy + s.top && y < oy + s.bottom).map(|(s, _)| s.note)
    }

    /// The caret and selection in the note being edited, on every page showing part of it.
    fn note_overlay(&mut self, host: &mut dyn Measurer, note: usize) -> Overlay {
        let mut out = Overlay::default();
        let places: Vec<(usize, NoteSlice, f64, f64)> = self
            .pages
            .iter()
            .enumerate()
            .flat_map(|(i, p)| note_places(p).into_iter().filter(|(s, _)| s.note == note).map(move |(s, oy)| (i, s, p.body_x, oy)))
            .collect();
        let Some(width) = self.pages.get(places.first().map_or(0, |p| p.0)).map(|p| self.note_width(p)) else { return out };
        let Some(flow) = self.note_flow(host, note, width) else { return out };
        let WordView { doc, measure, styles, sel, .. } = self;
        let blocks: &[Block] = word(doc).and_then(|w| w.notes.get(note)).map(|n| n.blocks.as_slice()).unwrap_or(&[]);
        let mut ctx = Ctx { host, measure, styles };
        let (from, to) = sel.ordered();
        for &(page, slice, ox, oy) in &places {
            let shown = |y: f64, h: f64| y + h > oy + slice.top + 0.01 && y < oy + slice.bottom - 0.01;
            if !sel.is_collapsed() {
                for path in pos::paras_between(blocks, &from.path, &to.path) {
                    let Some((pb, dx, dy)) = flow.find(&path) else { continue };
                    let len = pos::para(blocks, &path).map(text::len).unwrap_or(0);
                    let a = if path == from.path { from.offset } else { 0 };
                    let b = if path == to.path { to.offset } else { len };
                    for (x, y, w, h) in display::selection_rects(&mut ctx, pb, ox + dx, oy + dy, a, b, path != to.path) {
                        if shown(y, h) {
                            out.rects.push((page, x, y, w, h));
                        }
                    }
                }
            }
            if let Some((pb, dx, dy)) = flow.find(&sel.focus.path) {
                let (x, y, h) = display::caret_rect(&mut ctx, pb, ox + dx, oy + dy, sel.focus.offset, false);
                if out.caret.is_none() && shown(y, h) {
                    out.caret = Some((page, x, y, h));
                }
            }
        }
        out
    }

    fn story_overlay(&mut self, host: &mut dyn Measurer, story: usize) -> Overlay {
        let mut out = Overlay::default();
        let page = self.story_page;
        let Some(flow) = self.story_flow(host, story, page) else { return out };
        let (ox, oy) = self.story_origin(page, story, flow.height);
        let WordView { doc, measure, styles, sel, .. } = self;
        let blocks: &[Block] = word(doc).and_then(|w| w.headers.get(story)).map(|s| s.blocks.as_slice()).unwrap_or(&[]);
        let mut ctx = Ctx { host, measure, styles };
        let (from, to) = sel.ordered();
        if !sel.is_collapsed() {
            for path in pos::paras_between(blocks, &from.path, &to.path) {
                let Some((pb, dx, dy)) = flow.find(&path) else { continue };
                let len = pos::para(blocks, &path).map(text::len).unwrap_or(0);
                let a = if path == from.path { from.offset } else { 0 };
                let b = if path == to.path { to.offset } else { len };
                for (x, y, w, h) in display::selection_rects(&mut ctx, pb, ox + dx, oy + dy, a, b, path != to.path) {
                    out.rects.push((page, x, y, w, h));
                }
            }
        }
        if let Some((pb, dx, dy)) = flow.find(&sel.focus.path) {
            let (x, y, h) = display::caret_rect(&mut ctx, pb, ox + dx, oy + dy, sel.focus.offset, false);
            out.caret = Some((page, x, y, h));
        }
        out
    }

    /// The selected picture's box on its page, when the selection is exactly one picture.
    pub(crate) fn selected_image_box(&mut self, host: &mut dyn Measurer) -> Option<(usize, f64, f64, f64, f64)> {
        let (from, to) = self.sel.ordered();
        if from.path != to.path || self.story != StoryId::Body {
            return None;
        }
        let blocks = self.blocks(StoryId::Body);
        let p = pos::para(blocks, &from.path)?;
        let (i, at) = text::inline_at(p, from.offset)?;
        if at != from.offset || at + text::inline_len(&p.content[i]) != to.offset || !matches!(p.content[i], Inline::Image(_)) {
            return None;
        }
        let si = self.sections.iter().position(|s| from.path.0[0] >= s.first && from.path.0[0] <= s.last)?;
        let (pb, dx, dy) = self.sections[si].flow.find(&from.path)?;
        let pb = pb.clone();
        let _ = host;
        for line in &pb.layout.lines {
            for f in &line.frags {
                if let office_engine::para::FragKind::Object { inline, height } = f.kind {
                    if inline as usize == i {
                        let x = dx + f.x;
                        let y = dy + pb.y + line.top + line.baseline - height;
                        let (page, px, py) = self.to_page(si, x, y);
                        return Some((page, display::r(px), display::r(py), display::r(f.width), display::r(height)));
                    }
                }
            }
        }
        None
    }

    /// The story position at a point on a page. In the body unless a header or footer is open.
    pub fn hit(&mut self, host: &mut dyn Measurer, page: usize, x: f64, y: f64) -> Option<Pos> {
        self.layout(host);
        let p = self.pages.get(page)?.clone();
        match self.story {
            StoryId::Body => {
                let fy = (y - p.body_y + p.top).clamp(p.top, (p.bottom - 0.01).max(p.top));
                let fx = x - p.body_x;
                let WordView { sections, measure, styles, .. } = self;
                let mut ctx = Ctx { host, measure, styles };
                hit_flow(&mut ctx, &sections[p.section].flow, fx, fy)
            }
            StoryId::Note(note) => {
                let (slice, oy) = note_places(&p).into_iter().find(|(s, _)| s.note == note)?;
                let flow = self.note_flow(host, note, self.note_width(&p))?;
                let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
                hit_flow(&mut ctx, &flow, x - p.body_x, (y - oy).clamp(slice.top, (slice.bottom - 0.01).max(slice.top)))
            }
            StoryId::Story(story) => {
                let flow = self.story_flow(host, story, page)?;
                let (ox, oy) = self.story_origin(page, story, flow.height);
                let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
                hit_flow(&mut ctx, &flow, x - ox, (y - oy).clamp(0.0, flow.height.max(0.0)))
            }
        }
    }

    /// The page the caret is on.
    pub fn caret_page(&mut self, host: &mut dyn Measurer) -> usize {
        self.overlay(host).caret.map(|c| c.0).or_else(|| self.selected_image_box(host).map(|b| b.0)).unwrap_or(0)
    }

    /// Whether a page shows a header or footer of this kind.
    pub fn page_has_story(&self, page: usize, kind: StoryKind) -> bool {
        self.pages.get(page).is_some_and(|p| if kind == StoryKind::Header { p.header.is_some() } else { p.footer.is_some() })
    }

    /// Whether a point on a page is in its header or footer area (above the body or below it).
    pub fn in_header_footer(&self, page: usize, y: f64) -> Option<StoryKind> {
        let p = self.pages.get(page)?;
        if y < p.body_y {
            Some(StoryKind::Header)
        } else if y > p.body_y + (p.bottom - p.top).max(0.0) && y > p.height - emu_px(self.sections[p.section].section.margin_bottom) {
            Some(StoryKind::Footer)
        } else {
            None
        }
    }
}

fn para_has_notes(p: &Paragraph) -> bool {
    p.content.iter().any(|i| matches!(i, Inline::Raw { .. }))
}

fn para_matches_geometry(prev: &BlockBox, next: &BlockBox, block: &Block) -> bool {
    match (prev, next, block) {
        (BlockBox::Para(op), BlockBox::Para(np), Block::Paragraph(par)) => {
            !para_has_notes(par)
                && op.keep == np.keep
                && op.spec.space_before == np.spec.space_before
                && op.spec.space_after == np.spec.space_after
                && op.layout.height == np.layout.height
                && op.layout.lines.len() == np.layout.lines.len()
                && op.layout.lines.iter().zip(&np.layout.lines).all(|(a, b)| a.top == b.top && a.height == b.height && a.ends == b.ends)
        }
        _ => false,
    }
}

/// Updates only blocks near an in-place (`delta == 0`) edit with zero Vec reallocation across unchanged blocks.
#[allow(clippy::too_many_arguments)]
fn relayout_in_place(
    ctx: &mut Ctx,
    cache: &mut Cache,
    r: &mut WordResolver,
    mut old: SectionFlow,
    slice: &[Block],
    first: u32,
    width: f64,
    (from, end): (u32, u32),
) -> (FlowLayout, bool) {
    let sec_end = first + slice.len() as u32;
    let mut same_breaks = true;
    if !slice.is_empty() && from != u32::MAX && from <= sec_end && end >= first {
        let lo = from.saturating_sub(1).max(first) - first;
        let hi = end.min(sec_end.saturating_sub(1)) - first;
        for (k, block) in slice.iter().enumerate().take(hi as usize + 1).skip(lo as usize) {
            let i = first + k as u32;
            let new_box = flow::layout_block(ctx, cache, r, block, Path::top(i), width);
            let prev = &mut old.flow.blocks[k];
            if same_breaks && !para_matches_geometry(prev, &new_box, block) {
                same_breaks = false;
            }
            *prev = new_box;
        }
    }
    // The replaced blocks were laid out at the top; stacking puts every block back in place. It
    // is arithmetic over the section's blocks, and the same stacking a full layout does (spacing
    // carries across a block that isn't a paragraph).
    let height = flow::restack(&mut old.flow.blocks);
    (FlowLayout { blocks: old.flow.blocks, height }, same_breaks)
}
#[allow(clippy::too_many_arguments)]
fn relayout(
    ctx: &mut Ctx,
    cache: &mut Cache,
    r: &mut WordResolver,
    old: SectionFlow,
    slice: &[Block],
    first: u32,
    width: f64,
    (from, end, delta): (u32, u32, i64),
    labels: &HashMap<Path, String>,
) -> (FlowLayout, bool) {
    let old_height = old.flow.height;
    let mut old_blocks: Vec<Option<BlockBox>> = old.flow.blocks.into_iter().map(Some).collect();
    let old_first = old.first;
    let mut out = Vec::with_capacity(slice.len());
    let mut same_breaks = delta == 0;
    for (k, block) in slice.iter().enumerate() {
        let i = first + k as u32;
        let old_index = if i < from {
            Some(i as i64)
        } else if i >= end {
            Some(i as i64 - delta)
        } else {
            None
        };
        let near_edit = i + 1 == from || i == end;
        let oi = old_index.and_then(|oi| usize::try_from(oi - old_first as i64).ok());
        let mut candidate = if !near_edit { oi.and_then(|oi| old_blocks.get_mut(oi).and_then(Option::take)) } else { None };
        // Moved into place first, then compared: a list label is looked up by the block's path.
        let kept = candidate
            .take()
            .filter(|_| i < from || i >= end)
            .map(|mut b| {
                if i >= end && delta != 0 {
                    b.shift(delta);
                }
                b
            })
            .filter(|b| b.path().0.first() == Some(&i) && same_labels(b, labels, &[]));
        // If `!near_edit && delta == 0` didn't take `candidate` only when it failed labels, or if in `from..end`:
        // Re-grab old box at matching block index `k` when `delta == 0` so we can compare break geometry.
        let b = match kept {
            Some(b) => b,
            None => {
                let b = flow::layout_block(ctx, cache, r, block, Path::top(i), width);
                if same_breaks {
                    let prev_ref = oi
                        .and_then(|oi| old_blocks.get(oi).and_then(|o| o.as_ref()))
                        .or_else(|| old_blocks.get(k).and_then(|o| o.as_ref()));
                    let matches_old = match (prev_ref, &b, block) {
                        (Some(BlockBox::Para(op)), BlockBox::Para(np), Block::Paragraph(par)) => {
                            !para_has_notes(par)
                                && op.keep == np.keep
                                && op.spec.space_before == np.spec.space_before
                                && op.spec.space_after == np.spec.space_after
                                && op.layout.height == np.layout.height
                                && op.layout.lines.len() == np.layout.lines.len()
                                && op
                                    .layout
                                    .lines
                                    .iter()
                                    .zip(&np.layout.lines)
                                    .all(|(a, b)| a.top == b.top && a.height == b.height && a.ends == b.ends)
                        }
                        _ => false,
                    };
                    same_breaks &= matches_old;
                }
                b
            }
        };
        out.push(b);
    }
    let height = flow::restack(&mut out);
    same_breaks &= height == old_height;
    (FlowLayout { blocks: out, height }, same_breaks)
}

/// Whether a kept box's list labels are still the ones its paragraphs have.
fn same_labels(b: &BlockBox, labels: &HashMap<Path, String>, prefix: &[u32]) -> bool {
    match b {
        BlockBox::Para(p) => {
            p.spec.label.as_ref().map(|l| &l.text) == resolve::list_label(labels, prefix, &p.path).filter(|l| !l.is_empty())
        }
        BlockBox::Table(t) => t.cells.iter().all(|c| c.content.blocks.iter().all(|b| same_labels(b, labels, prefix))),
        BlockBox::Preview { path, flow, .. } => {
            let prefix = [prefix, &path.0].concat();
            flow.blocks.iter().all(|b| same_labels(b, labels, &prefix))
        }
        BlockBox::Empty { .. } => true,
    }
}

/// The page of section `si` showing flow position `y`.
fn page_at(pages: &[Page], si: usize, y: f64) -> usize {
    let start = pages.partition_point(|p| p.section < si);
    if start >= pages.len() || pages[start].section != si {
        return 0;
    }
    let end = start + pages[start..].partition_point(|p| p.section == si);
    let idx = start + pages[start..end].partition_point(|p| y >= p.bottom - 0.01);
    idx.min(end - 1)
}

/// Where on which page a flow position of section `si` is drawn.
fn to_page(pages: &[Page], si: usize, x: f64, y: f64) -> (usize, f64, f64) {
    let page = page_at(pages, si, y);
    let p = &pages[page];
    (page, p.body_x + x, p.body_y + y - p.top)
}

pub(crate) use office_engine::flow::hit as hit_flow;

/// The link on the text at `offset` of the paragraph at `path`.
fn link_at(blocks: &[Block], path: &Path, offset: u32) -> Option<String> {
    let p = pos::para(blocks, path)?;
    let (i, _) = text::inline_at(p, offset)?;
    match &p.content[i] {
        Inline::Text(r) => r.link.as_ref().and_then(|l| l.url.clone().or_else(|| l.anchor.as_ref().map(|a| format!("#{a}")))),
        _ => None,
    }
}

/// Draws pictures in Word text.
struct PageObjects<'a> {
    doc: &'a Document,
    story: StoryId,
}

impl Objects for PageObjects<'_> {
    fn object(&mut self, ctx: &mut Ctx, para: &Path, inline: u32, (x, y, w, h): display::Rect, out: &mut Vec<Item>) {
        let blocks: &[Block] = match self.story {
            StoryId::Body => match &self.doc.body {
                Body::Flow { blocks } => blocks,
                _ => return,
            },
            StoryId::Story(i) => match word(self.doc).and_then(|w| w.headers.get(i)) {
                Some(s) => &s.blocks,
                None => return,
            },
            StoryId::Note(_) => return,
        };
        let Some(p) = pos::para(blocks, para) else { return };
        if let Some(Inline::Raw { xml }) = p.content.get(inline as usize) {
            if let Some((tag, _, _)) = resolve::inline_latex_math(xml) {
                let id = format!("{}:{}", para.0.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("."), inline);
                out.push(Item::Image { x, y, w, h, media: tag.to_string(), crop: None, id: Some(id) });
                return;
            }
            self.diagram(ctx, xml, (x, y, w, h), out);
            return;
        }
        let Some(Inline::Image(img)) = p.content.get(inline as usize) else { return };
        let id = format!("{}:{}", para.0.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("."), inline);
        out.push(Item::Image { x, y, w, h, media: img.media.clone(), crop: image_crop(img), id: Some(id) });
    }

    fn preview_image(&mut self, img: &office_core::model::Image, (x, y, w, h): display::Rect, out: &mut Vec<Item>) {
        out.push(Item::Image { x, y, w, h, media: img.media.clone(), crop: image_crop(img), id: None });
    }

    fn preview_raw(&mut self, ctx: &mut Ctx, xml: &office_core::xml::Element, rect: display::Rect, out: &mut Vec<Item>) {
        if let Some((tag, _, _)) = resolve::inline_latex_math(xml) {
            let (x, y, w, h) = rect;
            out.push(Item::Image { x, y, w, h, media: tag.to_string(), crop: None, id: None });
            return;
        }
        self.diagram(ctx, xml, rect, out);
    }
}

impl PageObjects<'_> {
    /// A run holding an inline SmartArt diagram, drawn in `rect`.
    fn diagram(&self, ctx: &mut Ctx, xml: &office_core::xml::Element, (x, y, w, h): display::Rect, out: &mut Vec<Item>) {
        let part = match self.story {
            StoryId::Body => self.doc.package.as_ref().and_then(|p| p.main_part().ok()),
            StoryId::Story(i) => word(self.doc).and_then(|w| w.headers.get(i)).map(|s| s.part.clone()),
            StoryId::Note(_) => None,
        };
        if let (Some((data, cx, cy)), Some(part)) = (resolve::inline_diagram(xml), part) {
            out.push(diagram_item(office_slides::view::paint_diagram(self.doc, ctx, &part, data, (cx, cy), w, h), x, y, w, h));
        }
    }
}

/// A diagram's drawn shapes, placed at (`x`, `y`).
fn diagram_item(items: Vec<Item>, x: f64, y: f64, w: f64, h: f64) -> Item {
    Item::Group { x, y, w: display::r(w), h: display::r(h), rot: 0.0, fh: false, fv: false, clip: false, id: None, items }
}

/// Draws pictures in a footnote's text.
struct NoteObjects<'a> {
    blocks: &'a [Block],
}

impl Objects for NoteObjects<'_> {
    fn object(&mut self, _: &mut Ctx, para: &Path, inline: u32, rect: display::Rect, out: &mut Vec<Item>) {
        paint_image(self.blocks, para, inline, rect, out);
    }

    fn preview_image(&mut self, img: &office_core::model::Image, (x, y, w, h): display::Rect, out: &mut Vec<Item>) {
        out.push(Item::Image { x, y, w, h, media: img.media.clone(), crop: image_crop(img), id: None });
    }

    fn preview_raw(&mut self, _: &mut Ctx, xml: &office_core::xml::Element, (x, y, w, h): display::Rect, out: &mut Vec<Item>) {
        if let Some((tag, _, _)) = resolve::inline_latex_math(xml) {
            out.push(Item::Image { x, y, w, h, media: tag.to_string(), crop: None, id: None });
        }
    }
}

fn paint_image(blocks: &[Block], para: &Path, inline: u32, (x, y, w, h): display::Rect, out: &mut Vec<Item>) {
    let Some(p) = pos::para(blocks, para) else { return };
    let id = format!("{}:{}", para.0.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("."), inline);
    match p.content.get(inline as usize) {
        Some(Inline::Image(img)) => {
            out.push(Item::Image { x, y, w, h, media: img.media.clone(), crop: image_crop(img), id: Some(id) });
        }
        Some(Inline::Raw { xml }) => {
            if let Some((tag, _, _)) = resolve::inline_latex_math(xml) {
                out.push(Item::Image { x, y, w, h, media: tag.to_string(), crop: None, id: Some(id) });
            }
        }
        _ => {}
    }
}

/// A picture's crop (`a:srcRect`), as shares of its size cut from each side.
pub fn image_crop(img: &office_core::model::Image) -> Option<[f64; 4]> {
    let rect = img.origin.get()?.descendant(ns::A, "srcRect")?;
    let v = |k: &str| rect.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
    let c = [v("l"), v("t"), v("r"), v("b")];
    c.iter().any(|x| *x != 0.0).then_some(c)
}

/// Pictures and SmartArt diagrams anchored to the page rather than sitting in the text
/// (`wp:anchor`), drawn where they say, over the text: the layer reads their position; wrapping
/// text around them is not done.
fn paint_floating(ctx: &mut Ctx, doc: &Document, page: &Page, blocks: &[Block], flow: &FlowLayout, out: &mut Vec<Item>) {
    let Some(pkg) = &doc.package else { return };
    let Ok(main) = pkg.main_part() else { return };
    let Ok(rels) = pkg.rels(&main) else { return };
    let start = flow.blocks.partition_point(|b| b.bottom_y() <= page.top - 0.01);
    for b in &flow.blocks[start..] {
        if b.top_y() >= page.bottom - 0.01 {
            break;
        }
        let BlockBox::Para(pb) = b else { continue };
        let top = pb.y;
        // Drawn on the page where its paragraph starts, not on every page the paragraph reaches.
        if top < page.top - 0.01 {
            continue;
        }
        let Some(p) = pos::para(blocks, &pb.path) else { continue };
        for inline in &p.content {
            let Inline::Raw { xml } = inline else { continue };
            let Some(anchor) = xml.descendant(ns::WP, "anchor") else { continue };
            let diagram = anchor.path(ns::A, &["graphic", "graphicData"]).filter(|d| office_core::formats::diagram::is_diagram(d));
            let media = || {
                let rid = anchor.descendant(ns::A, "blip")?.attr_ns(ns::R, "embed")?;
                pkg.target(&main, rels.get(rid)?)
            };
            let media = if diagram.is_some() { None } else { media() };
            if diagram.is_none() && media.is_none() {
                continue;
            }
            let extent = anchor.child(ns::WP, "extent");
            let size = |k: &str| extent.and_then(|e| e.attr(k)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 9525.0;
            let (w, h) = (size("cx"), size("cy"));
            let offset = |axis: &str| -> f64 {
                let Some(pos) = anchor.child(ns::WP, axis) else { return 0.0 };
                let from = pos.attr("relativeFrom").unwrap_or("column");
                let off = pos.child(ns::WP, "posOffset").map(|o| o.text().trim().parse::<f64>().unwrap_or(0.0) / 9525.0);
                let align = pos.child(ns::WP, "align").map(|a| a.text());
                let horizontal = axis == "positionH";
                let (origin, extent) = match (from, horizontal) {
                    ("page", true) => (0.0, page.width),
                    ("page", false) => (0.0, page.height),
                    ("margin" | "column", true) => (page.body_x, page.width - 2.0 * page.body_x),
                    ("margin", false) => (page.body_y, page.height - page.body_y * 2.0),
                    (_, false) => (page.body_y + top - page.top, 0.0),
                    _ => (page.body_x, page.width - 2.0 * page.body_x),
                };
                let size = if horizontal { w } else { h };
                match align.as_deref() {
                    Some("center") => origin + (extent - size) / 2.0,
                    Some("right" | "bottom" | "outside") => origin + extent - size,
                    Some(_) => origin,
                    None => origin + off.unwrap_or(0.0),
                }
            };
            let (x, y) = (display::r(offset("positionH")), display::r(offset("positionV")));
            if let Some(data) = diagram {
                let emu = |k: &str| office_core::units::Emu(extent.and_then(|e| e.attr(k)).and_then(|v| v.parse().ok()).unwrap_or(0));
                out.push(diagram_item(office_slides::view::paint_diagram(doc, ctx, &main, data, (emu("cx"), emu("cy")), w, h), x, y, w, h));
                continue;
            }
            let Some(media) = media else { continue };
            let crop = anchor.descendant(ns::A, "srcRect").map(|rect| {
                let v = |k: &str| rect.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
                [v("l"), v("t"), v("r"), v("b")]
            });
            out.push(Item::Image { x, y, w: display::r(w), h: display::r(h), media, crop, id: None });
        }
    }
}

/// Where each section ends: the body block holding its `w:sectPr`; `None` for the last.
fn section_ends(doc: &Document) -> Vec<Option<usize>> {
    let blocks = match &doc.body {
        Body::Flow { blocks } => blocks.as_slice(),
        _ => &[],
    };
    let mut out: Vec<Option<usize>> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b, Block::Paragraph(p) if p.origin.get().and_then(|o| o.path(ns::W, &["pPr", "sectPr"])).is_some()))
        .map(|(i, _)| Some(i))
        .collect();
    out.push(None);
    out
}

/// Which list each paragraph is in, remembered by what decides it: its markup, style and level.
pub(crate) type ListMemo = HashMap<(usize, u64, Option<u8>), Option<(String, u8)>>;

fn list_key(p: &Paragraph) -> (usize, u64, Option<u8>) {
    let mut h = fingerprint::Fnv::default();
    std::hash::Hash::hash(&p.props.style, &mut h);
    (p.origin.id(), std::hash::Hasher::finish(&h), p.props.level)
}

/// List labels for the body's paragraphs, by path. Answers `None` when every paragraph is in the
/// same list at the same level as last time (`last`) and no paragraph moved, so the labels last
/// worked out still hold.
fn body_labels(doc: &Document, memo: &mut ListMemo, last: &mut Vec<Option<(String, u8)>>, moved: bool) -> Option<HashMap<Path, String>> {
    let blocks = match &doc.body {
        Body::Flow { blocks } => blocks.as_slice(),
        _ => return Some(HashMap::new()),
    };
    let Some(w) = word(doc) else { return Some(HashMap::new()) };
    fn walk<'a>(blocks: &'a [Block], f: &mut dyn FnMut(&'a Paragraph)) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => f(p),
                Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).filter(|c| !c.covered).for_each(|c| walk(&c.blocks, f)),
                Block::Raw { preview, .. } => walk(preview, f),
            }
        }
    }
    let mut lists = Vec::with_capacity(last.len());
    walk(blocks, &mut |p| {
        let key = list_key(p);
        lists.push(memo.entry(key).or_insert_with(|| crate::list_of(doc, p)).clone());
    });
    if !moved && lists == *last {
        return None;
    }
    *last = lists.clone();
    if lists.iter().all(Option::is_none) {
        return Some(HashMap::new());
    }
    let labels = w.numbering.labels(lists);
    // Paths only for the paragraphs that have a label.
    let mut out = HashMap::new();
    let mut i = 0;
    fn paths(blocks: &[Block], prefix: &mut Vec<u32>, labels: &[Option<String>], i: &mut usize, out: &mut HashMap<Path, String>) {
        for (k, b) in blocks.iter().enumerate() {
            match b {
                Block::Paragraph(_) => {
                    if let Some(Some(l)) = labels.get(*i) {
                        let mut v = prefix.clone();
                        v.push(k as u32);
                        out.insert(Path(v), l.clone());
                    }
                    *i += 1;
                }
                Block::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (c, cell) in row.cells.iter().enumerate() {
                            if cell.covered {
                                continue;
                            }
                            prefix.extend([k as u32, r as u32, c as u32]);
                            paths(&cell.blocks, prefix, labels, i, out);
                            prefix.truncate(prefix.len() - 3);
                        }
                    }
                }
                Block::Raw { preview, .. } => {
                    if !preview.is_empty() {
                        prefix.push(k as u32);
                        paths(preview, prefix, labels, i, out);
                        prefix.pop();
                    }
                }
            }
        }
    }
    paths(blocks, &mut Vec::new(), &labels, &mut i, &mut out);
    Some(out)
}

fn read_extras(doc: &Document) -> Extras {
    let mut out = read_parts_extras(doc);
    // The document's last section sets note formats too, over the settings.
    if let Some(sect) = doc.origin.get().and_then(|o| o.path(ns::W, &["body", "sectPr"])) {
        for (pr, format) in [("footnotePr", &mut out.footnote_format), ("endnotePr", &mut out.endnote_format)] {
            if let Some(v) = sect.path(ns::W, &[pr, "numFmt"]).and_then(|f| f.attr_ns(ns::W, "val")) {
                *format = Some(v.to_string());
            }
        }
    }
    out
}

fn read_parts_extras(doc: &Document) -> Extras {
    // Parts changed while editing (styles added, lists made) come from the layer, not the file.
    let edited_styles = crate::word(doc).and_then(|w| w.styles_xml.clone());
    let edited_numbering = crate::word(doc).filter(|w| w.numbering_changed).and_then(|w| w.numbering_xml.clone());
    let Some(pkg) = &doc.package else { return Extras::read(edited_styles.as_ref(), edited_numbering.as_ref(), None) };
    let Ok(main) = pkg.main_part() else { return Extras::read(None, None, None) };
    let part = |kind: &str| -> Option<office_core::xml::XmlDoc> {
        let (_, p) = pkg.related(&main, kind).ok()?.into_iter().next()?;
        pkg.xml(&p).ok()
    };
    use office_core::opc::rels::kind;
    let styles = edited_styles.or_else(|| part(kind::STYLES));
    let numbering = edited_numbering.or_else(|| part(kind::NUMBERING));
    let settings = part(kind::SETTINGS);
    Extras::read(styles.as_ref(), numbering.as_ref(), settings.as_ref())
}

/// Phase timings of a layout pass, printed when `OFFICE_TIMING` is set (native builds only).
mod timing {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn now() -> f64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    pub fn now() -> f64 {
        0.0
    }
    pub fn report(phases: &[(&str, f64)]) {
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var_os("OFFICE_TIMING").is_some() {
            eprintln!("{}", phases.iter().map(|(k, v)| format!("{k} {v:.3}ms")).collect::<Vec<_>>().join("  "));
        }
        let _ = phases;
    }
}

#[cfg(test)]
mod tests;

/// Where a page's footnotes start: under the body, at the bottom of the text area.
fn footnotes_y(p: &Page) -> f64 {
    let total: f64 = p.footnotes.iter().map(|s| s.bottom - s.top).sum();
    // Below the body even when a page's first line and its notes don't fit together.
    (p.notes_bottom - total).max(p.body_y + p.content_bottom - p.top + NOTE_SEPARATOR)
}

/// The notes painted on a page, each with the page y of its flow's top.
pub(crate) fn note_places(p: &Page) -> Vec<(NoteSlice, f64)> {
    let mut out = Vec::new();
    for (notes, mut y) in [(&p.footnotes, footnotes_y(p)), (&p.endnotes, p.endnotes_y)] {
        for s in notes {
            out.push((*s, y - s.top));
            y += s.bottom - s.top;
        }
    }
    out
}
