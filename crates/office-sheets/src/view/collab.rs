//! A workbook shared with other copies (see `office_engine::collab`).
//!
//! Steps go as a fingerprint of what they replaced and the new content, the way the Word layer
//! sends them: cells one by one (a step of cells applies only if every one still matches), a
//! sheet or the whole book as its JSON, a package part as its bytes. Cell references are
//! absolute, so a step isn't moved to where earlier changes put its content; what no longer
//! matches is dropped, on every copy alike.
//!
//! A cell's `style` is an index into the workbook's style sheet, which grows as formats are
//! added, outside the undo steps. So two copies can give the same new index to different
//! formats. An index from before sharing started is the same on every copy and goes as it is,
//! which keeps the file's own style entries exactly. A newer one goes as its [`CellFormat`],
//! and the receiver finds or adds that format. A row's `s` and a column's `style` are indexes
//! too, but nothing adds them, so they stay inside the markup as they are.
//!
//! A formula's result isn't part of the check: every copy works results out again after each
//! change, but not between taking back a pending change and checking the next one.

use office_core::model::{CellRef, Chart, Document, Origin, Range, Sheet, SheetCell, SheetPicture, SheetRow};
use office_engine::collab::{self, fingerprint, send_content, take_content, Replica, Shared};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::commands::put_cell;
use super::{CStep, SheetSel, SheetsView};
use crate::CellFormat;

impl CStep {
    pub(crate) fn inverse(&self) -> CStep {
        match self {
            CStep::Cells { sheet, cells } => {
                CStep::Cells { sheet: *sheet, cells: cells.iter().map(|(at, old, new)| (*at, new.clone(), old.clone())).collect() }
            }
            CStep::Sheet { sheet, before, after } => CStep::Sheet { sheet: *sheet, before: after.clone(), after: before.clone() },
            CStep::Book { before, after } => CStep::Book { before: after.clone(), after: before.clone() },
            CStep::Charts { sheet, before, after } => CStep::Charts { sheet: *sheet, before: after.clone(), after: before.clone() },
            CStep::Names { before, after } => CStep::Names { before: after.clone(), after: before.clone() },
            CStep::Part { name, before, after } => CStep::Part { name: name.clone(), before: after.clone(), after: before.clone() },
        }
    }
}

/// A cell's style as it's sent: an index every copy has (one from before sharing started), or
/// the format a newer index resolves to (see the module notes).
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SentStyle {
    Index(u32),
    Format(Box<CellFormat>),
}

/// A cell as it's sent: the model's JSON, but `style` is a [`SentStyle`].
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SentCell {
    col: u32,
    value: office_core::model::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    formula: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<SentStyle>,
    #[serde(default, skip_serializing_if = "office_core::model::wire::off", with = "office_core::model::wire::origin")]
    origin: Origin,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SentRow {
    index: u32,
    cells: Vec<SentCell>,
    #[serde(default, skip_serializing_if = "office_core::model::wire::off", with = "office_core::model::wire::origin")]
    origin: Origin,
}

/// A sheet as it's sent, cells and all.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SentSheet {
    name: String,
    rows: Vec<SentRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    merges: Vec<Range>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    charts: Vec<Chart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pictures: Vec<SheetPicture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part: Option<String>,
    #[serde(default, skip_serializing_if = "office_core::model::wire::off", with = "office_core::model::wire::origin")]
    origin: Origin,
}

/// A defined name, which the model keeps without serde.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SentName {
    name: String,
    /// The sheet it belongs to, by position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sheet: Option<usize>,
    formula: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
}

/// The workbook's defined names with the workbook markup they're saved in.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SentNames {
    names: Vec<SentName>,
    #[serde(default, skip_serializing_if = "office_core::model::wire::off", with = "office_core::model::wire::origin")]
    origin: Origin,
}

