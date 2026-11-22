//! The PowerPoint layer's view: slides drawn and edited on the shared engine. The engine lays
//! out text and tables and keeps the history; this adds what only presentations have: frames on
//! a slide, what they inherit from layouts and masters, shapes' geometry and theme styles, and
//! slide operations, notes and transitions.

mod art;
mod chart;
mod collab;
mod commands;
mod draw;
mod elements;
mod table_style;
mod templates;
mod text;

use office_core::model::{Block, Body, Document, Frame, FrameContent, Kind, Paragraph, RunProps, Slide, SlideSize, TextBody};
use office_core::Compiler;
use office_engine::display::{self, Item};
use office_engine::flow;
use office_engine::history::{History, Step};
use office_engine::measure::{Measure, Measurer};
use office_engine::para::{Ctx, Styles};
use office_engine::pos::Selection;
use serde::Serialize;

use crate::Decks;
pub use commands::{LinkedCell, RangeBoxes, TextOverlay, TextPatch, EXCEL_LINK_PREFIX, LATEX_MATH_PREFIX};
pub use draw::{paint_diagram, Boxed};
pub use elements::{Element, SlideInfo};

/// Text being edited in a frame: which frame (and table cell), and the selection in its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Editing {
    pub frame: u32,
    /// The cell, for a table.
    pub cell: Option<(u32, u32)>,
    pub sel: Selection,
}

/// What's selected: the slide, its picked frames, and text being edited.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sel {
    pub active: usize,
    pub frames: Vec<u32>,
    pub editing: Option<Editing>,
}

/// One change, for undo.
#[derive(Clone, Debug)]
pub enum SStep {
    /// A slide's content changed.
    Slide {
        index: usize,
        before: Box<Slide>,
        after: Box<Slide>,
    },
    /// Slides were added, removed or moved.
    Deck {
        before: Vec<Slide>,
        after: Vec<Slide>,
    },
    Notes {
        index: usize,
        before: Vec<Paragraph>,
        after: Vec<Paragraph>,
    },
}

impl Step for SStep {
    fn absorb(&mut self, next: &Self) -> bool {
        match (self, next) {
            (SStep::Slide { index, after, .. }, SStep::Slide { index: i2, before: b2, after: a2 }) if index == i2 && **after == **b2 => {
                *after = a2.clone();
                true
            }
            _ => false,
        }
    }
}

pub struct SlidesView {
    pub doc: Document,
    pub(crate) art: art::ArtSet,
    pub(crate) tables: table_style::TableStyles,
    pub(crate) measure: Measure,
    pub(crate) styles: Styles,
    pub(crate) cache: flow::Cache,
    pub(crate) sel: Sel,
    pub(crate) history: History<SStep, Sel>,
    pub(crate) saved: u64,
    pub(crate) styles_sent: usize,
    /// Each slide's revision: goes up when the slide changes, so drawings can be reused.
    pub(crate) revisions: Vec<u64>,
    pub(crate) next_revision: u64,
    /// The empty paragraph whose bullet shows because the caret is in it (slide, frame,
    /// paragraph), as last drawn.
    pub(crate) caret_shown: Option<(usize, u32, u32)>,
    /// Where the up and down arrows aim while editing text.
    pub(crate) goal_x: Option<f64>,
    /// Formatting picked with a caret while editing, for the next text typed.
    pub(crate) pending: Option<RunProps>,
    /// Chart clipboard payloads can only refer to the deck they were copied from.
    pub(crate) chart_clipboard_scope: std::sync::OnceLock<Option<[u8; 16]>>,
    /// Set while the presentation is shared with other copies (see `collab`).
    /// Another copy changed the document since it was saved here, while shared. Their changes
    /// aren't in the undo history, which [`Self::is_dirty`] otherwise goes by.
    pub(crate) others_changed: bool,
    pub(crate) replica: Option<office_engine::collab::Replica<SStep>>,
    /// Changes to the layer's own tables (copied notes) to send with the next shared change.
    pub(crate) collab_extra: Vec<serde_json::Value>,
}

/// A slide drawn.
#[derive(Clone, Debug, Serialize)]
pub struct SlidePaint {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
}

