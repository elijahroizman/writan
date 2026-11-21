//! Charts: reading a sheet's drawing and its chart parts into [`Chart`]s, and writing them back.
//!
//! A chart that's as it was read keeps its part, byte for byte, and its anchor in the drawing;
//! one that only moved gets a new anchor. A changed or new chart is written fresh, which keeps
//! what the model holds (kind, title, legend, series and their colors) and Excel's defaults for
//! the rest. Pictures and shapes in the same drawing stay as they are.

use std::collections::HashSet;

use crate::formats::common::{rel_attr, PartRels};
use crate::model::{
    format_number, ref_numbers, ref_texts, AnchorPoint, Chart, ChartAnchor, ChartGrouping, ChartKind, ChartSeries, ChartSource,
    LegendPosition, SeriesCache, Sheet, SheetPicture,
};
use crate::opc::{rels::kind, Package};
use crate::xml::{ns, Element, Node, XmlDoc};
use crate::Result;

pub const DRAWING_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
pub const CHART_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const CHART_REL: &str = "chart";
const CHART_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

// ---- reading ----

/// The charts in a worksheet's drawing, back to front.
pub(super) fn read_charts(pkg: &Package, sheet_part: &str) -> Result<Vec<Chart>> {
    let mut out = Vec::new();
    for (_, drawing) in pkg.related(sheet_part, kind::DRAWING)? {
        if !pkg.has(&drawing) {
            continue;
        }
        let Ok(doc) = pkg.xml(&drawing) else { continue };
        let rels = PartRels::new(pkg, &drawing)?;
        for anchor in doc.root.elements() {
            let Some(at) = read_anchor(anchor) else { continue };
            let Some(part) = chart_part_of(anchor, &rels) else { continue };
            let Ok(xml) = pkg.xml(&part) else { continue };
            let Some(mut chart) = read_chart(&xml.root) else { continue };
            chart.anchor = at;
            chart.source = Some(ChartSource { drawing: drawing.clone(), part, read_as: Box::new(chart.clone()) });
            out.push(chart);
        }
    }
    Ok(out)
}

/// The pictures in a worksheet's drawing, back to front.
pub(super) fn read_pictures(pkg: &Package, sheet_part: &str) -> Result<Vec<SheetPicture>> {
    let mut out = Vec::new();
    for (_, drawing) in pkg.related(sheet_part, kind::DRAWING)? {
        if !pkg.has(&drawing) {
            continue;
        }
        let Ok(doc) = pkg.xml(&drawing) else { continue };
        let rels = PartRels::new(pkg, &drawing)?;
        for anchor in doc.root.elements() {
            let Some(at) = read_anchor(anchor) else { continue };
            let Some(pic) = anchor.child(ns::XDR, "pic") else { continue };
            let Some(blip) = pic.descendant(ns::A, "blip") else { continue };
            let Some(media) = rel_attr(blip, "embed").and_then(|id| rels.part(id)).filter(|p| pkg.has(p)) else { continue };
            let crop = pic.descendant(ns::A, "srcRect").map(|r| {
                let f = |k: &str| r.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
                [f("l"), f("t"), f("r"), f("b")]
            });
            let props = pic.child(ns::XDR, "nvPicPr").and_then(|n| n.child(ns::XDR, "cNvPr"));
            let text = |k: &str| props.and_then(|p| p.attr(k)).filter(|v| !v.is_empty()).map(str::to_string);
            out.push(SheetPicture {
                anchor: at,
                media,
                crop: crop.filter(|c| c.iter().any(|v| *v != 0.0)),
                name: text("name"),
                description: text("descr"),
                read_as: Some(at),
            });
        }
    }
    Ok(out)
}

/// The chart part an anchor shows, if it shows one.
fn chart_part_of(anchor: &Element, rels: &PartRels) -> Option<String> {
    let c = anchor.descendant(ns::C, "chart")?;
    let part = rels.part(rel_attr(c, "id")?)?;
    rels.pkg.has(&part).then_some(part)
}

fn read_point(el: &Element) -> AnchorPoint {
    let n = |local: &str| el.child(ns::XDR, local).map(|e| e.text().trim().parse::<i64>().unwrap_or(0)).unwrap_or(0);
    AnchorPoint { col: n("col").max(0) as u32, col_off: n("colOff"), row: n("row").max(0) as u32, row_off: n("rowOff") }
}

fn read_ext(el: &Element) -> Option<(i64, i64)> {
    let e = el.child(ns::XDR, "ext")?;
    Some((e.attr("cx")?.parse().ok()?, e.attr("cy")?.parse().ok()?))
}