/// The same sheet on two steps' books: the part it was read from stays with it through
/// renames and moves; a new sheet (no part) goes by name.
fn same_sheet(a: &Sheet, b: &Sheet) -> bool {
    match (&a.part, &b.part) {
        (Some(x), Some(y)) => x == y,
        _ => a.name == b.name,
    }
}

fn sent_name(n: &crate::DefinedName) -> SentName {
    SentName { name: n.name.clone(), sheet: n.sheet, formula: n.formula.clone(), hidden: n.hidden }
}

fn real_name(n: SentName) -> crate::DefinedName {
    crate::DefinedName { name: n.name, sheet: n.sheet, formula: n.formula, hidden: n.hidden }
}

impl SheetsView {
    /// Starts sharing: from now on changes are kept to send, and undo goes through the checks.
    pub fn collab_start(&mut self, client: &str) {
        self.replica = Some(Replica::new(client, &self.doc));
        self.shared_styles = crate::excel(&self.doc).map_or(0, |x| x.styles.len());
        self.history = Default::default();
    }

    pub fn collab_stop(&mut self) {
        self.replica = None;
    }

    pub fn collab_take(&mut self) -> Option<Value> {
        collab::take(self)
    }

    /// Applies a change the relay ordered, then works the formulas out again, as a local
    /// change does.
    pub fn collab_receive(&mut self, op: &Value) -> bool {
        let changed = collab::receive(self, op);
        if self.replica.as_mut().is_some_and(|r| r.take_changed_by_others()) {
            self.others_changed = true;
        }
        if changed {
            self.pending_cut = None;
            self.recalculate();
            self.changed(None);
        }
        changed
    }

    pub fn collab_pending(&self) -> bool {
        self.replica.as_ref().is_some_and(|r| r.has_pending())
    }

    /// Notes steps made here for sending, when shared.
    pub(crate) fn shared_made(&mut self, steps: &[CStep]) {
        if let Some(r) = &mut self.replica {
            r.made(steps.iter().cloned());
        }
    }

    /// Undoes or redoes an entry's steps where the workbook is now; `false` if someone else's
    /// change is in the way.
    pub(crate) fn shared_replay(&mut self, steps: Vec<CStep>) -> bool {
        match collab::replay(self, &steps) {
            Some(done) => {
                self.shared_made(&done);
                true
            }
            None => false,
        }
    }

    /// What style index `index` says here.
    fn cell_format(&self, index: u32) -> CellFormat {
        crate::excel(&self.doc).map(|x| x.styles.format(index)).unwrap_or_default()
    }

    fn sent_cell(&self, c: &SheetCell) -> SentCell {
        SentCell {
            col: c.col,
            value: c.value.clone(),
            formula: c.formula.clone(),
            style: c.style.map(|i| {
                if (i as usize) < self.shared_styles {
                    SentStyle::Index(i)
                } else {
                    SentStyle::Format(Box::new(self.cell_format(i)))
                }
            }),
            origin: c.origin.clone(),
        }
    }

    /// A cell as checked against a step's fingerprint: a formula's result is left out, since
    /// it's worked out again after every change and a copy taking back its own pending change
    /// doesn't work it out again before checking.
    fn checked_cell(&self, c: &SheetCell) -> SentCell {
        let mut sent = self.sent_cell(c);
        if c.formula.is_some() || c.origin.get().is_some_and(|o| o.elements().any(|e| e.local == "f")) {
            sent.value = office_core::model::Value::Empty;
        }
        sent
    }

    fn checked_sheet(&self, s: &Sheet) -> Value {
        let mut sent = serde_json::to_value(self.sent_sheet(s)).unwrap_or(Value::Null);
        let cells = s.rows.iter().flat_map(|r| r.cells.iter());
        let sent_cells = sent["rows"].as_array_mut().into_iter().flatten().flat_map(|r| r["cells"].as_array_mut().into_iter().flatten());
        for (sent, c) in sent_cells.zip(cells) {
            *sent = serde_json::to_value(self.checked_cell(c)).unwrap_or(Value::Null);
        }
        sent
    }

