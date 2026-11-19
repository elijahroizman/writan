//! The style sheet (`xl/styles.xml`): what each cell's style index means. A cell's `s` points
//! into `cellXfs`, and each of those names a number format, a font, a fill and a border, plus
//! its alignment and protection.
//!
//! Reading resolves an index into a [`CellFormat`]; [`StyleSheet::add`] finds or makes the
//! index for a format, adding only the fonts, fills, borders and formats that aren't there yet,
//! and leaving everything else in the part (cell styles, differential formats for conditional
//! formatting, extensions) as it was.

use office_core::drawing::Color;
use office_core::formats::xlsx::{read_color, write_color};
use office_core::opc::{content_types::types, rels::kind, Package};
use office_core::units::parse_bool;
use office_core::xml::{ns, Element, Node, XmlDoc};
use serde::{Deserialize, Serialize};

use crate::numfmt::builtin;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Font {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// In points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    /// `single`, `double`, `singleAccounting`, `doubleAccounting`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    #[serde(default)]
    pub strike: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// `superscript` or `subscript`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vert_align: Option<String>,
    /// `major` or `minor`: the theme's heading or body font, which Excel shows in place of `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    /// `none`, `solid`, `gray125`, `darkGrid`… A gradient fill reads as `none`.
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<Color>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BorderSide {
    /// `thin`, `medium`, `thick`, `dashed`, `double`… `None` is no line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Border {
    #[serde(default)]
    pub left: BorderSide,
    #[serde(default)]
    pub right: BorderSide,
    #[serde(default)]
    pub top: BorderSide,
    #[serde(default)]
    pub bottom: BorderSide,
    #[serde(default)]
    pub diagonal: BorderSide,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alignment {
    /// `left`, `center`, `right`, `fill`, `justify`, `centerContinuous`, `distributed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<String>,
    /// `top`, `center`, `bottom`, `justify`, `distributed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<String>,
    #[serde(default)]
    pub wrap: bool,
    #[serde(default)]
    pub indent: u32,
    /// Degrees, 0–180 (91–180 are downward), or 255 for stacked text.
    #[serde(default)]
    pub rotation: u32,
    #[serde(default)]
    pub shrink: bool,
}

/// Everything a cell's style index says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellFormat {
    /// The format code: `General`, `0.00`, `d-mmm-yy`…
    pub number_format: String,
    pub font: Font,
    pub fill: Fill,
    pub border: Border,
    pub alignment: Alignment,
    /// Locked cells can't be edited once the sheet is protected (they're locked unless a format
    /// says not).
    pub locked: bool,
    /// Hidden cells' formulas don't show once the sheet is protected.
    pub hidden: bool,
}

impl Default for CellFormat {
    fn default() -> CellFormat {
        CellFormat {
            number_format: "General".into(),
            // Excel's: Calibri 11 from the theme, in the theme's text color.
            font: Font {
                name: Some("Calibri".into()),
                size: Some(11.0),
                scheme: Some("minor".into()),
                color: Some(Color::scheme("dk1")),
                ..Default::default()
            },
            fill: Fill { pattern: "none".into(), ..Default::default() },
            border: Border::default(),
            alignment: Alignment::default(),
            locked: true,
            hidden: false,
        }
    }
}

/// One `cellXfs` entry: indexes into the lists, and its own alignment and protection.
#[derive(Clone, Debug, PartialEq)]
struct Xf {
    num_fmt: u32,
    font: usize,
    fill: usize,
    border: usize,
    alignment: Alignment,
    locked: bool,
    hidden: bool,
}

#[derive(Clone, Debug)]
pub struct StyleSheet {
    /// The part as read (or the default for a new workbook), with what [`StyleSheet::add`] added.
    doc: XmlDoc,
    num_fmts: Vec<(u32, String)>,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    xfs: Vec<Xf>,
    /// Differential formats, which conditional formatting applies by index.
    dxfs: Vec<Dxf>,
    /// Whether anything was added since reading.
    changed: bool,
}

/// The children of `styleSheet`, in schema order.
const STYLE_SHEET: &[&[&str]] = &[
    &["numFmts"],
    &["fonts"],
    &["fills"],
    &["borders"],
    &["cellStyleXfs"],
    &["cellXfs"],
    &["cellStyles"],
    &["dxfs"],
    &["tableStyles"],
    &["colors"],
    &["extLst"],
];

fn val(el: &Element, local: &str) -> Option<String> {
    el.child(ns::X, local).and_then(|e| e.attr("val")).map(str::to_string)
}

/// `<b/>` is on, `<b val="0"/>` off.
fn flag(el: &Element, local: &str) -> bool {
    el.child(ns::X, local).is_some_and(|e| e.attr("val").and_then(parse_bool).unwrap_or(true))
}

fn read_font(el: &Element) -> Font {
    Font {
        name: val(el, "name"),
        size: val(el, "sz").and_then(|s| s.parse().ok()),
        bold: flag(el, "b"),
        italic: flag(el, "i"),
        underline: el.child(ns::X, "u").map(|u| u.attr("val").unwrap_or("single").to_string()),
        strike: flag(el, "strike"),
        color: el.child(ns::X, "color").and_then(read_color),
        vert_align: val(el, "vertAlign").filter(|v| v != "baseline"),
        scheme: val(el, "scheme").filter(|v| v != "none"),
    }
}

fn read_fill(el: &Element) -> Fill {
    let Some(p) = el.child(ns::X, "patternFill") else { return Fill { pattern: "none".into(), ..Default::default() } };
    Fill {
        // A pattern fill without a type is solid if it has a color, as Excel reads it.
        pattern: p.attr("patternType").map(str::to_string).unwrap_or_else(|| {
            if p.child(ns::X, "fgColor").is_some() {
                "solid".into()
            } else {
                "none".into()
            }
        }),
        fg: p.child(ns::X, "fgColor").and_then(read_color),
        bg: p.child(ns::X, "bgColor").and_then(read_color),
    }
}

fn read_border(el: &Element) -> Border {
    let side = |local: &str| {
        el.child(ns::X, local)
            .map(|s| BorderSide {
                style: s.attr("style").filter(|s| *s != "none").map(str::to_string),
                color: s.child(ns::X, "color").and_then(read_color),
            })
            .unwrap_or_default()
    };
    Border { left: side("left"), right: side("right"), top: side("top"), bottom: side("bottom"), diagonal: side("diagonal") }
}

/// What a differential format changes: only what it sets, laid over a cell's own format.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dxf {
    pub number_format: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub strike: Option<bool>,
    /// `Some(None)` turns underlining off.
    pub underline: Option<Option<String>>,
    pub color: Option<Color>,
    /// The fill color; a differential solid fill keeps it in `bgColor`.
    pub fill: Option<Color>,
    pub border: Border,
}