fn read_anchor(anchor: &Element) -> Option<ChartAnchor> {
    if !anchor.in_ns(ns::XDR) {
        return None;
    }
    match anchor.local.as_str() {
        "twoCellAnchor" => Some(ChartAnchor {
            from: read_point(anchor.child(ns::XDR, "from")?),
            to: Some(read_point(anchor.child(ns::XDR, "to")?)),
            size: None,
        }),
        "oneCellAnchor" => Some(ChartAnchor { from: read_point(anchor.child(ns::XDR, "from")?), to: None, size: read_ext(anchor) }),
        "absoluteAnchor" => {
            let pos = anchor.child(ns::XDR, "pos")?;
            let n = |k: &str| pos.attr(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            Some(ChartAnchor { from: AnchorPoint { col: 0, col_off: n("x"), row: 0, row_off: n("y") }, to: None, size: read_ext(anchor) })
        }
        _ => None,
    }
}

fn val<'a>(el: &'a Element, local: &str) -> Option<&'a str> {
    el.child(ns::C, local)?.attr("val")
}

/// A chart part's `chartSpace`, as far as the model goes.
pub fn read_chart(root: &Element) -> Option<Chart> {
    let chart = root.child(ns::C, "chart")?;
    let plot = chart.child(ns::C, "plotArea")?;
    let groups: Vec<&Element> = plot.elements().filter(|e| e.in_ns(ns::C) && e.local.ends_with("Chart")).collect();
    let first = groups.first()?;
    let kind = match first.local.as_str() {
        "barChart" | "bar3DChart" => {
            if val(first, "barDir") == Some("bar") {
                ChartKind::Bar
            } else {
                ChartKind::Column
            }
        }
        "lineChart" | "line3DChart" => ChartKind::Line,
        "areaChart" | "area3DChart" => ChartKind::Area,
        "pieChart" | "pie3DChart" => ChartKind::Pie,
        "doughnutChart" => ChartKind::Doughnut,
        "scatterChart" => ChartKind::Scatter,
        _ => ChartKind::Other,
    };
    let grouping = match val(first, "grouping") {
        Some("stacked") => ChartGrouping::Stacked,
        Some("percentStacked") => ChartGrouping::PercentStacked,
        _ => ChartGrouping::Standard,
    };
    let mut series: Vec<(u32, ChartSeries)> = Vec::new();
    for g in &groups {
        for s in g.children_named(ns::C, "ser") {
            let order = val(s, "order").and_then(|v| v.parse().ok()).unwrap_or(series.len() as u32);
            series.push((order, read_series(s)));
        }
    }
    series.sort_by_key(|(o, _)| *o);
    let series: Vec<ChartSeries> = series.into_iter().map(|(_, s)| s).collect();
    let deleted = val(chart, "autoTitleDeleted").and_then(crate::units::parse_bool).unwrap_or(false);
    let title = match chart.child(ns::C, "title") {
        Some(t) => title_text(t).or_else(|| auto_title(&series)),
        // Excel titles a chart of one series with its name unless the title was deleted.
        None if !deleted && series.len() == 1 => auto_title(&series),
        None => None,
    };
    let legend = chart.child(ns::C, "legend").map(|l| match val(l, "legendPos") {
        Some("b") => LegendPosition::Bottom,
        Some("t") => LegendPosition::Top,
        Some("l") => LegendPosition::Left,
        _ => LegendPosition::Right,
    });
    // Axis titles, from the axes the first group plots on. A scatter chart's two value axes
    // are told apart by the group's axId order (x first); other kinds read by the axis' kind.
    let ids: Vec<&str> = first.children_named(ns::C, "axId").filter_map(|a| a.attr("val")).collect();
    let (mut cat_title, mut val_title) = (None, None);
    for ax in plot.elements().filter(|e| e.in_ns(ns::C) && matches!(e.local.as_str(), "catAx" | "valAx" | "dateAx")) {
        let Some(t) = ax.child(ns::C, "title").and_then(title_text) else { continue };
        let id = ax.child(ns::C, "axId").and_then(|a| a.attr("val"));
        if !id.is_some_and(|i| ids.contains(&i)) {
            continue;
        }
        let slot = if ax.local == "valAx" && kind == ChartKind::Scatter {
            if id == ids.first().copied() {
                &mut cat_title
            } else if id == ids.get(1).copied() {
                &mut val_title
            } else {
                continue;
            }
        } else if ax.local == "valAx" {
            &mut val_title
        } else {
            &mut cat_title
        };
        if slot.is_none() {
            *slot = Some(t);
        }
    }
    Some(Chart { kind, grouping, title, cat_title, val_title, legend, series, anchor: ChartAnchor::default(), source: None })
}

fn auto_title(series: &[ChartSeries]) -> Option<String> {
    (series.len() == 1).then(|| series[0].cache.name.clone().or_else(|| series[0].name.clone())).flatten()
}

