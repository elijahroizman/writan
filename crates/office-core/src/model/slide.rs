//! Slides: positioned frames, the way a slide (and a Word or Excel drawing) lays things out.

use serde::{Deserialize, Serialize};

use super::{Image, Link, Origin, Paragraph, Table};
use crate::drawing::ShapeProps;
use crate::units::Emu;
use crate::xml::Element;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slide {
    pub frames: Vec<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// The part the slide was read from, so saving writes it back there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// For duplicated slides, the part that owns the original markup's relationship ids.
    #[serde(skip)]
    pub relationship_source: Option<String>,
    /// The layout part the slide is on (`ppt/slideLayouts/slideLayout2.xml`). A new slide
    /// without one goes on the layout of the slide before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(skip)]
    pub origin: Origin,
}

impl Slide {
    pub fn relationship_part(&self) -> Option<&str> {
        self.relationship_source.as_deref().or(self.part.as_deref())
    }
}

/// One object on a slide: a shape (often with text), a picture, a table, a group, or something
/// the core doesn't model (a chart, SmartArt, an embedded object), kept whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Position, size, outline, fill and line.
    #[serde(default)]
    pub shape: ShapeProps,
    /// The layout placeholder this frame fills, which it inherits position and formatting from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Placeholder>,
    /// A click on the frame during a slideshow opens this link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
    pub content: FrameContent,
    #[serde(skip)]
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum FrameContent {
    Shape {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<TextBody>,
        /// A connector line (`p:cxnSp`) rather than a shape.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        connector: bool,
    },
    Picture {
        image: Image,
    },
    Table {
        table: Table,
    },
    Group {
        frames: Vec<Frame>,
    },
    Raw {
        xml: Element,
        /// A copied chart's package part, independent of the destination slide's relationship ids.
        #[serde(skip)]
        chart_part: Option<String>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placeholder {
    /// `title`, `body`, `ctrTitle`, `subTitle`, `dt`, `ftr`, `sldNum`, `pic`… (`None` is body).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

/// The text in a shape. The body's own settings (insets, wrapping, autofit, columns) and its
/// list styles stay in `origin` for the PowerPoint layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextBody {
    pub paragraphs: Vec<Paragraph>,
    #[serde(skip)]
    pub origin: Origin,
}

/// A slide's size. PowerPoint's default is 16:9, 12 192 000 × 6 858 000 EMUs (13.33 × 7.5 in).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideSize {
    pub width: Emu,
    pub height: Emu,
}

impl Default for SlideSize {
    fn default() -> SlideSize {
        SlideSize { width: Emu(12_192_000), height: Emu(6_858_000) }
    }
}

impl Frame {
    /// A text box: a rectangle shape at `xfrm` holding `paragraphs`.
    pub fn text_box(id: u32, xfrm: crate::drawing::Xfrm, paragraphs: Vec<Paragraph>) -> Frame {
        Frame {
            id,
            name: format!("TextBox {id}"),
            description: None,
            hidden: false,
            shape: ShapeProps { xfrm: Some(xfrm), preset: Some("rect".into()), ..Default::default() },
            placeholder: None,
            link: None,
            content: FrameContent::Shape { text: Some(TextBody { paragraphs, origin: Origin::default() }), connector: false },
            origin: Origin::default(),
        }
    }

    /// Every paragraph in the frame and, for groups, its children, in order.
    pub fn paragraphs(&self) -> Vec<&Paragraph> {
        let mut out = Vec::new();
        match &self.content {
            FrameContent::Shape { text: Some(body), .. } => out.extend(&body.paragraphs),
            FrameContent::Table { table } => {
                for cell in table.rows.iter().flat_map(|r| &r.cells) {
                    out.extend(cell.blocks.iter().filter_map(|b| match b {
                        super::Block::Paragraph(p) => Some(p),
                        _ => None,
                    }));
                }
            }
            FrameContent::Group { frames } => out.extend(frames.iter().flat_map(Frame::paragraphs)),
            _ => {}
        }
        out
    }
}