    fn checked_sheets(&self, sheets: &[Sheet]) -> Vec<Value> {
        sheets.iter().map(|s| self.checked_sheet(s)).collect()
    }

    /// A sent cell back in this copy's terms: a newer format finds or takes a local index.
    fn real_cell(&mut self, c: SentCell) -> SheetCell {
        let style = c.style.and_then(|s| match s {
            SentStyle::Index(i) => Some(i),
            SentStyle::Format(f) => {
                let i = crate::excel_mut(&mut self.doc).styles.add(&f);
                (i != 0).then_some(i)
            }
        });
        SheetCell { col: c.col, value: c.value, formula: c.formula, style, origin: c.origin }
    }

    fn sent_sheet(&self, s: &Sheet) -> SentSheet {
        SentSheet {
            name: s.name.clone(),
            rows: s
                .rows
                .iter()
                .map(|r| SentRow { index: r.index, cells: r.cells.iter().map(|c| self.sent_cell(c)).collect(), origin: r.origin.clone() })
                .collect(),
            merges: s.merges.clone(),
            hidden: s.hidden,
            charts: s.charts.clone(),
            pictures: s.pictures.clone(),
            size: s.size,
            part: s.part.clone(),
            origin: s.origin.clone(),
        }
    }

    fn sent_sheets(&self, sheets: &[Sheet]) -> Vec<SentSheet> {
        sheets.iter().map(|s| self.sent_sheet(s)).collect()
    }

    fn real_sheet(&mut self, s: SentSheet) -> Sheet {
        let mut sheet = Sheet {
            name: s.name,
            merges: s.merges,
            hidden: s.hidden,
            charts: s.charts,
            pictures: s.pictures,
            size: s.size,
            part: s.part,
            origin: s.origin,
            ..Default::default()
        };
        for r in s.rows {
            let cells = r.cells.into_iter().map(|c| self.real_cell(c)).collect();
            sheet.rows.push(SheetRow { index: r.index, cells, origin: r.origin });
        }
        sheet
    }

    fn sent_names(&self) -> SentNames {
        SentNames { names: crate::names(&self.doc).iter().map(sent_name).collect(), origin: self.doc.origin.clone() }
    }

    /// Where `index` points now, past the book changes in `since` (sheets other copies added,
    /// removed or moved that the sender hadn't seen). `None` when its sheet is gone.
    fn map_sheet(&self, index: usize, since: &[&CStep]) -> Option<usize> {
        let mut i = index;
        for s in since {
            let CStep::Book { before, after } = s else { continue };
            let Some(sheet) = before.get(i) else { continue };
            i = after.iter().position(|s| same_sheet(s, sheet))?;
        }
        Some(i)
    }

    fn take_cells(&mut self, step: &Value, since: &[&CStep]) -> Option<CStep> {
        let sheet = self.map_sheet(step["sheet"].as_u64()? as usize, since)?;
        let sent = step["cells"].as_array()?;
        // Every cell must still be what the step replaced before any of them changes.
        let mut olds = Vec::with_capacity(sent.len());
        {
            let s = self.sheets().get(sheet)?;
            for e in sent {
                let at: CellRef = serde_json::from_value(e[0].clone()).ok()?;
                if fingerprint(&s.cell(at).map(|c| self.checked_cell(c))) != e[1].as_str()? {
                    return None;
                }
                olds.push((at, s.cell(at).map(|c| self.sent_cell(c))));
            }
        }
        let mut cells = Vec::with_capacity(sent.len());
        for ((at, old), e) in olds.iter().zip(sent) {
            let before = self.sheets()[sheet].cell(*at).cloned();
            let new: Option<SentCell> = take_content(old, &e[2])?;
            let after = new.map(|c| self.real_cell(c));
            put_cell(&mut self.sheets_mut()[sheet], *at, after.clone());
            cells.push((*at, before, after));
        }
        Some(CStep::Cells { sheet, cells })
    }