fn title_text(t: &Element) -> Option<String> {
    let tx = t.child(ns::C, "tx")?;
    if let Some(rich) = tx.child(ns::C, "rich") {
        let paras: Vec<String> = rich
            .children_named(ns::A, "p")
            .map(|p| p.elements().filter(|r| r.is(ns::A, "r") || r.is(ns::A, "fld")).map(|r| r.text()).collect())
            .collect();
        let text = paras.join(" ").trim().to_string();
        return (!text.is_empty()).then_some(text);
    }
    tx.child(ns::C, "strRef").and_then(|r| cache_texts(r).into_iter().next())
}

fn cache_texts(r: &Element) -> Vec<String> {
    let Some(cache) = r.child(ns::C, "strCache").or_else(|| r.child(ns::C, "numCache")).or_else(|| r.child(ns::C, "multiLvlStrCache"))
    else {
        return Vec::new();
    };
    let cache = cache.child(ns::C, "lvl").unwrap_or(cache);
    let count = r.child(ns::C, "strCache").or(r.child(ns::C, "numCache")).and_then(|c| val(c, "ptCount")).and_then(|v| v.parse().ok());
    points(cache, count)
}

fn points(cache: &Element, count: Option<usize>) -> Vec<String> {
    let pts: Vec<(usize, String)> = cache
        .children_named(ns::C, "pt")
        .filter_map(|p| Some((p.attr("idx")?.parse().ok()?, p.child(ns::C, "v").map(Element::text).unwrap_or_default())))
        .collect();
    let n = count
        .or_else(|| val(cache, "ptCount").and_then(|v| v.parse().ok()))
        .unwrap_or(0)
        .max(pts.iter().map(|(i, _)| i + 1).max().unwrap_or(0))
        .min(100_000);
    let mut out = vec![String::new(); n];
    for (i, v) in pts {
        if i < n {
            out[i] = v;
        }
    }
    out
}

/// A `cat`, `val`, `xVal` or `yVal`: its reference and cached points.
fn read_data(el: &Element) -> (Option<String>, Vec<String>) {
    for local in ["numRef", "strRef", "multiLvlStrRef"] {
        if let Some(r) = el.child(ns::C, local) {
            let f = r.child(ns::C, "f").map(Element::text).filter(|f| !f.trim().is_empty());
            return (f, cache_texts(r));
        }
    }
    for local in ["numLit", "strLit"] {
        if let Some(lit) = el.child(ns::C, local) {
            return (None, points(lit, None));
        }
    }
    (None, Vec::new())
}

fn read_series(s: &Element) -> ChartSeries {
    let mut out = ChartSeries::default();
    if let Some(tx) = s.child(ns::C, "tx") {
        if let Some(r) = tx.child(ns::C, "strRef") {
            out.name_ref = r.child(ns::C, "f").map(Element::text).filter(|f| !f.trim().is_empty());
            out.cache.name = cache_texts(r).into_iter().next();
        } else if let Some(v) = tx.child(ns::C, "v") {
            out.name = Some(v.text());
        }
    }
    if let Some(cat) = s.child(ns::C, "cat").or_else(|| s.child(ns::C, "xVal")) {
        let (f, cache) = read_data(cat);
        out.categories = f;
        out.cache.categories = cache;
    }
    if let Some(v) = s.child(ns::C, "val").or_else(|| s.child(ns::C, "yVal")) {
        let (f, cache) = read_data(v);
        out.values = f.unwrap_or_default();
        out.cache.values = cache.iter().map(|v| v.trim().parse::<f64>().ok()).collect();
    }
    out.color = s.child(ns::C, "spPr").and_then(|sp| {
        let fill = sp.child(ns::A, "solidFill").or_else(|| sp.child(ns::A, "ln").and_then(|l| l.child(ns::A, "solidFill")))?;
        let c = fill.elements().next()?;
        match c.local.as_str() {
            "srgbClr" => c.attr("val").map(|v| v.to_ascii_uppercase()),
            "schemeClr" => c.attr("val").map(str::to_string),
            _ => None,
        }
    });
    out
}

// ---- writing ----

/// What saving did to a sheet's drawing, for the sheet's own `<drawing>` element.
pub(super) enum DrawingLink {
    /// Nothing changed.
    Same,
    /// The sheet shows this drawing (by relationship id).
    Linked(String),
    /// The sheet has no drawing any more.
    Gone,
}

