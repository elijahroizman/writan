use office_core::formats::common::rel_attr;
use office_core::model::{CellRef, Range, Sheet};
use office_core::opc::Package;
use office_core::xml::{ns, Element};

use crate::formula::{RefPart, Reference, TableReference, TableRows};
use crate::{MAX_COLS, MAX_ROWS};

pub(crate) fn parts(pkg: &Package, sheet: &Sheet) -> Vec<(String, String)> {
    let Some((source, list)) = sheet.part.as_deref().zip(sheet.origin.get().and_then(|s| s.child(ns::X, "tableParts"))) else {
        return Vec::new();
    };
    let Ok(rels) = pkg.rels(source) else { return Vec::new() };
    list.children_named(ns::X, "tablePart")
        .filter_map(|entry| {
            let id = rel_attr(entry, "id")?;
            let rel = rels.get(id).filter(|r| r.is("table"))?;
            Some((id.to_string(), pkg.target(source, rel)?))
        })
        .collect()
}

pub(crate) fn range(el: &Element) -> Option<Range> {
    Range::parse(el.attr("ref")?).filter(|r| r.end.row < MAX_ROWS && r.end.col < MAX_COLS)
}

pub(crate) struct Table<'a> {
    sheet: &'a str,
    index: usize,
    name: String,
    display_name: String,
    range: Range,
    headers: u32,
    totals: u32,
    columns: Vec<String>,
}

impl<'a> Table<'a> {
    pub(crate) fn read(pkg: &Package, sheets: &'a [Sheet]) -> Vec<Self> {
        sheets
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                parts(pkg, s).into_iter().filter_map(move |(_, part)| {
                    let el = pkg.xml(&part).ok()?.root;
                    if !el.is(ns::X, "table") {
                        return None;
                    }
                    let range = range(&el)?;
                    let headers = el.attr("headerRowCount").and_then(|n| n.parse::<u32>().ok()).unwrap_or(1).min(range.rows());
                    let totals = el.attr("totalsRowCount").and_then(|n| n.parse::<u32>().ok()).unwrap_or(0).min(range.rows() - headers);
                    let display_name = el.attr("displayName")?.to_string();
                    Some(Self {
                        sheet: &s.name,
                        index: i,
                        name: el.attr("name").unwrap_or(&display_name).to_string(),
                        display_name,
                        range,
                        headers,
                        totals,
                        columns: el
                            .child(ns::X, "tableColumns")?
                            .children_named(ns::X, "tableColumn")
                            .map(|c| c.attr("name").unwrap_or("").to_string())
                            .collect(),
                    })
                })
            })
            .collect()
    }

    pub(crate) fn find<'b>(tables: &'b [Self], name: Option<&str>, here: Option<(usize, CellRef)>) -> Option<&'b Self> {
        let name = name.map(str::to_uppercase);
        tables.iter().find(|t| match &name {
            Some(n) => t.display_name.to_uppercase() == *n || t.name.to_uppercase() == *n,
            None => here.is_some_and(|(index, at)| index == t.index && t.range.contains(at)),
        })
    }

    pub(crate) fn resolve(&self, r: &TableReference, here: Option<(usize, CellRef)>) -> Result<Vec<Reference>, &'static str> {
        let column = |name: &str| {
            let name = name.to_uppercase();
            self.columns
                .iter()
                .position(|c| c.to_uppercase() == name)
                .and_then(|c| u32::try_from(c).ok())
                .filter(|&c| c < self.range.cols())
                .map(|c| self.range.start.col + c)
                .ok_or("#REF!")
        };
        let first = r.first_column.as_deref().map(column).transpose()?;
        let last = r.last_column.as_deref().map(column).transpose()?;
        let (left, right) = match (first, last) {
            (Some(a), Some(b)) => (a.min(b), a.max(b)),
            (Some(c), None) => (c, c),
            _ => (self.range.start.col, self.range.end.col),
        };
        let top = self.range.start.row;
        let bottom = self.range.end.row + 1;
        let data = top + self.headers..bottom - self.totals;
        let sections = if r.rows.is_empty() { &[TableRows::Data] } else { r.rows.as_slice() };
        let mut spans = Vec::new();
        for section in sections {
            let span = match section {
                TableRows::All => top..bottom,
                TableRows::Headers => top..data.start,
                TableRows::Data => data.clone(),
                TableRows::Totals => data.end..bottom,
                TableRows::ThisRow => {
                    let (_, at) = here.filter(|(index, at)| *index == self.index && data.contains(&at.row)).ok_or("#VALUE!")?;
                    at.row..at.row + 1
                }
            };
            if !span.is_empty() {
                spans.push(span);
            }
        }
        spans.sort_unstable_by_key(|r| r.start);
        let mut joined: Vec<std::ops::Range<u32>> = Vec::new();
        for span in spans {
            if let Some(last) = joined.last_mut().filter(|last| last.end >= span.start) {
                last.end = last.end.max(span.end);
            } else {
                joined.push(span);
            }
        }
        let part = |row, col| RefPart { row: Some(row), col: Some(col), row_abs: true, col_abs: true };
        Ok(joined
            .into_iter()
            .map(|span| Reference {
                sheet: (!here.is_some_and(|(index, _)| index == self.index)).then(|| self.sheet.to_string()),
                start: part(span.start, left),
                end: Some(part(span.end - 1, right)),
            })
            .collect())
    }
}
