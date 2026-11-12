//! What the editor's toolbar and inspector show: the selection's formatting, the table or
//! picture it's in, undo, pages.

use office_core::drawing::ColorBase;
use office_core::model::{Baseline, Inline, LineSpacing, RunProps, Strike};
use office_engine::pos;
use office_engine::text;
use serde::Serialize;

use super::{image_crop, StoryId, WordView};
use crate::{word, StoryKind, StyleKind};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordState {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub font: Option<String>,
    /// Half-points.
    pub size: Option<u32>,
    /// `RRGGBB`.
    pub color: Option<String>,
    /// Word's highlight name.
    pub highlight: Option<String>,
    pub style: Option<String>,
    /// As on the page: a right-to-left paragraph aligned to its start is `right`.
    pub align: Option<&'static str>,
    /// The paragraph is right to left.
    pub rtl: bool,
    /// `bullet` or `numbered`.
    pub list: Option<String>,
    /// Set on the paragraph itself (not from its style).
    pub line_spacing: Option<f64>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dirty: bool,
    /// Text is selected (not just a caret).
    pub has_selection: bool,
    pub table: Option<TableState>,
    pub image: Option<ImageState>,
    /// `header` or `footer` while one is being edited.
    pub header_footer: Option<&'static str>,
    pub link: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableState {
    pub rows: usize,
    pub columns: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageState {
    pub width_emu: i64,
    pub height_emu: i64,
    /// Percent cut from each side.
    pub crop: CropState,
    pub can_crop: bool,
    pub can_resize: bool,
    pub rotation_degrees: f64,
    pub media: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CropState {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleOption {
    pub style_id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSetup {
    pub page_width_twips: i64,
    pub page_height_twips: i64,
    pub orientation: &'static str,
    pub margins_twips: Margins,
}

#[derive(Clone, Debug, Serialize)]
pub struct Margins {
    pub top: i64,
    pub bottom: i64,
    pub left: i64,
    pub right: i64,
}

impl WordView {
    /// The selection's formatting and surroundings.
    pub fn state(&self) -> WordState {
        self.state_now()
    }

    pub(crate) fn state_now(&self) -> WordState {
        let blocks = self.blocks(self.story);
        let (from, to) = self.sel.ordered();
        let mut st = WordState {
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            dirty: self.is_dirty(),
            has_selection: !self.sel.is_collapsed() && self.selected_image().is_none(),
            header_footer: self.editing_header_footer().map(|k| if k == StoryKind::Header { "header" } else { "footer" }),
            link: self.link(),
            ..Default::default()
        };
        let Some(p) = pos::para(blocks, &from.path) else { return st };
        // Formatting: the runs the selection covers, resolved through styles. A toggle is on
        // only if it's on for all of them.
        let resolve = |para: &office_core::model::Paragraph, direct: &RunProps| match word(&self.doc) {
            Some(w) => w.styles.resolve_run(&para.props, direct, self.doc.theme.as_ref()),
            None => direct.clone(),
        };
        let mut runs: Vec<RunProps> = Vec::new();
        if self.sel.is_collapsed() {
            let direct = self.pending.clone().unwrap_or_else(|| text::props_at(p, from.offset));
            runs.push(resolve(p, &direct));
        } else {
            for path in pos::paras_between(blocks, &from.path, &to.path) {
                let Some(q) = pos::para(blocks, &path) else { continue };
                let a = if path == from.path { from.offset } else { 0 };
                let b = if path == to.path { to.offset } else { text::len(q) };
                let flat = text::flat(q);
                for (i, inline) in q.content.iter().enumerate() {
                    let (s, e) = flat.spans[i];
                    if let Inline::Text(r) = inline {
                        if e > a && s < b {
                            runs.push(resolve(q, &r.props));
                        }
                    }
                }
                if runs.len() > 2000 {
                    break;
                }
            }
            if runs.is_empty() {
                runs.push(resolve(p, &text::props_at(p, from.offset)));
            }
        }
        // Right-to-left runs show Word's complex-script formatting, as Word's toolbar does.
        for r in &mut runs {
            if r.rtl == Some(true) {
                let c = super::resolve::complex_props(r);
                (r.font, r.size, r.bold, r.italic) = (c.font, c.size, c.bold, c.italic);
            }
        }
        let all = |f: &dyn Fn(&RunProps) -> bool| runs.iter().all(f);
        st.bold = all(&|r| r.bold == Some(true));
        st.italic = all(&|r| r.italic == Some(true));
        st.underline = all(&|r| r.underline.as_deref().is_some_and(|u| u != "none"));
        st.strike = all(&|r| r.strike.is_some_and(|s| s != Strike::None));
        st.superscript = all(&|r| r.baseline == Some(Baseline::Superscript));
        st.subscript = all(&|r| r.baseline == Some(Baseline::Subscript));
        let same = |f: &dyn Fn(&RunProps) -> Option<String>| {
            let first = f(&runs[0]);
            runs.iter().all(|r| f(r) == first).then_some(first).flatten()
        };
        st.font = same(&|r| Some(r.font.clone().unwrap_or_else(|| super::resolve::DEFAULT_FONT.to_string())));
        st.size = same(&|r| r.size.map(|s| s.half_points().to_string()))
            .and_then(|s| s.parse().ok())
            .or_else(|| runs[0].size.is_none().then_some(20));
        st.color = same(&|r| r.color.as_ref().map(|c| super::resolve::paint_color(self.doc.theme.as_ref(), c).rgb.hex()));
        st.highlight = same(&|r| {
            r.highlight.as_ref().map(|c| match &c.base {
                ColorBase::Preset { name } => name.clone(),
                ColorBase::Rgb { value } => value.hex(),
                _ => String::new(),
            })
        });
        let para = match word(&self.doc) {
            Some(w) => w.styles.resolve_para(&p.props),
            None => p.props.clone(),
        };
        st.style = para.style.clone();
        st.rtl = para.rtl == Some(true);
        st.align = Some(match (para.align, st.rtl) {
            (Some(office_core::model::Align::Center), _) => "center",
            (Some(office_core::model::Align::Justify | office_core::model::Align::Distribute), _) => "justify",
            (Some(office_core::model::Align::Right), false) | (Some(office_core::model::Align::Left) | None, true) => "right",
            _ => "left",
        });
        st.line_spacing = match p.props.line_spacing {
            Some(LineSpacing::Multiple(m)) => Some((m.fraction() * 100.0).round() / 100.0),
            _ => None,
        };
        if self.story == StoryId::Body {
            if let Some((num, _)) = crate::list_of(&self.doc, p).filter(|(n, _)| n != "0") {
                let bullet =
                    word(&self.doc).and_then(|w| w.numbering.levels(&num)).and_then(|l| l.first()).is_some_and(|l| l.format == "bullet");
                st.list = Some(if bullet { "bullet" } else { "numbered" }.into());
            }
        }
        if let Some((_, _, _, t)) = self.table_at_caret() {
            st.table =
                Some(TableState { rows: t.rows.len(), columns: t.columns.len().max(t.rows.first().map(|r| r.cells.len()).unwrap_or(0)) });
        }
        if let Some((_, _, img)) = self.selected_image() {
            let crop = image_crop(img).unwrap_or([0.0; 4]);
            let rot = img
                .origin
                .get()
                .and_then(|o| o.descendant(office_core::xml::ns::A, "xfrm"))
                .and_then(|x| x.attr("rot"))
                .and_then(|r| r.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 60_000.0;
            st.image = Some(ImageState {
                width_emu: img.width.0,
                height_emu: img.height.0,
                crop: CropState { left: crop[0] * 100.0, top: crop[1] * 100.0, right: crop[2] * 100.0, bottom: crop[3] * 100.0 },
                can_crop: true,
                can_resize: true,
                rotation_degrees: rot,
                media: img.media.clone(),
            });
        }
        st
    }

    /// Paragraph styles to offer: the document's quick styles and the ones in use, with Word's
    /// common built-in styles.
    pub fn style_options(&self) -> Vec<StyleOption> {
        let Some(w) = word(&self.doc) else { return Vec::new() };
        let mut out: Vec<StyleOption> = w
            .styles
            .styles
            .iter()
            .filter(|s| s.kind == StyleKind::Paragraph && (s.quick || s.default))
            .map(|s| StyleOption { style_id: s.id.clone(), name: display_name(&s.name) })
            .collect();
        for (id, name) in [
            ("Normal", "Normal"),
            ("Title", "Title"),
            ("Subtitle", "Subtitle"),
            ("Heading1", "Heading 1"),
            ("Heading2", "Heading 2"),
            ("Heading3", "Heading 3"),
            ("Quote", "Quote"),
        ] {
            if !out.iter().any(|o| o.style_id == id || o.name == name) {
                out.push(StyleOption { style_id: id.into(), name: name.into() });
            }
        }
        out
    }

    pub fn page_setup_state(&self) -> PageSetup {
        let s = self.page_setup();
        let tw = |e: office_core::units::Emu| e.twips();
        PageSetup {
            page_width_twips: tw(s.page_width),
            page_height_twips: tw(s.page_height),
            orientation: if s.landscape || s.page_width > s.page_height { "landscape" } else { "portrait" },
            margins_twips: Margins {
                top: tw(s.margin_top),
                bottom: tw(s.margin_bottom),
                left: tw(s.margin_left),
                right: tw(s.margin_right),
            },
        }
    }

    /// The fonts the document's text uses, for the font menu.
    pub fn document_fonts(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let blocks = self.blocks(StoryId::Body);
        let mut at = pos::story_start(blocks).map(|p| p.path);
        let mut n = 0;
        while let Some(path) = at {
            if let Some(p) = pos::para(blocks, &path) {
                for r in p.runs() {
                    if let Some(f) = &r.props.font {
                        if !out.contains(f) {
                            out.push(f.clone());
                        }
                    }
                }
            }
            n += 1;
            if n > 5000 {
                break;
            }
            at = pos::next_para(blocks, &path);
        }
        out
    }
}

/// Word's style names as its menus show them ("heading 1" is "Heading 1").
fn display_name(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