/// Writes a sheet's charts into its drawing (made if it hasn't one), when they've changed.
/// `same` says the package being written is the one the sheet was read from.
pub(super) fn write_charts(sheet: &Sheet, sheet_part: &str, sheets: &[Sheet], same: bool, pkg: &mut Package) -> Result<DrawingLink> {
    let existing =
        if same && pkg.has(sheet_part) { pkg.related(sheet_part, kind::DRAWING)?.into_iter().find(|(_, p)| pkg.has(p)) } else { None };
    let charts: Vec<&Chart> = sheet.charts.iter().filter(|c| c.kind != ChartKind::Other || (same && c.source.is_some())).collect();
    let Some((drawing_rel, drawing)) = existing else {
        if charts.is_empty() {
            return Ok(DrawingLink::Same);
        }
        // A new drawing with only these charts.
        let drawing = pkg.unused_name("xl/drawings/drawing{}.xml");
        let mut root = Element::new(ns::XDR, "wsDr");
        root.declare(Some("xdr"), ns::XDR);
        root.declare(Some("a"), ns::A);
        pkg.put(&drawing, Vec::new(), DRAWING_TYPE);
        let mut ids = 1u32;
        for chart in &charts {
            let part = new_chart_part(pkg);
            put_chart(pkg, &part, chart, sheets)?;
            let rid = pkg.relate(&drawing, CHART_REL, &part)?;
            ids += 1;
            root.push(new_anchor(chart, &rid, ids));
        }
        pkg.put_xml(&drawing, &XmlDoc::new(root), DRAWING_TYPE);
        let rid = pkg.relate(sheet_part, kind::DRAWING, &drawing)?;
        return Ok(DrawingLink::Linked(rid));
    };

    let mut doc = pkg.xml(&drawing)?;
    let rels = PartRels::new(pkg, &drawing)?;
    // The chart parts the drawing shows now, in order.
    let shown: Vec<Option<String>> = doc.root.elements().map(|a| chart_part_of(a, &rels)).collect();
    drop(rels);
    let kept: HashSet<&str> =
        charts.iter().filter_map(|c| c.source.as_ref().filter(|s| s.drawing == drawing).map(|s| s.part.as_str())).collect();
    let in_drawing: Vec<&str> = shown.iter().flatten().map(String::as_str).collect();
    let unchanged = charts.iter().all(|c| c.untouched() && c.source.as_ref().is_some_and(|s| s.drawing == drawing))
        && in_drawing.len() == charts.len()
        && in_drawing.iter().all(|p| kept.contains(p))
        && sheet.pictures.iter().all(|p| p.read_as.is_none_or(|r| r == p.anchor));
    if unchanged {
        return Ok(DrawingLink::Same);
    }

    // Anchors that aren't charts stay where they are; charts follow the model's order after them.
    let mut ids = doc.root.descendants_max_id();
    let mut old_anchors: Vec<(String, Element)> = Vec::new();
    let mut children = Vec::new();
    let mut index = 0;
    let mut placed = vec![false; sheet.pictures.len()];
    for n in std::mem::take(&mut doc.root.children) {
        match n {
            Node::Element(e) if e.in_ns(ns::XDR) && e.local.ends_with("Anchor") => {
                if let Some(Some(part)) = shown.get(index) {
                    old_anchors.push((part.clone(), e));
                } else {
                    // A picture that moved with its cells gets its new anchor.
                    let was = e.child(ns::XDR, "pic").and_then(|_| read_anchor(&e));
                    let moved = was.and_then(|was| {
                        let i = (0..sheet.pictures.len()).find(|&i| !placed[i] && sheet.pictures[i].read_as == Some(was))?;
                        placed[i] = true;
                        Some(sheet.pictures[i].anchor).filter(|a| *a != was)
                    });
                    children.push(Node::Element(match moved {
                        Some(at) => moved_anchor(e, &at),
                        None => e,
                    }));
                }
                index += 1;
            }
            n => children.push(n),
        }
    }
    doc.root.children = children;
    let mut drawing_rels = pkg.rels(&drawing)?;
    // Charts that are gone take their parts with them.
    for (part, anchor) in &old_anchors {
        if !kept.contains(part.as_str()) {
            if let Some(id) = anchor.descendant(ns::C, "chart").and_then(|c| rel_attr(c, "id")) {
                drawing_rels.remove(id);
            }
            pkg.remove(part);
        }
    }
    pkg.put_rels(&drawing, &drawing_rels);
    for chart in &charts {
        let old = chart.source.as_ref().filter(|s| s.drawing == drawing).and_then(|s| old_anchors.iter().find(|(p, _)| *p == s.part));
        match old {
            Some((part, anchor)) => {
                if !chart.content_untouched() {
                    put_chart(pkg, part, chart, sheets)?;
                }
                let mut anchor = anchor.clone();
                if chart.source.as_ref().is_some_and(|s| s.read_as.anchor != chart.anchor) {
                    anchor = moved_anchor(anchor, &chart.anchor);
                }
                doc.root.push(anchor);
            }
            None => {
                let part = new_chart_part(pkg);
                put_chart(pkg, &part, chart, sheets)?;
                let rid = pkg.relate(&drawing, CHART_REL, &part)?;
                ids += 1;
                doc.root.push(new_anchor(chart, &rid, ids));
            }
        }
    }
    let empty = !doc.root.elements().any(|e| e.local.ends_with("Anchor"));
    if empty {
        pkg.remove(&drawing);
        let mut sheet_rels = pkg.rels(sheet_part)?;
        sheet_rels.remove(&drawing_rel.id);
        pkg.put_rels(sheet_part, &sheet_rels);
        return Ok(DrawingLink::Gone);
    }
    pkg.put_xml(&drawing, &doc, DRAWING_TYPE);
    Ok(DrawingLink::Linked(drawing_rel.id))
}

