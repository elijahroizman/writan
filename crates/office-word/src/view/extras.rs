//! What drawing a Word document needs that the model doesn't read: where paragraphs may break
//! across pages, tab stops, list indents, and what table styles draw. All of it comes from the
//! styles, numbering and paragraphs' own markup, read once when the view opens.

use std::collections::HashMap;

use office_core::formats::docx::val;
use office_core::units::{parse_bool, Emu};
use office_core::xml::{ns, Element, XmlDoc};
use office_engine::para::{Tab, TabKind};

/// Paragraph settings outside the model, as a style or a paragraph sets them (`None`: not set).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParaExtras {
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow_control: Option<bool>,
    pub contextual_spacing: Option<bool>,
    /// Tab stops set (positive) and cleared (`clear`).
    pub tabs: Vec<(f64, TabKind, bool)>,
}

impl ParaExtras {
    pub fn read(ppr: &Element) -> ParaExtras {
        let flag = |local: &str| ppr.child(ns::W, local).map(|e| e.attr_ns(ns::W, "val").and_then(parse_bool).unwrap_or(true));
        let tabs = ppr
            .child(ns::W, "tabs")
            .map(|t| {
                t.children_named(ns::W, "tab")
                    .filter_map(|tab| {
                        let pos = tab.attr_ns(ns::W, "pos")?.parse::<i64>().ok()?;
                        let kind = match tab.attr_ns(ns::W, "val").unwrap_or("left") {
                            "center" => TabKind::Center,
                            "right" | "end" => TabKind::Right,
                            "decimal" => TabKind::Decimal,
                            "clear" => return Some((twips_px(pos), TabKind::Left, true)),
                            _ => TabKind::Left,
                        };
                        Some((twips_px(pos), kind, false))
                    })
                    .collect()
            })
            .unwrap_or_default();
        ParaExtras {
            keep_next: flag("keepNext"),
            keep_lines: flag("keepLines"),
            page_break_before: flag("pageBreakBefore"),
            widow_control: flag("widowControl"),
            contextual_spacing: flag("contextualSpacing"),
            tabs,
        }
    }

    /// Lays `over` on these.
    pub fn overlay(&mut self, over: &ParaExtras) {
        macro_rules! take {
            ($($f:ident),*) => { $( if over.$f.is_some() { self.$f = over.$f; } )* };
        }
        take!(keep_next, keep_lines, page_break_before, widow_control, contextual_spacing);
        for (pos, kind, clear) in &over.tabs {
            self.tabs.retain(|(p, _, _)| (p - pos).abs() > 0.5);
            if !clear {
                self.tabs.push((*pos, *kind, false));
            }
        }
        self.tabs.sort_by(|a, b| a.0.total_cmp(&b.0));
    }

    pub fn tab_stops(&self) -> Vec<Tab> {
        self.tabs.iter().filter(|(_, _, c)| !c).map(|(pos, kind, _)| Tab { pos: *pos, kind: *kind }).collect()
    }
}

pub fn twips_px(t: i64) -> f64 {
    t as f64 / 15.0
}

/// A border as Word writes it (`w:top w:val="single" w:sz="4" w:color="auto"`).
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    /// `None` for `nil`/`none`: no line.
    pub style: Option<&'static str>,
    /// In pixels.
    pub width: f64,
    /// `RRGGBB`, or `None` for automatic (black).
    pub color: Option<String>,
}

impl Border {
    pub fn read(el: &Element) -> Border {
        let style = match el.attr_ns(ns::W, "val").unwrap_or("single") {
            "nil" | "none" => None,
            "double" | "triple" | "thinThickSmallGap" | "thickThinSmallGap" => Some("double"),
            "dotted" => Some("dotted"),
            "dashed" | "dashSmallGap" | "dotDash" | "dotDotDash" => Some("dashed"),
            _ => Some("solid"),
        };
        // Eighths of a point.
        let size = el.attr_ns(ns::W, "sz").and_then(|v| v.parse::<f64>().ok()).unwrap_or(4.0);
        let color = el.attr_ns(ns::W, "color").filter(|c| *c != "auto").map(str::to_string);
        Border { style, width: (size / 8.0 * 96.0 / 72.0).max(0.75), color }
    }
}

/// Borders by side: top, left (start), bottom, right (end), and the lines between cells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Borders {
    pub top: Option<Border>,
    pub left: Option<Border>,
    pub bottom: Option<Border>,
    pub right: Option<Border>,
    pub inside_h: Option<Border>,
    pub inside_v: Option<Border>,
}

