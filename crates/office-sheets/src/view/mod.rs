//! The Excel layer's view: workbooks (and CSV files) drawn as grids and edited on the shared
//! engine. The engine lays out cells' text and keeps the history; this adds the grid: column
//! widths and row heights, number formats, fills and borders, merged cells, formulas worked out
//! again after each change, and sheets.

mod charts;
mod commands;

use std::collections::HashMap;

use office_core::drawing::{Rgb, Rgba};
use office_core::model::{Body, CellRef, Document, Kind, Range, RunProps, Sheet, Value};
use office_core::units::FontSize;
use office_core::xml::ns;
use office_core::Compiler;
use office_engine::display::{self, r, Item, NoObjects, Sink};
use office_engine::flow::{self, BlockBox, FlowLayout, Keep, ParaBox};
use office_engine::history::{History, Step};
use office_engine::measure::{Measure, Measurer};
use office_engine::para::{self, Ctx, ParaSpec, Styles};
use office_engine::pos::Path;
use office_engine::style::TextStyle;
use serde::Serialize;

use crate::{format_of, CellFormat, Sheets};
pub use charts::{ChartPatch, ChartView, SeriesView};
pub use commands::{ClipboardRange, FormatPatch};

/// A picked range: where it started and where it's been extended to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize)]
pub struct SheetSel {
    pub sheet: usize,
    pub anchor: (u32, u32),
    pub focus: (u32, u32),
}

impl SheetSel {
    pub fn range(&self) -> Range {
        let (r0, r1) = (self.anchor.0.min(self.focus.0), self.anchor.0.max(self.focus.0));
        let (c0, c1) = (self.anchor.1.min(self.focus.1), self.anchor.1.max(self.focus.1));
        Range { start: CellRef::new(r0, c0), end: CellRef::new(r1, c1) }
    }
}

/// One change, for undo.
#[derive(Clone, Debug)]
pub enum CStep {
    /// Cells as they were and became (`None`: no cell).
    Cells { sheet: usize, cells: Vec<(CellRef, Option<office_core::model::SheetCell>, Option<office_core::model::SheetCell>)> },
    /// A whole sheet (rows or columns added or removed, widths, merges).
    Sheet { sheet: usize, before: Box<Sheet>, after: Box<Sheet> },
    /// The sheets themselves (added, removed, renamed, moved).
    Book { before: Vec<Sheet>, after: Vec<Sheet> },
    /// A sheet's charts.
    Charts { sheet: usize, before: Vec<office_core::model::Chart>, after: Vec<office_core::model::Chart> },
}

impl Step for CStep {}

/// A sheet's columns and rows: each one's size in pixels, for a grid big enough to scroll.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Grid {
    pub rows: u32,
    pub cols: u32,
    /// The rows and columns that have something in them (cells, merges, charts), from the top left.
    pub content: (u32, u32),
    pub default_width: f64,
    pub default_height: f64,
    /// Columns with their own width: (column, pixels).
    pub widths: Vec<(u32, f64)>,
    /// Rows with their own height: (row, pixels).
    pub heights: Vec<(u32, f64)>,
    /// Frozen rows and columns at the top left.
    pub frozen: (u32, u32),
    #[serde(skip)]
    pub col_x: Vec<f64>,
    #[serde(skip)]
    pub row_y: Vec<f64>,
}

impl Grid {
    pub fn x(&self, col: u32) -> f64 {
        self.col_x.get(col as usize).copied().unwrap_or_else(|| {
            self.col_x.last().copied().unwrap_or(0.0) + (col as usize + 1 - self.col_x.len()) as f64 * self.default_width
        })
    }

    pub fn y(&self, row: u32) -> f64 {
        self.row_y.get(row as usize).copied().unwrap_or_else(|| {
            self.row_y.last().copied().unwrap_or(0.0) + (row as usize + 1 - self.row_y.len()) as f64 * self.default_height
        })
    }