trait MaxId {
    fn descendants_max_id(&self) -> u32;
}

impl MaxId for Element {
    /// The highest shape id (`cNvPr id`) in a drawing, so new ones don't clash.
    fn descendants_max_id(&self) -> u32 {
        let own = if self.local == "cNvPr" { self.attr("id").and_then(|v| v.parse().ok()).unwrap_or(0) } else { 0 };
        self.elements().map(|e| e.descendants_max_id()).fold(own, u32::max)
    }
}

fn new_chart_part(pkg: &Package) -> String {
    pkg.unused_name("xl/charts/chart{}.xml")
}

fn put_chart(pkg: &mut Package, part: &str, chart: &Chart, sheets: &[Sheet]) -> Result<()> {
    let root = chart_space(chart, sheets);
    pkg.put_xml(part, &XmlDoc::new(root), CHART_TYPE);
    Ok(())
}

fn xdr(local: &str) -> Element {
    Element::new(ns::XDR, local)
}

fn point(local: &str, p: &AnchorPoint) -> Element {
    xdr(local)
        .with_child(xdr("col").with_text(p.col.to_string()))
        .with_child(xdr("colOff").with_text(p.col_off.to_string()))
        .with_child(xdr("row").with_text(p.row.to_string()))
        .with_child(xdr("rowOff").with_text(p.row_off.to_string()))
}

/// An anchor's position set to `at`, keeping the rest of it (and its kind, when it can).
fn moved_anchor(mut anchor: Element, at: &ChartAnchor) -> Element {
    let Some(to) = at.to else {
        // A one-cell anchor that moved with its cell (rows or columns inserted above it) keeps
        // its size; an absolute one isn't tied to cells.
        if anchor.local == "oneCellAnchor" {
            anchor.remove_children(ns::XDR, "from");
            anchor.children.insert(0, Node::Element(point("from", &at.from)));
        }
        return anchor;
    };
    if anchor.local != "twoCellAnchor" {
        anchor.local = "twoCellAnchor".into();
        anchor.remove_children(ns::XDR, "pos");
        anchor.remove_children(ns::XDR, "ext");
        anchor.set_attr("", "editAs", "oneCell");
    }
    anchor.remove_children(ns::XDR, "from");
    anchor.remove_children(ns::XDR, "to");
    anchor.children.insert(0, Node::Element(point("to", &to)));
    anchor.children.insert(0, Node::Element(point("from", &at.from)));
    anchor
}

fn new_anchor(chart: &Chart, rid: &str, id: u32) -> Element {
    let from = chart.anchor.from;
    let to = chart.anchor.to.unwrap_or(AnchorPoint { col: from.col + 8, col_off: 0, row: from.row + 15, row_off: 0 });
    let c = Element::new(ns::C, "chart").with_attr(ns::R, "id", rid);
    let mut c = c;
    c.declare(Some("c"), ns::C);
    c.declare(Some("r"), ns::R);
    let frame = xdr("graphicFrame")
        .with_attr("", "macro", "")
        .with_child(
            xdr("nvGraphicFramePr")
                .with_child(xdr("cNvPr").with_attr("", "id", id.to_string()).with_attr(
                    "",
                    "name",
                    format!("Chart {}", id.saturating_sub(1)),
                ))
                .with_child(xdr("cNvGraphicFramePr")),
        )
        .with_child(
            xdr("xfrm")
                .with_child(Element::new(ns::A, "off").with_attr("", "x", "0").with_attr("", "y", "0"))
                .with_child(Element::new(ns::A, "ext").with_attr("", "cx", "0").with_attr("", "cy", "0")),
        )
        .with_child(
            Element::new(ns::A, "graphic").with_child(Element::new(ns::A, "graphicData").with_attr("", "uri", CHART_URI).with_child(c)),
        );
    xdr("twoCellAnchor")
        .with_attr("", "editAs", "oneCell")
        .with_child(point("from", &from))
        .with_child(point("to", &to))
        .with_child(frame)
        .with_child(xdr("clientData"))
}

fn cv(local: &str, v: impl Into<String>) -> Element {
    Element::new(ns::C, local).with_attr("", "val", v)
}