impl Dxf {
    /// `f` with this format laid over it.
    pub fn apply(&self, f: &mut CellFormat) {
        if let Some(n) = &self.number_format {
            f.number_format = n.clone();
        }
        if let Some(b) = self.bold {
            f.font.bold = b;
        }
        if let Some(i) = self.italic {
            f.font.italic = i;
        }
        if let Some(s) = self.strike {
            f.font.strike = s;
        }
        if let Some(u) = &self.underline {
            f.font.underline = u.clone();
        }
        if let Some(c) = &self.color {
            f.font.color = Some(c.clone());
        }
        if let Some(c) = &self.fill {
            f.fill = Fill { pattern: "solid".into(), fg: Some(c.clone()), bg: None };
        }
        for (to, from) in [
            (&mut f.border.left, &self.border.left),
            (&mut f.border.right, &self.border.right),
            (&mut f.border.top, &self.border.top),
            (&mut f.border.bottom, &self.border.bottom),
        ] {
            if from.style.is_some() {
                *to = from.clone();
            }
        }
    }
}

/// A `dxf` element, in the main namespace or (inline in an `x14:cfRule`) the 2009 one.
pub fn read_dxf(el: &Element) -> Dxf {
    let child = |e: &Element, local: &str| e.elements().find(|c| c.local == local).cloned();
    let mut d = Dxf::default();
    if let Some(n) = child(el, "numFmt") {
        d.number_format = n.attr("formatCode").map(str::to_string);
    }
    if let Some(font) = child(el, "font") {
        let on = |local: &str| child(&font, local).map(|e| e.attr("val").and_then(parse_bool).unwrap_or(true));
        d.bold = on("b");
        d.italic = on("i");
        d.strike = on("strike");
        d.underline = child(&font, "u").map(|u| Some(u.attr("val").unwrap_or("single").to_string()).filter(|v| v != "none"));
        d.color = child(&font, "color").as_ref().and_then(read_color);
    }
    if let Some(p) = child(el, "fill").and_then(|f| child(&f, "patternFill")) {
        if p.attr("patternType") != Some("none") {
            d.fill = child(&p, "bgColor").as_ref().and_then(read_color).or_else(|| child(&p, "fgColor").as_ref().and_then(read_color));
        }
    }
    if let Some(b) = child(el, "border") {
        let side = |local: &str| {
            child(&b, local)
                .map(|s| BorderSide {
                    style: s.attr("style").filter(|s| *s != "none").map(str::to_string),
                    color: child(&s, "color").as_ref().and_then(read_color),
                })
                .unwrap_or_default()
        };
        d.border =
            Border { left: side("left"), right: side("right"), top: side("top"), bottom: side("bottom"), diagonal: BorderSide::default() };
    }
    d
}

