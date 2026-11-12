//! PowerPoint table styles (`ppt/tableStyles.xml`, or PowerPoint's built-in ones a deck names
//! without defining): each part of a table (the whole, banded rows, header row…) with its fill,
//! borders and text, laid under the cells' own formatting.

use std::collections::HashMap;

use office_core::drawing::{Color, Fill};
use office_core::model::{Document, Table};
use office_core::units::parse_bool;
use office_core::xml::{ns, parse_element, Element};
use office_engine::flow::{BorderLine, CellLook, TableLook};

use super::text::{emu_px, Paint};

/// Table styles by id.
#[derive(Default)]
pub struct TableStyles {
    styles: HashMap<String, Element>,
    default: Option<String>,
}

impl TableStyles {
    pub fn read(doc: &Document) -> TableStyles {
        let mut out = TableStyles::default();
        let Some(pkg) = &doc.package else { return out };
        let Some(part) = pkg.part_names().find(|p| p.ends_with("tableStyles.xml")).map(str::to_string) else { return out };
        let Ok(xml) = pkg.xml(&part) else { return out };
        out.default = xml.root.attr("def").map(str::to_string);
        for s in xml.root.children_named(ns::A, "tblStyle") {
            if let Some(id) = s.attr("styleId") {
                out.styles.insert(id.to_string(), s.clone());
            }
        }
        out
    }
}

/// A table's style, resolved: its parts by name (`wholeTbl`, `firstRow`, `band1H`…).
#[derive(Clone, Debug, Default)]
pub struct TableStyle {
    parts: HashMap<String, Element>,
}

/// PowerPoint's default table style (Medium Style 2 – Accent 1), for decks that use it without
/// defining it.
const MEDIUM_2_ACCENT_1: &str = r#"<a:tblStyle xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" styleId="{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}">
<a:wholeTbl><a:tcTxStyle><a:schemeClr val="dk1"/></a:tcTxStyle><a:tcStyle><a:tcBdr>
<a:left><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:left>
<a:right><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:right>
<a:top><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:top>
<a:bottom><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:bottom>
<a:insideH><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:insideH>
<a:insideV><a:ln w="12700"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:insideV>
</a:tcBdr><a:fill><a:solidFill><a:schemeClr val="accent1"><a:tint val="20000"/></a:schemeClr></a:solidFill></a:fill></a:tcStyle></a:wholeTbl>
<a:band1H><a:tcStyle><a:tcBdr/><a:fill><a:solidFill><a:schemeClr val="accent1"><a:tint val="40000"/></a:schemeClr></a:solidFill></a:fill></a:tcStyle></a:band1H>
<a:band1V><a:tcStyle><a:tcBdr/><a:fill><a:solidFill><a:schemeClr val="accent1"><a:tint val="40000"/></a:schemeClr></a:solidFill></a:fill></a:tcStyle></a:band1V>
<a:lastCol><a:tcTxStyle b="on"><a:schemeClr val="lt1"/></a:tcTxStyle><a:tcStyle><a:tcBdr/><a:fill><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:fill></a:tcStyle></a:lastCol>
<a:firstCol><a:tcTxStyle b="on"><a:schemeClr val="lt1"/></a:tcTxStyle><a:tcStyle><a:tcBdr/><a:fill><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:fill></a:tcStyle></a:firstCol>
<a:lastRow><a:tcTxStyle b="on"><a:schemeClr val="lt1"/></a:tcTxStyle><a:tcStyle><a:tcBdr><a:top><a:ln w="38100"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:top></a:tcBdr><a:fill><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:fill></a:tcStyle></a:lastRow>
<a:firstRow><a:tcTxStyle b="on"><a:schemeClr val="lt1"/></a:tcTxStyle><a:tcStyle><a:tcBdr><a:bottom><a:ln w="38100"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:bottom></a:tcBdr><a:fill><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:fill></a:tcStyle></a:firstRow>
</a:tblStyle>"#;

