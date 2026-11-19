//! Charts in the view: what each one shows (worked out from the cells now), where it sits in
//! grid pixels, and the commands that make, change, move and delete them.

use office_core::model::CellRef;
use office_core::model::{
    make_ref, parse_ref, ref_cells, AnchorPoint, Chart, ChartAnchor, ChartGrouping, ChartKind, ChartSeries, LegendPosition, Range, Sheet,
    Value,
};
use serde::{Deserialize, Serialize};

use super::{CStep, Grid, SheetsView};

/// EMUs in a pixel at 96 dpi.
const EMU_PX: f64 = 9525.0;

/// A chart as the app draws it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartView {
    pub kind: ChartKind,
    pub grouping: ChartGrouping,
    pub title: Option<String>,
    pub legend: Option<LegendPosition>,
    /// Grid pixels.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Category labels as the cells show them (a scatter chart's x values as text).
    pub categories: Vec<String>,
    pub series: Vec<SeriesView>,
    /// The theme's accent colors, `#RRGGBB`, for series and pie slices without their own.
    pub palette: Vec<String>,
    /// The cells it plots, `A1:C7` (with the sheet if it's another), when they're one block.
    pub range: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesView {
    pub name: String,
    pub color: Option<String>,
    pub values: Vec<Option<f64>>,
    /// A scatter chart's x values.
    pub x: Option<Vec<Option<f64>>>,
}

/// A change to a chart, from the inspector. Absent fields stay as they are.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartPatch {
    pub kind: Option<ChartKind>,
    pub grouping: Option<ChartGrouping>,
    /// Empty for no title.
    pub title: Option<String>,
    /// `none`, `right`, `bottom`, `top`, `left`.
    pub legend: Option<String>,
    /// New cells to plot, `A1:C7` or `Sheet2!A1:C7`.
    pub range: Option<String>,
    /// `RRGGBB` per series, by index; empty for automatic.
    pub color: Option<(usize, String)>,
}

impl Grid {
    /// The cell at a pixel position and how far into it, as an anchor point.
    fn point_at(&self, x: f64, y: f64) -> AnchorPoint {
        let (col, cx) = locate(&self.col_x, self.default_width, x.max(0.0));
        let (row, ry) = locate(&self.row_y, self.default_height, y.max(0.0));
        AnchorPoint { col, col_off: ((x.max(0.0) - cx) * EMU_PX).round() as i64, row, row_off: ((y.max(0.0) - ry) * EMU_PX).round() as i64 }
    }

    fn anchor_px(&self, p: &AnchorPoint) -> (f64, f64) {
        (self.x(p.col) + p.col_off as f64 / EMU_PX, self.y(p.row) + p.row_off as f64 / EMU_PX)
    }

    /// A chart's box in pixels.
    pub fn chart_rect(&self, a: &ChartAnchor) -> (f64, f64, f64, f64) {
        let (x0, y0) = self.anchor_px(&a.from);
        let (x1, y1) = match (a.to, a.size) {
            (Some(to), _) => self.anchor_px(&to),
            (None, Some((cx, cy))) => (x0 + cx as f64 / EMU_PX, y0 + cy as f64 / EMU_PX),
            (None, None) => (x0 + 480.0, y0 + 288.0),
        };
        (x0, y0, (x1 - x0).max(8.0), (y1 - y0).max(8.0))
    }
}

/// The index and start of the slot holding `v`, in starts `edges` (extended by `default`).
fn locate(edges: &[f64], default: f64, v: f64) -> (u32, f64) {
    let last = edges.len().saturating_sub(1);
    let end = edges.get(last).copied().unwrap_or(0.0);
    if v >= end && default > 0.0 {
        let extra = ((v - end) / default).floor();
        return (last as u32 + extra as u32, end + extra * default);
    }
    let i = edges.partition_point(|&e| e <= v).saturating_sub(1);
    (i as u32, edges.get(i).copied().unwrap_or(0.0))
}