fn c(local: &str) -> Element {
    Element::new(ns::C, local)
}

fn color_el(color: &str) -> Element {
    if color.len() == 6 && color.chars().all(|c| c.is_ascii_hexdigit()) {
        Element::new(ns::A, "srgbClr").with_attr("", "val", color.to_ascii_uppercase())
    } else {
        Element::new(ns::A, "schemeClr").with_attr("", "val", color)
    }
}

fn solid(color: &str) -> Element {
    Element::new(ns::A, "solidFill").with_child(color_el(color))
}

fn str_cache(points: &[String]) -> Element {
    let mut cache = c("strCache").with_child(cv("ptCount", points.len().to_string()));
    for (i, p) in points.iter().enumerate() {
        cache.push(c("pt").with_attr("", "idx", i.to_string()).with_child(c("v").with_text(p.clone())));
    }
    cache
}

fn num_cache(points: &[Option<f64>]) -> Element {
    let mut cache = c("numCache").with_child(c("formatCode").with_text("General")).with_child(cv("ptCount", points.len().to_string()));
    for (i, p) in points.iter().enumerate() {
        if let Some(n) = p {
            cache.push(c("pt").with_attr("", "idx", i.to_string()).with_child(c("v").with_text(format_number(*n))));
        }
    }
    cache
}

fn num_data(local: &str, f: Option<&str>, values: &[Option<f64>]) -> Element {
    match f {
        Some(f) => c(local).with_child(c("numRef").with_child(c("f").with_text(f)).with_child(num_cache(values))),
        None => {
            let mut lit = num_cache(values);
            lit.local = "numLit".into();
            c(local).with_child(lit)
        }
    }
}

fn str_data(local: &str, f: Option<&str>, texts: &[String]) -> Element {
    match f {
        Some(f) => c(local).with_child(c("strRef").with_child(c("f").with_text(f)).with_child(str_cache(texts))),
        None => {
            let mut lit = str_cache(texts);
            lit.local = "strLit".into();
            c(local).with_child(lit)
        }
    }
}

/// What a series shows now: its name, categories (text, or numbers for a scatter chart's x) and values.
pub struct SeriesData {
    pub name: Option<String>,
    pub cat_text: Vec<String>,
    pub cat_numbers: Option<Vec<Option<f64>>>,
    pub values: Vec<Option<f64>>,
}

fn series_data(s: &ChartSeries, sheets: &[Sheet], scatter: bool) -> SeriesData {
    let name = s
        .name_ref
        .as_deref()
        .and_then(|f| ref_texts(sheets, f))
        .and_then(|t| t.into_iter().next())
        .or_else(|| s.name.clone())
        .or_else(|| s.cache.name.clone());
    let cat_text = s.categories.as_deref().and_then(|f| ref_texts(sheets, f)).unwrap_or_else(|| s.cache.categories.clone());
    let numeric = s.categories.as_deref().and_then(|f| ref_numbers(sheets, f));
    let cat_numbers = match numeric {
        Some(n) if scatter || (!n.is_empty() && n.iter().all(Option::is_some)) => Some(n),
        _ if scatter => Some(cat_text.iter().map(|t| t.trim().parse().ok()).collect()),
        _ => None,
    };
    let values = ref_numbers(sheets, &s.values).unwrap_or_else(|| s.cache.values.clone());
    SeriesData { name, cat_text, cat_numbers, values }
}

const AX_CAT: &str = "500000001";
const AX_VAL: &str = "500000002";

/// A `c:title` of rich text at `sz` hundredths of a point, turned for a vertical axis.
fn rich_title(text: &str, sz: &str, vertical: bool) -> Element {
    let size = |e: Element| e.with_attr("", "sz", sz).with_attr("", "b", "0");
    let body = Element::new(ns::A, "bodyPr");
    let body = if vertical { body.with_attr("", "rot", "-5400000").with_attr("", "vert", "horz") } else { body };
    let p = Element::new(ns::A, "p").with_child(Element::new(ns::A, "pPr").with_child(size(Element::new(ns::A, "defRPr")))).with_child(
        Element::new(ns::A, "r")
            .with_child(size(Element::new(ns::A, "rPr").with_attr("", "lang", "en-US")))
            .with_child(Element::new(ns::A, "t").with_text(text)),
    );
    let rich = c("rich").with_child(body).with_child(Element::new(ns::A, "lstStyle")).with_child(p);
    c("title").with_child(c("tx").with_child(rich)).with_child(cv("overlay", "0"))
}