    pub fn width(&self, col: u32) -> f64 {
        self.x(col + 1) - self.x(col)
    }

    pub fn height(&self, row: u32) -> f64 {
        self.y(row + 1) - self.y(row)
    }
}

pub struct SheetsView {
    pub doc: Document,
    /// What it saves as: a workbook, or CSV.
    pub kind: Kind,
    pub(crate) measure: Measure,
    pub(crate) styles: Styles,
    pub(crate) cache: flow::Cache,
    pub(crate) sel: SheetSel,
    pub(crate) history: History<CStep, SheetSel>,
    pub(crate) saved: u64,
    pub(crate) styles_sent: usize,
    pub(crate) grids: HashMap<usize, Grid>,
    /// Goes up with every change, so the app knows to draw again.
    pub(crate) revision: u64,
    /// What NOW() and TODAY() give: a serial number.
    pub(crate) now: Option<f64>,
    pending_cut: Option<commands::PendingCut>,
    /// The formulas in a circular reference, as of the last recalculation.
    pub(crate) circular: Vec<(usize, CellRef)>,
}

/// A block of the grid drawn: items in grid pixels.
#[derive(Clone, Debug, Serialize)]
pub struct GridPaint {
    pub items: Vec<Item>,
}

/// A cell as the formula bar and inspector see it.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellInfo {
    /// `B7`.
    pub address: String,
    /// What typing in it starts from: the formula with `=`, or the value as text.
    pub input: String,
    /// What it shows.
    pub display: String,
    pub number_format: String,
    pub font: Option<String>,
    /// Points.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<String>,
    pub fill: Option<String>,
    pub align: Option<String>,
    pub valign: Option<String>,
    pub wrap: bool,
    /// Whether the cell is part of a merged range.
    pub merged: bool,
}

/// Excel's default font and size, when the style sheet says nothing.
const DEFAULT_FONT: &str = "Calibri";
const DEFAULT_PT: f64 = 11.0;

impl SheetsView {
    /// Opens a workbook or a CSV file (`name` tells which, with the bytes).
    pub fn open(bytes: &[u8], name: &str) -> office_core::Result<SheetsView> {
        let mut doc = Compiler::new().with_layer(Sheets).read(bytes, Some(name))?;
        let kind = doc.kind;
        if !matches!(kind, Kind::Sheets | Kind::Csv) {
            return Err(office_core::Error::Invalid(format!("this is {}, not an Excel workbook", kind.described())));
        }
        if kind == Kind::Csv {
            // A CSV file is edited as a workbook of one sheet, and saved back as CSV.
            crate::excel_mut(&mut doc);
        }
        Ok(SheetsView::from_document(doc, kind))
    }

    /// A new, empty workbook (or CSV file).
    pub fn blank(kind: Kind) -> SheetsView {
        let doc = Document::new(Kind::Sheets);
        let bytes = Compiler::new().with_layer(Sheets).write(&doc, Kind::Sheets).expect("a new workbook writes");
        let mut v = SheetsView::open(&bytes, "Book.xlsx").expect("a new workbook reads");
        v.kind = kind;
        v
    }

    pub fn from_document(doc: Document, kind: Kind) -> SheetsView {
        let circular = crate::circular_references(&doc, None);
        SheetsView {
            doc,
            kind,
            measure: Measure::new(),
            styles: Styles::new(),
            cache: flow::Cache::new(),
            sel: SheetSel::default(),
            history: History::new(),
            saved: 0,
            styles_sent: 0,
            grids: HashMap::new(),
            revision: 1,
            pending_cut: None,
            now: None,
            circular,
        }
    }

    pub fn save(&mut self) -> office_core::Result<Vec<u8>> {
        Compiler::new().with_layer(Sheets).write(&self.doc, if self.kind == Kind::Csv { Kind::Csv } else { Kind::Sheets })
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.history.version();
    }

