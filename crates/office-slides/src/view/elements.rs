//! What the editor's inspector and slide strip read about slides and their frames: positions in
//! pixels, the formatting text and shapes show, tables' shape, slides' layouts, notes and
//! transitions.

use office_core::drawing::Fill;
use office_core::model::{Frame, FrameContent, Strike};
use office_core::xml::ns;
use serde::Serialize;

use super::draw::{Drawer, Map};
use super::SlidesView;
use office_engine::measure::Measurer;
use office_engine::para::Ctx;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLook {
    pub font_family: Option<String>,
    /// Points.
    pub font_size: Option<f64>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// `#rrggbb`.
    pub color: Option<String>,
    pub align: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeLook {
    /// `solid` or `none`.
    pub fill_mode: &'static str,
    pub fill_color: Option<String>,
    pub stroke_color: Option<String>,
    /// Points.
    pub stroke_width: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub rows: usize,
    pub columns: usize,
    pub header: bool,
    pub banded: bool,
    pub style: Option<String>,
    pub text_style: Option<TextLook>,
}

/// A frame as the inspector sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub id: String,
    /// `text`, `shape`, `connector`, `image`, `table`, `group`, `other`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<office_core::model::Link>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_style: Option<TextLook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_style: Option<ShapeLook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<TableInfo>,
    /// All the frame's text, paragraphs on their own lines.
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Element>,
    /// The picture's media, for cropping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// Crop shares (left, top, right, bottom), 0 to 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
}

/// A slide as the strip and inspector see it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideInfo {
    pub id: String,
    pub hidden: bool,
    pub layout: Option<String>,
    pub layout_name: Option<String>,
    pub notes: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<crate::Transition>,
    /// What the slideshow animates on the slide, one step per click.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub animations: Vec<crate::AnimationStep>,
    pub revision: u64,
    /// Whether it shows its master's and layout's shapes.
    pub show_master_shapes: bool,
}

/// A layout new slides can go on.
#[derive(Clone, Debug, Serialize)]
pub struct LayoutInfo {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

impl SlidesView {
    pub fn slide_infos(&self) -> Vec<SlideInfo> {
        self.slides()
            .iter()
            .enumerate()
            .map(|(i, s)| SlideInfo {
                id: s.part.clone().unwrap_or_else(|| format!("new-{}", self.revisions.get(i).copied().unwrap_or(0))),
                hidden: s.hidden,
                layout: s.layout.clone(),
                layout_name: crate::layouts(&self.doc).into_iter().find(|t| Some(&t.part) == s.layout.as_ref()).map(|t| t.name.clone()),
                notes: crate::notes(&self.doc, i).iter().map(|p| p.plain_text()).collect::<Vec<_>>().join("\n"),
                transition: crate::transition(s).filter(|t| !t.kind.is_empty() || !t.on_click || t.after_ms.is_some()),
                animations: crate::animation_steps(s),
                revision: self.revisions.get(i).copied().unwrap_or(0),
                show_master_shapes: s
                    .origin
                    .get()
                    .and_then(|o| o.attr("showMasterSp"))
                    .and_then(office_core::units::parse_bool)
                    .unwrap_or(true),
            })
            .collect()
    }

    pub fn layout_infos(&self) -> Vec<LayoutInfo> {
        crate::layouts(&self.doc)
            .into_iter()
            .map(|t| LayoutInfo {
                path: t.part.clone(),
                name: if t.name.is_empty() { t.layout_type.clone().unwrap_or_default() } else { t.name.clone() },
                kind: t.layout_type.clone(),
            })
            .collect()
    }