impl Borders {
    pub fn read(el: &Element) -> Borders {
        let side = |names: &[&str]| names.iter().find_map(|n| el.child(ns::W, n)).map(Border::read);
        Borders {
            top: side(&["top"]),
            left: side(&["start", "left"]),
            bottom: side(&["bottom"]),
            right: side(&["end", "right"]),
            inside_h: side(&["insideH"]),
            inside_v: side(&["insideV"]),
        }
    }

    pub fn overlay(&mut self, over: &Borders) {
        for (a, b) in [
            (&mut self.top, &over.top),
            (&mut self.left, &over.left),
            (&mut self.bottom, &over.bottom),
            (&mut self.right, &over.right),
            (&mut self.inside_h, &over.inside_h),
            (&mut self.inside_v, &over.inside_v),
        ] {
            if b.is_some() {
                *a = b.clone();
            }
        }
    }
}

/// What a table style (or a table's own properties) says about drawing it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableLookStyle {
    pub borders: Borders,
    pub shading: Option<String>,
    /// Cell margins: top, left, bottom, right (pixels).
    pub margins: [Option<f64>; 4],
    /// Conditional parts: `firstRow`, `lastRow`, `firstCol`, `lastCol`, `band1Horz`, `band1Vert`…
    pub parts: HashMap<String, PartLook>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartLook {
    pub shading: Option<String>,
    pub borders: Borders,
    pub bold: Option<bool>,
    /// `RRGGBB`, or a theme color left to the resolver.
    pub color: Option<office_core::drawing::Color>,
}

/// A `w:color`: a theme color if it names one, else its RGB value; `auto` is no color.
fn word_color(el: &Element) -> Option<office_core::drawing::Color> {
    if let Some(theme) = el.attr_ns(ns::W, "themeColor") {
        return Some(office_core::drawing::Color::scheme(theme));
    }
    office_core::drawing::Rgb::parse(el.attr_ns(ns::W, "val")?).map(office_core::drawing::Color::rgb)
}

fn shading(el: Option<&Element>) -> Option<String> {
    let fill = el?.attr_ns(ns::W, "fill")?;
    (fill != "auto").then(|| fill.to_string())
}

fn margins(el: &Element) -> [Option<f64>; 4] {
    let m = |names: &[&str]| {
        names.iter().find_map(|n| el.child(ns::W, n)).and_then(|e| e.attr_ns(ns::W, "w")).and_then(|v| v.parse::<i64>().ok()).map(twips_px)
    };
    [m(&["top"]), m(&["start", "left"]), m(&["bottom"]), m(&["end", "right"])]
}

impl TableLookStyle {
    /// Reads a style's or a table's `w:tblPr` (and the style's conditional parts).
    pub fn read(tbl_pr: Option<&Element>, style: Option<&Element>) -> TableLookStyle {
        let mut out = TableLookStyle::default();
        if let Some(p) = tbl_pr {
            out.read_pr(p);
        }
        if let Some(style) = style {
            if let Some(tc) = style.child(ns::W, "tcPr") {
                out.shading = shading(tc.child(ns::W, "shd")).or(out.shading.take());
            }
            for part in style.children_named(ns::W, "tblStylePr") {
                let Some(kind) = part.attr_ns(ns::W, "type") else { continue };
                let tc = part.child(ns::W, "tcPr");
                let rpr = part.child(ns::W, "rPr");
                out.parts.insert(
                    kind.to_string(),
                    PartLook {
                        shading: shading(tc.and_then(|t| t.child(ns::W, "shd"))),
                        borders: tc.and_then(|t| t.child(ns::W, "tcBorders")).map(Borders::read).unwrap_or_default(),
                        bold: rpr.and_then(|r| r.child(ns::W, "b")).map(|b| b.attr_ns(ns::W, "val").and_then(parse_bool).unwrap_or(true)),
                        color: rpr.and_then(|r| r.child(ns::W, "color")).and_then(word_color),
                    },
                );
            }
        }
        out
    }

    fn read_pr(&mut self, p: &Element) {
        if let Some(b) = p.child(ns::W, "tblBorders") {
            self.borders.overlay(&Borders::read(b));
        }
        if let Some(s) = shading(p.child(ns::W, "shd")) {
            self.shading = Some(s);
        }
        if let Some(m) = p.child(ns::W, "tblCellMar") {
            for (a, b) in self.margins.iter_mut().zip(margins(m)) {
                if b.is_some() {
                    *a = b;
                }
            }
        }
    }