fn read_xf(el: &Element) -> Xf {
    let n = |k: &str| el.attr(k).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let a = el.child(ns::X, "alignment");
    let p = el.child(ns::X, "protection");
    Xf {
        num_fmt: n("numFmtId") as u32,
        font: n("fontId"),
        fill: n("fillId"),
        border: n("borderId"),
        alignment: a
            .map(|a| Alignment {
                horizontal: a.attr("horizontal").filter(|h| *h != "general").map(str::to_string),
                vertical: a.attr("vertical").filter(|v| *v != "bottom").map(str::to_string),
                wrap: a.attr("wrapText").and_then(parse_bool).unwrap_or(false),
                indent: a.attr("indent").and_then(|v| v.parse().ok()).unwrap_or(0),
                rotation: a.attr("textRotation").and_then(|v| v.parse().ok()).unwrap_or(0),
                shrink: a.attr("shrinkToFit").and_then(parse_bool).unwrap_or(false),
            })
            .unwrap_or_default(),
        locked: p.and_then(|p| p.attr("locked")).and_then(parse_bool).unwrap_or(true),
        hidden: p.and_then(|p| p.attr("hidden")).and_then(parse_bool).unwrap_or(false),
    }
}

fn x(local: &str) -> Element {
    Element::new(ns::X, local)
}

fn with_val(local: &str, v: impl Into<String>) -> Element {
    x(local).with_attr("", "val", v)
}

fn write_font(f: &Font) -> Element {
    let mut el = x("font");
    if f.bold {
        el.push(x("b"));
    }
    if f.italic {
        el.push(x("i"));
    }
    if f.strike {
        el.push(x("strike"));
    }
    if let Some(u) = &f.underline {
        el.push(if u == "single" { x("u") } else { with_val("u", u) });
    }
    if let Some(v) = &f.vert_align {
        el.push(with_val("vertAlign", v));
    }
    if let Some(s) = f.size {
        el.push(with_val("sz", office_core::model::format_number(s)));
    }
    if let Some(c) = &f.color {
        el.push(write_color(c));
    }
    if let Some(n) = &f.name {
        el.push(with_val("name", n));
    }
    if let Some(s) = &f.scheme {
        el.push(with_val("scheme", s));
    }
    el
}