    /// The slide's frames as the inspector sees them.
    pub fn elements(&mut self, host: &mut dyn Measurer, index: usize) -> Vec<Element> {
        let Some(slide) = self.slides().get(index).cloned() else { return Vec::new() };
        let caret = self.caret_para(index);
        let mut ctx = Ctx { host, measure: &mut self.measure, styles: &mut self.styles };
        let d = Drawer { doc: &self.doc, art: &self.art, tables: &self.tables, ctx: &mut ctx, cache: &mut self.cache, caret };
        slide.frames.iter().map(|f| element(&d, &slide, f, Map::IDENTITY)).collect()
    }
}

fn element(d: &Drawer, slide: &office_core::model::Slide, f: &Frame, map: Map) -> Element {
    let b = d.boxed(slide, f, map);
    let txbox = f.origin.get().and_then(|o| o.path(ns::P, &["nvSpPr", "cNvSpPr"])).and_then(|c| c.attr("txBox")).is_some_and(|v| v == "1");
    let kind = match &f.content {
        FrameContent::Shape { connector: true, .. } => "connector",
        FrameContent::Shape { .. } if txbox || (f.placeholder.is_some() && f.shape.fill.is_none()) => "text",
        FrameContent::Shape { .. } => "shape",
        FrameContent::Picture { .. } => "image",
        FrameContent::Table { .. } => "table",
        FrameContent::Group { .. } => "group",
        FrameContent::Raw { .. } => "other",
    };
    let resolved = crate::resolve(d.doc, slide, f);
    let text_style = match &f.content {
        FrameContent::Shape { text: Some(body), .. } => {
            let first = resolved.paragraphs.first();
            let run =
                first.and_then(|(_, runs)| runs.first()).or_else(|| body.paragraphs.first()?.mark.as_ref()).cloned().unwrap_or_default();
            let look = d.look(slide, f);
            let paint = super::text::Paint { doc: d.doc, slide };
            let color = run.color.as_ref().or(look.text_color.as_ref()).map(|c| paint.color(c).css());
            Some(TextLook {
                font_family: run.font.clone().or_else(|| Some(paint.body_font())),
                font_size: Some(run.size.map(|s| s.points()).unwrap_or(18.0)),
                bold: run.bold == Some(true),
                italic: run.italic == Some(true),
                underline: run.underline.as_deref().is_some_and(|u| u != "none"),
                strikethrough: run.strike.is_some_and(|s| s != Strike::None),
                color: color.map(|c| if c.starts_with('#') { c } else { "#000000".into() }),
                align: match first.and_then(|(p, _)| p.align) {
                    Some(office_core::model::Align::Center) => "center",
                    Some(office_core::model::Align::Right) => "right",
                    Some(office_core::model::Align::Justify | office_core::model::Align::Distribute) => "justify",
                    _ => "left",
                },
            })
        }
        _ => None,
    };
    let shape_style = match &f.content {
        FrameContent::Shape { .. } => {
            let look = d.look(slide, f);
            Some(ShapeLook {
                fill_mode: if look.fill.is_some() && !matches!(f.shape.fill, Some(Fill::None)) { "solid" } else { "none" },
                fill_color: look.fill.clone(),
                stroke_color: look.line.as_ref().map(|l| l.0.clone()),
                stroke_width: look.line.as_ref().map(|l| (l.1 * 72.0 / 96.0 * 100.0).round() / 100.0).unwrap_or(0.0),
            })
        }
        _ => None,
    };
    let table = match &f.content {
        FrameContent::Table { table } => {
            let pr = table.origin.get().and_then(|o| o.child(ns::A, "tblPr"));
            let flag = |k: &str| pr.and_then(|p| p.attr(k)).and_then(office_core::units::parse_bool).unwrap_or(false);
            Some(TableInfo {
                rows: table.rows.len(),
                columns: table.columns.len(),
                header: flag("firstRow"),
                banded: flag("bandRow"),
                style: table.style.clone(),
                text_style: table
                    .rows
                    .first()
                    .and_then(|r| r.cells.first())
                    .and_then(|c| {
                        c.blocks.iter().find_map(|b| match b {
                            office_core::model::Block::Paragraph(p) => Some(p),
                            _ => None,
                        })
                    })
                    .map(|p| {
                        let props = p.runs().next().map(|r| &r.props).or(p.mark.as_ref());
                        TextLook {
                            font_family: props
                                .and_then(|r| r.font.clone())
                                .or_else(|| Some(super::text::Paint { doc: d.doc, slide }.body_font())),
                            font_size: Some(props.and_then(|r| r.size).map(|s| s.points()).unwrap_or(18.0)),
                            bold: props.and_then(|r| r.bold).unwrap_or(false),
                            italic: props.and_then(|r| r.italic).unwrap_or(false),
                            underline: props.and_then(|r| r.underline.as_deref()).is_some_and(|u| u != "none"),
                            strikethrough: props.and_then(|r| r.strike).is_some_and(|s| s != Strike::None),
                            color: props.and_then(|r| r.color.as_ref()).map(|c| super::text::Paint { doc: d.doc, slide }.color(c).css()),
                            align: match p.props.align {
                                Some(office_core::model::Align::Center) => "center",
                                Some(office_core::model::Align::Right) => "right",
                                Some(office_core::model::Align::Justify) => "justify",
                                _ => "left",
                            },
                        }
                    }),
            })
        }
        _ => None,
    };
    let children = match &f.content {
        FrameContent::Group { frames } => {
            let child = map.child(&f.shape.xfrm.clone().unwrap_or_default());
            frames.iter().map(|c| element(d, slide, c, child)).collect()
        }
        _ => Vec::new(),
    };
    let (media, crop) = match &f.content {
        FrameContent::Picture { image } => (
            Some(image.media.clone()),
            f.origin.get().and_then(|o| o.descendant(ns::A, "srcRect")).map(|rect| {
                let v = |k: &str| rect.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
                [v("l"), v("t"), v("r"), v("b")]
            }),
        ),
        _ => (None, None),
    };
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    Element {
        id: f.id.to_string(),
        kind,
        name: f.name.clone(),
        link_tag: f.description.as_deref().filter(|t| t.starts_with(super::EXCEL_LINK_PREFIX)).map(str::to_string),
        link: f.link.clone(),
        x: r2(b.x),
        y: r2(b.y),
        width: r2(b.w),
        height: r2(b.h),
        rotation: r2(b.rot),
        flip_horizontal: b.flip_h,
        flip_vertical: b.flip_v,
        text_style,
        shape_style,
        table,
        text: f.paragraphs().iter().map(|p| p.plain_text()).collect::<Vec<_>>().join("\n"),
        placeholder: f.placeholder.as_ref().map(|p| p.kind.clone().unwrap_or_else(|| "body".into())),
        children,
        media,
        crop,
    }
}