    fn take_sheet(&mut self, step: &Value, since: &[&CStep]) -> Option<CStep> {
        let index = self.map_sheet(step["sheet"].as_u64()? as usize, since)?;
        let before = self.sheets().get(index)?.clone();
        if fingerprint(&self.checked_sheet(&before)) != step["before"].as_str()? {
            return None;
        }
        let sent: SentSheet = take_content(&self.sent_sheet(&before), &step["after"])?;
        let after = self.real_sheet(sent);
        self.sheets_mut()[index] = after.clone();
        Some(CStep::Sheet { sheet: index, before: Box::new(before), after: Box::new(after) })
    }

    fn take_book(&mut self, step: &Value) -> Option<CStep> {
        let before = self.sheets().to_vec();
        if fingerprint(&self.checked_sheets(&before)) != step["before"].as_str()? {
            return None;
        }
        let sent: Vec<SentSheet> = take_content(&self.sent_sheets(&before), &step["after"])?;
        let mut after = Vec::with_capacity(sent.len());
        for s in sent {
            after.push(self.real_sheet(s));
        }
        *self.sheets_mut() = after.clone();
        Some(CStep::Book { before, after })
    }

    fn take_charts(&mut self, step: &Value, since: &[&CStep]) -> Option<CStep> {
        let index = self.map_sheet(step["sheet"].as_u64()? as usize, since)?;
        let s = self.sheets().get(index)?;
        if fingerprint(&s.charts) != step["before"].as_str()? {
            return None;
        }
        let before = s.charts.clone();
        let after: Vec<Chart> = take_content(&before, &step["after"])?;
        self.sheets_mut()[index].charts = after.clone();
        Some(CStep::Charts { sheet: index, before, after })
    }

    fn take_names(&mut self, step: &Value) -> Option<CStep> {
        let before = self.sent_names();
        if fingerprint(&before) != step["before"].as_str()? {
            return None;
        }
        let sent: SentNames = take_content(&before, &step["after"])?;
        let old = (crate::names(&self.doc).to_vec(), self.doc.origin.clone());
        self.doc.origin = sent.origin;
        crate::excel_mut(&mut self.doc).names = sent.names.into_iter().map(real_name).collect();
        Some(CStep::Names { before: Box::new(old), after: Box::new((crate::names(&self.doc).to_vec(), self.doc.origin.clone())) })
    }

    fn take_part(&mut self, step: &Value) -> Option<CStep> {
        let name = step["name"].as_str()?.to_string();
        let before = self.doc.package.as_ref()?.part(&name).ok()?.to_vec();
        if fingerprint(&collab::base64(&before)) != step["before"].as_str()? {
            return None;
        }
        let after = collab::unbase64(step["after"].as_str()?)?;
        self.doc.package.as_mut()?.replace(&name, after.clone()).ok()?;
        Some(CStep::Part { name, before, after })
    }
}

impl Shared for SheetsView {
    type Step = CStep;
    type Mark = SheetSel;

    fn replica(&mut self) -> &mut Option<Replica<CStep>> {
        &mut self.replica
    }

    fn doc(&mut self) -> &mut Document {
        &mut self.doc
    }

