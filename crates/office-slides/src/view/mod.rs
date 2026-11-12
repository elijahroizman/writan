//! The PowerPoint layer's view: slides drawn and edited on the shared engine. The engine lays
//! out text and tables and keeps the history; this adds what only presentations have: frames on
//! a slide, what they inherit from layouts and masters, shapes' geometry and theme styles, and
//! slide operations, notes and transitions.

mod art;
mod commands;
mod draw;
mod elements;
mod table_style;
mod templates;
mod text;

use office_core::model::{Block, Body, Document, Frame, FrameContent, Kind, Paragraph, Run, RunProps, Slide, SlideSize, TextBody};
use office_core::units::{Emu, FontSize};
use office_core::Compiler;
use office_engine::display::{self, Item};
use office_engine::flow;
use office_engine::history::{History, Step};
use office_engine::measure::{Measure, Measurer};
use office_engine::para::{Ctx, Styles};
use office_engine::pos::Selection;
use serde::Serialize;

use crate::Decks;
pub use commands::{RangeBoxes, TextOverlay, TextPatch};
pub use draw::Boxed;
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
    /// Where the up and down arrows aim while editing text.
    pub(crate) goal_x: Option<f64>,
    /// Formatting picked with a caret while editing, for the next text typed.
    pub(crate) pending: Option<RunProps>,
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
        let inch = |v: f64| Emu::from_inches(v);
        let text = |s: &str, pt: f64, bold: bool, color: &str| {
            let mut p = Paragraph::default();
            p.content.push(office_core::model::Inline::Text(Run::new(s).with_props(RunProps {
                size: Some(FontSize::from_points(pt)),
                bold: bold.then_some(true),
                color: office_core::drawing::Rgb::parse(color).map(office_core::drawing::Color::rgb),
                font: Some("Helvetica Neue".into()),
                ..Default::default()
            })));
            p
        };
        let boxed = |id: u32, x: f64, y: f64, w: f64, h: f64, paras: Vec<Paragraph>| {
            Frame::text_box(
                id,
                office_core::drawing::Xfrm { x: inch(x), y: inch(y), width: inch(w), height: inch(h), ..Default::default() },
                paras,
            )
        };
        let title = Slide {
            frames: vec![
                boxed(2, 0.9, 2.6, 11.5, 1.2, vec![text("Presentation title", 48.0, true, "111111")]),
                boxed(3, 0.9, 3.8, 11.5, 0.7, vec![text("Subtitle or presenter", 22.0, false, "6B6B70")]),
            ],
            ..Default::default()
        };
        let mut bullets = Vec::new();
        for s in ["First point", "Second point", "Third point"] {
            bullets.push(text(&format!("• {s}"), 24.0, false, "333333"));
        }
        let content = Slide {
            frames: vec![
                boxed(2, 0.9, 0.6, 11.5, 1.0, vec![text("Slide title", 34.0, true, "111111")]),
                boxed(3, 0.9, 1.9, 11.5, 4.2, bullets),
            ],
            ..Default::default()
        };
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
            goal_x: None,
            pending: None,
        }
    }

    pub fn save(&mut self) -> office_core::Result<Vec<u8>> {
        Compiler::new().with_layer(Decks).write(&self.doc, Kind::Slides)
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.history.version();
    }

    pub fn is_dirty(&self) -> bool {
        self.history.version() != self.saved
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

    /// Draws a slide.
    pub fn paint(&mut self, host: &mut dyn Measurer, index: usize) -> SlidePaint {
        let (w, h) = self.size();
        let Some(slide) = self.slides().get(index).cloned() else { return SlidePaint { width: w, height: h, items: Vec::new() } };
        let mut items = Vec::new();
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let mut d = draw::Drawer { doc: &self.doc, art: &self.art, tables: &self.tables, ctx: &mut ctx, cache: &mut self.cache };
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