/// Built-in accent variants share the same geometry and differ in their theme color slot.
fn builtin(id: Option<&str>) -> Option<Element> {
    let accent = match id.unwrap_or_default() {
        "{21E4AEA4-8DFA-4A89-87EB-49C32662AFE0}" | "{21E4AEA4-8DFA-4A89-87EB-49C32662AFE8}" => "accent2",
        "{F5AB1C69-6EDB-4FF4-983F-18BD219EF322}" => "accent3",
        "{00A15C55-8517-42AA-B614-E9B94910E393}" => "accent4",
        "{7DF18680-E054-41AD-8BC1-D1AEF772440D}" => "accent5",
        "{93296810-A885-4BE3-A3E7-6D5BEEA58F35}" => "accent6",
        "{2D5ABB26-0587-4C30-8999-92F81FD0307C}" | "{5940675A-B579-460E-94D1-54222C63F5DA}" => return None,
        "{9D7B26C5-4107-4FEC-AEDC-1716B250A1EF}" => return Some(light_style()),
        "{E8034E78-7F5D-4C2E-B375-FC64B27BC917}" => return Some(dark_style()),
        _ => "accent1",
    };
    parse_element(&MEDIUM_2_ACCENT_1.replace("accent1", accent)).ok()
}

fn fill_xml(fill: Option<Color>) -> Element {
    match fill {
        Some(color) => Element::new(ns::A, "solidFill").with_child(color.to_drawingml()),
        None => Element::new(ns::A, "noFill"),
    }
}

fn style_part(name: &str, fill: Option<Color>, bold: bool, text: Option<&str>) -> Element {
    let mut part = Element::new(ns::A, name);
    if bold || text.is_some() {
        let mut tx = Element::new(ns::A, "tcTxStyle");
        if bold {
            tx.set_attr("", "b", "on");
        }
        if let Some(slot) = text {
            tx.push(Color::scheme(slot).to_drawingml());
        }
        part.push(tx);
    }
    part.push(Element::new(ns::A, "tcStyle").with_child(Element::new(ns::A, "fill").with_child(fill_xml(fill))));
    part
}

fn style_border(part: &mut Element, side: &str, slot: Option<&str>, width: i64) {
    let ln = Element::new(ns::A, "ln").with_attr("", "w", width.to_string()).with_child(fill_xml(slot.map(Color::scheme)));
    part.child_or_insert(ns::A, "tcStyle").child_or_insert(ns::A, "tcBdr").push(Element::new(ns::A, side).with_child(ln));
}

fn light_style() -> Element {
    let mut root = Element::new(ns::A, "tblStyle");
    let mut whole = style_part("wholeTbl", None, false, Some("tx1"));
    for side in ["left", "right", "insideH", "insideV"] {
        style_border(&mut whole, side, None, 0);
    }
    for side in ["top", "bottom"] {
        style_border(&mut whole, side, Some("tx1"), 12700);
    }
    root.push(whole);
    for name in ["band1H", "band1V"] {
        root.push(style_part(name, Some(Color::scheme("tx1").with("alpha", 20000)), false, None));
    }
    for name in ["firstCol", "lastCol", "firstRow", "lastRow"] {
        let mut part = style_part(name, None, true, None);
        if name.ends_with("Row") {
            style_border(&mut part, if name == "firstRow" { "bottom" } else { "top" }, Some("tx1"), 12700);
        }
        root.push(part);
    }
    root
}

fn dark_style() -> Element {
    let mut root = Element::new(ns::A, "tblStyle");
    let solid = |tint| Some(Color::scheme("dk1").with("tint", tint));
    let mut whole = style_part("wholeTbl", solid(20000), false, Some("lt1"));
    for side in ["left", "right", "top", "bottom", "insideH", "insideV"] {
        style_border(&mut whole, side, None, 0);
    }
    root.push(whole);
    for name in ["band1H", "band1V"] {
        root.push(style_part(name, solid(40000), false, None));
    }
    for (name, side) in [("firstCol", "right"), ("lastCol", "left"), ("lastRow", "top"), ("firstRow", "bottom")] {
        let fill = if name == "firstRow" { Some(Color::scheme("dk1")) } else { solid(60000) };
        let mut part = style_part(name, fill, true, None);
        style_border(&mut part, side, Some("lt1"), 38100);
        root.push(part);
    }
    root
}

/// The style a table uses: its own, the deck's default, or PowerPoint's default.
pub fn style_for(styles: &TableStyles, t: &Table) -> TableStyle {
    let id = t.style.as_deref().or(styles.default.as_deref());
    let el = id.and_then(|id| styles.styles.get(id).cloned()).or_else(|| builtin(id));
    let mut out = TableStyle::default();
    if let Some(el) = el {
        for part in el.elements() {
            out.parts.insert(part.local.clone(), part.clone());
        }
    }
    // "Table Grid": thin black lines everywhere.
    if id == Some("{5940675A-B579-460E-94D1-54222C63F5DA}") {
        if let Ok(grid) = parse_element(
            r#"<a:wholeTbl xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:tcStyle><a:tcBdr><a:left><a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:left><a:right><a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:right><a:top><a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:top><a:bottom><a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:bottom><a:insideH><a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:insideH><a:insideV><a:ln w="12700"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:insideV></a:tcBdr></a:tcStyle></a:wholeTbl>"#,
        ) {
            out.parts.insert("wholeTbl".into(), grid);
        }
    }
    out
}

