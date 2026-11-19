//! Editing workbooks: typing into cells, formatting them, columns and rows, merged cells,
//! sheets, the clipboard, find and replace, and undo. Formulas are worked out again after each
//! change.

use std::collections::HashMap;

use office_core::drawing::{Color, Rgb};
use office_core::model::{CellRef, Origin, Range, Sheet, SheetCell, SheetRow, Value};
use office_core::xml::ns;
use serde::{Deserialize, Serialize};

use super::{CStep, SheetSel, SheetsView};
use crate::formula::{RefPart, Reference};
use crate::{format_of, set_format};

/// A change to cells' formatting, from the inspector.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatPatch {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub font: Option<String>,
    /// Points.
    pub size: Option<f64>,
    /// `RRGGBB`, or empty for automatic.
    pub color: Option<String>,
    /// `RRGGBB`, or empty for none.
    pub fill: Option<String>,
    /// `left`, `center`, `right`, or empty for general.
    pub align: Option<String>,
    /// `top`, `center`, `bottom`.
    pub valign: Option<String>,
    pub wrap: Option<bool>,
    pub number_format: Option<String>,
    /// `all`, `outside`, `bottom`, `none`.
    pub borders: Option<String>,
}

/// The spreadsheet-only clipboard format, alongside the plain-text values for other apps.
#[derive(Deserialize, Serialize)]
pub struct ClipboardRange {
    from: (u32, u32),
    cut: bool,
    rows: Vec<Vec<ClipboardCell>>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ClipboardCell {
    Text(String),
    Formula { formula: String },
}

impl SheetsView {
    // ---- selection ----

    pub fn select(&mut self, sheet: usize, anchor: (u32, u32), focus: (u32, u32)) {
        if sheet != self.sel.sheet {
            self.history.seal();
        }
        self.sel = SheetSel { sheet: sheet.min(self.sheets().len().saturating_sub(1)), anchor, focus };
    }

    // ---- recording changes ----

    pub(super) fn record_step(&mut self, step: CStep, before: SheetSel) {
        self.record(vec![step], before);
    }

    fn record(&mut self, steps: Vec<CStep>, before: SheetSel) {
        let after = self.sel;
        self.history.record(steps, before, after, None);
        crate::recalculate(&mut self.doc, self.now);
    }

    /// Changes cells through `f`, recording each cell's before and after.
    fn change_cells(&mut self, sheet: usize, cells: &[CellRef], f: impl Fn(&mut SheetsView, CellRef)) {
        let before = self.sel;
        let mut steps = Vec::with_capacity(cells.len());
        for &at in cells {
            let old = self.sheets().get(sheet).and_then(|s| s.cell(at)).cloned();
            f(self, at);
            let new = self.sheets().get(sheet).and_then(|s| s.cell(at)).cloned();
            if old != new || old.as_ref().map(|c| c.style) != new.as_ref().map(|c| c.style) {
                steps.push((at, old, new));
            }
        }
        if steps.is_empty() {
            return;
        }
        self.changed(Some(sheet));
        self.record(vec![CStep::Cells { sheet, cells: steps }], before);
    }

    /// Changes a whole sheet through `f` (rows and columns, widths, merges), as one step.
    fn change_sheet(&mut self, sheet: usize, f: impl FnOnce(&mut SheetsView)) {
        let Some(before) = self.sheets().get(sheet).cloned() else { return };
        let sel = self.sel;
        f(self);
        let Some(after) = self.sheets().get(sheet).cloned() else { return };
        self.changed(Some(sheet));
        self.record(vec![CStep::Sheet { sheet, before: Box::new(before), after: Box::new(after) }], sel);
    }

    /// `change_sheet` for inserting or deleting rows or columns, or `change_book` when another
    /// sheet has charts whose references may move with it.
    fn change_for_shift(&mut self, sheet: usize, f: impl FnOnce(&mut SheetsView)) {
        if self.sheets().iter().enumerate().any(|(i, s)| i != sheet && !s.charts.is_empty()) {
            self.change_book(f);
        } else {
            self.change_sheet(sheet, f);
        }
    }

    fn change_book(&mut self, f: impl FnOnce(&mut SheetsView)) {
        let before = self.sheets().to_vec();
        let sel = self.sel;
        f(self);
        let after = self.sheets().to_vec();
        self.changed(None);
        self.sel.sheet = self.sel.sheet.min(after.len().saturating_sub(1));
        self.record(vec![CStep::Book { before, after }], sel);
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.history.undo() else { return false };
        for s in e.steps.iter().rev() {
            self.apply(s, true);
        }
        self.sel = e.before;
        crate::recalculate(&mut self.doc, self.now);
        self.changed(None);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.history.redo() else { return false };
        for s in &e.steps {
            self.apply(s, false);
        }
        self.sel = e.after;
        crate::recalculate(&mut self.doc, self.now);
        self.changed(None);
        true
    }