/// A picture as the app draws it: its box in grid pixels and the part its image is in.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureView {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub media: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl SheetsView {
    /// A sheet's pictures, back to front.
    pub fn pictures(&mut self, sheet: usize) -> Vec<PictureView> {
        self.grid(sheet);
        let grid = &self.grids[&sheet];
        let Some(s) = self.sheets().get(sheet) else { return Vec::new() };
        s.pictures
            .iter()
            .map(|p| {
                let (x, y, w, h) = grid.chart_rect(&p.anchor);
                PictureView { x, y, w, h, media: p.media.clone(), crop: p.crop, name: p.description.clone().or_else(|| p.name.clone()) }
            })
            .collect()
    }

    /// A picture's bytes and media type, by part name.
    pub fn media(&self, part: &str) -> Option<(&[u8], Option<&str>)> {
        let pkg = self.doc.package.as_ref()?;
        Some((pkg.part(part).ok()?, pkg.content_type(part)))
    }

    /// A sheet's charts as the app draws them.
    pub fn charts(&mut self, sheet: usize) -> Vec<ChartView> {
        self.grid(sheet);
        let grid = &self.grids[&sheet];
        let doc = &self.doc;
        let sheets = self.sheets();
        let Some(s) = sheets.get(sheet) else { return Vec::new() };
        let theme = doc.theme.clone();
        let accent = |i: usize| {
            let rgb = theme.as_ref().and_then(|t| t.scheme_color(&format!("accent{}", i % 6 + 1)));
            format!(
                "#{}",
                rgb.map(|c| c.hex()).unwrap_or_else(|| ["4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"][i % 6].into())
            )
        };
        let palette: Vec<String> = (0..6).map(accent).collect();
        let css = |c: &str| {
            if c.len() == 6 && c.chars().all(|c| c.is_ascii_hexdigit()) {
                Some(format!("#{c}"))
            } else {
                theme.as_ref().and_then(|t| t.scheme_color(c)).map(|rgb| format!("#{}", rgb.hex()))
            }
        };
        s.charts
            .iter()
            .map(|c| {
                let (x, y, w, h) = grid.chart_rect(&c.anchor);
                let scatter = c.kind == ChartKind::Scatter;
                let mut categories = Vec::new();
                let series = c
                    .series
                    .iter()
                    .enumerate()
                    .map(|(i, ser)| {
                        let data = office_core::formats::xlsx::chart::series_values(ser, sheets, scatter);
                        let (name, cats, xs, values) = (data.name, data.cat_text, data.cat_numbers, data.values);
                        // Labels as the cells show them: dates as dates, numbers in their format.
                        let shown = ser
                            .categories
                            .as_deref()
                            .and_then(|f| ref_cells(sheets, f))
                            .map(|(si, cells)| cells.iter().map(|&at| crate::display(doc, si, at)).collect::<Vec<_>>());
                        if categories.is_empty() {
                            categories = shown.unwrap_or(cats);
                        }
                        SeriesView {
                            name: name.unwrap_or_else(|| format!("Series {}", i + 1)),
                            color: ser.color.as_deref().and_then(css),
                            values,
                            x: if scatter { xs } else { None },
                        }
                    })
                    .collect();
                ChartView {
                    kind: c.kind,
                    grouping: c.grouping,
                    title: c.title.clone(),
                    legend: c.legend,
                    x,
                    y,
                    w,
                    h,
                    categories,
                    series,
                    palette: palette.clone(),
                    range: data_range(c, &s.name),
                }
            })
            .collect()
    }

    /// Makes a chart of the cells in `range` (or, for one cell, the block of filled cells around
    /// it), beside them. Returns its index, or none if there's nothing to plot.
    pub fn add_chart(&mut self, sheet: usize, range: Range, kind: ChartKind) -> Option<usize> {
        let s = self.sheets().get(sheet)?;
        let range = if range.start == range.end { current_region(s, range.start) } else { range };
        let (series, title) = self.series_for(sheet, range, kind)?;
        self.grid(sheet);
        let grid = &self.grids[&sheet];
        let x = grid.x(range.end.col + 1) + grid.default_width.min(64.0) / 2.0;
        let y = grid.y(range.start.row) + grid.default_height.min(20.0) / 2.0;
        let (w, h) = (480.0, 288.0);
        let anchor = ChartAnchor { from: grid.point_at(x, y), to: Some(grid.point_at(x + w, y + h)), size: None };
        let legend = (series.len() > 1 || !kind.has_axes()).then_some(LegendPosition::Bottom);
        let chart = Chart { kind, grouping: ChartGrouping::Standard, title, legend, series, anchor, source: None };
        let index = self.sheets()[sheet].charts.len();
        self.change_charts(sheet, |charts| charts.push(chart));
        Some(index)
    }

    /// Changes a chart's kind, title, legend, colors or cells.
    pub fn set_chart(&mut self, sheet: usize, index: usize, patch: &ChartPatch) {
        let Some(old) = self.sheets().get(sheet).and_then(|s| s.charts.get(index)).cloned() else { return };
        let mut chart = old.clone();
        if let Some(k) = patch.kind {
            if k != ChartKind::Other {
                // Scatter charts plot x against y; other kinds take x back as categories.
                chart.kind = k;
                if !k.has_axes() || k == ChartKind::Scatter {
                    chart.grouping = ChartGrouping::Standard;
                }
                if !k.has_axes() && chart.legend.is_none() {
                    chart.legend = Some(LegendPosition::Bottom);
                }
            }
        }
        if let Some(g) = patch.grouping {
            chart.grouping = g;
        }
        if let Some(t) = &patch.title {
            chart.title = (!t.trim().is_empty()).then(|| t.trim().to_string());
        }
        if let Some(l) = &patch.legend {
            chart.legend = match l.as_str() {
                "right" => Some(LegendPosition::Right),
                "bottom" => Some(LegendPosition::Bottom),
                "top" => Some(LegendPosition::Top),
                "left" => Some(LegendPosition::Left),
                _ => None,
            };
        }
        if let Some((i, color)) = &patch.color {
            if let Some(s) = chart.series.get_mut(*i) {
                let hex = color.trim().trim_start_matches('#').to_ascii_uppercase();
                s.color = (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex);
            }
        }
        if let Some(r) = &patch.range {
            let name = self.sheets()[sheet].name.clone();
            let text = r.trim().trim_start_matches('=');
            let parsed = if text.contains('!') { parse_ref(text) } else { Range::parse(&text.replace('$', "")).map(|r| (name.clone(), r)) };
            if let Some((sheet_name, range)) = parsed {
                if let Some(data_sheet) = self.sheets().iter().position(|s| s.name.eq_ignore_ascii_case(&sheet_name)) {
                    if let Some((series, _)) = self.series_for(data_sheet, range, chart.kind) {
                        // Colors stay with the series' places.
                        let colors: Vec<_> = chart.series.iter().map(|s| s.color.clone()).collect();
                        chart.series = series;
                        for (s, c) in chart.series.iter_mut().zip(colors) {
                            s.color = c;
                        }
                    }
                }
            }
        } else if patch.kind.is_some_and(|k| (k == ChartKind::Scatter) != (old.kind == ChartKind::Scatter)) {
            // Between scatter and the rest, x values and categories trade places: plot the same cells again.
            if let Some((data_sheet, range)) =
                data_block(&old).and_then(|(n, r)| Some((self.sheets().iter().position(|s| s.name.eq_ignore_ascii_case(&n))?, r)))
            {
                if let Some((series, _)) = self.series_for(data_sheet, range, chart.kind) {
                    chart.series = series;
                }
            }
        }
        if chart == old {
            return;
        }
        self.change_charts(sheet, |charts| charts[index] = chart);
    }

    /// Moves or resizes a chart to a box in grid pixels.
    pub fn move_chart(&mut self, sheet: usize, index: usize, x: f64, y: f64, w: f64, h: f64) {
        self.grid(sheet);
        let grid = &self.grids[&sheet];
        let anchor = ChartAnchor { from: grid.point_at(x, y), to: Some(grid.point_at(x + w.max(24.0), y + h.max(24.0))), size: None };
        if self.sheets().get(sheet).and_then(|s| s.charts.get(index)).is_none_or(|c| c.anchor == anchor) {
            return;
        }
        self.change_charts(sheet, |charts| charts[index].anchor = anchor);
    }

    pub fn delete_chart(&mut self, sheet: usize, index: usize) {
        if self.sheets().get(sheet).and_then(|s| s.charts.get(index)).is_none() {
            return;
        }
        self.change_charts(sheet, |charts| {
            charts.remove(index);
        });
    }

    /// Changes a sheet's charts as one step.
    fn change_charts(&mut self, sheet: usize, f: impl FnOnce(&mut Vec<Chart>)) {
        let before = self.sheets()[sheet].charts.clone();
        let sel = self.sel;
        f(&mut self.sheets_mut()[sheet].charts);
        let after = self.sheets()[sheet].charts.clone();
        self.changed(Some(sheet));
        self.record_step(CStep::Charts { sheet, before, after }, sel);
    }

    /// Series for the cells in `range` on sheet `sheet`, the way Excel lays them out: a header
    /// row and a first column of labels if they're there, and a series per column when the block
    /// is taller than it is wide (per row otherwise). Also a title, for a chart of one series.
    fn series_for(&self, sheet: usize, range: Range, kind: ChartKind) -> Option<(Vec<ChartSeries>, Option<String>)> {
        let s = self.sheets().get(sheet)?;
        let doc = &self.doc;
        let number = |r: u32, c: u32| matches!(s.value(CellRef::new(r, c)), Value::Number(_));
        let date =
            |r: u32, c: u32| number(r, c) && crate::numfmt::is_date_format(&crate::format_of(doc, sheet, CellRef::new(r, c)).number_format);
        let text = |r: u32, c: u32| matches!(s.value(CellRef::new(r, c)), Value::Text(_) | Value::Rich(_));
        let (r0, r1, c0, c1) = (range.start.row, range.end.row, range.start.col, range.end.col);
        let header = r1 > r0 && (c0..=c1).all(|c| !number(r0, c) || date(r0, c)) && (c0..=c1).any(|c| text(r0, c) || date(r0, c));
        let dr0 = if header { r0 + 1 } else { r0 };
        let labels = c1 > c0 && (dr0..=r1).all(|r| !number(r, c0) || date(r, c0)) && (dr0..=r1).any(|r| text(r, c0) || date(r, c0))
            || (c1 > c0 && header && s.value(CellRef::new(r0, c0)).is_empty() && kind != ChartKind::Scatter);
        let dc0 = if labels { c0 + 1 } else { c0 };
        if dr0 > r1 || dc0 > c1 {
            return None;
        }
        let name = &s.name;
        let block = |a: (u32, u32), b: (u32, u32)| make_ref(name, Range { start: CellRef::new(a.0, a.1), end: CellRef::new(b.0, b.1) });
        let by_cols = (r1 - dr0 + 1) >= (c1 - dc0 + 1) || kind == ChartKind::Scatter;
        let mut series = Vec::new();
        if by_cols {
            for c in dc0..=c1 {
                series.push(ChartSeries {
                    name_ref: header.then(|| block((r0, c), (r0, c))),
                    name: (!header).then(|| format!("Series {}", c - dc0 + 1)),
                    categories: labels.then(|| block((dr0, c0), (r1, c0))),
                    values: block((dr0, c), (r1, c)),
                    ..Default::default()
                });
            }
        } else {
            for r in dr0..=r1 {
                series.push(ChartSeries {
                    name_ref: labels.then(|| block((r, c0), (r, c0))),
                    name: (!labels).then(|| format!("Series {}", r - dr0 + 1)),
                    categories: header.then(|| block((r0, dc0), (r0, c1))),
                    values: block((r, dc0), (r, c1)),
                    ..Default::default()
                });
            }
        }
        // A scatter chart's first column of numbers is x.
        if kind == ChartKind::Scatter && series.len() > 1 && series[0].categories.is_none() {
            let x = series.remove(0);
            for s in &mut series {
                s.categories = Some(x.values.clone());
            }
        }
        let title = match series.as_slice() {
            [one] => {
                one.name_ref.as_deref().and_then(parse_ref).map(|(_, r)| crate::display(doc, sheet, r.start)).filter(|t| !t.is_empty())
            }
            _ => None,
        };
        Some((series, title))
    }
}