/// Which parts apply to a cell, least to most important, as the table's flags turn them on.
fn parts_for(t: &Table, row: usize, col: usize) -> Vec<&'static str> {
    let pr = t.origin.get().and_then(|o| o.child(ns::A, "tblPr"));
    let flag = |k: &str| pr.and_then(|p| p.attr(k)).and_then(parse_bool).unwrap_or(false);
    let rows = t.rows.len();
    let cols = t.columns.len().max(1);
    let first_row = flag("firstRow");
    let first_col = flag("firstCol");
    let mut out = vec!["wholeTbl"];
    if flag("bandRow") {
        let r = row.saturating_sub(first_row as usize);
        if !(first_row && row == 0) {
            out.push(if r.is_multiple_of(2) { "band1H" } else { "band2H" });
        }
    }
    if flag("bandCol") {
        let c = col.saturating_sub(first_col as usize);
        out.push(if c.is_multiple_of(2) { "band1V" } else { "band2V" });
    }
    if flag("lastCol") && col + 1 == cols {
        out.push("lastCol");
    }
    if first_col && col == 0 {
        out.push("firstCol");
    }
    if flag("lastRow") && row + 1 == rows {
        out.push("lastRow");
    }
    if first_row && row == 0 {
        out.push("firstRow");
    }
    out
}

/// A table style's text formatting for a cell.
#[derive(Clone, Debug, Default)]
pub struct PartText {
    pub bold: Option<bool>,
    pub color: Option<Color>,
}

pub fn text_for(t: &Table, style: &TableStyle, row: usize, col: usize) -> PartText {
    let mut out = PartText::default();
    for name in parts_for(t, row, col) {
        let Some(tx) = style.parts.get(name).and_then(|p| p.child(ns::A, "tcTxStyle")) else { continue };
        if let Some(b) = tx.attr("b") {
            out.bold = Some(b == "on");
        }
        if let Some(c) = tx.elements().find(|e| !e.is(ns::A, "fontRef") && !e.is(ns::A, "font")).and_then(Color::from_drawingml) {
            out.color = Some(c);
        }
    }
    out
}

fn line(paint: &Paint, ln: &Element) -> Option<BorderLine> {
    let fill = Fill::in_children(ln);
    let color = match fill {
        Some(Fill::None) => return None,
        Some(f) => super::draw::fill_css(paint, &f)?,
        None => return None,
    };
    let w = ln.attr("w").and_then(|w| w.parse::<f64>().ok()).map(|w| w / 9525.0).unwrap_or(1.0);
    Some(BorderLine { width: w.max(0.5), color, style: "solid" })
}