impl SlidesView {
    pub fn open(bytes: &[u8]) -> office_core::Result<SlidesView> {
        let doc = Compiler::new().with_layer(Decks).read(bytes, Some("deck.pptx"))?;
        if doc.kind != Kind::Slides {
            return Err(office_core::Error::Invalid(format!("this is {}, not a PowerPoint presentation", doc.kind.described())));
        }
        Ok(SlidesView::from_document(doc))
    }

    /// A new 16:9 deck: a title slide and a content slide to duplicate.
    pub fn blank() -> SlidesView {
        let mut doc = Document::new(Kind::Slides);
        let size = SlideSize::default();
        let title = Slide { frames: templates::plain_title(1.0, 1.0), ..Default::default() };
        let content = Slide { frames: templates::plain_content(1.0, 1.0), ..Default::default() };
        doc.body = Body::Slides { size, slides: vec![title, content] };
        let bytes = Compiler::new().with_layer(Decks).write(&doc, Kind::Slides).expect("a new deck writes");
        SlidesView::open(&bytes).expect("a new deck reads")
    }

    pub fn from_document(doc: Document) -> SlidesView {
        let art = art::ArtSet::read(&doc);
        let tables = table_style::TableStyles::read(&doc);
        let n = slides_of(&doc).len();
        SlidesView {
            doc,
            art,
            tables,
            measure: Measure::new(),
            styles: Styles::new(),
            cache: flow::Cache::new(),
            sel: Sel::default(),
            history: History::new(),
            saved: 0,
            styles_sent: 0,
            revisions: (1..=n as u64).collect(),
            next_revision: n as u64 + 1,
            caret_shown: None,
            goal_x: None,
            pending: None,
            chart_clipboard_scope: Default::default(),
            others_changed: false,
            replica: None,
            collab_extra: Vec::new(),
        }
    }

    pub fn save(&mut self) -> office_core::Result<Vec<u8>> {
        Compiler::new().with_layer(Decks).write(&self.doc, Kind::Slides)
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.history.version();
        self.others_changed = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.history.version() != self.saved || self.others_changed
    }

    pub fn slides(&self) -> &[Slide] {
        slides_of(&self.doc)
    }

    pub(crate) fn slides_mut(&mut self) -> &mut Vec<Slide> {
        match &mut self.doc.body {
            Body::Slides { slides, .. } => slides,
            _ => panic!("a slides view holds slides"),
        }
    }

    /// The slide size in pixels.
    pub fn size(&self) -> (f64, f64) {
        match &self.doc.body {
            Body::Slides { size, .. } => (size.width.0 as f64 / 9525.0, size.height.0 as f64 / 9525.0),
            _ => (1280.0, 720.0),
        }
    }

    /// Each slide's revision (see the field).
    pub fn revisions(&self) -> &[u64] {
        &self.revisions
    }

    /// Notes that slide `index` changed.
    pub(crate) fn touch(&mut self, index: usize) {
        if let Some(r) = self.revisions.get_mut(index) {
            *r = self.next_revision;
            self.next_revision += 1;
        }
    }

    /// Every slide changed (added, removed, moved).
    pub(crate) fn touch_all(&mut self) {
        let n = self.slides().len();
        self.revisions = (0..n as u64).map(|i| self.next_revision + i).collect();
        self.next_revision += n as u64;
    }

    /// The frame and paragraph the caret is in on slide `index`, while editing text outside a table.
    pub(crate) fn caret_para(&self, index: usize) -> Option<(u32, u32)> {
        let e = self.sel.editing.as_ref().filter(|_| index == self.sel.active)?;
        e.sel.focus.path.container().is_empty().then(|| (e.frame, e.sel.focus.path.block()))
    }