    fn apply(&mut self, step: &CStep, undo: bool) {
        match step {
            CStep::Cells { sheet, cells } => {
                for (at, old, new) in cells {
                    let target = if undo { old } else { new };
                    if let Some(s) = self.sheets_mut().get_mut(*sheet) {
                        put_cell(s, *at, target.clone());
                    }
                }
            }
            CStep::Sheet { sheet, before, after } => {
                if let Some(s) = self.sheets_mut().get_mut(*sheet) {
                    *s = if undo { (**before).clone() } else { (**after).clone() };
                }
            }
            CStep::Book { before, after } => *self.sheets_mut() = if undo { before.clone() } else { after.clone() },
            CStep::Charts { sheet, before, after } => {
                if let Some(s) = self.sheets_mut().get_mut(*sheet) {
                    s.charts = if undo { before.clone() } else { after.clone() };
                }
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    // ---- typing into cells ----

    /// Puts what was typed into a cell: a formula (`=…`), a number (with `%`, a currency sign,
    /// thousands separators), a date, TRUE or FALSE, or text. Empty clears the value.
    pub fn set_input(&mut self, sheet: usize, at: CellRef, input: &str) {
        let input = input.to_string();
        self.change_cells(sheet, &[at], |v, at| v.put_input(sheet, at, &input));
    }

    fn put_input(&mut self, sheet: usize, at: CellRef, input: &str) {
        let trimmed = input.trim();
        let system = crate::excel(&self.doc).map(|x| x.date_system).unwrap_or_default();
        let mut format: Option<&str> = None;
        let (value, formula) = if let Some(f) = trimmed.strip_prefix('=').filter(|f| !f.is_empty()) {
            (Value::Empty, Some(crate::formula::close_brackets(f)))
        } else if trimmed.is_empty() {
            (Value::Empty, None)
        } else if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
            (Value::Bool(trimmed.eq_ignore_ascii_case("true")), None)
        } else if let Some(n) = parse_number(trimmed) {
            if trimmed.ends_with('%') {
                format = Some("0%");
            } else if let Some(sym) = trimmed.trim_start_matches(['-', '(']).chars().next().filter(|c| matches!(c, '$' | '€' | '£')) {
                format = Some(match sym {
                    '€' => "\"€\"#,##0.00",
                    '£' => "\"£\"#,##0.00",
                    _ => "\"$\"#,##0.00",
                });
            }
            (Value::Number(n), None)
        } else if let Some((date, fmt)) = parse_date(trimmed) {
            format = Some(fmt);
            (Value::Number(crate::dates::to_serial(date, system)), None)
        } else {
            (Value::Text(input.to_string()), None)
        };
        let Some(s) = self.sheets_mut().get_mut(sheet) else { return };
        let cell = s.cell_mut(at);
        cell.value = value;
        cell.formula = formula;
        // The cell's markup said what its value was; that no longer holds.
        cell.origin = Default::default();
        if let Some(code) = format {
            let mut f = format_of(&self.doc, sheet, at);
            if f.number_format == "General" {
                f.number_format = code.to_string();
                set_format(&mut self.doc, sheet, at, &f);
            }
        }
    }

    /// Clears the values (and formulas) in a range, keeping formatting.
    pub fn clear(&mut self, sheet: usize, range: Range) {
        let cells = existing_cells(self.sheets().get(sheet), range);
        self.change_cells(sheet, &cells, |v, at| {
            if let Some(s) = v.sheets_mut().get_mut(sheet) {
                let c = s.cell_mut(at);
                c.value = Value::Empty;
                c.formula = None;
                c.origin = Default::default();
            }
        });
    }

    // ---- formatting ----

    /// Formats every cell in a range (empty ones too, as Excel does).
    pub fn format(&mut self, sheet: usize, range: Range, patch: &FormatPatch) {
        let cells = all_cells(range, 20_000);
        let p = patch.clone();
        let (r0, r1, c0, c1) = (range.start.row, range.end.row, range.start.col, range.end.col);
        self.change_cells(sheet, &cells, |v, at| {
            let mut f = format_of(&v.doc, sheet, at);
            if let Some(b) = p.bold {
                f.font.bold = b;
            }
            if let Some(i) = p.italic {
                f.font.italic = i;
            }
            if let Some(u) = p.underline {
                f.font.underline = u.then(|| "single".into());
            }
            if let Some(s) = p.strike {
                f.font.strike = s;
            }
            if let Some(n) = &p.font {
                f.font.name = Some(n.clone());
            }
            if let Some(s) = p.size {
                f.font.size = Some(s.clamp(1.0, 409.0));
            }
            if let Some(c) = &p.color {
                f.font.color = Rgb::parse(c).map(Color::rgb);
            }
            if let Some(c) = &p.fill {
                match Rgb::parse(c) {
                    Some(rgb) => {
                        f.fill.pattern = "solid".into();
                        f.fill.fg = Some(Color::rgb(rgb));
                    }
                    None => {
                        f.fill.pattern = "none".into();
                        f.fill.fg = None;
                    }
                }
            }
            if let Some(a) = &p.align {
                f.alignment.horizontal = (!a.is_empty()).then(|| a.clone());
            }
            if let Some(a) = &p.valign {
                f.alignment.vertical = (!a.is_empty() && a != "bottom").then(|| a.clone());
            }
            if let Some(w) = p.wrap {
                f.alignment.wrap = w;
            }
            if let Some(code) = &p.number_format {
                f.number_format = code.clone();
            }
            if let Some(b) = &p.borders {
                let thin = || crate::BorderSide { style: Some("thin".into()), color: None };
                let none = crate::BorderSide::default;
                match b.as_str() {
                    "all" => {
                        f.border.top = thin();
                        f.border.bottom = thin();
                        f.border.left = thin();
                        f.border.right = thin();
                    }
                    "outside" => {
                        if at.row == r0 {
                            f.border.top = thin();
                        }
                        if at.row == r1 {
                            f.border.bottom = thin();
                        }
                        if at.col == c0 {
                            f.border.left = thin();
                        }
                        if at.col == c1 {
                            f.border.right = thin();
                        }
                    }
                    "bottom" => {
                        if at.row == r1 {
                            f.border.bottom = thin();
                        }
                    }
                    _ => {
                        f.border.top = none();
                        f.border.bottom = none();
                        f.border.left = none();
                        f.border.right = none();
                    }
                }
            }
            set_format(&mut v.doc, sheet, at, &f);
        });
    }

    // ---- columns and rows ----

    /// Sets a column's width in pixels (`None`: back to the default).
    pub fn set_column_width(&mut self, sheet: usize, col: u32, px: Option<f64>) {
        self.change_sheet(sheet, |v| crate::set_column_width(&mut v.doc, sheet, col, px.map(|p| ((p - 5.0) / 7.0).max(0.0))));
    }

    /// Sets a row's height in pixels (`None`: back to the default).
    pub fn set_row_height(&mut self, sheet: usize, row: u32, px: Option<f64>) {
        self.change_sheet(sheet, |v| crate::set_row_height(&mut v.doc, sheet, row, px.map(|p| p * 72.0 / 96.0)));
    }

    /// Inserts `n` rows above row `at` (or deletes `n` rows from it, for a negative `n`).
    pub fn shift_rows(&mut self, sheet: usize, at: u32, n: i64) {
        self.change_for_shift(sheet, |v| {
            let Some(s) = v.sheets_mut().get_mut(sheet) else { return };
            // Shared followers keep their formula only on the group's origin. Copy every formula in a
            // group that this shift touches before those origins are cleared, and move references the
            // way inserting or deleting rows does.
            prepare_shift(s, Axis::Row, at, n);
            let name = s.name.clone();
            super::charts::shift_charts(s, true, at, n, |f| adjust_formula(f, &name, Axis::Row, at, n));
            super::charts::shift_other_chart_refs(v.sheets_mut(), sheet, |f| adjust_formula(f, &name, Axis::Row, at, n));
            let Some(s) = v.sheets_mut().get_mut(sheet) else { return };
            if n < 0 {
                let end = at as i64 - n;
                s.rows.retain(|r| (r.index as i64) < at as i64 || r.index as i64 >= end);
            }
            for r in &mut s.rows {
                if r.index >= at && (n > 0 || r.index as i64 >= at as i64 - n) {
                    r.index = (r.index as i64 + n).max(0) as u32;
                    r.origin = clear_attr(&r.origin, "r");
                    for c in &mut r.cells {
                        c.origin = Default::default();
                    }
                }
            }
            s.merges.retain(|m| n > 0 || !(m.start.row as i64 >= at as i64 && (m.end.row as i64) < at as i64 - n));
            for m in &mut s.merges {
                for p in [&mut m.start, &mut m.end] {
                    if p.row >= at {
                        p.row = (p.row as i64 + n).max(at as i64) as u32;
                    }
                }
            }
            // A size someone set grows and shrinks with the rows.
            if let Some((r, c)) = s.size {
                s.size = Some(((r as i64 + n).clamp(1, crate::MAX_ROWS as i64) as u32, c));
            }
        });
    }

    /// Inserts `n` columns left of column `at` (or deletes `n` from it, for a negative `n`).
    pub fn shift_cols(&mut self, sheet: usize, at: u32, n: i64) {
        self.change_for_shift(sheet, |v| {
            let Some(s) = v.sheets_mut().get_mut(sheet) else { return };
            prepare_shift(s, Axis::Col, at, n);
            let name = s.name.clone();
            super::charts::shift_charts(s, false, at, n, |f| adjust_formula(f, &name, Axis::Col, at, n));
            super::charts::shift_other_chart_refs(v.sheets_mut(), sheet, |f| adjust_formula(f, &name, Axis::Col, at, n));
            let Some(s) = v.sheets_mut().get_mut(sheet) else { return };
            for r in &mut s.rows {
                if n < 0 {
                    let end = at as i64 - n;
                    r.cells.retain(|c| (c.col as i64) < at as i64 || c.col as i64 >= end);
                }
                for c in &mut r.cells {
                    if c.col >= at {
                        c.col = (c.col as i64 + n).max(0) as u32;
                        c.origin = Default::default();
                    }
                }
                r.origin = clear_attr(&r.origin, "spans");
            }
            s.merges.retain(|m| n > 0 || !(m.start.col as i64 >= at as i64 && (m.end.col as i64) < at as i64 - n));
            for m in &mut s.merges {
                for p in [&mut m.start, &mut m.end] {
                    if p.col >= at {
                        p.col = (p.col as i64 + n).max(at as i64) as u32;
                    }
                }
            }
            if let Some((r, c)) = s.size {
                s.size = Some((r, (c as i64 + n).clamp(1, crate::MAX_COLS as i64) as u32));
            }
        });
    }

    /// Sets how many rows and columns a sheet shows. It never shows fewer than it has in use.
    pub fn set_sheet_size(&mut self, sheet: usize, rows: u32, cols: u32) {
        let size = (rows.clamp(1, crate::MAX_ROWS), cols.clamp(1, crate::MAX_COLS));
        if self.sheets().get(sheet).is_none_or(|s| s.size == Some(size)) {
            return;
        }
        self.change_sheet(sheet, |v| {
            if let Some(s) = v.sheets_mut().get_mut(sheet) {
                s.size = Some(size);
            }
        });
    }

    /// Merges a range (keeping the top-left value), or unmerges any merge it touches.
    pub fn toggle_merge(&mut self, sheet: usize, range: Range) {
        self.change_sheet(sheet, |v| {
            let Some(s) = v.sheets_mut().get_mut(sheet) else { return };
            let touching: Vec<usize> = s.merges.iter().enumerate().filter(|(_, m)| overlaps(m, &range)).map(|(i, _)| i).collect();
            if !touching.is_empty() {
                for i in touching.into_iter().rev() {
                    s.merges.remove(i);
                }
                return;
            }
            if range.start == range.end {
                return;
            }
            for r in &mut s.rows {
                if r.index < range.start.row || r.index > range.end.row {
                    continue;
                }
                for c in &mut r.cells {
                    let at = CellRef::new(r.index, c.col);
                    if range.contains(at) && at != range.start {
                        c.value = Value::Empty;
                        c.formula = None;
                    }
                }
            }
            s.merges.push(range);
        });
    }

    // ---- sheets ----

    pub fn add_sheet(&mut self) {
        let n = self.sheets().len();
        let name =
            (n + 1..).map(|k| format!("Sheet{k}")).find(|name| !self.sheets().iter().any(|s| s.name.eq_ignore_ascii_case(name))).unwrap();
        self.change_book(|v| v.sheets_mut().push(Sheet::new(name)));
        self.sel = SheetSel { sheet: n, ..Default::default() };
    }

    pub fn rename_sheet(&mut self, sheet: usize, name: &str) {
        let name: String = name.chars().filter(|c| !matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']')).take(31).collect();
        let name = name.trim().trim_matches('\'').to_string();
        if name.is_empty() || self.sheets().iter().enumerate().any(|(i, s)| i != sheet && s.name.eq_ignore_ascii_case(&name)) {
            return;
        }
        let Some(old) = self.sheets().get(sheet).map(|s| s.name.clone()) else { return };
        self.change_book(|v| {
            super::charts::rename_charts(v.sheets_mut(), &old, &name);
            if let Some(s) = v.sheets_mut().get_mut(sheet) {
                s.name = name;
            }
        });
    }

    /// Deletes a sheet; a workbook keeps at least one.
    pub fn delete_sheet(&mut self, sheet: usize) {
        if self.sheets().len() <= 1 {
            return;
        }
        self.change_book(|v| {
            v.sheets_mut().remove(sheet);
        });
        self.sel = SheetSel { sheet: sheet.saturating_sub(1), ..Default::default() };
    }

    pub fn move_sheet(&mut self, from: usize, to: usize) {
        let n = self.sheets().len();
        if from >= n || to >= n || from == to {
            return;
        }
        self.change_book(|v| {
            let s = v.sheets_mut().remove(from);
            v.sheets_mut().insert(to, s);
        });
        self.sel.sheet = to;
    }

    // ---- clipboard ----

    /// A range as tab-separated displayed values, plus spreadsheet data when it contains formulas.
    pub fn copy(&self, sheet: usize, range: Range, cut: bool) -> (String, Option<ClipboardRange>) {
        let mut text = String::new();
        let mut rows = Vec::new();
        let mut has_formula = false;
        for r in range.start.row..=range.end.row.min(range.start.row + 100_000) {
            let mut row = Vec::new();
            for c in range.start.col..=range.end.col {
                let at = CellRef::new(r, c);
                let display = crate::display(&self.doc, sheet, at);
                if c > range.start.col {
                    text.push('\t');
                }
                text.push_str(&display);
                row.push(match crate::formula(&self.doc, sheet, at) {
                    Some(formula) => {
                        has_formula = true;
                        ClipboardCell::Formula { formula }
                    }
                    None => ClipboardCell::Text(display),
                });
            }
            rows.push(row);
            text.push('\n');
        }
        (text, has_formula.then_some(ClipboardRange { from: (range.start.row, range.start.col), cut, rows }))
    }

    /// Pastes a range at `at`; a single cell fills the selection.
    pub fn paste(&mut self, sheet: usize, at: Range, text: &str, clipboard: Option<ClipboardRange>) {
        let (rows, copy_from) = if let Some(clipboard) = clipboard {
            (clipboard.rows, (!clipboard.cut).then_some(clipboard.from))
        } else {
            let rows = text
                .trim_end_matches(['\n', '\r'])
                .split('\n')
                .map(|l| l.trim_end_matches('\r').split('\t').map(|s| ClipboardCell::Text(s.to_string())).collect())
                .collect();
            (rows, None)
        };
        let single = rows.len() == 1 && rows[0].len() == 1;
        let cells = if single {
            all_cells(at, 20_000)
        } else {
            rows.iter()
                .enumerate()
                .flat_map(|(r, row)| (0..row.len()).map(move |c| CellRef::new(at.start.row + r as u32, at.start.col + c as u32)))
                .collect()
        };
        self.change_cells(sheet, &cells, |v, dest| {
            let (r, c) = if single { (0, 0) } else { (dest.row - at.start.row, dest.col - at.start.col) };
            match &rows[r as usize][c as usize] {
                ClipboardCell::Text(input) => v.put_input(sheet, dest, input),
                ClipboardCell::Formula { formula } => {
                    let Some(s) = v.sheets_mut().get_mut(sheet) else { return };
                    let cell = s.cell_mut(dest);
                    let anchor = if single { dest } else { at.start };
                    cell.value = Value::Empty;
                    cell.formula = Some(
                        copy_from
                            .and_then(|(row, col)| {
                                crate::formula::shift(
                                    formula,
                                    i64::from(anchor.row) - i64::from(row),
                                    i64::from(anchor.col) - i64::from(col),
                                )
                                .ok()
                            })
                            .unwrap_or_else(|| formula.clone()),
                    );
                    cell.origin = Default::default();
                }
            }
        });
    }

    /// Copies the top row of a range down through it (⌘D), formulas shifted as Excel does.
    pub fn fill_down(&mut self, sheet: usize, range: Range) {
        if range.end.row <= range.start.row {
            return;
        }
        let cells = all_cells(Range { start: CellRef::new(range.start.row + 1, range.start.col), end: range.end }, 20_000);
        let src_row = range.start.row;
        self.change_cells(sheet, &cells, |v, at| {
            let src = CellRef::new(src_row, at.col);
            let input = match crate::formula(&v.doc, sheet, src) {
                Some(f) => crate::formula::shift(&f, at.row as i64 - src_row as i64, 0).map(|f| format!("={f}")).unwrap_or_default(),
                None => v.cell_info(sheet, src).input,
            };
            v.put_input(sheet, at, &input);
            let f = format_of(&v.doc, sheet, src);
            set_format(&mut v.doc, sheet, at, &f);
        });
    }

    // ---- find and replace ----

    /// Cells showing `query` (any case), in reading order, on every sheet: (sheet, cell).
    pub fn find(&self, query: &str) -> Vec<(usize, CellRef)> {
        let q = query.to_lowercase();
        let mut out = Vec::new();
        if q.is_empty() {
            return out;
        }
        for (si, s) in self.sheets().iter().enumerate() {
            for r in &s.rows {
                for c in &r.cells {
                    let at = CellRef::new(r.index, c.col);
                    if crate::display(&self.doc, si, at).to_lowercase().contains(&q) {
                        out.push((si, at));
                    }
                }
            }
        }
        out
    }

    /// Replaces `query` (any case) with `by` in the given cells' values (not formulas).
    pub fn replace(&mut self, matches: &[(usize, CellRef)], query: &str, by: &str) {
        let mut by_sheet: std::collections::BTreeMap<usize, Vec<CellRef>> = Default::default();
        for (s, c) in matches {
            by_sheet.entry(*s).or_default().push(*c);
        }
        self.history.begin_group(self.sel);
        for (sheet, cells) in by_sheet {
            let (q, b) = (query.to_string(), by.to_string());
            self.change_cells(sheet, &cells, |v, at| {
                if crate::formula(&v.doc, sheet, at).is_some() {
                    return;
                }
                let text = v.cell_info(sheet, at).input;
                let next = replace_ci(&text, &q, &b);
                if next != text {
                    v.put_input(sheet, at, &next);
                }
            });
        }
        self.history.end_group();
    }
}

/// Puts a cell back as it was (or removes it).
fn put_cell(s: &mut Sheet, at: CellRef, cell: Option<SheetCell>) {
    match cell {
        Some(c) => *s.cell_mut(at) = c,
        None => {
            if let Ok(i) = s.rows.binary_search_by_key(&at.row, |r| r.index) {
                s.rows[i].cells.retain(|c| c.col != at.col);
            }
        }
    }
}

fn existing_cells(s: Option<&Sheet>, range: Range) -> Vec<CellRef> {
    let Some(s) = s else { return Vec::new() };
    s.rows
        .iter()
        .filter(|r| r.index >= range.start.row && r.index <= range.end.row)
        .flat_map(|r: &SheetRow| {
            r.cells.iter().filter(|c| c.col >= range.start.col && c.col <= range.end.col).map(move |c| CellRef::new(r.index, c.col))
        })
        .collect()
}

/// Every cell in a range, up to `max` of them.
fn all_cells(range: Range, max: usize) -> Vec<CellRef> {
    let mut out = Vec::new();
    for r in range.start.row..=range.end.row {
        for c in range.start.col..=range.end.col {
            out.push(CellRef::new(r, c));
            if out.len() >= max {
                return out;
            }
        }
    }
    out
}

fn overlaps(a: &Range, b: &Range) -> bool {
    a.start.row <= b.end.row && b.start.row <= a.end.row && a.start.col <= b.end.col && b.start.col <= a.end.col
}

fn clear_attr(o: &office_core::model::Origin, k: &str) -> office_core::model::Origin {
    match o.get() {
        Some(el) => {
            let mut el = el.clone();
            el.remove_attr("", k);
            office_core::model::Origin::new(el)
        }
        None => Default::default(),
    }
}

/// A number as typed: `1,234.5`, `-3`, `12%`, `$9.99`, `(4)`.
fn parse_number(s: &str) -> Option<f64> {
    let mut t = s.trim().to_string();
    let mut sign = 1.0;
    if t.starts_with('(') && t.ends_with(')') {
        sign = -1.0;
        t = t[1..t.len() - 1].to_string();
    }
    if let Some(rest) = t.strip_prefix('-') {
        sign = -sign;
        t = rest.to_string();
    }
    let t = t.trim_start_matches(['$', '€', '£']).to_string();
    let (t, pct) = match t.strip_suffix('%') {
        Some(x) => (x.to_string(), true),
        None => (t, false),
    };
    if t.is_empty()
        || !t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | 'e' | 'E' | '+' | '-'))
        || !t.chars().next()?.is_ascii_digit() && !t.starts_with('.')
    {
        return None;
    }
    // Thousands separators only between groups of three.
    if t.contains(',') {
        let int = t.split('.').next()?;
        let groups: Vec<&str> = int.split(',').collect();
        if groups.iter().skip(1).any(|g| g.len() != 3) || groups[0].is_empty() || groups[0].len() > 3 {
            return None;
        }
    }
    let n: f64 = t.replace(',', "").parse().ok()?;
    Some(sign * if pct { n / 100.0 } else { n })
}