    fn send_step(&self, step: &CStep) -> Value {
        match step {
            CStep::Cells { sheet, cells } => json!({
                "k": "cells",
                "sheet": sheet,
                "cells": cells.iter().map(|(at, old, new)| {
                    let check = fingerprint(&old.as_ref().map(|c| self.checked_cell(c)));
                    let old = old.as_ref().map(|c| self.sent_cell(c));
                    json!([at, check, send_content(&old, &new.as_ref().map(|c| self.sent_cell(c)))])
                }).collect::<Vec<_>>(),
            }),
            CStep::Sheet { sheet, before, after } => json!({
                "k": "sheet",
                "sheet": sheet,
                "before": fingerprint(&self.checked_sheet(before)),
                "after": send_content(&self.sent_sheet(before), &self.sent_sheet(after)),
            }),
            CStep::Book { before, after } => json!({
                "k": "book",
                "before": fingerprint(&self.checked_sheets(before)),
                "after": send_content(&self.sent_sheets(before), &self.sent_sheets(after)),
            }),
            CStep::Charts { sheet, before, after } => json!({
                "k": "charts",
                "sheet": sheet,
                "before": fingerprint(before),
                "after": send_content(before, after),
            }),
            CStep::Names { before, after } => json!({
                "k": "names",
                "before": fingerprint(&SentNames { names: before.0.iter().map(sent_name).collect(), origin: before.1.clone() }),
                "after": send_content(
                    &SentNames { names: before.0.iter().map(sent_name).collect(), origin: before.1.clone() },
                    &SentNames { names: after.0.iter().map(sent_name).collect(), origin: after.1.clone() },
                ),
            }),
            CStep::Part { name, before, after } => json!({
                "k": "part",
                "name": name,
                "before": fingerprint(&collab::base64(before)),
                "after": collab::base64(after),
            }),
        }
    }

    fn take_step(&mut self, step: &Value, since: Option<&[(bool, &CStep)]>) -> Option<CStep> {
        // Sheets moved by other copies; the sender's own moves are already where it made this.
        let since: Vec<&CStep> = since.unwrap_or(&[]).iter().filter(|(own, _)| !own).map(|(_, s)| *s).collect();
        let since = since.as_slice();
        let s = match step["k"].as_str()? {
            "cells" => self.take_cells(step, since),
            "sheet" => self.take_sheet(step, since),
            "book" => self.take_book(step),
            "charts" => self.take_charts(step, since),
            "names" => self.take_names(step),
            "part" => self.take_part(step),
            _ => None,
        };
        if s.is_some() {
            self.changed(None);
        }
        s
    }

    fn revert_step(&mut self, step: &CStep) {
        self.apply(step, true);
        self.changed(None);
    }

    fn mark(&self) -> SheetSel {
        self.sel
    }

    /// Puts the selection back: the sheet it was on, moved past book changes just applied,
    /// or the last sheet if it's gone.
    fn restore(&mut self, mark: SheetSel, through: &[CStep]) {
        let through: Vec<&CStep> = through.iter().collect();
        let sheet = self.map_sheet(mark.sheet, &through).unwrap_or(usize::MAX);
        self.sel = SheetSel { sheet: sheet.min(self.sheets().len().saturating_sub(1)), ..mark };
    }
}

#[cfg(test)]
mod tests {
    use office_core::model::Value;
    use office_core::opc::Package;

    use super::*;
    use crate::view::FormatPatch;

    /// A small workbook's bytes: two cells on Sheet1.
    fn book() -> Vec<u8> {
        let mut v = SheetsView::blank(office_core::model::Kind::Sheets);
        v.sheets_mut()[0].set(CellRef::new(0, 0), Value::Number(2.0));
        v.sheets_mut()[0].set(CellRef::new(0, 1), Value::Text("a".into()));
        v.save().unwrap()
    }

    /// Copies of one workbook, and a relay that orders what they send.
    struct Room {
        views: Vec<SheetsView>,
        seq: u64,
        /// Sent and not yet delivered, in order.
        queue: Vec<serde_json::Value>,
    }

    impl Room {
        fn new(bytes: &[u8], n: usize) -> Room {
            let views = (0..n)
                .map(|i| {
                    let mut v = SheetsView::open(bytes, "Book.xlsx").unwrap();
                    v.collab_start(&format!("c{i}"));
                    v
                })
                .collect();
            Room { views, seq: 0, queue: Vec::new() }
        }

