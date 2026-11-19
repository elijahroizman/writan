//! Charts on a sheet: what they plot, from which cells, and where they sit.
//!
//! The model covers the common charts (column, bar, line, area, pie, doughnut, scatter). A chart
//! read from a file remembers its parts ([`ChartSource`]); saving leaves the chart's part alone
//! while the chart is as it was read, so its styling survives. A chart of another kind (radar,
//! stock, bubble…) is read as [`ChartKind::Other`] and kept as it is unless it's changed.

use serde::{Deserialize, Serialize};

use super::{CellRef, Range, Sheet, Value};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartKind {
    #[default]
    Column,
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
    /// A kind the model doesn't draw or write.
    Other,
}

impl ChartKind {
    /// Whether the chart has axes (pies and doughnuts don't).
    pub fn has_axes(self) -> bool {
        !matches!(self, ChartKind::Pie | ChartKind::Doughnut)
    }
}

/// How the series of a column, bar, line or area chart share the space.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartGrouping {
    /// Side by side (columns and bars) or each on its own (lines and areas).
    #[default]
    Standard,
    Stacked,
    PercentStacked,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegendPosition {
    Right,
    Bottom,
    Top,
    Left,
}

/// A corner of a chart: a cell and how far into it, in EMUs.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchorPoint {
    pub col: u32,
    pub col_off: i64,
    pub row: u32,
    pub row_off: i64,
}

/// Where a chart sits: from a cell to a cell, or from a cell with a size.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartAnchor {
    pub from: AnchorPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<AnchorPoint>,
    /// Width and height in EMUs, when there's no `to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<(i64, i64)>,
}

/// A picture on a worksheet, as its drawing places it. Pictures are shown, not edited: the
/// drawing that holds them is saved as it was read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetPicture {
    pub anchor: ChartAnchor,
    /// The image part.
    pub media: String,
    /// How much is cut off each side (left, top, right, bottom), as fractions of the image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Where the drawing had it when read, so a picture that moved with its cells is found again.
    #[serde(skip)]
    pub read_as: Option<ChartAnchor>,
}

/// What a file had cached for a series, for references the app can't follow.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesCache {
    pub name: Option<String>,
    pub categories: Vec<String>,
    pub values: Vec<Option<f64>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeries {
    /// The cell holding the series' name, as a formula reference (`Sheet1!$B$1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_ref: Option<String>,
    /// The name itself, when no cell holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Category labels (or a scatter chart's x values), as a reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<String>,
    /// The values, as a reference.
    #[serde(default)]
    pub values: String,
    /// `RRGGBB`, or a theme slot (`accent2`); none for the theme's color in turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub cache: SeriesCache,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chart {
    pub kind: ChartKind,
    #[serde(default)]
    pub grouping: ChartGrouping,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend: Option<LegendPosition>,
    pub series: Vec<ChartSeries>,
    pub anchor: ChartAnchor,
    /// Where it was read from, if it was.
    #[serde(skip)]
    pub source: Option<ChartSource>,
}

/// A chart's parts in the file it was read from, and the chart as read.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSource {
    pub drawing: String,
    pub part: String,
    pub read_as: Box<Chart>,
}

impl Chart {
    /// Whether what it plots and how is as read (where it sits may have changed).
    pub fn content_untouched(&self) -> bool {
        self.source.as_ref().is_some_and(|s| {
            let r = &s.read_as;
            r.kind == self.kind
                && r.grouping == self.grouping
                && r.title == self.title
                && r.legend == self.legend
                && r.series == self.series
        })
    }

    /// Whether it's exactly as read.
    pub fn untouched(&self) -> bool {
        self.content_untouched() && self.source.as_ref().is_some_and(|s| s.read_as.anchor == self.anchor)
    }
}

/// A reference in a chart, `'My sheet'!$A$2:$A$9`: the sheet's name and the range.
pub fn parse_ref(f: &str) -> Option<(String, Range)> {
    let f = f.trim().trim_start_matches('=');
    let f = f.strip_prefix('(').and_then(|f| f.strip_suffix(')')).unwrap_or(f);
    let bang = f.rfind('!')?;
    let (sheet, range) = (&f[..bang], &f[bang + 1..]);
    let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(q) => q.replace("''", "'"),
        None => sheet.to_string(),
    };
    if sheet.is_empty() || range.contains(',') {
        return None;
    }
    Some((sheet, Range::parse(range)?))
}