    pub fn overlay(&mut self, over: &TableLookStyle) {
        self.borders.overlay(&over.borders);
        if over.shading.is_some() {
            self.shading = over.shading.clone();
        }
        for (a, b) in self.margins.iter_mut().zip(over.margins) {
            if b.is_some() {
                *a = b;
            }
        }
        for (k, v) in &over.parts {
            self.parts.insert(k.clone(), v.clone());
        }
    }
}

/// A list level's indent and label formatting (`w:lvl/w:pPr`, `w:lvl/w:rPr`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelLook {
    pub indent_start: Option<Emu>,
    pub first_line: Option<Emu>,
    pub tabs: Vec<(f64, TabKind, bool)>,
    pub run: office_core::model::RunProps,
}

/// Everything above, for one document.
#[derive(Clone, Debug, Default)]
pub struct Extras {
    /// By paragraph style id, with the style's chain already applied.
    pub para_styles: HashMap<String, ParaExtras>,
    pub default_para: ParaExtras,
    /// By table style id, chain applied.
    pub table_styles: HashMap<String, TableLookStyle>,
    pub default_table: Option<String>,
    /// By `numId`: each level's look.
    pub levels: HashMap<String, Vec<LevelLook>>,
    /// The default tab stop interval, in pixels.
    pub default_tab: f64,
    /// The document's number formats for footnote and endnote marks (`w:numFmt`), when set.
    pub footnote_format: Option<String>,
    pub endnote_format: Option<String>,
}

impl Extras {
    /// A note's mark as it shows: its number in the document's format (Word numbers footnotes
    /// 1, 2, 3 and endnotes i, ii, iii unless told otherwise).
    pub fn note_label(&self, kind: crate::NoteKind, id: &str) -> String {
        let format = match kind {
            crate::NoteKind::Footnote => self.footnote_format.as_deref().unwrap_or("decimal"),
            crate::NoteKind::Endnote => self.endnote_format.as_deref().unwrap_or("lowerRoman"),
        };
        match id.parse::<u32>() {
            Ok(n) => crate::numbering::format_number(n, format),
            Err(_) => id.to_string(),
        }
    }

    pub fn read(styles: Option<&XmlDoc>, numbering: Option<&XmlDoc>, settings: Option<&XmlDoc>) -> Extras {
        let mut out = Extras { default_tab: 48.0, ..Default::default() };
        if let Some(s) = settings {
            if let Some(v) = s.root.child(ns::W, "defaultTabStop").and_then(val).and_then(|v| v.parse::<i64>().ok()) {
                if v > 0 {
                    out.default_tab = twips_px(v);
                }
            }
            let format = |pr: &str| s.root.path(ns::W, &[pr, "numFmt"]).and_then(val).map(str::to_string);
            out.footnote_format = format("footnotePr");
            out.endnote_format = format("endnotePr");
        }
        if let Some(doc) = styles {
            let root = &doc.root;
            if let Some(ppr) = root.path(ns::W, &["docDefaults", "pPrDefault", "pPr"]) {
                out.default_para = ParaExtras::read(ppr);
            }
            let by_id: HashMap<&str, &Element> =
                root.children_named(ns::W, "style").filter_map(|s| Some((s.attr_ns(ns::W, "styleId")?, s))).collect();
            let chain = |id: &str| {
                let mut out: Vec<&Element> = Vec::new();
                let mut next = by_id.get(id).copied();
                while let Some(s) = next {
                    if out.iter().any(|o| std::ptr::eq(*o, s)) || out.len() > 32 {
                        break;
                    }
                    out.push(s);
                    next = s.child(ns::W, "basedOn").and_then(val).and_then(|b| by_id.get(b).copied());
                }
                out.reverse();
                out
            };
            for (id, s) in &by_id {
                match s.attr_ns(ns::W, "type").unwrap_or("paragraph") {
                    "paragraph" => {
                        let mut e = ParaExtras::default();
                        for st in chain(id) {
                            if let Some(ppr) = st.child(ns::W, "pPr") {
                                e.overlay(&ParaExtras::read(ppr));
                            }
                        }
                        out.para_styles.insert(id.to_string(), e);
                    }
                    "table" => {
                        let mut look = TableLookStyle::default();
                        for st in chain(id) {
                            look.overlay(&TableLookStyle::read(st.child(ns::W, "tblPr"), Some(st)));
                        }
                        if s.attr_ns(ns::W, "default").and_then(parse_bool).unwrap_or(false) {
                            out.default_table = Some(id.to_string());
                        }
                        out.table_styles.insert(id.to_string(), look);
                    }
                    _ => {}
                }
            }
        }
        if let Some(doc) = numbering {
            let root = &doc.root;
            let read_level = |l: &Element| {
                let ppr = l.child(ns::W, "pPr");
                let ind = ppr.and_then(|p| p.child(ns::W, "ind"));
                let tw = |k: &str| ind.and_then(|i| i.attr_ns(ns::W, k)).and_then(|v| Emu::parse(v, Emu::PER_TWIP));
                LevelLook {
                    indent_start: tw("start").or_else(|| tw("left")),
                    first_line: tw("firstLine").or_else(|| tw("hanging").map(|h| Emu(-h.0))),
                    tabs: ppr.map(|p| ParaExtras::read(p).tabs).unwrap_or_default(),
                    run: l.child(ns::W, "rPr").map(office_core::formats::docx::read_run_props).unwrap_or_default(),
                }
            };
            let mut abstracts: HashMap<&str, Vec<LevelLook>> = HashMap::new();
            for a in root.children_named(ns::W, "abstractNum") {
                let Some(id) = a.attr_ns(ns::W, "abstractNumId") else { continue };
                let mut levels = vec![LevelLook::default(); 9];
                for l in a.children_named(ns::W, "lvl") {
                    if let Some(i) = l.attr_ns(ns::W, "ilvl").and_then(|v| v.parse::<usize>().ok()).filter(|i| *i < 9) {
                        levels[i] = read_level(l);
                    }
                }
                abstracts.insert(id, levels);
            }
            for num in root.children_named(ns::W, "num") {
                let (Some(id), Some(abs)) = (num.attr_ns(ns::W, "numId"), num.child(ns::W, "abstractNumId").and_then(val)) else {
                    continue;
                };
                let mut levels = abstracts.get(abs).cloned().unwrap_or_else(|| vec![LevelLook::default(); 9]);
                for o in num.children_named(ns::W, "lvlOverride") {
                    let (Some(i), Some(l)) =
                        (o.attr_ns(ns::W, "ilvl").and_then(|v| v.parse::<usize>().ok()).filter(|i| *i < 9), o.child(ns::W, "lvl"))
                    else {
                        continue;
                    };
                    levels[i] = read_level(l);
                }
                out.levels.insert(id.to_string(), levels);
            }
        }
        out
    }