        /// Sends everything copy `i` has to send.
        fn send(&mut self, i: usize) {
            while let Some(op) = self.views[i].collab_take() {
                self.seq += 1;
                self.queue
                    .push(json!({ "seq": self.seq, "client": format!("c{i}"), "id": op["id"], "base": op["base"], "steps": op["steps"] }));
            }
        }

        fn deliver(&mut self) {
            for op in std::mem::take(&mut self.queue) {
                for v in &mut self.views {
                    v.collab_receive(&op);
                }
            }
        }

        fn sync(&mut self) {
            for i in 0..self.views.len() {
                self.send(i);
            }
            self.deliver();
        }

        /// Every sheet as sent on the wire, so copies agree even where their style sheets'
        /// indexes don't.
        fn agree(&self) -> serde_json::Value {
            let first = self.normalized(0);
            for i in 1..self.views.len() {
                assert_eq!(self.normalized(i), first, "copy {i} differs");
            }
            first
        }

        fn normalized(&self, i: usize) -> serde_json::Value {
            serde_json::to_value(self.views[i].sent_sheets(self.views[i].sheets())).unwrap()
        }
    }

    fn part(bytes: &[u8], name: &str) -> Vec<u8> {
        Package::read(bytes).unwrap().part(name).unwrap().to_vec()
    }

    #[test]
    fn typing_reaches_every_copy() {
        let mut room = Room::new(&book(), 2);
        room.views[0].set_input(0, CellRef::new(1, 0), "7");
        room.sync();
        assert_eq!(room.views[1].sheets()[0].value(CellRef::new(1, 0)), &Value::Number(7.0));
        room.agree();
        // Origins come along, so the copies save the same file.
        let a = room.views[0].save().unwrap();
        let b = room.views[1].save().unwrap();
        assert_eq!(part(&a, "xl/worksheets/sheet1.xml"), part(&b, "xl/worksheets/sheet1.xml"));
    }

    #[test]
    fn a_formulas_result_updates_on_both() {
        let mut room = Room::new(&book(), 2);
        room.views[0].set_input(0, CellRef::new(1, 0), "=A1*10");
        room.sync();
        assert_eq!(room.views[0].cell_info(0, CellRef::new(1, 0)).display, "20");
        assert_eq!(room.views[1].cell_info(0, CellRef::new(1, 0)).display, "20");
        room.agree();
    }

    #[test]
    fn edits_to_different_cells_both_survive() {
        let mut room = Room::new(&book(), 2);
        room.views[0].set_input(0, CellRef::new(1, 0), "one");
        room.views[1].set_input(0, CellRef::new(1, 1), "two");
        room.sync();
        for i in 0..2 {
            assert_eq!(room.views[i].sheets()[0].value(CellRef::new(1, 0)), &Value::Text("one".into()));
            assert_eq!(room.views[i].sheets()[0].value(CellRef::new(1, 1)), &Value::Text("two".into()));
        }
        room.agree();
    }

    #[test]
    fn the_second_of_two_edits_to_one_cell_is_dropped() {
        let mut room = Room::new(&book(), 2);
        room.views[0].set_input(0, CellRef::new(1, 0), "first");
        room.views[1].set_input(0, CellRef::new(1, 0), "second");
        room.sync();
        assert_eq!(room.views[0].sheets()[0].value(CellRef::new(1, 0)), &Value::Text("first".into()));
        assert_eq!(room.views[1].sheets()[0].value(CellRef::new(1, 0)), &Value::Text("first".into()));
        assert!(!room.views[1].collab_pending());
        room.agree();
    }

