//! Sheets: sparse grids of values, as Excel and CSV have them.

use std::fmt;

use serde::{Deserialize, Serialize};

use std::hash::{Hash, Hasher};

use super::{Fingerprint, Origin, Run};

/// The most grid slots a sheet is laid out in, as a table or as rows of text. Sheets are
/// sparse, so two cells far apart (A1 and XFD1048576) make a grid of 17 billion slots; a
/// sheet bigger than this is refused rather than laid out.
pub const MAX_LAID_OUT: u64 = 4_000_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    pub name: String,
    /// Rows that have something in them, in order.
    pub rows: Vec<SheetRow>,
    /// Merged ranges: the value is in each range's top-left cell.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merges: Vec<Range>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Charts on the sheet, back to front.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub charts: Vec<super::Chart>,
    /// Pictures on the sheet, back to front.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<super::SheetPicture>,
    /// The rows and columns the sheet is shown with, when someone set them. Excel has no such
    /// thing; the sheets layer keeps it in a hidden name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<(u32, u32)>,
    /// The part the sheet was read from, so saving writes it back there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(default, skip_serializing_if = "crate::model::wire::off", with = "crate::model::wire::origin")]
    pub origin: Origin,
    /// [`Sheet::fingerprint`] as read, so saving can leave an untouched sheet's part alone
    /// without reading it again.
    #[serde(default, skip_serializing_if = "crate::model::wire::off")]
    pub read_as: Fingerprint,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRow {
    /// From 0.
    pub index: u32,
    /// Cells with something in them, in column order.
    pub cells: Vec<SheetCell>,
    #[serde(default, skip_serializing_if = "crate::model::wire::off", with = "crate::model::wire::origin")]
    pub origin: Origin,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetCell {
    /// From 0.
    pub col: u32,
    pub value: Value,
    /// The formula without its `=`, as Excel stores it; `value` is its last result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// Excel's cell format (an index into the style sheet's `cellXfs`), for the Excel layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<u32>,
    #[serde(default, skip_serializing_if = "crate::model::wire::off", with = "crate::model::wire::origin")]
    pub origin: Origin,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum Value {
    #[default]
    Empty,
    /// Dates are numbers too (days since 1900 or 1904); the cell's format says it's a date.
    Number(f64),
    Text(String),
    /// Text with formatting, from Excel's rich-text strings.
    Rich(Vec<Run>),
    Bool(bool),
    /// `#DIV/0!`, `#N/A`…
    Error(String),
}

impl Value {
    /// The value as text the way a CSV file holds it.
    pub fn display(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Text(t) | Value::Error(t) => t.clone(),
            Value::Rich(runs) => runs.iter().map(|r| r.text.as_str()).collect(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }
}

/// A number in the shortest form that reads back the same, without an exponent for the sizes
/// spreadsheets usually hold.
pub fn format_number(n: f64) -> String {
    if n.is_finite() && n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// A cell's address, from 0: `CellRef { row: 0, col: 0 }` is A1.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub const fn new(row: u32, col: u32) -> CellRef {
        CellRef { row, col }
    }

    /// Reads `A1`, `$B$7`, `XFD1048576`.
    pub fn parse(s: &str) -> Option<CellRef> {
        // Every cell of a big sheet comes through here, so no allocating.
        let mut bytes = s.trim().bytes().filter(|&b| b != b'$').peekable();
        let (mut col, mut letters) = (0u32, 0);
        while let Some(b) = bytes.next_if(u8::is_ascii_alphabetic) {
            if letters == 3 {
                return None;
            }
            col = col.checked_mul(26)?.checked_add((b.to_ascii_uppercase() - b'A' + 1) as u32)?;
            letters += 1;
        }
        let (mut row, mut digits) = (0u32, 0);
        for b in bytes {
            if !b.is_ascii_digit() || digits >= 7 {
                return None;
            }
            row = row * 10 + (b - b'0') as u32;
            digits += 1;
        }
        if !(1..=3).contains(&letters) || digits == 0 || row == 0 {
            return None;
        }
        Some(CellRef { row: row - 1, col: col - 1 })
    }

    /// `A`, `Z`, `AA`… for a column from 0.
    pub fn column_name(mut col: u32) -> String {
        let mut out = Vec::new();
        loop {
            out.push(b'A' + (col % 26) as u8);
            if col < 26 {
                break;
            }
            col = col / 26 - 1;
        }
        out.reverse();
        String::from_utf8(out).unwrap()
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", CellRef::column_name(self.col), self.row + 1)
    }
}

impl Serialize for CellRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CellRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<CellRef, D::Error> {
        let s = String::deserialize(d)?;
        CellRef::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not a cell: {s}")))
    }
}

/// A rectangle of cells, `A1:C3`, both corners included.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: CellRef,
    pub end: CellRef,
}