    /// A paragraph's extras: the document's defaults, its style's, then its own.
    pub fn para(&self, style: Option<&str>, own: Option<&Element>) -> ParaExtras {
        let mut e = self.default_para.clone();
        if let Some(s) = style.and_then(|s| self.para_styles.get(s)) {
            e.overlay(s);
        }
        if let Some(ppr) = own {
            e.overlay(&ParaExtras::read(ppr));
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use office_core::xml::parse_element;

    const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";

    #[test]
    fn style_chains_and_tabs() {
        let styles = parse_element(&format!(
            r#"<w:styles {W}>
              <w:style w:type="paragraph" w:styleId="Heading1"><w:pPr><w:keepNext/><w:tabs><w:tab w:val="right" w:pos="9000"/></w:tabs></w:pPr></w:style>
              <w:style w:type="paragraph" w:styleId="Heading2"><w:basedOn w:val="Heading1"/><w:pPr><w:keepNext w:val="0"/></w:pPr></w:style>
              <w:style w:type="table" w:styleId="TableGrid"><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="8" w:color="FF0000"/></w:tblBorders><w:tblCellMar><w:left w:w="108"/></w:tblCellMar></w:tblPr></w:style>
            </w:styles>"#
        ))
        .unwrap();
        let e = Extras::read(Some(&XmlDoc::new(styles)), None, None);
        let h1 = e.para(Some("Heading1"), None);
        assert_eq!(h1.keep_next, Some(true));
        assert_eq!(h1.tab_stops(), vec![Tab { pos: 600.0, kind: TabKind::Right }]);
        assert_eq!(e.para(Some("Heading2"), None).keep_next, Some(false));
        let grid = &e.table_styles["TableGrid"];
        assert_eq!(grid.borders.inside_h.as_ref().unwrap().color.as_deref(), Some("FF0000"));
        assert_eq!(grid.margins[1], Some(7.2));
    }

    #[test]
    fn list_level_indents() {
        let numbering = parse_element(&format!(
            r#"<w:numbering {W}>
              <w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
            </w:numbering>"#
        ))
        .unwrap();
        let e = Extras::read(None, Some(&XmlDoc::new(numbering)), None);
        let l = &e.levels["1"][0];
        assert_eq!(l.indent_start, Some(Emu::from_twips(720)));
        assert_eq!(l.first_line, Some(Emu::from_twips(-360)));
    }
}