    /// After a command: redraws the slide where an empty paragraph's bullet shows or goes as the
    /// caret moves into or out of it.
    pub fn sync_caret(&mut self) {
        let active = self.sel.active;
        let now = self.caret_para(active).filter(|&(id, i)| {
            self.slides().get(active).and_then(|s| find_frame(&s.frames, id)).is_some_and(|f| match &f.content {
                FrameContent::Shape { text: Some(body), .. } => {
                    body.paragraphs.get(i as usize).is_some_and(|p| office_engine::text::flat(p).text.trim().is_empty())
                }
                _ => false,
            })
        });
        let now = now.map(|(id, i)| (active, id, i));
        if now != self.caret_shown {
            for (slide, _, _) in [self.caret_shown, now].into_iter().flatten() {
                self.touch(slide);
            }
            self.caret_shown = now;
        }
    }

    /// Draws a slide.
    pub fn paint(&mut self, host: &mut dyn Measurer, index: usize) -> SlidePaint {
        let (w, h) = self.size();
        let Some(slide) = self.slides().get(index).cloned() else { return SlidePaint { width: w, height: h, items: Vec::new() } };
        let mut items = Vec::new();
        let caret = self.caret_para(index);
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let mut d = draw::Drawer { doc: &self.doc, art: &self.art, tables: &self.tables, ctx: &mut ctx, cache: &mut self.cache, caret };
        d.slide(&slide, &mut items, w, h);
        SlidePaint { width: display::r(w), height: display::r(h), items }
    }

    /// Paint styles the host hasn't had yet.
    pub fn new_styles(&mut self) -> Vec<office_engine::style::PaintStyle> {
        let out = display::styles_since(&self.styles, self.styles_sent);
        self.styles_sent = self.styles.len();
        out
    }

    pub fn fonts_changed(&mut self) {
        self.measure.clear();
        self.cache.clear();
        self.touch_all();
    }

    /// A picture's bytes: from the presentation's media, or a part in the package (pictures on
    /// layouts, masters and backgrounds).
    pub fn media(&self, id: &str) -> Option<Vec<u8>> {
        if let Ok(b) = self.doc.media_bytes(id) {
            return Some(b.to_vec());
        }
        self.doc.package.as_ref()?.part(id).ok().map(<[u8]>::to_vec)
    }

    pub fn media_type(&self, id: &str) -> Option<String> {
        self.doc.media(id).map(|m| m.content_type.clone()).or_else(|| self.doc.package.as_ref()?.content_type(id).map(str::to_string))
    }
}

pub(crate) fn slides_of(doc: &Document) -> &[Slide] {
    match &doc.body {
        Body::Slides { slides, .. } => slides,
        _ => &[],
    }
}

/// A frame's paragraphs as blocks, for the engine's editing, and back.
pub(crate) fn text_blocks(body: &mut TextBody) -> Vec<Block> {
    std::mem::take(&mut body.paragraphs).into_iter().map(Block::Paragraph).collect()
}

pub(crate) fn from_blocks(body: &mut TextBody, blocks: Vec<Block>) {
    body.paragraphs = blocks
        .into_iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
        .collect();
    if body.paragraphs.is_empty() {
        body.paragraphs.push(Paragraph::default());
    }
}

/// The frame with `id`, searching into groups.
pub(crate) fn find_frame(frames: &[Frame], id: u32) -> Option<&Frame> {
    for f in frames {
        if f.id == id {
            return Some(f);
        }
        if let FrameContent::Group { frames } = &f.content {
            if let Some(x) = find_frame(frames, id) {
                return Some(x);
            }
        }
    }
    None
}

pub(crate) fn find_frame_mut(frames: &mut [Frame], id: u32) -> Option<&mut Frame> {
    for f in frames {
        if f.id == id {
            return Some(f);
        }
        if let FrameContent::Group { frames } = &mut f.content {
            if let Some(x) = find_frame_mut(frames, id) {
                return Some(x);
            }
        }
    }
    None
}

/// A frame id no frame on the slide (or in its groups) has.
pub(crate) fn free_id(frames: &[Frame]) -> u32 {
    fn max(frames: &[Frame]) -> u32 {
        frames
            .iter()
            .map(|f| match &f.content {
                FrameContent::Group { frames } => f.id.max(max(frames)),
                _ => f.id,
            })
            .max()
            .unwrap_or(1)
    }
    max(frames) + 1
}

#[cfg(test)]
mod tests;