/// A date as typed: `2026-01-31` or `1/31/2026`, with the format to show it in.
fn parse_date(s: &str) -> Option<(crate::DateTime, &'static str)> {
    let parts: Vec<&str> = s.split(['-', '/']).collect();
    if parts.len() != 3 || !parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let n: Vec<i32> = parts.iter().map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let (y, m, d, fmt) = if s.contains('-') && parts[0].len() == 4 {
        (n[0], n[1], n[2], "yyyy-mm-dd")
    } else if s.contains('/') && parts[2].len() == 4 {
        (n[2], n[0], n[1], "m/d/yyyy")
    } else {
        return None;
    };
    if !(1900..=9999).contains(&y) || !(1..=12).contains(&m) {
        return None;
    }
    // 1900 is not a leap year. A day the month doesn't have stays text, as Excel keeps it.
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=days[m as usize - 1]).contains(&d) {
        return None;
    }
    Some((crate::DateTime::date(y, m as u32, d as u32), fmt))
}

#[derive(Clone, Copy)]
enum Axis {
    Row,
    Col,
}

struct SharedBuild {
    master: Option<(u32, u32, String)>,
    members: Vec<(u32, u32)>,
}

/// The row or column this shift moves.
fn axis_index(axis: Axis, row: u32, col: u32) -> u32 {
    match axis {
        Axis::Row => row,
        Axis::Col => col,
    }
}

