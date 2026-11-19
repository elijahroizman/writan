//! The shared model every format reads into and writes from.
//!
//! It covers what the formats have in common. What it doesn't cover is kept, not dropped: as
//! [`Inline::Raw`], [`Block::Raw`] and [`FrameContent::Raw`] for whole objects, and as each node's
//! [`Origin`] for the markup around the parts the model does read (a paragraph's unknown
//! properties, a shape's effects). Back ends rebuild from the origin, so a round trip keeps them.

mod chart;
mod sheet;
mod slide;
mod table;
mod text;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use chart::{
    make_ref, parse_ref, quote_sheet, ref_cells, ref_numbers, ref_texts, rename_in_ref, AnchorPoint, Chart, ChartAnchor, ChartGrouping,
    ChartKind, ChartSeries, ChartSource, LegendPosition, SeriesCache, SheetPicture,
};
pub use sheet::{format_number, CellRef, Range, Sheet, SheetCell, SheetRow, Value};
pub use slide::{Frame, FrameContent, Placeholder, Slide, SlideSize, TextBody};
pub use table::{Cell, Row, Table};
pub use text::{
    underline_from_drawingml, underline_to_drawingml, Align, Baseline, BreakKind, Inline, LineSpacing, Link, ParaProps, Paragraph, Run,
    RunProps, Strike,
};

use crate::drawing::Theme;
use crate::opc::{Package, Properties};
use crate::units::Emu;
use crate::xml::Element;

/// Which kind of document: what its body is and which back end writes it by default.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Word,
    Slides,
    Sheets,
    Csv,
}

impl Kind {
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Word => "docx",
            Kind::Slides => "pptx",
            Kind::Sheets => "xlsx",
            Kind::Csv => "csv",
        }
    }

    /// What the kind is called in a message: "a Word document".
    pub fn described(self) -> &'static str {
        match self {
            Kind::Word => "a Word document",
            Kind::Slides => "a PowerPoint presentation",
            Kind::Sheets => "an Excel workbook",
            Kind::Csv => "a CSV file",
        }
    }

    /// From a file extension. Templates, slide shows and macro-enabled files are the same formats.
    pub fn from_extension(ext: &str) -> Option<Kind> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "docx" | "docm" | "dotx" | "dotm" => Kind::Word,
            "pptx" | "pptm" | "ppsx" | "ppsm" | "potx" | "potm" => Kind::Slides,
            "xlsx" | "xlsm" | "xltx" | "xltm" => Kind::Sheets,
            "csv" | "tsv" => Kind::Csv,
            _ => return None,
        })
    }

    /// The kind whose main part has this media type.
    pub fn from_main_content_type(media: &str) -> Option<Kind> {
        let media = media.to_ascii_lowercase();
        if media.contains("wordprocessingml") || media.contains("ms-word") {
            Some(Kind::Word)
        } else if media.contains("presentationml") || media.contains("ms-powerpoint") {
            Some(Kind::Slides)
        } else if media.contains("spreadsheetml") || media.contains("ms-excel") {
            Some(Kind::Sheets)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub kind: Kind,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<Theme>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<Media>,
    pub body: Body,
    /// The package the document was read from. Saving in the same format starts from it, so
    /// everything the model doesn't read comes along.
    #[serde(skip)]
    pub package: Option<Package>,
    /// Data that belongs to one format or layer (see [`Extensions`]).
    #[serde(skip)]
    pub extensions: Extensions,
    /// The main part's markup around the body: Word's `w:document` (with the body's final
    /// section properties), PowerPoint's `p:presentation`, Excel's `workbook`.
    #[serde(skip)]
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Body {
    /// Word: paragraphs and tables, top to bottom.
    Flow { blocks: Vec<Block> },
    Slides {
        #[serde(default)]
        size: SlideSize,
        slides: Vec<Slide>,
    },
    /// Excel, and CSV as a workbook of one sheet.
    Workbook { sheets: Vec<Sheet> },
}

// Paragraphs are nearly every block, as runs are nearly every inline.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
    /// Markup the core doesn't model at block level (Word's section breaks inside the body,
    /// content controls, custom XML), whole.
    Raw {
        xml: Element,
    },
}

/// A picture, wherever it sits: inline in text, on a slide, in a table cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    /// The [`Media`] it shows.
    pub media: String,
    pub width: Emu,
    pub height: Emu,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Alt text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip)]
    pub origin: Origin,
}