impl Range {
    pub fn parse(s: &str) -> Option<Range> {
        let (a, b) = s.split_once(':').unwrap_or((s, s));
        let (a, b) = (CellRef::parse(a)?, CellRef::parse(b)?);
        Some(Range { start: CellRef::new(a.row.min(b.row), a.col.min(b.col)), end: CellRef::new(a.row.max(b.row), a.col.max(b.col)) })
    }

    pub fn contains(&self, c: CellRef) -> bool {
        (self.start.row..=self.end.row).contains(&c.row) && (self.start.col..=self.end.col).contains(&c.col)
    }

    pub fn rows(&self) -> u32 {
        self.end.row - self.start.row + 1
    }

    pub fn cols(&self) -> u32 {
        self.end.col - self.start.col + 1
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}:{}", self.start, self.end)
        }
    }
}

impl Serialize for Range {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Range {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Range, D::Error> {
        let s = String::deserialize(d)?;
        Range::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not a range: {s}")))
    }
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Sheet {
        Sheet { name: name.into(), ..Default::default() }
    }

    /// A hash of everything the sheet's part holds: its cells, merges and the origins around
    /// them (by identity). The name and visibility live in the workbook, so they're not in it.
    pub fn fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.origin.id().hash(&mut h);
        for m in &self.merges {
            m.hash(&mut h);
        }
        for row in &self.rows {
            (row.index, row.origin.id(), row.cells.len()).hash(&mut h);
            for c in &row.cells {
                (c.col, &c.formula, c.style, c.origin.id()).hash(&mut h);
                match &c.value {
                    Value::Empty => 0u8.hash(&mut h),
                    Value::Number(n) => (1u8, n.to_bits()).hash(&mut h),
                    Value::Text(t) => (2u8, t).hash(&mut h),
                    Value::Rich(runs) => {
                        3u8.hash(&mut h);
                        for r in runs {
                            (&r.text, format!("{:?}", r.props)).hash(&mut h);
                        }
                    }
                    Value::Bool(b) => (4u8, b).hash(&mut h),
                    Value::Error(e) => (5u8, e).hash(&mut h),
                }
            }
        }
        h.finish()
    }

    /// Whether the sheet is as it was read.
    pub fn untouched(&self) -> bool {
        self.read_as.0 == Some(self.fingerprint())
    }

    pub fn cell(&self, at: CellRef) -> Option<&SheetCell> {
        let row = self.rows.binary_search_by_key(&at.row, |r| r.index).ok()?;
        let cells = &self.rows[row].cells;
        cells.binary_search_by_key(&at.col, |c| c.col).ok().map(|i| &cells[i])
    }

    pub fn value(&self, at: CellRef) -> &Value {
        static EMPTY: Value = Value::Empty;
        self.cell(at).map(|c| &c.value).unwrap_or(&EMPTY)
    }

    /// The cell at `at`, made if it isn't there yet.
    pub fn cell_mut(&mut self, at: CellRef) -> &mut SheetCell {
        let row = match self.rows.binary_search_by_key(&at.row, |r| r.index) {
            Ok(i) => i,
            Err(i) => {
                self.rows.insert(i, SheetRow { index: at.row, ..Default::default() });
                i
            }
        };
        let cells = &mut self.rows[row].cells;
        let i = match cells.binary_search_by_key(&at.col, |c| c.col) {
            Ok(i) => i,
            Err(i) => {
                cells.insert(i, SheetCell { col: at.col, ..Default::default() });
                i
            }
        };
        &mut cells[i]
    }