/// A shared formula's group id, and whether this cell is the one that carries the text (`ref`).
fn shared_mark(origin: &Origin) -> Option<(String, bool)> {
    let f = origin.get()?.child(ns::X, "f")?;
    if f.attr("t") != Some("shared") {
        return None;
    }
    Some((f.attr("si").unwrap_or("").to_string(), f.attr("ref").is_some()))
}

/// Drops the stored `<f>` so a materialized formula is written as an ordinary formula.
fn drop_formula_markup(origin: &mut Origin) {
    let Some(el) = origin.get() else { return };
    if el.child(ns::X, "f").is_none() {
        return;
    }
    let mut el = el.clone();
    el.remove_children(ns::X, "f");
    *origin = Origin::new(el);
}

/// Copies shared formulas onto every cell of a group this shift will break, then moves every
/// formula's references. Followers whose leader is deleted keep the formula they had.
fn prepare_shift(sheet: &mut Sheet, axis: Axis, at: u32, n: i64) {
    let name = sheet.name.clone();
    let mut order = Vec::new();
    let mut groups: HashMap<String, SharedBuild> = HashMap::new();
    for row in &sheet.rows {
        for c in &row.cells {
            let Some((si, is_master)) = shared_mark(&c.origin) else { continue };
            if !groups.contains_key(&si) {
                order.push(si.clone());
            }
            let g = groups.entry(si).or_insert_with(|| SharedBuild { master: None, members: Vec::new() });
            g.members.push((row.index, c.col));
            if is_master {
                if let Some(text) = &c.formula {
                    g.master = Some((row.index, c.col, text.clone()));
                }
            }
        }
    }
    let mut strip = Vec::new();
    for si in order {
        let Some(g) = groups.remove(&si) else { continue };
        let Some((mr, mc, text)) = g.master else { continue };
        // A group breaks when any cell moves or is deleted (its origin is about to go), or when
        // inserting rows inside a copied range would make one cell's formula no longer the
        // leader's formula shifted by the same offset.
        let structural = g.members.iter().any(|&(r, c)| axis_index(axis, r, c) >= at);
        let refs_move = g.members.iter().any(|&(r, c)| {
            crate::formula::shift(&text, r as i64 - mr as i64, c as i64 - mc as i64)
                .is_ok_and(|f| adjust_formula(&f, &name, axis, at, n) != f)
        });
        if !structural && !refs_move {
            continue;
        }
        for (r, c) in g.members {
            let base = crate::formula::shift(&text, r as i64 - mr as i64, c as i64 - mc as i64).unwrap_or_else(|_| text.clone());
            sheet.cell_mut(CellRef::new(r, c)).formula = Some(base);
            strip.push((r, c));
        }
    }
    for row in &mut sheet.rows {
        for c in &mut row.cells {
            let Some(current) = c.formula.clone() else { continue };
            let next = adjust_formula(&current, &name, axis, at, n);
            if next != current {
                c.formula = Some(next);
            }
        }
    }
    for (r, c) in strip {
        drop_formula_markup(&mut sheet.cell_mut(CellRef::new(r, c)).origin);
    }
}