fn write_fill(f: &Fill) -> Element {
    let mut p = x("patternFill").with_attr("", "patternType", &f.pattern);
    let named = |local: &str, c: &Color| {
        let mut e = write_color(c);
        e.local = local.to_string();
        e
    };
    if let Some(c) = &f.fg {
        p.push(named("fgColor", c));
    }
    if let Some(c) = &f.bg {
        p.push(named("bgColor", c));
    }
    x("fill").with_child(p)
}

fn write_border(b: &Border) -> Element {
    let side = |local: &str, s: &BorderSide| {
        let mut e = x(local);
        if let Some(style) = &s.style {
            e.set_attr("", "style", style);
        }
        if let Some(c) = &s.color {
            e.push(write_color(c));
        }
        e
    };
    x("border")
        .with_child(side("left", &b.left))
        .with_child(side("right", &b.right))
        .with_child(side("top", &b.top))
        .with_child(side("bottom", &b.bottom))
        .with_child(side("diagonal", &b.diagonal))
}

fn write_xf(xf: &Xf) -> Element {
    let mut el = x("xf")
        .with_attr("", "numFmtId", xf.num_fmt.to_string())
        .with_attr("", "fontId", xf.font.to_string())
        .with_attr("", "fillId", xf.fill.to_string())
        .with_attr("", "borderId", xf.border.to_string())
        .with_attr("", "xfId", "0");
    for (k, on) in
        [("applyNumberFormat", xf.num_fmt != 0), ("applyFont", xf.font != 0), ("applyFill", xf.fill != 0), ("applyBorder", xf.border != 0)]
    {
        if on {
            el.set_attr("", k, "1");
        }
    }
    let a = &xf.alignment;
    if a != &Alignment::default() {
        el.set_attr("", "applyAlignment", "1");
        let mut al = x("alignment");
        if let Some(h) = &a.horizontal {
            al.set_attr("", "horizontal", h);
        }
        if let Some(v) = &a.vertical {
            al.set_attr("", "vertical", v);
        }
        if a.rotation != 0 {
            al.set_attr("", "textRotation", a.rotation.to_string());
        }
        if a.wrap {
            al.set_attr("", "wrapText", "1");
        }
        if a.indent != 0 {
            al.set_attr("", "indent", a.indent.to_string());
        }
        if a.shrink {
            al.set_attr("", "shrinkToFit", "1");
        }
        el.push(al);
    }
    if !xf.locked || xf.hidden {
        el.set_attr("", "applyProtection", "1");
        let mut p = x("protection");
        if !xf.locked {
            p.set_attr("", "locked", "0");
        }
        if xf.hidden {
            p.set_attr("", "hidden", "1");
        }
        el.push(p);
    }
    el
}

impl StyleSheet {
    /// The style sheet a workbook relates to, if it has one.
    pub fn read(pkg: &Package) -> office_core::Result<Option<StyleSheet>> {
        let main = pkg.main_part()?;
        let Some((_, part)) = pkg.related(&main, kind::STYLES)?.into_iter().next() else { return Ok(None) };
        if !pkg.has(&part) {
            return Ok(None);
        }
        Ok(Some(StyleSheet::parse(pkg.xml(&part)?)))
    }

    pub fn parse(doc: XmlDoc) -> StyleSheet {
        let root = &doc.root;
        let list = |name: &str, item: &str| {
            root.child(ns::X, name).map(|l| l.children_named(ns::X, item).cloned().collect::<Vec<_>>()).unwrap_or_default()
        };
        let num_fmts = list("numFmts", "numFmt")
            .iter()
            .filter_map(|f| Some((f.attr("numFmtId")?.parse().ok()?, f.attr("formatCode")?.to_string())))
            .collect();
        let fonts = list("fonts", "font").iter().map(read_font).collect();
        let fills = list("fills", "fill").iter().map(read_fill).collect();
        let borders = list("borders", "border").iter().map(read_border).collect();
        let xfs = list("cellXfs", "xf").iter().map(read_xf).collect();
        let dxfs = list("dxfs", "dxf").iter().map(read_dxf).collect();
        StyleSheet { num_fmts, fonts, fills, borders, xfs, dxfs, doc, changed: false }
    }

