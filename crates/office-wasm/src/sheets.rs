//! Workbooks and CSV files.

use office_core::model::ChartKind;
use office_core::model::{CellRef, Kind, Range};
use office_sheets::view::{ChartPatch, FormatPatch, SheetsView};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::{err, json, Host, JsMeasurer};

#[wasm_bindgen]
pub struct SheetsDoc {
    view: SheetsView,
    host: Host,
    matches: Vec<(usize, CellRef)>,
    query: String,
}

#[derive(Deserialize, Clone, Copy)]
struct R {
    r0: u32,
    c0: u32,
    r1: u32,
    c1: u32,
}

impl R {
    fn range(self) -> Range {
        Range {
            start: CellRef::new(self.r0.min(self.r1), self.c0.min(self.c1)),
            end: CellRef::new(self.r0.max(self.r1), self.c0.max(self.c1)),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Cmd {
    Select { sheet: usize, anchor: (u32, u32), focus: (u32, u32) },
    Input { sheet: usize, row: u32, col: u32, text: String },
    Clear { sheet: usize, range: R },
    Cut { sheet: usize, range: R, move_id: String },
    Format { sheet: usize, range: R, patch: FormatPatch },
    ColumnWidth { sheet: usize, col: u32, px: Option<f64> },
    RowHeight { sheet: usize, row: u32, px: Option<f64> },
    ShiftRows { sheet: usize, at: u32, n: i64 },
    ShiftCols { sheet: usize, at: u32, n: i64 },
    SheetSize { sheet: usize, rows: u32, cols: u32 },
    Merge { sheet: usize, range: R },
    AddSheet,
    RenameSheet { sheet: usize, name: String },
    DeleteSheet { sheet: usize },
    MoveSheet { from: usize, to: usize },
    Paste { sheet: usize, range: R, text: String, clipboard: Option<String> },
    FillDown { sheet: usize, range: R },
    Undo,
    Redo,
    MarkSaved,
    Find { query: String },
    ReplaceMatch { index: usize, text: String },
    ReplaceAll { text: String },
    AddChart { sheet: usize, range: R, kind: ChartKind },
    SetChart { sheet: usize, index: usize, patch: ChartPatch },
    MoveChart { sheet: usize, index: usize, x: f64, y: f64, w: f64, h: f64 },
    DeleteChart { sheet: usize, index: usize },
}

#[wasm_bindgen]
impl SheetsDoc {
    /// Opens a workbook or CSV file (`name` says which), or makes a new one when `bytes` is empty
    /// (`csv` in the name for a CSV file).
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], name: &str, host: Host) -> Result<SheetsDoc, JsError> {
        let csv = name.to_ascii_lowercase().ends_with(".csv") || name.to_ascii_lowercase().ends_with(".tsv");
        let mut view = if bytes.is_empty() {
            SheetsView::blank(if csv { Kind::Csv } else { Kind::Sheets })
        } else {
            SheetsView::open(bytes, name).map_err(err)?
        };
        if view.kind == Kind::Csv && !bytes.is_empty() {
            view.fit_columns(&mut JsMeasurer(&host), 0);
        }
        Ok(SheetsDoc { view, host, matches: Vec::new(), query: String::new() })
    }

    /// The sheets: `[{ name, hidden }]`.
    pub fn sheets(&self) -> String {
        json(&self.view.sheets().iter().map(|s| serde_json::json!({ "name": s.name, "hidden": s.hidden })).collect::<Vec<_>>())
    }

    /// A sheet's columns and rows: counts, default sizes and the ones with their own.
    pub fn grid(&mut self, sheet: usize) -> String {
        json(self.view.grid(sheet))
    }

    /// Draws rows `r0..r1`, columns `c0..c1` of a sheet: `{ items, styles }` in grid pixels.
    pub fn paint(&mut self, sheet: usize, r0: u32, r1: u32, c0: u32, c1: u32) -> String {
        let mut m = JsMeasurer(&self.host);
        let p = self.view.paint(&mut m, sheet, r0, r1, c0, c1);
        let styles = self.view.new_styles();
        json(&serde_json::json!({ "items": p.items, "styles": styles }))
    }

    /// A sheet's charts as the app draws them (see `ChartView`).
    pub fn charts(&mut self, sheet: usize) -> String {
        json(&self.view.charts(sheet))
    }

    /// A sheet's pictures as the app draws them: `{ x, y, w, h, media, crop, name }`.
    pub fn pictures(&mut self, sheet: usize) -> String {
        json(&self.view.pictures(sheet))
    }

    /// A picture's bytes, by media id.
    pub fn media(&self, id: &str) -> Option<Vec<u8>> {
        self.view.media(id).map(|(b, _)| b.to_vec())
    }

    /// A picture's media type.
    #[wasm_bindgen(js_name = mediaType)]
    pub fn media_type(&self, id: &str) -> Option<String> {
        self.view.media(id).and_then(|(_, t)| t.map(str::to_string))
    }

    /// A cell as the formula bar and inspector see it.
    pub fn cell(&self, sheet: usize, row: u32, col: u32) -> String {
        json(&self.view.cell_info(sheet, CellRef::new(row, col)))
    }

    /// Why typing `text` into a cell would be refused by its data validation, as
    /// `{ style, title, message }`, or `null` when it's taken.
    pub fn validate(&self, sheet: usize, row: u32, col: u32, text: &str) -> String {
        json(&self.view.refusal(sheet, CellRef::new(row, col), text))
    }

    /// `{ sheet, anchor, focus, canUndo, canRedo, dirty, csv, circular }`, `circular` being the
    /// formulas in a circular reference as `{ sheet, row, col }`.
    pub fn state(&self) -> String {
        let s = self.view.selection();
        json(&serde_json::json!({
            "sheet": s.sheet,
            "anchor": s.anchor,
            "focus": s.focus,
            "canUndo": self.view.can_undo(),
            "canRedo": self.view.can_redo(),
            "dirty": self.view.is_dirty(),
            "csv": self.view.kind == Kind::Csv,
            "circular": self
                .view
                .circular()
                .iter()
                .map(|(s, c)| serde_json::json!({ "sheet": s, "row": c.row, "col": c.col }))
                .collect::<Vec<_>>(),
        }))
    }

    /// Goes up with every change.
    pub fn revision(&self) -> f64 {
        self.view.revision() as f64
    }

    /// A range as clipboard data: `{ text, clipboard }`.
    pub fn copy(&self, sheet: usize, r0: u32, c0: u32, r1: u32, c1: u32, move_id: Option<String>) -> String {
        let (text, clipboard) = self.view.copy(sheet, R { r0, c0, r1, c1 }.range(), move_id.as_deref());
        json(&serde_json::json!({ "text": text, "clipboard": clipboard }))
    }

    /// What NOW() and TODAY() give, as a serial number (days since 1900, with the time as a fraction).
    #[wasm_bindgen(js_name = setNow)]
    pub fn set_now(&mut self, serial: f64) {
        self.view.set_now(serial);
    }

    #[wasm_bindgen(js_name = fontsChanged)]
    pub fn fonts_changed(&mut self) {
        self.view.fonts_changed();
    }

    pub fn command(&mut self, cmd: &str) -> Result<String, JsError> {
        let cmd: Cmd = serde_json::from_str(cmd).map_err(err)?;
        let v = &mut self.view;
        let null = serde_json::Value::Null;
        let out = match cmd {
            Cmd::Select { sheet, anchor, focus } => {
                v.select(sheet, anchor, focus);
                null
            }
            Cmd::Input { sheet, row, col, text } => {
                v.set_input(sheet, CellRef::new(row, col), &text);
                null
            }
            Cmd::Clear { sheet, range } => {
                v.clear(sheet, range.range());
                null
            }
            Cmd::Cut { sheet, range, move_id } => {
                v.cut(sheet, range.range(), move_id);
                null
            }
            Cmd::Format { sheet, range, patch } => {
                v.format(sheet, range.range(), &patch);
                null
            }
            Cmd::ColumnWidth { sheet, col, px } => {
                v.set_column_width(sheet, col, px);
                null
            }
            Cmd::RowHeight { sheet, row, px } => {
                v.set_row_height(sheet, row, px);
                null
            }
            Cmd::ShiftRows { sheet, at, n } => {
                v.shift_rows(sheet, at, n);
                null
            }
            Cmd::ShiftCols { sheet, at, n } => {
                v.shift_cols(sheet, at, n);
                null
            }
            Cmd::SheetSize { sheet, rows, cols } => {
                v.set_sheet_size(sheet, rows, cols);
                null
            }
            Cmd::Merge { sheet, range } => {
                v.toggle_merge(sheet, range.range());
                null
            }
            Cmd::AddSheet => {
                v.add_sheet();
                null
            }
            Cmd::RenameSheet { sheet, name } => {
                v.rename_sheet(sheet, &name);
                null
            }
            Cmd::DeleteSheet { sheet } => {
                v.delete_sheet(sheet);
                null
            }
            Cmd::MoveSheet { from, to } => {
                v.move_sheet(from, to);
                null
            }
            Cmd::Paste { sheet, range, text, clipboard } => {
                let clipboard = clipboard.as_deref().and_then(|data| serde_json::from_str(data).ok());
                if clipboard.is_some() || !text.is_empty() {
                    v.paste(sheet, range.range(), &text, clipboard);
                }
                null
            }
            Cmd::FillDown { sheet, range } => {
                v.fill_down(sheet, range.range());
                null
            }
            Cmd::Undo => serde_json::json!(v.undo()),
            Cmd::Redo => serde_json::json!(v.redo()),
            Cmd::MarkSaved => {
                v.mark_saved();
                null
            }
            Cmd::Find { query } => {
                self.matches = v.find(&query);
                self.query = query;
                serde_json::json!(self
                    .matches
                    .iter()
                    .map(|(s, c)| serde_json::json!({ "sheet": s, "row": c.row, "col": c.col }))
                    .collect::<Vec<_>>())
            }
            Cmd::ReplaceMatch { index, text } => {
                if let Some(m) = self.matches.get(index).copied() {
                    v.replace(&[m], &self.query, &text);
                }
                null
            }
            Cmd::AddChart { sheet, range, kind } => serde_json::json!(v.add_chart(sheet, range.range(), kind)),
            Cmd::SetChart { sheet, index, patch } => {
                v.set_chart(sheet, index, &patch);
                null
            }
            Cmd::MoveChart { sheet, index, x, y, w, h } => {
                v.move_chart(sheet, index, x, y, w, h);
                null
            }
            Cmd::DeleteChart { sheet, index } => {
                v.delete_chart(sheet, index);
                null
            }
            Cmd::ReplaceAll { text } => {
                let all = self.matches.clone();
                v.replace(&all, &self.query, &text);
                serde_json::json!(all.len())
            }
        };
        Ok(json(&out))
    }

    /// Starts sharing the document as copy `client` (see `office_engine::collab`).
    #[wasm_bindgen(js_name = collabStart)]
    pub fn collab_start(&mut self, client: &str) {
        self.view.collab_start(client);
    }

    #[wasm_bindgen(js_name = collabStop)]
    pub fn collab_stop(&mut self) {
        self.view.collab_stop();
    }

    /// The changes made here since the last call, as JSON for the relay, if there are any.
    #[wasm_bindgen(js_name = collabTake)]
    pub fn collab_take(&mut self) -> Option<String> {
        self.view.collab_take().map(|v| v.to_string())
    }

    /// Applies a change from the relay (JSON); returns whether the document changed.
    #[wasm_bindgen(js_name = collabReceive)]
    pub fn collab_receive(&mut self, op: &str) -> bool {
        serde_json::from_str(op).is_ok_and(|op| self.view.collab_receive(&op))
    }

    /// Whether changes made here haven't come back from the relay yet.
    #[wasm_bindgen(js_name = collabPending)]
    pub fn collab_pending(&self) -> bool {
        self.view.collab_pending()
    }

    pub fn save(&mut self) -> Result<Vec<u8>, JsError> {
        self.view.save().map_err(err)
    }
}