const MAX_ROW: u32 = 1_048_575;
const MAX_COL: u32 = 16_383;

fn map_point(i: u32, at: u32, n: i64, max: u32) -> Option<u32> {
    let i = i as i64;
    let at = at as i64;
    let v = if n > 0 {
        if i >= at {
            i.checked_add(n)?
        } else {
            i
        }
    } else if n < 0 {
        let count = n.checked_neg()?;
        if i < at {
            i
        } else if i >= at.checked_add(count)? {
            i.checked_sub(count)?
        } else {
            return None;
        }
    } else {
        i
    };
    u32::try_from(v).ok().filter(|v| *v <= max)
}

/// How a range's ends move. An insertion inside the range expands it; a deletion that covers
/// only part of it shrinks it; a range that disappears entirely is `None`.
fn map_span(a: u32, b: u32, at: u32, n: i64, max: u32) -> Option<(u32, u32)> {
    let (lo, hi, flipped) = if a <= b { (a, b, false) } else { (b, a, true) };
    let (nlo, nhi) = if n > 0 {
        let add = u32::try_from(n).ok()?;
        if lo >= at {
            (lo.checked_add(add)?, hi.checked_add(add)?)
        } else if hi >= at {
            (lo, hi.checked_add(add)?)
        } else {
            (lo, hi)
        }
    } else if n < 0 {
        let count = u32::try_from(-n).ok()?;
        let end = at.checked_add(count)?;
        let nlo = if lo < at {
            lo
        } else if lo >= end {
            lo - count
        } else if end <= hi {
            at
        } else {
            return None;
        };
        let nhi = if hi < at {
            hi
        } else if hi >= end {
            hi - count
        } else if lo < at {
            at.checked_sub(1)?
        } else {
            return None;
        };
        if nlo > nhi {
            return None;
        }
        (nlo, nhi)
    } else {
        (lo, hi)
    };
    if nlo > max || nhi > max {
        return None;
    }
    if flipped {
        Some((nhi, nlo))
    } else {
        Some((nlo, nhi))
    }
}