    #[test]
    fn new_formats_go_by_content_not_index() {
        let mut room = Room::new(&book(), 2);
        // Both copies add a format at the same time, giving it the same local index.
        let a1 = Range { start: CellRef::new(0, 0), end: CellRef::new(0, 0) };
        let b1 = Range { start: CellRef::new(0, 1), end: CellRef::new(0, 1) };
        room.views[0].format(0, a1, &FormatPatch { bold: Some(true), ..Default::default() });
        room.views[1].format(0, b1, &FormatPatch { italic: Some(true), ..Default::default() });
        assert_eq!(
            room.views[0].sheets()[0].cell(CellRef::new(0, 0)).unwrap().style,
            room.views[1].sheets()[0].cell(CellRef::new(0, 1)).unwrap().style,
            "same index, different format"
        );
        room.sync();
        for i in 0..2 {
            let v = &room.views[i];
            assert!(crate::format_of(&v.doc, 0, CellRef::new(0, 0)).font.bold, "copy {i}: A1 bold");
            assert!(!crate::format_of(&v.doc, 0, CellRef::new(0, 0)).font.italic, "copy {i}: A1 not italic");
            assert!(crate::format_of(&v.doc, 0, CellRef::new(0, 1)).font.italic, "copy {i}: B1 italic");
            assert!(!crate::format_of(&v.doc, 0, CellRef::new(0, 1)).font.bold, "copy {i}: B1 not bold");
        }
        room.agree();
    }

    #[test]
    fn undo_after_another_copys_change() {
        let mut room = Room::new(&book(), 2);
        room.views[0].set_input(0, CellRef::new(1, 0), "mine");
        room.sync();
        room.views[1].set_input(0, CellRef::new(1, 1), "theirs");
        room.sync();
        assert!(room.views[0].undo());
        room.sync();
        assert_eq!(room.views[0].sheets()[0].value(CellRef::new(1, 0)), &Value::Empty);
        assert_eq!(room.views[1].sheets()[0].value(CellRef::new(1, 0)), &Value::Empty);
        assert_eq!(room.views[1].sheets()[0].value(CellRef::new(1, 1)), &Value::Text("theirs".into()));
        room.agree();
    }

    #[test]
    fn a_sheet_added_reaches_every_copy() {
        let mut room = Room::new(&book(), 2);
        room.views[0].add_sheet();
        room.views[0].rename_sheet(1, "Data");
        room.views[0].set_input(1, CellRef::new(0, 0), "42");
        room.sync();
        assert_eq!(room.views[1].sheets().len(), 2);
        assert_eq!(room.views[1].sheets()[1].name, "Data");
        assert_eq!(room.views[1].sheets()[1].value(CellRef::new(0, 0)), &Value::Number(42.0));
        room.agree();
    }

    #[test]
    fn a_formulas_result_doesnt_decide_a_conflict() {
        let mut room = Room::new(&book(), 2);
        room.views[0].set_input(0, CellRef::new(0, 2), "=A1*2");
        room.sync();
        // Copy 0 changes what the formula reads while copy 1 replaces the formula; copy 0's
        // result was worked out from its own pending change when copy 1's arrives.
        room.views[0].set_input(0, CellRef::new(0, 0), "5");
        room.views[1].set_input(0, CellRef::new(0, 2), "7");
        room.send(1);
        room.send(0);
        room.deliver();
        room.agree();
        assert_eq!(room.views[0].sheets()[0].value(CellRef::new(0, 2)), &Value::Number(7.0));
        assert_eq!(room.views[0].sheets()[0].value(CellRef::new(0, 0)), &Value::Number(5.0));
    }

    #[test]
    fn taking_back_a_cell_in_a_new_row_leaves_no_row() {
        let mut room = Room::new(&book(), 2);
        room.views[0].set_input(0, CellRef::new(5, 0), "new row");
        room.views[1].rename_sheet(0, "Renamed");
        room.send(1);
        room.send(0);
        room.deliver();
        room.agree();
        assert_eq!(room.views[0].sheets()[0].name, "Renamed");
        assert_eq!(room.views[0].sheets()[0].value(CellRef::new(5, 0)), &Value::Text("new row".into()));
    }
}