/// How a table looks: columns, each cell's fill, borders, margins and anchoring.
pub fn look(paint: &Paint, t: &Table, style: &TableStyle, width: f64) -> TableLook {
    let ncols = t.columns.len().max(t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0)).max(1);
    let mut columns: Vec<f64> = (0..ncols).map(|c| t.columns.get(c).map(|e| emu_px(*e)).unwrap_or(0.0)).collect();
    if columns.iter().sum::<f64>() <= 1.0 {
        columns = vec![width / ncols as f64; ncols];
    }
    let rows = t.rows.len();
    let cells = t
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            row.cells
                .iter()
                .enumerate()
                .map(|(c, cell)| {
                    let mut look = CellLook { fill: None, borders: [None, None, None, None], margins: [4.8, 9.6, 4.8, 9.6], valign: 0 };
                    let last_row = r + cell.row_span.max(1) as usize >= rows;
                    let last_col = c + cell.col_span.max(1) as usize >= ncols;
                    for name in parts_for(t, r, c) {
                        let Some(tc) = style.parts.get(name).and_then(|p| p.child(ns::A, "tcStyle")) else { continue };
                        if let Some(f) = tc.child(ns::A, "fill").and_then(Fill::in_children) {
                            look.fill = super::draw::fill_css(paint, &f);
                        }
                        if let Some(bdr) = tc.child(ns::A, "tcBdr") {
                            let side = |outer: &str, inner: &str, edge: bool| {
                                bdr.child(ns::A, if edge { outer } else { inner }).and_then(|s| s.child(ns::A, "ln"))
                            };
                            let sides = [
                                side("top", "insideH", r == 0 || name == "firstRow" || name == "lastRow"),
                                side("right", "insideV", last_col || name == "firstCol" || name == "lastCol"),
                                side("bottom", "insideH", last_row || name == "firstRow" || name == "lastRow"),
                                side("left", "insideV", c == 0 || name == "firstCol" || name == "lastCol"),
                            ];
                            for (i, s) in sides.into_iter().enumerate() {
                                if let Some(ln) = s {
                                    look.borders[i] = line(paint, ln);
                                }
                            }
                        }
                    }
                    // The cell's own formatting.
                    if let Some(f) = &cell.fill {
                        look.fill = Some(paint.color(f).css());
                    }
                    if let Some(pr) = cell.origin.get().and_then(|o| o.child(ns::A, "tcPr")) {
                        if pr.child(ns::A, "noFill").is_some() {
                            look.fill = None;
                        }
                        for (i, k) in ["lnT", "lnR", "lnB", "lnL"].iter().enumerate() {
                            if let Some(ln) = pr.child(ns::A, k) {
                                look.borders[i] = line(paint, ln);
                            }
                        }
                        let m = |k: &str| pr.attr(k).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 9525.0);
                        for (i, k) in ["marT", "marR", "marB", "marL"].iter().enumerate() {
                            if let Some(v) = m(k) {
                                look.margins[i] = v;
                            }
                        }
                        look.valign = match pr.attr("anchor") {
                            Some("ctr") => 1,
                            Some("b") => 2,
                            _ => 0,
                        };
                    }
                    look
                })
                .collect()
        })
        .collect();
    let heights = t.rows.iter().map(|row| row.height.map(|h| (emu_px(h), false))).collect();
    TableLook { columns, indent: 0.0, cells, heights, rtl: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::SlidesView;

    #[test]
    fn offered_styles_resolve_their_own_accent_and_grid() {
        let mut v = SlidesView::blank();
        v.insert_table(3, 2);
        let slide = &v.slides()[0];
        let FrameContent::Table { table } = &slide.frames.last().unwrap().content else { panic!("table") };
        let paint = Paint { doc: &v.doc, slide };
        let styles = TableStyles::default();
        for (id, accent) in [
            ("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}", "accent1"),
            ("{21E4AEA4-8DFA-4A89-87EB-49C32662AFE0}", "accent2"),
            ("{F5AB1C69-6EDB-4FF4-983F-18BD219EF322}", "accent3"),
            ("{00A15C55-8517-42AA-B614-E9B94910E393}", "accent4"),
            ("{7DF18680-E054-41AD-8BC1-D1AEF772440D}", "accent5"),
            ("{93296810-A885-4BE3-A3E7-6D5BEEA58F35}", "accent6"),
        ] {
            let mut t = table.clone();
            t.style = Some(id.into());
            let resolved = style_for(&styles, &t);
            assert_eq!(look(&paint, &t, &resolved, 600.0).cells[0][0].fill, Some(paint.color(&Color::scheme(accent)).css()), "{id}");
        }
        for id in [
            "{2D5ABB26-0587-4C30-8999-92F81FD0307C}",
            "{5940675A-B579-460E-94D1-54222C63F5DA}",
            "{9D7B26C5-4107-4FEC-AEDC-1716B250A1EF}",
            "{E8034E78-7F5D-4C2E-B375-FC64B27BC917}",
        ] {
            let mut t = table.clone();
            t.style = Some(id.into());
            let resolved = style_for(&styles, &t);
            let cell = &look(&paint, &t, &resolved, 600.0).cells[0][0];
            if id.starts_with("{2D5") {
                assert!(cell.borders.iter().all(Option::is_none));
            }
            if id.starts_with("{594") {
                assert!(cell.borders.iter().all(Option::is_some));
            }
            if id.starts_with("{9D7") {
                assert!(cell.borders[1].is_none() && cell.borders[2].is_some());
            }
            if id.starts_with("{E80") {
                assert_eq!(cell.fill, Some(paint.color(&Color::scheme("dk1")).css()));
            }
        }
    }
    use office_core::model::FrameContent;
}