fn shift_point(p: RefPart, axis: Axis, at: u32, n: i64) -> Option<RefPart> {
    let max = match axis {
        Axis::Row => MAX_ROW,
        Axis::Col => MAX_COL,
    };
    match axis {
        Axis::Row => {
            let row = match p.row {
                Some(r) => Some(map_point(r, at, n, max)?),
                None => None,
            };
            Some(RefPart { row, ..p })
        }
        Axis::Col => {
            let col = match p.col {
                Some(c) => Some(map_point(c, at, n, max)?),
                None => None,
            };
            Some(RefPart { col, ..p })
        }
    }
}

fn shift_ends(start: RefPart, end: Option<RefPart>, axis: Axis, at: u32, n: i64) -> Option<(RefPart, Option<RefPart>)> {
    let Some(end) = end else {
        return Some((shift_point(start, axis, at, n)?, None));
    };
    let max = match axis {
        Axis::Row => MAX_ROW,
        Axis::Col => MAX_COL,
    };
    let (start, end) = match axis {
        Axis::Row => match (start.row, end.row) {
            (Some(r1), Some(r2)) => {
                let (r1, r2) = map_span(r1, r2, at, n, max)?;
                (RefPart { row: Some(r1), ..start }, RefPart { row: Some(r2), ..end })
            }
            _ => (start, end),
        },
        Axis::Col => match (start.col, end.col) {
            (Some(c1), Some(c2)) => {
                let (c1, c2) = map_span(c1, c2, at, n, max)?;
                (RefPart { col: Some(c1), ..start }, RefPart { col: Some(c2), ..end })
            }
            _ => (start, end),
        },
    };
    Some((start, Some(end)))
}