    pub fn set(&mut self, at: CellRef, value: Value) {
        self.cell_mut(at).value = value;
    }

    /// The smallest range holding every cell with something in it.
    pub fn used_range(&self) -> Option<Range> {
        let first = self.rows.iter().find(|r| !r.cells.is_empty())?;
        let last = self.rows.iter().rev().find(|r| !r.cells.is_empty())?;
        let min_col = self.rows.iter().filter_map(|r| r.cells.first()).map(|c| c.col).min()?;
        let max_col = self.rows.iter().filter_map(|r| r.cells.last()).map(|c| c.col).max()?;
        Some(Range { start: CellRef::new(first.index, min_col), end: CellRef::new(last.index, max_col) })
    }

    /// Refuses to lay out a grid from A1 to `end` that's bigger than [`MAX_LAID_OUT`].
    pub fn check_laid_out(&self, end: CellRef) -> crate::Result<()> {
        if (end.row as u64 + 1) * (end.col as u64 + 1) > MAX_LAID_OUT {
            return Err(crate::Error::Invalid(format!("{}: too big to lay out as a table (it reaches {end})", self.name)));
        }
        Ok(())
    }

    /// The sheet as rows of text, from A1 to the last used cell, the way a CSV file holds it.
    pub fn to_text_rows(&self) -> crate::Result<Vec<Vec<String>>> {
        let Some(used) = self.used_range() else { return Ok(Vec::new()) };
        self.check_laid_out(used.end)?;
        let mut out = vec![Vec::new(); used.end.row as usize + 1];
        for row in self.rows.iter().filter(|r| !r.cells.is_empty()) {
            let line = &mut out[row.index as usize];
            for c in &row.cells {
                if line.len() <= c.col as usize {
                    line.resize(c.col as usize + 1, String::new());
                }
                line[c.col as usize] = c.value.display();
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_references() {
        assert_eq!(CellRef::parse("A1"), Some(CellRef::new(0, 0)));
        assert_eq!(CellRef::parse("$AB$12"), Some(CellRef::new(11, 27)));
        assert_eq!(CellRef::parse("XFD1048576"), Some(CellRef::new(1_048_575, 16_383)));
        assert_eq!(CellRef::parse("A0"), None);
        assert_eq!(CellRef::parse("12"), None);
        assert_eq!(CellRef::parse("ABCD1"), None);
        for col in [0, 25, 26, 51, 52, 701, 702, 16_383] {
            let r = CellRef::new(4, col);
            assert_eq!(CellRef::parse(&r.to_string()), Some(r));
        }
        assert_eq!(CellRef::column_name(27), "AB");
        assert_eq!(Range::parse("C3:A1").unwrap().to_string(), "A1:C3");
        assert_eq!(Range::parse("B2").unwrap().rows(), 1);
    }

    #[test]
    fn sparse_cells() {
        let mut s = Sheet::new("Data");
        s.set(CellRef::new(2, 1), Value::Number(3.0));
        s.set(CellRef::new(0, 3), Value::Text("x".into()));
        s.set(CellRef::new(2, 0), Value::Bool(true));
        assert_eq!(s.rows.iter().map(|r| r.index).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(s.value(CellRef::new(2, 1)), &Value::Number(3.0));
        assert_eq!(s.value(CellRef::new(9, 9)), &Value::Empty);
        assert_eq!(s.used_range().unwrap().to_string(), "A1:D3");
        assert_eq!(s.to_text_rows().unwrap(), vec![vec!["", "", "", "x"], vec![], vec!["TRUE", "3"]]);
    }

    #[test]
    fn numbers_print_shortest() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(1.25), "1.25");
        assert_eq!(format_number(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(format_number(-0.5), "-0.5");
        assert_eq!(format_number(1e20), "100000000000000000000");
    }
}