    /// The style sheet a new workbook gets: what the core writes for one, so indexes agree.
    pub fn new_default() -> StyleSheet {
        let d = CellFormat::default();
        let mut root = x("styleSheet");
        root.declare(None, ns::X);
        let gray = Fill { pattern: "gray125".into(), ..Default::default() };
        let xf = Xf { num_fmt: 0, font: 0, fill: 0, border: 0, alignment: Alignment::default(), locked: true, hidden: false };
        let lists = [
            ("fonts", vec![write_font(&d.font)]),
            ("fills", vec![write_fill(&d.fill), write_fill(&gray)]),
            ("borders", vec![write_border(&d.border)]),
            (
                "cellStyleXfs",
                vec![x("xf")
                    .with_attr("", "numFmtId", "0")
                    .with_attr("", "fontId", "0")
                    .with_attr("", "fillId", "0")
                    .with_attr("", "borderId", "0")],
            ),
            ("cellXfs", vec![write_xf(&xf)]),
            ("cellStyles", vec![x("cellStyle").with_attr("", "name", "Normal").with_attr("", "xfId", "0").with_attr("", "builtinId", "0")]),
        ];
        for (name, items) in lists {
            let n = items.len();
            root.push(items.into_iter().fold(x(name), |l, i| l.with_child(i)).with_attr("", "count", n.to_string()));
        }
        let mut s = StyleSheet::parse(XmlDoc::new(root));
        s.changed = true;
        s
    }

    /// How many cell formats there are; style indexes run from 0 to one less.
    pub fn len(&self) -> usize {
        self.xfs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.xfs.is_empty()
    }

    /// The format code for a number format id: the workbook's own, or a built-in.
    pub fn number_format(&self, id: u32) -> &str {
        self.num_fmts.iter().find(|(i, _)| *i == id).map(|(_, c)| c.as_str()).or_else(|| builtin(id)).unwrap_or("General")
    }

    /// Differential format `index`, as conditional formatting refers to it.
    pub fn dxf(&self, index: usize) -> Option<&Dxf> {
        self.dxfs.get(index)
    }

    /// What style index `index` says. An index past the end (a broken file) is the default.
    pub fn format(&self, index: u32) -> CellFormat {
        let Some(xf) = self.xfs.get(index as usize) else { return CellFormat::default() };
        CellFormat {
            number_format: self.number_format(xf.num_fmt).to_string(),
            font: self.fonts.get(xf.font).cloned().unwrap_or_default(),
            fill: self.fills.get(xf.fill).cloned().unwrap_or_default(),
            border: self.borders.get(xf.border).cloned().unwrap_or_default(),
            alignment: xf.alignment.clone(),
            locked: xf.locked,
            hidden: xf.hidden,
        }
    }

    /// The style index for `format`: one that already says it, or a new one.
    pub fn add(&mut self, format: &CellFormat) -> u32 {
        let num_fmt = self.find_or_add_num_fmt(&format.number_format);
        let font = find_or_push(&mut self.fonts, &format.font, &mut self.doc.root, "fonts", write_font, &mut self.changed);
        let fill = find_or_push(&mut self.fills, &format.fill, &mut self.doc.root, "fills", write_fill, &mut self.changed);
        let border = find_or_push(&mut self.borders, &format.border, &mut self.doc.root, "borders", write_border, &mut self.changed);
        let xf = Xf { num_fmt, font, fill, border, alignment: format.alignment.clone(), locked: format.locked, hidden: format.hidden };
        find_or_push(&mut self.xfs, &xf, &mut self.doc.root, "cellXfs", write_xf, &mut self.changed) as u32
    }