fn write_part(p: &RefPart) -> String {
    let mut s = String::new();
    if let Some(c) = p.col {
        if p.col_abs {
            s.push('$');
        }
        s.push_str(&CellRef::column_name(c));
    }
    if let Some(r) = p.row {
        if p.row_abs {
            s.push('$');
        }
        s.push_str(&(r + 1).to_string());
    }
    s
}

fn write_coords(start: &RefPart, end: Option<&RefPart>) -> String {
    match end {
        None => write_part(start),
        Some(end) => format!("{}:{}", write_part(start), write_part(end)),
    }
}

/// The sheet being shifted, and where rows or columns were inserted or deleted.
struct FormulaShift<'a> {
    sheet_name: &'a str,
    axis: Axis,
    at: u32,
    n: i64,
}

/// Moves references in `src` for a row or column insert (`n > 0`) or delete (`n < 0`) at `at`.
/// Absolute references move when the cell they name moves. A reference on another sheet stays.
/// Same rules as Excel: a range grows when rows are inserted inside it and shrinks when rows
/// are deleted from it.
fn adjust_formula(src: &str, sheet_name: &str, axis: Axis, at: u32, n: i64) -> String {
    let shift = FormulaShift { sheet_name, axis, at, n };
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        let c = src[i..].chars().next().unwrap();
        match c {
            '"' => {
                out.push('"');
                i += 1;
                while i < b.len() {
                    if b[i] == b'"' {
                        out.push('"');
                        i += 1;
                        if b.get(i) == Some(&b'"') {
                            out.push('"');
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        let ch = src[i..].chars().next().unwrap();
                        out.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            '#' => {
                const ERRORS: [&str; 8] = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"];
                if let Some(e) = ERRORS.iter().find(|e| src[i..].to_ascii_uppercase().starts_with(**e)) {
                    out.push_str(&src[i..i + e.len()]);
                    i += e.len();
                } else {
                    out.push('#');
                    i += 1;
                }
            }
            '\'' => {
                if let Some((sheet, coord)) = quoted_sheet(src, i) {
                    if let Some((r, len)) = parse_ref(src, coord, Some(sheet)) {
                        rewrite(&mut out, src, i, coord, coord + len, r, &shift);
                        i = coord + len;
                        continue;
                    }
                }
                out.push('\'');
                i += 1;
            }
            c if c == '$' || c == '_' || c == '\\' || c.is_alphabetic() => {
                let word_end = scan_word(src, i);
                if word_end > i && b.get(word_end) == Some(&b'!') {
                    let sheet = src[i..word_end].to_string();
                    if let Some((r, len)) = parse_ref(src, word_end + 1, Some(sheet)) {
                        rewrite(&mut out, src, i, word_end + 1, word_end + 1 + len, r, &shift);
                        i = word_end + 1 + len;
                        continue;
                    }
                }
                if let Some((r, len)) = parse_ref(src, i, None) {
                    rewrite(&mut out, src, i, i, i + len, r, &shift);
                    i += len;
                    continue;
                }
                let end = if word_end > i { word_end } else { i + c.len_utf8() };
                out.push_str(&src[i..end]);
                i = end;
            }
            '0'..='9' | '.' => {
                if let Some((r, len)) = parse_ref(src, i, None) {
                    rewrite(&mut out, src, i, i, i + len, r, &shift);
                    i += len;
                } else {
                    out.push(c);
                    i += c.len_utf8();
                }
            }
            _ => {
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

fn quoted_sheet(src: &str, i: usize) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let mut j = i + 1;
    let mut name = String::new();
    loop {
        match b.get(j) {
            Some(b'\'') if b.get(j + 1) == Some(&b'\'') => {
                name.push('\'');
                j += 2;
            }
            Some(b'\'') => break,
            Some(_) => {
                let ch = src[j..].chars().next().unwrap();
                name.push(ch);
                j += ch.len_utf8();
            }
            None => return None,
        }
    }
    if b.get(j + 1) != Some(&b'!') {
        return None;
    }
    Some((name, j + 2))
}

fn scan_word(src: &str, i: usize) -> usize {
    let mut j = i;
    while j < src.len() {
        let ch = src[j..].chars().next().unwrap();
        if ch.is_alphanumeric() || matches!(ch, '_' | '.' | '\\' | '$') {
            j += ch.len_utf8();
        } else {
            break;
        }
    }
    j
}

fn rewrite(out: &mut String, src: &str, start: usize, coord: usize, end: usize, reference: Reference, shift: &FormulaShift<'_>) {
    let same_sheet = reference.sheet.as_ref().map(|s| s.eq_ignore_ascii_case(shift.sheet_name)).unwrap_or(true);
    if !same_sheet {
        out.push_str(&src[start..end]);
        return;
    }
    let Some((a, b)) = shift_ends(reference.start, reference.end, shift.axis, shift.at, shift.n) else {
        out.push_str("#REF!");
        return;
    };
    let coords = write_coords(&a, b.as_ref());
    if coords == src[coord..end] {
        out.push_str(&src[start..end]);
    } else {
        out.push_str(&src[start..coord]);
        out.push_str(&coords);
    }
}

/// A reference at `at`, matching the formula parser: `A1`, `$A$1`, `A1:B2`, `A:C`, `1:3`.
fn parse_ref(src: &str, at: usize, sheet: Option<String>) -> Option<(Reference, usize)> {
    let s = src.as_bytes();
    let (first, n1) = parse_part(s, at)?;
    let mut len = n1;
    let mut end = None;
    if s.get(at + len) == Some(&b':') {
        if let Some((second, n2)) = parse_part(s, at + len + 1) {
            let kind = |p: &RefPart| (p.row.is_some(), p.col.is_some());
            if kind(&first) == kind(&second) {
                end = Some(second);
                len += 1 + n2;
            }
        }
    }
    if end.is_none() && (first.row.is_none() || first.col.is_none()) {
        return None;
    }
    let next = src[at + len..].chars().next();
    if next.is_some_and(|c| c == '(' || c.is_alphanumeric() || c == '_' || c == '.') {
        return None;
    }
    Some((Reference { sheet, start: first, end }, len))
}

fn parse_part(s: &[u8], mut i: usize) -> Option<(RefPart, usize)> {
    let start = i;
    let col_abs = s.get(i) == Some(&b'$');
    if col_abs {
        i += 1;
    }
    let cs = i;
    while i < s.len() && s[i].is_ascii_alphabetic() {
        i += 1;
    }
    let col = if i > cs && i - cs <= 3 {
        Some(s[cs..i].iter().fold(0u32, |n, c| n * 26 + (c.to_ascii_uppercase() - b'A' + 1) as u32) - 1)
    } else if i == cs {
        None
    } else {
        return None;
    };
    let row_abs = s.get(i) == Some(&b'$');
    let ri = if row_abs { i + 1 } else { i };
    let mut j = ri;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
    }
    let row = if j > ri { std::str::from_utf8(&s[ri..j]).ok()?.parse::<u32>().ok().filter(|r| *r >= 1).map(|r| r - 1) } else { None };
    if j > ri {
        i = j;
    } else if row_abs {
        return None;
    }
    if col.is_none() && row.is_none() || col.is_some_and(|c| c > 16_383) || row.is_some_and(|r| r > 1_048_575) {
        return None;
    }
    Some((RefPart { row, col, row_abs: row_abs && row.is_some(), col_abs: col_abs && col.is_some() }, i - start))
}

fn replace_ci(text: &str, query: &str, by: &str) -> String {
    if query.is_empty() {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let q = query.to_lowercase();
    if lower.len() != text.len() {
        return text.replace(query, by);
    }
    let mut out = String::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&q) {
        out.push_str(&text[from..from + at]);
        out.push_str(by);
        from += at + q.len();
    }
    out.push_str(&text[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_dates_as_typed() {
        assert_eq!(parse_number("1,234.5"), Some(1234.5));
        assert_eq!(parse_number("12%"), Some(0.12));
        assert_eq!(parse_number("$9.99"), Some(9.99));
        assert_eq!(parse_number("€5"), Some(5.0));
        assert_eq!(parse_number("£2.5"), Some(2.5));
        assert_eq!(parse_number("(€4)"), Some(-4.0));
        assert_eq!(parse_number("(4)"), Some(-4.0));
        assert_eq!(parse_number("12,34"), None);
        assert_eq!(parse_number("abc"), None);
        assert!(parse_date("2026-01-31").is_some());
        assert!(parse_date("1/31/2026").is_some());
        assert!(parse_date("2024-02-29").is_some(), "a leap day");
        assert!(parse_date("2000-02-29").is_some(), "a century divisible by 400");
        assert!(parse_date("2026-02-29").is_none(), "2026 is not a leap year");
        assert!(parse_date("2026-02-31").is_none());
        assert!(parse_date("2/31/2026").is_none());
        assert!(parse_date("4/31/2026").is_none(), "April has 30 days");
        assert!(parse_date("1900-02-29").is_none(), "1900 is not a Gregorian leap year");
        assert!(parse_date("31/31/2026").is_none());
    }

    fn adj(src: &str, axis: Axis, at: u32, n: i64) -> String {
        adjust_formula(src, "Data", axis, at, n)
    }

    #[test]
    fn inserted_and_deleted_rows_move_references() {
        assert_eq!(adj("$A$1+A5", Axis::Row, 3, 1), "$A$1+A6");
        assert_eq!(adj("SUM(A1:A10)", Axis::Row, 4, 1), "SUM(A1:A11)", "an insertion inside a range expands it");
        assert_eq!(adj("SUM(Z6:Z8)", Axis::Row, 5, 1), "SUM(Z7:Z9)", "a range at the insertion point moves with it");
        assert_eq!(adj("SUM(A1:A10)", Axis::Row, 0, -1), "SUM(A1:A9)");
        assert_eq!(adj("A1", Axis::Row, 0, -1), "#REF!");
        assert_eq!(adj("Other!A5+A5", Axis::Row, 0, 1), "Other!A5+A6");
        assert_eq!(adj("\"A1\"&A1", Axis::Row, 0, 1), "\"A1\"&A2");
        assert_eq!(adj("Data!A1", Axis::Row, 0, 1), "Data!A2");
        assert_eq!(adj("'Data'!$B$2", Axis::Row, 0, 2), "'Data'!$B$4");
        assert_eq!(adj("'Bob''s'!A1", Axis::Row, 0, 1), "'Bob''s'!A1", "another sheet, even with a quote in the name");
        assert_eq!(adj("Z:Z", Axis::Row, 0, 1), "Z:Z");
        assert_eq!(adj("2:3", Axis::Row, 1, 1), "3:4");
        assert_eq!(adj("A1+B1", Axis::Col, 1, 1), "A1+C1");
        assert_eq!(adj("SUM(B1:D1)", Axis::Col, 2, -1), "SUM(B1:C1)");
        assert_eq!(adj("SUM(A1:A3)+\"A1\"+Other!B2", Axis::Row, 10, 1), "SUM(A1:A3)+\"A1\"+Other!B2");
    }
}