/// A fresh `chartSpace` for a chart, with caches from the sheets' values.
pub fn chart_space(chart: &Chart, sheets: &[Sheet]) -> Element {
    let mut root = c("chartSpace");
    root.declare(Some("c"), ns::C);
    root.declare(Some("a"), ns::A);
    root.declare(Some("r"), ns::R);
    root.push(cv("roundedCorners", "0"));
    let mut ch = c("chart");
    if let Some(title) = chart.title.as_deref().filter(|t| !t.trim().is_empty()) {
        ch.push(rich_title(title, "1400", false));
        ch.push(cv("autoTitleDeleted", "0"));
    } else {
        ch.push(cv("autoTitleDeleted", "1"));
    }
    let kind = if chart.kind == ChartKind::Other { ChartKind::Column } else { chart.kind };
    let scatter = kind == ChartKind::Scatter;
    let mut group = c(match kind {
        ChartKind::Column | ChartKind::Bar | ChartKind::Other => "barChart",
        ChartKind::Line => "lineChart",
        ChartKind::Area => "areaChart",
        ChartKind::Pie => "pieChart",
        ChartKind::Doughnut => "doughnutChart",
        ChartKind::Scatter => "scatterChart",
    });
    let grouping = match chart.grouping {
        ChartGrouping::Standard if matches!(kind, ChartKind::Column | ChartKind::Bar) => "clustered",
        ChartGrouping::Standard => "standard",
        ChartGrouping::Stacked => "stacked",
        ChartGrouping::PercentStacked => "percentStacked",
    };
    match kind {
        ChartKind::Column | ChartKind::Bar => {
            group.push(cv("barDir", if kind == ChartKind::Bar { "bar" } else { "col" }));
            group.push(cv("grouping", grouping));
        }
        ChartKind::Line | ChartKind::Area => {
            group.push(cv("grouping", grouping));
        }
        ChartKind::Scatter => {
            group.push(cv("scatterStyle", "lineMarker"));
        }
        _ => {}
    }
    group.push(cv("varyColors", if kind.has_axes() { "0" } else { "1" }));
    for (i, s) in chart.series.iter().enumerate() {
        let d = series_data(s, sheets, scatter);
        let mut ser = c("ser").with_child(cv("idx", i.to_string())).with_child(cv("order", i.to_string()));
        match (&s.name_ref, &d.name) {
            (Some(f), name) => {
                ser.push(c("tx").with_child(
                    c("strRef").with_child(c("f").with_text(f.clone())).with_child(str_cache(&[name.clone().unwrap_or_default()])),
                ));
            }
            (None, Some(name)) => {
                ser.push(c("tx").with_child(c("v").with_text(name.clone())));
            }
            (None, None) => {}
        }
        let line_like = matches!(kind, ChartKind::Line | ChartKind::Scatter);
        if let Some(color) = &s.color {
            if line_like {
                let ln = Element::new(ns::A, "ln")
                    .with_attr("", "w", "28575")
                    .with_attr("", "cap", "rnd")
                    .with_child(solid(color))
                    .with_child(Element::new(ns::A, "round"));
                ser.push(c("spPr").with_child(if scatter {
                    Element::new(ns::A, "ln").with_attr("", "w", "19050").with_child(Element::new(ns::A, "noFill"))
                } else {
                    ln
                }));
            } else {
                ser.push(c("spPr").with_child(solid(color)));
            }
        } else if scatter {
            ser.push(c("spPr").with_child(Element::new(ns::A, "ln").with_attr("", "w", "19050").with_child(Element::new(ns::A, "noFill"))));
        }
        if matches!(kind, ChartKind::Column | ChartKind::Bar) {
            ser.push(cv("invertIfNegative", "0"));
        }
        if kind == ChartKind::Line {
            ser.push(c("marker").with_child(cv("symbol", "none")));
        }
        if scatter {
            let mut marker = c("marker").with_child(cv("symbol", "circle")).with_child(cv("size", "5"));
            if let Some(color) = &s.color {
                marker.push(c("spPr").with_child(solid(color)).with_child(Element::new(ns::A, "ln").with_child(solid(color))));
            }
            ser.push(marker);
        }
        let cat_local = if scatter { "xVal" } else { "cat" };
        if s.categories.is_some() || !d.cat_text.is_empty() {
            ser.push(match &d.cat_numbers {
                Some(n) => num_data(cat_local, s.categories.as_deref(), n),
                None => str_data(cat_local, s.categories.as_deref(), &d.cat_text),
            });
        }
        let values_ref = (!s.values.trim().is_empty()).then_some(s.values.as_str());
        ser.push(num_data(if scatter { "yVal" } else { "val" }, values_ref, &d.values));
        if matches!(kind, ChartKind::Line | ChartKind::Scatter) {
            ser.push(cv("smooth", "0"));
        }
        group.push(ser);
    }
    match kind {
        ChartKind::Column | ChartKind::Bar => {
            group.push(cv("gapWidth", "150"));
            if chart.grouping != ChartGrouping::Standard {
                group.push(cv("overlap", "100"));
            }
        }
        ChartKind::Line => {
            group.push(cv("marker", "1"));
        }
        ChartKind::Pie => {
            group.push(cv("firstSliceAng", "0"));
        }
        ChartKind::Doughnut => {
            group.push(cv("firstSliceAng", "0"));
            group.push(cv("holeSize", "55"));
        }
        _ => {}
    }
    let mut plot = c("plotArea").with_child(c("layout"));
    if kind.has_axes() {
        group.push(cv("axId", AX_CAT));
        group.push(cv("axId", AX_VAL));
        plot.push(group);
        let (cat_pos, val_pos) = if kind == ChartKind::Bar { ("l", "b") } else { ("b", "l") };
        let gridlines = || {
            c("majorGridlines")
                .with_child(c("spPr").with_child(Element::new(ns::A, "ln").with_attr("", "w", "9525").with_child(solid("D9D9D9"))))
        };
        let axis_line = || c("spPr").with_child(Element::new(ns::A, "ln").with_attr("", "w", "9525").with_child(solid("BFBFBF")));
        let num_fmt = |code: &str| {
            c("numFmt").with_attr("", "formatCode", code).with_attr("", "sourceLinked", if code == "General" { "1" } else { "0" })
        };
        let ticks = |e: Element| {
            e.with_child(cv("majorTickMark", "none")).with_child(cv("minorTickMark", "none")).with_child(cv("tickLblPos", "nextTo"))
        };
        let axis_head = |local: &str, id: &str, pos: &str| {
            c(local)
                .with_child(cv("axId", id))
                .with_child(c("scaling").with_child(cv("orientation", "minMax")))
                .with_child(cv("delete", "0"))
                .with_child(cv("axPos", pos))
        };
        // An axis's title sits where the axis is: along the bottom, or turned beside it.
        let axis_title =
            |t: &Option<String>, pos: &str| t.as_deref().filter(|t| !t.trim().is_empty()).map(|t| rich_title(t, "1000", pos == "l"));
        let mut first = axis_head(if scatter { "valAx" } else { "catAx" }, AX_CAT, cat_pos);
        if let Some(t) = axis_title(&chart.cat_title, cat_pos) {
            first.push(t);
        }
        let first = ticks(first.with_child(num_fmt("General")))
            .with_child(axis_line())
            .with_child(cv("crossAx", AX_VAL))
            .with_child(cv("crosses", "autoZero"));
        let first = if scatter {
            first.with_child(cv("crossBetween", "midCat"))
        } else {
            first
                .with_child(cv("auto", "1"))
                .with_child(cv("lblAlgn", "ctr"))
                .with_child(cv("lblOffset", "100"))
                .with_child(cv("noMultiLvlLbl", "0"))
        };
        plot.push(first);
        let percent = chart.grouping == ChartGrouping::PercentStacked;
        let mut second = axis_head("valAx", AX_VAL, val_pos).with_child(gridlines());
        if let Some(t) = axis_title(&chart.val_title, val_pos) {
            second.push(t);
        }
        let second = ticks(second.with_child(num_fmt(if percent { "0%" } else { "General" })))
            .with_child(c("spPr").with_child(Element::new(ns::A, "ln").with_child(Element::new(ns::A, "noFill"))))
            .with_child(cv("crossAx", AX_CAT))
            .with_child(cv("crosses", "autoZero"))
            .with_child(cv("crossBetween", if scatter { "midCat" } else { "between" }));
        plot.push(second);
    } else {
        plot.push(group);
    }
    ch.push(plot);
    if let Some(pos) = chart.legend {
        let p = match pos {
            LegendPosition::Right => "r",
            LegendPosition::Bottom => "b",
            LegendPosition::Top => "t",
            LegendPosition::Left => "l",
        };
        ch.push(c("legend").with_child(cv("legendPos", p)).with_child(cv("overlay", "0")));
    }
    ch.push(cv("plotVisOnly", "1"));
    ch.push(cv("dispBlanksAs", "gap"));
    root.push(ch);
    let border = Element::new(ns::A, "ln").with_attr("", "w", "9525").with_child(solid("D9D9D9"));
    root.push(c("spPr").with_child(solid("FFFFFF")).with_child(border));
    root
}

/// The series a chart would show, worked out from the sheets' values now (or the file's caches).
pub fn series_values(s: &ChartSeries, sheets: &[Sheet], scatter: bool) -> SeriesData {
    series_data(s, sheets, scatter)
}

/// The cache a file would get for a series (for tests and the view's fallback).
pub fn cache_of(s: &ChartSeries, sheets: &[Sheet]) -> SeriesCache {
    let d = series_data(s, sheets, false);
    SeriesCache { name: d.name, categories: d.cat_text, values: d.values }
}