    pub fn is_dirty(&self) -> bool {
        self.history.version() != self.saved
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Sets what NOW() and TODAY() give, and works out the formulas that have no result yet.
    /// Opening a file must keep its cached results: our evaluator cannot reproduce every
    /// formula Excel supports.
    pub fn set_now(&mut self, serial: f64) {
        self.now = Some(serial);
        // A circular reference's cached result is whatever the app that saved it got to round
        // the loop, so it is 0 instead, before the formulas that read it are worked out.
        // Without the clock, NOW() would make some look circular.
        self.circular = crate::circular_references(&self.doc, self.now);
        let mut zeroed = false;
        if let Body::Workbook { sheets } = &mut self.doc.body {
            for &(s, c) in &self.circular {
                let cell = sheets[s].cell_mut(c);
                if cell.value != Value::Number(0.0) {
                    cell.value = Value::Number(0.0);
                    zeroed = true;
                }
            }
        }
        let worked_out = crate::calculate_uncached(&mut self.doc, self.now) > 0;
        if worked_out || zeroed {
            self.changed(None);
        }
    }

    /// The formulas in a circular reference (sheet index and cell), in sheet order.
    pub fn circular(&self) -> &[(usize, CellRef)] {
        &self.circular
    }

    /// Works the formulas out again after a change.
    pub(crate) fn recalculate(&mut self) {
        self.circular = crate::recalculate_with_report(&mut self.doc, self.now).circular;
    }

    pub fn sheets(&self) -> &[Sheet] {
        match &self.doc.body {
            Body::Workbook { sheets } => sheets,
            _ => &[],
        }
    }

    pub(crate) fn sheets_mut(&mut self) -> &mut Vec<Sheet> {
        match &mut self.doc.body {
            Body::Workbook { sheets } => sheets,
            _ => panic!("a sheets view holds a workbook"),
        }
    }

    pub fn selection(&self) -> SheetSel {
        self.sel
    }

    /// The sheet's columns and rows, worked out once per change.
    pub fn grid(&mut self, sheet: usize) -> &Grid {
        if !self.grids.contains_key(&sheet) {
            let g = self.make_grid(sheet);
            self.grids.insert(sheet, g);
        }
        &self.grids[&sheet]
    }

    pub(crate) fn changed(&mut self, sheet: Option<usize>) {
        match sheet {
            Some(s) => {
                self.grids.remove(&s);
            }
            None => self.grids.clear(),
        }
        self.revision += 1;
    }

    fn make_grid(&self, sheet: usize) -> Grid {
        let Some(s) = self.sheets().get(sheet) else { return Grid::default() };
        let used = s.used_range();
        let merges_end = s.merges.iter().map(|m| (m.end.row, m.end.col)).fold((0, 0), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        // Room for the charts too.
        let charts_end =
            s.charts.iter().map(|c| c.anchor.to.unwrap_or(c.anchor.from)).fold((0, 0), |a, p| (a.0.max(p.row), a.1.max(p.col)));
        let content = (
            used.map(|u| u.end.row + 1).unwrap_or(0).max(merges_end.0 + 1).max(charts_end.0 + 1),
            used.map(|u| u.end.col + 1).unwrap_or(0).max(merges_end.1 + 1).max(charts_end.1 + 1),
        );
        // A size someone set, but never cutting off what's there; otherwise room to carry on.
        let (rows, cols) = match s.size {
            Some((r, c)) => (r.max(content.0), c.max(content.1)),
            None => (content.0 + 200, content.1.max(26) + 26),
        };
        // No further than Excel's last row and column: a row numbered in the billions (a damaged
        // file) would otherwise ask for gigabytes of row positions.
        let (rows, cols) = (rows.min(crate::MAX_ROWS), cols.min(crate::MAX_COLS));
        let content = (content.0.min(rows), content.1.min(cols));
        let origin = s.origin.get();
        let fmt = origin.and_then(|o| o.child(ns::X, "sheetFormatPr"));
        let chars = |w: f64| (w * 7.0 + 5.0).round();
        let default_width = fmt
            .and_then(|f| f.attr("defaultColWidth"))
            .and_then(|v| v.parse::<f64>().ok())
            .map(|w| (w * 7.0).round())
            .unwrap_or_else(|| chars(8.43));
        let default_height = fmt
            .and_then(|f| f.attr("defaultRowHeight"))
            .and_then(|v| v.parse::<f64>().ok())
            .map(|pt| (pt * 96.0 / 72.0).round())
            .unwrap_or(20.0);
        let mut widths = Vec::new();
        if let Some(cols_el) = origin.and_then(|o| o.child(ns::X, "cols")) {
            for c in cols_el.children_named(ns::X, "col") {
                let n = |k: &str| c.attr(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                let hidden = c.attr("hidden").and_then(office_core::units::parse_bool).unwrap_or(false);
                let w = if hidden { Some(0.0) } else { c.attr("width").and_then(|v| v.parse::<f64>().ok()).map(chars) };
                if let Some(w) = w {
                    for col in n("min").max(1)..=n("max").min(cols) {
                        widths.push((col - 1, w));
                    }
                }
            }
        }
        let mut heights = Vec::new();
        for row in &s.rows {
            let Some(o) = row.origin.get() else { continue };
            if o.attr("hidden").and_then(office_core::units::parse_bool).unwrap_or(false) {
                heights.push((row.index, 0.0));
            } else if let Some(h) = o.attr("ht").and_then(|v| v.parse::<f64>().ok()) {
                if o.attr("customHeight").is_some() || (h - default_height * 0.75).abs() > 0.1 {
                    heights.push((row.index, (h * 96.0 / 72.0).round()));
                }
            }
        }
        let pane = origin.and_then(|o| o.descendant(ns::X, "pane")).filter(|p| p.attr("state").is_some_and(|s| s.starts_with("frozen")));
        let frozen = pane
            .map(|p| {
                (
                    p.attr("ySplit").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) as u32,
                    p.attr("xSplit").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) as u32,
                )
            })
            .unwrap_or((0, 0));
        let mut col_x = Vec::with_capacity(cols as usize + 1);
        let mut x = 0.0;
        let wmap: HashMap<u32, f64> = widths.iter().copied().collect();
        for c in 0..=cols {
            col_x.push(x);
            x += wmap.get(&c).copied().unwrap_or(default_width);
        }
        let hmap: HashMap<u32, f64> = heights.iter().copied().collect();
        let mut row_y = Vec::with_capacity(rows as usize + 1);
        let mut y = 0.0;
        for rr in 0..=rows {
            row_y.push(y);
            y += hmap.get(&rr).copied().unwrap_or(default_height);
        }
        Grid { rows, cols, content, default_width, default_height, widths, heights, frozen, col_x, row_y }
    }

    fn style_of(&mut self, f: &CellFormat) -> para::StyleId {
        style_of(&self.doc, &mut self.styles, &mut self.measure, f)
    }

    /// Draws a block of a sheet: rows `r0..r1`, columns `c0..c1` (ends excluded), in grid pixels.
    pub fn paint(&mut self, host: &mut dyn Measurer, sheet: usize, r0: u32, r1: u32, c0: u32, c1: u32) -> GridPaint {
        self.grid(sheet);
        let gridlines = self.show_gridlines(sheet);
        let SheetsView { doc, measure, styles, cache, grids, .. } = self;
        let doc: &Document = doc;
        let grid = &grids[&sheet];
        let Some(s) = (match &doc.body {
            Body::Workbook { sheets } => sheets.get(sheet),
            _ => None,
        }) else {
            return GridPaint { items: Vec::new() };
        };
        let (r1, c1) = (r1.min(grid.rows), c1.min(grid.cols));
        let mut items = Vec::new();
        if gridlines {
            let (x0, x1, y0, y1) = (grid.x(c0), grid.x(c1), grid.y(r0), grid.y(r1));
            for c in c0..=c1 {
                let x = grid.x(c);
                items.push(Item::Line { x1: r(x), y1: r(y0), x2: r(x), y2: r(y1), color: "#e1e1e1".into(), w: 1.0, dash: "solid" });
            }
            for rr in r0..=r1 {
                let y = grid.y(rr);
                items.push(Item::Line { x1: r(x0), y1: r(y), x2: r(x1), y2: r(y), color: "#e1e1e1".into(), w: 1.0, dash: "solid" });
            }
        }
        let covered = |at: CellRef| s.merges.iter().find(|m| m.contains(at));
        let mut texts = Vec::new();
        let mut borders = Vec::new();
        let start = s.rows.partition_point(|row| row.index < r0);
        // Merged ranges that start above or left of the block but reach into it are drawn whole.
        let mut extra: Vec<CellRef> = s
            .merges
            .iter()
            .filter(|m| {
                m.end.row >= r0 && m.start.row < r1 && m.end.col >= c0 && m.start.col < c1 && (m.start.row < r0 || m.start.col < c0)
            })
            .map(|m| m.start)
            .collect();
        extra.sort();
        let mut cells: Vec<CellRef> = extra;
        for row in s.rows[start..].iter().take_while(|row| row.index < r1) {
            // Text running into the block from the cell to its left: start a column early.
            let lo = row.cells.partition_point(|c| c.col < c0);
            let from = lo.saturating_sub(1);
            for c in row.cells[from..].iter().take_while(|c| c.col < c1) {
                cells.push(CellRef::new(row.index, c.col));
            }
        }
        for at in cells {
            let merge = covered(at);
            if merge.is_some_and(|m| m.start != at) {
                continue;
            }
            let (cx, cy) = (grid.x(at.col), grid.y(at.row));
            let (cw, ch) = match merge {
                Some(m) => (grid.x(m.end.col + 1) - cx, grid.y(m.end.row + 1) - cy),
                None => (grid.width(at.col), grid.height(at.row)),
            };
            if cw <= 0.0 || ch <= 0.0 {
                continue;
            }
            let f = format_of(doc, sheet, at);
            if let Some(fill) = fill_css(doc, &f) {
                items.push(Item::Rect { x: r(cx), y: r(cy), w: r(cw), h: r(ch), fill: Some(fill) });
            }
            borders_of(doc, &f, cx, cy, cw, ch, &mut borders);
            let value = s.value(at);
            if value.is_empty() {
                continue;
            }
            let formatted = crate::numfmt::format(value, &f.number_format, crate::excel(doc).map(|x| x.date_system).unwrap_or_default());
            let mut shown = formatted.text.clone();
            let is_number = matches!(value, Value::Number(_));
            let align = f.alignment.horizontal.clone().unwrap_or_else(|| match value {
                Value::Number(_) => "right".into(),
                Value::Bool(_) | Value::Error(_) => "center".into(),
                _ => "left".into(),
            });
            let mut style = style_of(doc, styles, measure, &f);
            if let Some(color) = formatted.color.as_deref().and_then(numfmt_color) {
                let mut ts = styles.get(style).clone();
                ts.color = color;
                style = styles.add(measure, ts);
            }
            // Text running past the cell into empty cells, as Excel draws it (left-aligned only).
            let mut room = cw;
            if merge.is_none() && !f.alignment.wrap && align == "left" && !is_number {
                let row = s.rows.binary_search_by_key(&at.row, |row| row.index).ok().map(|i| &s.rows[i]);
                let mut next = at.col + 1;
                while next < c1.max(at.col + 40)
                    && row.is_none_or(|row| row.cells.iter().find(|c| c.col == next).is_none_or(|c| c.value.is_empty()))
                    && covered(CellRef::new(at.row, next)).is_none()
                {
                    room += grid.width(next);
                    next += 1;
                    if room > 4000.0 {
                        break;
                    }
                }
            }
            let mut ctx = Ctx { host, measure, styles };
            // A number too wide for its cell shows as ###, as in Excel. In General it first
            // loses decimal places until it fits.
            if is_number && !f.alignment.wrap {
                let mut w = ctx.text_width(style, &shown);
                if let (Value::Number(n), true) =
                    (value, w > cw - 4.0 && f.number_format.eq_ignore_ascii_case("General") && shown.contains('.') && !shown.contains('E'))
                {
                    let decimals = shown.split_once('.').map_or(0, |(_, d)| d.len());
                    for d in (0..decimals).rev() {
                        let s = format!("{:.*}", d, n);
                        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
                        w = ctx.text_width(style, &s);
                        shown = s;
                        if w <= cw - 4.0 {
                            break;
                        }
                    }
                }
                if w > cw - 4.0 {
                    let hash = ctx.text_width(style, "#").max(1.0);
                    shown = "#".repeat(((cw - 4.0) / hash).floor().max(1.0) as usize);
                }
            }
            let mut spec = ParaSpec::plain(&shown, style);
            spec.align = match align.as_str() {
                "center" | "centerContinuous" => para::Align::Center,
                "right" => para::Align::Right,
                "justify" | "distributed" => para::Align::Justify,
                _ => para::Align::Left,
            };
            spec.indent_start = 3.0 + f.alignment.indent as f64 * 9.0;
            spec.indent_end = 3.0;
            spec.line_gap = false;
            // Unwrapped text is one line at its own width: centered or right-aligned text wider
            // than its cell runs out on both sides or to the left, as in Excel.
            let natural = ctx.text_width(style, &shown) + spec.indent_start + spec.indent_end + 1.0;
            let (layout_w, dx) = if f.alignment.wrap {
                (cw, 0.0)
            } else if spec.align == para::Align::Left {
                (natural.max(room), 0.0)
            } else {
                let w = natural.max(cw);
                (w, if spec.align == para::Align::Center { (cw - w) / 2.0 } else { cw - w })
            };
            let spec = std::sync::Arc::new(spec);
            let layout = cache.layout(&mut ctx, &spec, layout_w);
            let flow = FlowLayout {
                height: layout.height,
                blocks: vec![BlockBox::Para(ParaBox { path: Path::top(0), spec, layout, keep: Keep::default(), y: 0.0 })],
            };
            let dy = match f.alignment.vertical.as_deref() {
                Some("top") => 1.0,
                Some("center") => (ch - flow.height) / 2.0,
                _ => ch - flow.height - 2.0,
            };
            let mut out = Vec::new();
            let links = |_: &Path, _: u32| None;
            display::paint_flow(
                &mut ctx,
                &flow,
                dx,
                dy,
                (f64::MIN, f64::MAX),
                &mut Sink { objects: &mut NoObjects, links: &links, out: &mut out },
            );
            let (clip_x, clip_w) = if f.alignment.wrap || is_number {
                (0.0, cw)
            } else if spec_align_left(&flow) {
                (0.0, room)
            } else {
                (dx.min(0.0), cw - 2.0 * dx.min(0.0))
            };
            for it in &mut out {
                if let Item::Text { x, .. } = it {
                    *x = r(*x - clip_x);
                }
            }
            texts.push(Item::Group {
                x: r(cx + clip_x),
                y: r(cy),
                w: r(clip_w),
                h: r(ch),
                rot: 0.0,
                fh: false,
                fv: false,
                clip: true,
                id: None,
                items: out,
            });
        }
        items.extend(texts);
        items.extend(borders);
        GridPaint { items }
    }

    /// Whether the sheet shows gridlines (`showGridLines`).
    fn show_gridlines(&self, sheet: usize) -> bool {
        self.sheets()
            .get(sheet)
            .and_then(|s| s.origin.get())
            .and_then(|o| o.descendant(ns::X, "sheetView"))
            .and_then(|v| v.attr("showGridLines"))
            .and_then(office_core::units::parse_bool)
            .unwrap_or(true)
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
        self.revision += 1;
    }

    /// Widens a sheet's columns to fit what the first rows hold (up to 320px), for a CSV file,
    /// which keeps no widths. Not an edit: nothing to undo, and the file isn't dirty.
    pub fn fit_columns(&mut self, host: &mut dyn Measurer, sheet: usize) {
        const SAMPLE: usize = 500;
        let default = self.grid(sheet).default_width;
        let Some(s) = self.sheets().get(sheet) else { return };
        let system = crate::excel(&self.doc).map(|x| x.date_system).unwrap_or_default();
        let mut cells = Vec::new();
        for row in s.rows.iter().take(SAMPLE) {
            for c in &row.cells {
                let at = CellRef::new(row.index, c.col);
                let f = format_of(&self.doc, sheet, at);
                let mut text = crate::numfmt::format(&c.value, &f.number_format, system).text;
                // General numbers give up decimals to fit, so a long one needn't widen the column.
                // Cut on a character: `123.45 €` is only eight characters, but byte 9 is inside `€`.
                if matches!(c.value, Value::Number(_)) {
                    if let Some((i, _)) = text.char_indices().nth(9) {
                        text.truncate(i);
                    }
                }
                cells.push((c.col, f, text));
            }
        }
        let mut widths: HashMap<u32, f64> = HashMap::new();
        for (col, f, text) in cells {
            if text.is_empty() {
                continue;
            }
            let style = self.style_of(&f);
            let w = Ctx { host, measure: &mut self.measure, styles: &mut self.styles }.text_width(style, &text) + 10.0;
            let e = widths.entry(col).or_insert(0.0);
            *e = e.max(w);
        }
        let mut widths: Vec<_> = widths.into_iter().filter(|&(_, w)| w > default).collect();
        widths.sort_by_key(|&(c, _)| c);
        for (col, w) in widths {
            crate::set_column_width(&mut self.doc, sheet, col, Some(((w.min(320.0).ceil() - 5.0) / 7.0).max(0.0)));
        }
        self.changed(Some(sheet));
    }

    /// A cell, as the formula bar and inspector see it.
    pub fn cell_info(&self, sheet: usize, at: CellRef) -> CellInfo {
        let f = format_of(&self.doc, sheet, at);
        let s = self.sheets().get(sheet);
        let value = s.map(|s| s.value(at).clone()).unwrap_or_default();
        let input = match crate::formula(&self.doc, sheet, at) {
            Some(fm) => format!("={}", crate::formula::display_text(&fm)),
            None => match &value {
                Value::Number(n) if crate::numfmt::is_date_format(&f.number_format) => crate::display(&self.doc, sheet, at),
                Value::Number(n) => office_core::model::format_number(*n),
                v => v.display(),
            },
        };
        let theme = self.doc.theme.clone();
        let css = |c: &office_core::drawing::Color| c.resolve(&|n| theme.as_ref()?.scheme_color(n)).rgb.hex();
        CellInfo {
            address: format!("{}{}", CellRef::column_name(at.col), at.row + 1),
            input,
            display: crate::display(&self.doc, sheet, at),
            number_format: f.number_format.clone(),
            font: f.font.name.clone(),
            size: f.font.size.unwrap_or(DEFAULT_PT),
            bold: f.font.bold,
            italic: f.font.italic,
            underline: f.font.underline.as_deref().is_some_and(|u| u != "none"),
            strike: f.font.strike,
            color: f.font.color.as_ref().map(css),
            fill: if f.fill.pattern == "solid" { f.fill.fg.as_ref().map(css) } else { None },
            align: f.alignment.horizontal.clone(),
            valign: f.alignment.vertical.clone(),
            wrap: f.alignment.wrap,
            merged: s.is_some_and(|s| s.merges.iter().any(|m| m.contains(at))),
        }
    }
}

fn spec_align_left(flow: &FlowLayout) -> bool {
    matches!(flow.blocks.first(), Some(BlockBox::Para(p)) if p.spec.align == para::Align::Left)
}

/// A number format's color name (`[Red]`) as paint.
fn numfmt_color(name: &str) -> Option<Rgba> {
    let rgb = match name.to_ascii_lowercase().as_str() {
        "red" => Rgb(255, 0, 0),
        "blue" => Rgb(0, 0, 255),
        "green" => Rgb(0, 128, 0),
        "magenta" => Rgb(255, 0, 255),
        "cyan" => Rgb(0, 255, 255),
        "yellow" => Rgb(255, 255, 0),
        "white" => Rgb(255, 255, 255),
        "black" => Rgb(0, 0, 0),
        _ => return None,
    };
    Some(Rgba { rgb, alpha: 1.0 })
}

fn fill_css(doc: &Document, f: &CellFormat) -> Option<String> {
    if f.fill.pattern == "none" || f.fill.pattern.is_empty() {
        return None;
    }
    let c = f.fill.fg.as_ref().or(f.fill.bg.as_ref())?;
    Some(c.resolve(&|n| doc.theme.as_ref()?.scheme_color(n)).css())
}

/// A cell format's text style.
fn style_of(doc: &Document, styles: &mut Styles, measure: &mut Measure, f: &CellFormat) -> para::StyleId {
    let props = RunProps {
        font: Some(f.font.name.clone().unwrap_or_else(|| DEFAULT_FONT.into())),
        size: Some(FontSize::from_points(f.font.size.unwrap_or(DEFAULT_PT))),
        bold: f.font.bold.then_some(true),
        italic: f.font.italic.then_some(true),
        underline: f.font.underline.clone(),
        strike: f.font.strike.then_some(office_core::model::Strike::Single),
        ..Default::default()
    };
    let theme = doc.theme.clone();
    let paint = move |c: &office_core::drawing::Color| c.resolve(&|n| theme.as_ref()?.scheme_color(n));
    let mut s = TextStyle::from_props(&props, DEFAULT_PT * 96.0 / 72.0, &paint);
    if let Some(c) = &f.font.color {
        s.color = paint(c);
    }
    styles.add(measure, s)
}

fn borders_of(doc: &Document, f: &CellFormat, x: f64, y: f64, w: f64, h: f64, out: &mut Vec<Item>) {
    let sides = [
        (&f.border.top, (x, y, x + w, y)),
        (&f.border.bottom, (x, y + h, x + w, y + h)),
        (&f.border.left, (x, y, x, y + h)),
        (&f.border.right, (x + w, y, x + w, y + h)),
    ];
    for (side, (x1, y1, x2, y2)) in sides {
        let Some(style) = side.style.as_deref().filter(|s| *s != "none") else { continue };
        let (width, dash) = match style {
            "medium" | "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" => (2.0, if style == "medium" { "solid" } else { "dashed" }),
            "thick" => (3.0, "solid"),
            "double" => (3.0, "double"),
            "dashed" | "dashDot" | "dashDotDot" | "slantDashDot" => (1.0, "dashed"),
            "dotted" | "hair" => (1.0, "dotted"),
            _ => (1.0, "solid"),
        };
        let color =
            side.color.as_ref().map(|c| c.resolve(&|n| doc.theme.as_ref()?.scheme_color(n)).css()).unwrap_or_else(|| "#000000".into());
        out.push(Item::Line { x1: r(x1), y1: r(y1), x2: r(x2), y2: r(y2), color, w: width, dash });
    }
}

#[cfg(test)]
mod tests;