/// A picture's (or video's) bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    /// How images refer to it. Media read from a file use their part name (`word/media/image1.png`).
    pub id: String,
    pub content_type: String,
    #[serde(skip)]
    pub data: MediaData,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum MediaData {
    /// In the document's package under this part name, read when needed.
    #[default]
    InPackage,
    /// Added since the document was read.
    Bytes(Arc<[u8]>),
}

impl Document {
    pub fn new(kind: Kind) -> Document {
        let body = match kind {
            Kind::Word => Body::Flow { blocks: Vec::new() },
            Kind::Slides => Body::Slides { size: SlideSize::default(), slides: Vec::new() },
            Kind::Sheets | Kind::Csv => Body::Workbook { sheets: vec![Sheet::new("Sheet1")] },
        };
        Document {
            kind,
            properties: Properties::default(),
            theme: None,
            media: Vec::new(),
            body,
            package: None,
            extensions: Extensions::default(),
            origin: Origin::default(),
        }
    }

    pub fn media(&self, id: &str) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    /// A media item's bytes, from the package or from memory.
    pub fn media_bytes(&self, id: &str) -> crate::Result<Arc<[u8]>> {
        let m = self.media(id).ok_or_else(|| crate::Error::MissingPart(id.to_string()))?;
        match &m.data {
            MediaData::Bytes(b) => Ok(b.clone()),
            MediaData::InPackage => {
                let pkg = self.package.as_ref().ok_or_else(|| crate::Error::MissingPart(id.to_string()))?;
                Ok(Arc::from(pkg.part(id)?))
            }
        }
    }

    /// The value at `at` in the first sheet, for workbooks and CSV files.
    pub fn sheet_value(&self, at: CellRef) -> Option<&Value> {
        match &self.body {
            Body::Workbook { sheets } => sheets.first().map(|s| s.value(at)),
            _ => None,
        }
    }

    /// Adds a picture and returns its id, for an [`Image`] to use.
    pub fn add_media(&mut self, content_type: &str, data: Vec<u8>) -> String {
        let id = (1..).map(|n| format!("new:{n}")).find(|id| self.media(id).is_none()).unwrap();
        self.media.push(Media { id: id.clone(), content_type: content_type.to_string(), data: MediaData::Bytes(Arc::from(data)) });
        id
    }
}

/// The markup a model node was read from, with the parts the model reads taken out: a
/// paragraph's `w:p` with its attributes and `w:pPr` but not its runs, a shape's `p:sp` with its
/// `p:nvSpPr` and `p:style` but not its text. Back ends start from it, so whatever the model
/// doesn't read survives. New nodes have none, and back ends write them from scratch.
///
/// Origins never make two nodes unequal, and aren't part of the model's JSON.
#[derive(Clone, Debug, Default)]
pub struct Origin(pub Option<Arc<Element>>);

impl Origin {
    pub fn new(el: Element) -> Origin {
        Origin(Some(Arc::new(el)))
    }

    pub fn get(&self) -> Option<&Element> {
        self.0.as_deref()
    }
}

impl PartialEq for Origin {
    fn eq(&self, _: &Origin) -> bool {
        true
    }
}

impl Origin {
    /// Which origin this is: the same for clones of it, different for one made anew. A layer
    /// that changes a node's origin replaces it, so a changed id means a changed origin.
    pub fn id(&self) -> usize {
        self.0.as_ref().map(|a| Arc::as_ptr(a) as *const () as usize).unwrap_or(0)
    }
}

/// A hash of what a node said when it was read, so a back end can tell an untouched node
/// without comparing it with the file: cheap where the file is huge. Like origins, it never
/// makes two nodes unequal and isn't part of the JSON.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fingerprint(pub Option<u64>);

impl PartialEq for Fingerprint {
    fn eq(&self, _: &Fingerprint) -> bool {
        true
    }
}