/// A sheet's name as a formula writes it: quoted when it has to be.
pub fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && name.starts_with(|c: char| c.is_alphabetic() || c == '_')
        && CellRef::parse(name).is_none()
        && !name.eq_ignore_ascii_case("TRUE")
        && !name.eq_ignore_ascii_case("FALSE")
        && !looks_like_r1c1(name);
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn looks_like_r1c1(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    let b = u.as_bytes();
    (b.first() == Some(&b'R') || b.first() == Some(&b'C')) && b[1..].iter().all(|c| c.is_ascii_digit() || *c == b'C')
}

/// An absolute reference to `range` on sheet `name`: `Sheet1!$A$2:$A$9`.
pub fn make_ref(name: &str, range: Range) -> String {
    let abs = |c: CellRef| format!("${}${}", CellRef::column_name(c.col), c.row + 1);
    if range.start == range.end {
        format!("{}!{}", quote_sheet(name), abs(range.start))
    } else {
        format!("{}!{}:{}", quote_sheet(name), abs(range.start), abs(range.end))
    }
}

/// The cells a reference names, in order (a row's or column's worth, or a block row by row).
pub fn ref_cells(sheets: &[Sheet], f: &str) -> Option<(usize, Vec<CellRef>)> {
    let (name, range) = parse_ref(f)?;
    let sheet = sheets.iter().position(|s| s.name.eq_ignore_ascii_case(&name))?;
    // A whole column (A1:A1048576) is cut to what the sheet uses.
    let used = sheets[sheet].used_range();
    let last_row = used.map(|u| u.end.row).unwrap_or(0).min(range.end.row);
    let last_col = used.map(|u| u.end.col).unwrap_or(0).min(range.end.col);
    let (r1, c1) =
        (if range.rows() > 10_000 { last_row } else { range.end.row }, if range.cols() > 1_000 { last_col } else { range.end.col });
    let mut cells = Vec::new();
    for r in range.start.row..=r1.max(range.start.row) {
        for c in range.start.col..=c1.max(range.start.col) {
            cells.push(CellRef::new(r, c));
            if cells.len() >= 100_000 {
                return Some((sheet, cells));
            }
        }
    }
    Some((sheet, cells))
}

/// A reference's values as numbers (`None` for a cell without one).
pub fn ref_numbers(sheets: &[Sheet], f: &str) -> Option<Vec<Option<f64>>> {
    let (sheet, cells) = ref_cells(sheets, f)?;
    Some(
        cells
            .iter()
            .map(|&at| match sheets[sheet].value(at) {
                Value::Number(n) => Some(*n),
                _ => None,
            })
            .collect(),
    )
}

/// A reference's values as text, as the file's cache holds them.
pub fn ref_texts(sheets: &[Sheet], f: &str) -> Option<Vec<String>> {
    let (sheet, cells) = ref_cells(sheets, f)?;
    Some(cells.iter().map(|&at| sheets[sheet].value(at).display()).collect())
}

/// Points a reference to sheet `old` at the sheet's new name.
pub fn rename_in_ref(f: &str, old: &str, new: &str) -> String {
    match parse_ref(f) {
        Some((sheet, _)) if sheet.eq_ignore_ascii_case(old) => {
            let bang = f.rfind('!').unwrap();
            format!("{}{}", quote_sheet(new), &f[bang..])
        }
        _ => f.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references() {
        assert_eq!(parse_ref("Sheet1!$B$2:$B$5"), Some(("Sheet1".into(), Range::parse("B2:B5").unwrap())));
        assert_eq!(parse_ref("'It''s'!A1"), Some(("It's".into(), Range::parse("A1").unwrap())));
        assert_eq!(make_ref("My data", Range::parse("A2:A9").unwrap()), "'My data'!$A$2:$A$9");
        assert_eq!(make_ref("Sheet1", Range::parse("C3").unwrap()), "Sheet1!$C$3");
        assert_eq!(quote_sheet("A1"), "'A1'");
        assert_eq!(quote_sheet("٣ Data"), "'٣ Data'");
        assert_eq!(quote_sheet("٣Data"), "'٣Data'");
        assert_eq!(rename_in_ref("Sheet1!$A$1:$A$3", "sheet1", "Q 3"), "'Q 3'!$A$1:$A$3");
        assert_eq!(rename_in_ref("Other!$A$1", "Sheet1", "X"), "Other!$A$1");
    }
}