    fn find_or_add_num_fmt(&mut self, code: &str) -> u32 {
        if let Some((id, _)) = self.num_fmts.iter().find(|(_, c)| c == code) {
            return *id;
        }
        if let Some(id) = (0..164).find(|&i| builtin(i) == Some(code)) {
            return id;
        }
        // The workbook's own formats are numbered from 164.
        let id = self.num_fmts.iter().map(|(i, _)| *i + 1).max().unwrap_or(164).max(164);
        self.num_fmts.push((id, code.to_string()));
        let list = list_mut(&mut self.doc.root, "numFmts");
        list.push(x("numFmt").with_attr("", "numFmtId", id.to_string()).with_attr("", "formatCode", code));
        recount(list);
        self.changed = true;
        id
    }

    /// Writes the style sheet into `pkg` for the workbook, if anything was added to it.
    pub fn write(&self, pkg: &mut Package) -> office_core::Result<()> {
        if !self.changed {
            return Ok(());
        }
        let main = pkg.main_part()?;
        let part = pkg.related(&main, kind::STYLES)?.into_iter().next().map(|(_, p)| p).unwrap_or_else(|| "xl/styles.xml".to_string());
        pkg.put_xml(&part, &self.doc, types::XLSX_STYLES);
        pkg.relate(&main, kind::STYLES, &part)?;
        Ok(())
    }
}

/// The list element `name` in the style sheet, made in its place if there isn't one.
fn list_mut<'a>(root: &'a mut Element, name: &str) -> &'a mut Element {
    if root.child(ns::X, name).is_none() {
        root.place(x(name).with_attr("", "count", "0"), STYLE_SHEET);
    }
    root.child_mut(ns::X, name).unwrap()
}

fn recount(list: &mut Element) {
    let n = list.elements().count();
    list.set_attr("", "count", n.to_string());
}

fn find_or_push<T: PartialEq + Clone>(
    items: &mut Vec<T>,
    item: &T,
    root: &mut Element,
    list: &str,
    write: fn(&T) -> Element,
    changed: &mut bool,
) -> usize {
    if let Some(i) = items.iter().position(|x| x == item) {
        return i;
    }
    items.push(item.clone());
    let l = list_mut(root, list);
    l.children.push(Node::Element(write(item)));
    recount(l);
    *changed = true;
    items.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use office_core::drawing::Rgb;

    #[test]
    fn formats_round_trip_through_the_part() {
        let mut s = StyleSheet::new_default();
        assert_eq!(s.format(0), CellFormat::default());
        let money = CellFormat {
            number_format: "\"$\"#,##0.00".into(),
            font: Font { bold: true, color: Some(Color::rgb(Rgb(0xC0, 0, 0))), ..CellFormat::default().font },
            fill: Fill { pattern: "solid".into(), fg: Some(Color::scheme("accent1")), bg: None },
            border: Border { bottom: BorderSide { style: Some("thin".into()), color: None }, ..Default::default() },
            alignment: Alignment { horizontal: Some("right".into()), wrap: true, ..Default::default() },
            locked: false,
            hidden: false,
        };
        let i = s.add(&money);
        assert_eq!(i, 1);
        assert_eq!(s.add(&money), 1, "the same format is found, not added again");
        let percent = CellFormat { number_format: "0%".into(), ..CellFormat::default() };
        assert_eq!(s.add(&percent), 2);
        // Read back from the XML it wrote.
        let again = StyleSheet::parse(office_core::xml::parse(&s.doc.to_bytes()).unwrap());
        assert_eq!(again.format(1), money);
        assert_eq!(again.format(2).number_format, "0%");
        assert_eq!(again.num_fmts, vec![(164, "\"$\"#,##0.00".to_string())], "0% is built in");
        assert_eq!(again.format(99), CellFormat::default());
    }
}