/// Data that belongs to one format or layer, by type: CSV's dialect, the Word layer's styles.
/// The core never looks inside.
#[derive(Clone, Debug, Default)]
pub struct Extensions(HashMap<TypeId, Box<dyn Extension>>);

/// Anything a layer keeps on a document. Implemented for every `Clone + Debug` type.
pub trait Extension: Any + Send + Sync + std::fmt::Debug {
    fn clone_box(&self) -> Box<dyn Extension>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any + Send + Sync + Clone + std::fmt::Debug> Extension for T {
    fn clone_box(&self) -> Box<dyn Extension> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Clone for Box<dyn Extension> {
    fn clone(&self) -> Self {
        (**self).clone_box()
    }
}

impl Extensions {
    pub fn get<T: Extension>(&self) -> Option<&T> {
        self.0.get(&TypeId::of::<T>()).and_then(|b| (**b).as_any().downcast_ref())
    }

    pub fn get_mut<T: Extension>(&mut self) -> Option<&mut T> {
        self.0.get_mut(&TypeId::of::<T>()).and_then(|b| (**b).as_any_mut().downcast_mut())
    }

    pub fn insert<T: Extension>(&mut self, value: T) {
        self.0.insert(TypeId::of::<T>(), Box::new(value));
    }

    pub fn remove<T: Extension>(&mut self) -> Option<T> {
        let b = self.0.remove(&TypeId::of::<T>())?;
        let any: Box<dyn Any> = b;
        any.downcast().ok().map(|b| *b)
    }
}

/// Blocks' text, paragraphs on their own lines, table cells separated by tabs.
pub fn blocks_text(blocks: &[Block]) -> String {
    let mut lines = Vec::new();
    for b in blocks {
        match b {
            Block::Paragraph(p) => lines.push(p.plain_text()),
            Block::Table(t) => {
                for row in &t.rows {
                    let cells: Vec<String> = row.cells.iter().filter(|c| !c.covered).map(|c| c.plain_text().replace('\n', " ")).collect();
                    lines.push(cells.join("\t"));
                }
            }
            Block::Raw { .. } => {}
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Styles(Vec<String>);

    #[test]
    fn extensions_by_type() {
        let mut doc = Document::new(Kind::Word);
        doc.extensions.insert(Styles(vec!["Heading1".into()]));
        let copy = doc.clone();
        doc.extensions.get_mut::<Styles>().unwrap().0.push("Title".into());
        assert_eq!(copy.extensions.get::<Styles>().unwrap().0.len(), 1);
        assert_eq!(doc.extensions.remove::<Styles>().unwrap().0.len(), 2);
        assert!(doc.extensions.get::<Styles>().is_none());
    }

    #[test]
    fn origins_dont_affect_equality_or_json() {
        let a = Paragraph::text("hi");
        let mut b = a.clone();
        b.origin = Origin::new(Element::new(crate::xml::ns::W, "p"));
        assert_eq!(a, b);
        assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    }

    #[test]
    fn model_json_round_trips() {
        let mut doc = Document::new(Kind::Word);
        let mut p = Paragraph::text("Hello");
        p.props.align = Some(Align::Center);
        let run = Run::new(" world").with_props(RunProps {
            bold: Some(true),
            color: Some(crate::drawing::Color::scheme("accent1")),
            ..Default::default()
        });
        p.content.push(Inline::Text(run));
        doc.body =
            Body::Flow { blocks: vec![Block::Paragraph(p), Block::Table(Table::from_text(&[vec!["a".into(), "b".into()]], Emu(1000)))] };
        let json = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back.body, doc.body);
        assert_eq!(
            blocks_text(match &back.body {
                Body::Flow { blocks } => blocks,
                _ => unreachable!(),
            }),
            "Hello world\na\tb"
        );
    }

    #[test]
    fn kinds() {
        assert_eq!(Kind::from_extension("PPSX"), Some(Kind::Slides));
        assert_eq!(Kind::from_main_content_type(crate::opc::content_types::types::XLSX_MAIN), Some(Kind::Sheets));
        assert_eq!(Kind::from_main_content_type("application/vnd.ms-word.document.macroEnabled.main+xml"), Some(Kind::Word));
    }
}