/// The block of filled cells around `at`, as Excel picks for a chart from one cell.
fn current_region(s: &Sheet, at: CellRef) -> Range {
    let filled = |r: u32, c: u32| !s.value(CellRef::new(r, c)).is_empty();
    let (mut r0, mut r1, mut c0, mut c1) = (at.row, at.row, at.col, at.col);
    for _ in 0..10_000 {
        let mut grew = false;
        let (lo_c, hi_c) = (c0.saturating_sub(1), c1 + 1);
        if r0 > 0 && (lo_c..=hi_c).any(|c| filled(r0 - 1, c)) {
            r0 -= 1;
            grew = true;
        }
        if (lo_c..=hi_c).any(|c| filled(r1 + 1, c)) {
            r1 += 1;
            grew = true;
        }
        let (lo_r, hi_r) = (r0.saturating_sub(1), r1 + 1);
        if c0 > 0 && (lo_r..=hi_r).any(|r| filled(r, c0 - 1)) {
            c0 -= 1;
            grew = true;
        }
        if (lo_r..=hi_r).any(|r| filled(r, c1 + 1)) {
            c1 += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    Range { start: CellRef::new(r0, c0), end: CellRef::new(r1, c1) }
}

/// The one block a chart's references cover, with its sheet's name.
fn data_block(c: &Chart) -> Option<(String, Range)> {
    let mut sheet: Option<String> = None;
    let mut out: Option<Range> = None;
    let refs = c.series.iter().flat_map(|s| [s.name_ref.as_deref(), s.categories.as_deref(), Some(s.values.as_str())]).flatten();
    for f in refs {
        let (name, r) = parse_ref(f)?;
        match &sheet {
            Some(n) if !n.eq_ignore_ascii_case(&name) => return None,
            _ => sheet = Some(name),
        }
        out = Some(match out {
            None => r,
            Some(o) => Range {
                start: CellRef::new(o.start.row.min(r.start.row), o.start.col.min(r.start.col)),
                end: CellRef::new(o.end.row.max(r.end.row), o.end.col.max(r.end.col)),
            },
        });
    }
    Some((sheet?, out?))
}

fn data_range(c: &Chart, here: &str) -> Option<String> {
    let (sheet, r) = data_block(c)?;
    Some(if sheet.eq_ignore_ascii_case(here) { r.to_string() } else { format!("{}!{}", office_core::model::quote_sheet(&sheet), r) })
}

/// Moves references into sheet `skip` held by the other sheets' charts (chart references always
/// name their sheet, so `adjust` leaves references to any other sheet alone).
pub(super) fn shift_other_chart_refs(sheets: &mut [Sheet], skip: usize, adjust: impl Fn(&str) -> String) {
    for (i, s) in sheets.iter_mut().enumerate() {
        if i == skip {
            continue;
        }
        for c in &mut s.charts {
            for ser in &mut c.series {
                for f in [ser.name_ref.as_mut(), ser.categories.as_mut(), Some(&mut ser.values)].into_iter().flatten() {
                    *f = adjust(f);
                }
            }
        }
    }
}

/// Moves a sheet's charts and pictures, and the charts' references, for rows (`rows`) or columns inserted at `at`
/// (`n > 0`) or deleted (`n < 0`), with `adjust` moving one reference.
pub(super) fn shift_charts(s: &mut Sheet, rows: bool, at: u32, n: i64, adjust: impl Fn(&str) -> String) {
    let shift = |i: u32| -> u32 {
        if i < at {
            i
        } else if n < 0 && (i as i64) < at as i64 - n {
            at
        } else {
            (i as i64 + n).max(0) as u32
        }
    };
    let move_anchor = |a: &mut ChartAnchor| {
        for p in [Some(&mut a.from), a.to.as_mut()].into_iter().flatten() {
            if rows {
                p.row = shift(p.row);
            } else {
                p.col = shift(p.col);
            }
        }
    };
    for p in &mut s.pictures {
        move_anchor(&mut p.anchor);
    }
    for c in &mut s.charts {
        move_anchor(&mut c.anchor);
        for ser in &mut c.series {
            for f in [ser.name_ref.as_mut(), ser.categories.as_mut(), Some(&mut ser.values)].into_iter().flatten() {
                *f = adjust(f);
            }
        }
    }
}
