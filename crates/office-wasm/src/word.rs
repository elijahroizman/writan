//! Word documents.

use office_engine::pos::{Path, Pos, Selection};
use office_word::view::{BorderScope, Move, TableOp, Toggle, WordView};
use office_word::{ListKind, StoryKind};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::{err, json, Host, JsMeasurer};

#[wasm_bindgen]
pub struct WordDoc {
    view: WordView,
    host: Host,
    /// The last find's matches, for stepping and replacing.
    matches: Vec<(Path, u32, u32)>,
    /// A hash of each page as last drawn, so a page that didn't change isn't sent again.
    drawn: std::collections::HashMap<usize, u64>,
}

/// A command from the editor.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Cmd {
    Click {
        page: usize,
        x: f64,
        y: f64,
        extend: bool,
        count: u32,
    },
    Drag {
        page: usize,
        x: f64,
        y: f64,
    },
    Move {
        to: String,
        extend: bool,
    },
    SelectAll,
    InsertText {
        text: String,
    },
    Backspace {
        word: bool,
    },
    Delete {
        word: bool,
    },
    Enter,
    LineBreak,
    PageBreak,
    Tab {
        shift: bool,
    },
    Toggle {
        what: String,
    },
    Font {
        family: String,
    },
    Size {
        half_points: u32,
    },
    Color {
        hex: String,
    },
    Highlight {
        name: String,
    },
    ClearFormatting,
    Align {
        to: String,
    },
    Style {
        id: String,
    },
    LineSpacing {
        value: f64,
    },
    List {
        kind: String,
    },
    Indent {
        delta: i32,
    },
    Undo,
    Redo,
    BeginGroup,
    EndGroup,
    Paste {
        text: String,
        json: Option<String>,
    },
    Cut,
    Copy,
    SetLink {
        url: String,
    },
    RemoveLink,
    Find {
        query: String,
        match_case: bool,
        whole_word: bool,
    },
    SelectMatch {
        index: usize,
    },
    ReplaceMatch {
        index: usize,
        text: String,
    },
    ReplaceAll {
        text: String,
    },
    GoTo {
        path: Vec<u32>,
    },
    InsertTable {
        rows: u32,
        cols: u32,
    },
    Table {
        op: String,
        color: Option<String>,
    },
    ImageSize {
        width_emu: i64,
        height_emu: i64,
    },
    CropImage {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
        width_emu: i64,
        height_emu: i64,
    },
    ResetImageSize,
    PageSetup {
        width_twips: Option<i64>,
        height_twips: Option<i64>,
        landscape: Option<bool>,
        top: Option<i64>,
        bottom: Option<i64>,
        left: Option<i64>,
        right: Option<i64>,
    },
    EditHeaderFooter {
        page: usize,
        which: String,
    },
    ExitHeaderFooter,
    RemoveHeaderFooter,
    PageField {
        field: String,
    },
    MarkSaved,
    Select {
        anchor: (Vec<u32>, u32),
        focus: (Vec<u32>, u32),
    },
    ClearFind,
    /// Highlights ranges of body text: each (id, path, start, end), offsets in UTF-16 units
    /// of the paragraph's text as `paragraphs()` gives it.
    SetMarks {
        set: String,
        marks: Vec<(String, Vec<u32>, u32, u32)>,
    },
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// A UTF-16 offset into a paragraph's text as a byte offset into its flat text.
pub(crate) fn byte_offset(text: &str, units: u32) -> u32 {
    let mut n = 0;
    for (i, c) in text.char_indices() {
        if n >= units {
            return i as u32;
        }
        n += c.len_utf16() as u32;
    }
    text.len() as u32
}

/// A byte offset into flat text as UTF-16 units.
fn utf16_offset(text: &str, bytes: u32) -> u32 {
    text[..(bytes as usize).min(text.len())].encode_utf16().count() as u32
}

#[wasm_bindgen]
impl WordDoc {
    /// Opens a Word file, or makes a new A4 document when `bytes` is empty.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], host: Host) -> Result<WordDoc, JsError> {
        let view = if bytes.is_empty() { WordView::blank() } else { WordView::open(bytes).map_err(err)? };
        Ok(WordDoc { view, host, matches: Vec::new(), drawn: Default::default() })
    }

    /// Lays the document out and returns every page's size: `[[w, h], …]`.
    pub fn layout(&mut self) -> String {
        let mut m = JsMeasurer(&self.host);
        self.view.layout(&mut m);
        json(&self.view.page_sizes())
    }

    /// Draws a page: `{ width, height, items, styles }`, where `styles` are paint styles the
    /// app hasn't been sent before (numbered on from the last ones sent). Answers `null` when the
    /// page is drawn the same as last time, unless `force` (the app dropped what it drew).
    pub fn paint(&mut self, page: usize, force: bool) -> String {
        let mut m = JsMeasurer(&self.host);
        let p = self.view.paint(&mut m, page);
        let items = json(&p.items);
        let hash = fnv(items.as_bytes()) ^ (p.width.to_bits().rotate_left(7)) ^ p.height.to_bits();
        if !force && self.drawn.get(&page) == Some(&hash) {
            return "null".into();
        }
        self.drawn.insert(page, hash);
        let styles = json(&self.view.new_styles());
        format!("{{\"width\":{},\"height\":{},\"items\":{items},\"styles\":{styles}}}", p.width, p.height)
    }

    /// Send every paint style again with the next page drawn.
    #[wasm_bindgen(js_name = resendStyles)]
    pub fn resend_styles(&mut self) {
        self.view.resend_styles();
    }

    /// The caret and selection on the pages.
    pub fn overlay(&mut self) -> String {
        let mut m = JsMeasurer(&self.host);
        json(&self.view.overlay(&mut m))
    }

    /// The toolbar and inspector state.
    pub fn state(&mut self) -> String {
        let mut s = serde_json::to_value(self.view.state()).unwrap_or_default();
        s["pages"] = serde_json::json!(self.view.page_count());
        json(&s)
    }

    #[wasm_bindgen(js_name = wordCount)]
    pub fn word_count(&self) -> usize {
        self.view.word_count()
    }

    /// Goes up when the pages need drawing again.
    pub fn revision(&self) -> f64 {
        self.view.revision() as f64
    }

    /// A font finished loading: everything is measured again at the next layout.
    #[wasm_bindgen(js_name = fontsChanged)]
    pub fn fonts_changed(&mut self) {
        self.view.fonts_changed();
        self.drawn.clear();
    }

    #[wasm_bindgen(js_name = styleOptions)]
    pub fn style_options(&self) -> String {
        json(&self.view.style_options())
    }

    #[wasm_bindgen(js_name = pageSetup)]
    pub fn page_setup(&self) -> String {
        json(&self.view.page_setup_state())
    }

    #[wasm_bindgen(js_name = documentFonts)]
    pub fn document_fonts(&self) -> String {
        json(&self.view.document_fonts())
    }

    pub fn outline(&self) -> String {
        let items: Vec<_> = self
            .view
            .outline()
            .into_iter()
            .map(|(text, depth, path)| serde_json::json!({ "text": text, "depth": depth, "path": path.0 }))
            .collect();
        json(&items)
    }

    /// Whether a point on a page is in its header (`header`) or footer (`footer`) area.
    #[wasm_bindgen(js_name = headerFooterAt)]
    pub fn header_footer_at(&self, page: usize, y: f64) -> Option<String> {
        self.view.in_header_footer(page, y).map(|k| if k == StoryKind::Header { "header".into() } else { "footer".into() })
    }

    /// The body's paragraphs in reading order (table cells included): `[{ path, text }]`, with
    /// U+FFFC for each picture or other object.
    pub fn paragraphs(&self) -> String {
        let blocks = self.view.body();
        let mut out = Vec::new();
        let mut at = office_engine::pos::story_start(blocks).map(|p| p.path);
        while let Some(path) = at {
            if let Some(p) = office_engine::pos::para(blocks, &path) {
                // Word's w14:paraId, which comments key their anchors by.
                let para_id = p.origin.get().and_then(|o| o.attrs.iter().find(|a| a.local == "paraId").map(|a| a.value.clone()));
                out.push(serde_json::json!({ "path": path.0, "text": office_engine::text::flat(p).text, "paraId": para_id }));
            }
            at = office_engine::pos::next_para(blocks, &path);
        }
        json(&out)
    }

    /// The selection: `{ anchor: [path, offset], focus: [path, offset], story }`, offsets in
    /// UTF-16 units of the paragraphs' text.
    pub fn selection(&self) -> String {
        let s = self.view.selection();
        let blocks = self.view.current_blocks();
        let conv = |p: &Pos| -> serde_json::Value {
            let text = office_engine::pos::para(blocks, &p.path).map(|q| office_engine::text::flat(q).text).unwrap_or_default();
            serde_json::json!([p.path.0, utf16_offset(&text, p.offset)])
        };
        json(
            &serde_json::json!({ "anchor": conv(&s.anchor), "focus": conv(&s.focus), "body": self.view.story() == office_word::view::StoryId::Body }),
        )
    }

    /// Whether a page has a header (`header`) or footer (`footer`).
    #[wasm_bindgen(js_name = hasHeaderFooter)]
    pub fn has_header_footer(&self, page: usize, which: &str) -> bool {
        self.view.page_has_story(page, if which == "footer" { StoryKind::Footer } else { StoryKind::Header })
    }

    /// Puts a picture in at the selection, `width` by `height` pixels (shrunk to fit the page).
    #[wasm_bindgen(js_name = insertImage)]
    pub fn insert_image(&mut self, bytes: Vec<u8>, content_type: &str, width: f64, height: f64) {
        self.view.insert_image(bytes, content_type, width, height);
    }

    /// Whether the document has comments.
    #[wasm_bindgen(js_name = hasComments)]
    pub fn has_comments(&self) -> bool {
        office_word::word(&self.view.doc).is_some_and(|w| !w.comments.is_empty())
    }

    pub fn dirty(&self) -> bool {
        self.view.is_dirty()
    }

    /// Runs an editing command (JSON); returns its result as JSON (`null` for most).
    pub fn command(&mut self, cmd: &str) -> Result<String, JsError> {
        let cmd: Cmd = serde_json::from_str(cmd).map_err(err)?;
        let mut m = JsMeasurer(&self.host);
        let v = &mut self.view;
        let out = match cmd {
            Cmd::Click { page, x, y, extend, count } => serde_json::json!(v.click(&mut m, page, x, y, extend, count)),
            Cmd::Drag { page, x, y } => {
                v.drag(&mut m, page, x, y);
                serde_json::Value::Null
            }
            Cmd::Move { to, extend } => {
                let to = match to.as_str() {
                    "left" => Move::Left,
                    "right" => Move::Right,
                    "up" => Move::Up,
                    "down" => Move::Down,
                    "wordLeft" => Move::WordLeft,
                    "wordRight" => Move::WordRight,
                    "lineStart" => Move::LineStart,
                    "lineEnd" => Move::LineEnd,
                    "docStart" => Move::DocStart,
                    "docEnd" => Move::DocEnd,
                    "pageUp" => Move::PageUp,
                    "pageDown" => Move::PageDown,
                    other => return Err(err(format!("no such move: {other}"))),
                };
                v.move_caret(&mut m, to, extend);
                serde_json::Value::Null
            }
            Cmd::SelectAll => {
                v.select_all();
                serde_json::Value::Null
            }
            Cmd::InsertText { text } => {
                v.insert_text(&text);
                serde_json::Value::Null
            }
            Cmd::Backspace { word } => {
                v.backspace(word);
                serde_json::Value::Null
            }
            Cmd::Delete { word } => {
                v.delete_forward(word);
                serde_json::Value::Null
            }
            Cmd::Enter => {
                v.enter();
                serde_json::Value::Null
            }
            Cmd::LineBreak => {
                v.line_break();
                serde_json::Value::Null
            }
            Cmd::PageBreak => {
                v.page_break();
                serde_json::Value::Null
            }
            Cmd::Tab { shift } => {
                v.tab(shift);
                serde_json::Value::Null
            }
            Cmd::Toggle { what } => {
                let t = match what.as_str() {
                    "bold" => Toggle::Bold,
                    "italic" => Toggle::Italic,
                    "underline" => Toggle::Underline,
                    "strike" => Toggle::Strike,
                    "superscript" => Toggle::Superscript,
                    "subscript" => Toggle::Subscript,
                    other => return Err(err(format!("no such toggle: {other}"))),
                };
                v.toggle(t);
                serde_json::Value::Null
            }
            Cmd::Font { family } => {
                v.set_font(&family);
                serde_json::Value::Null
            }
            Cmd::Size { half_points } => {
                v.set_size(half_points);
                serde_json::Value::Null
            }
            Cmd::Color { hex } => {
                v.set_color(&hex);
                serde_json::Value::Null
            }
            Cmd::Highlight { name } => {
                v.set_highlight(&name);
                serde_json::Value::Null
            }
            Cmd::ClearFormatting => {
                v.clear_formatting();
                serde_json::Value::Null
            }
            Cmd::Align { to } => {
                v.align(&to);
                serde_json::Value::Null
            }
            Cmd::Style { id } => {
                v.set_style(&id);
                serde_json::Value::Null
            }
            Cmd::LineSpacing { value } => {
                v.line_spacing(value);
                serde_json::Value::Null
            }
            Cmd::List { kind } => {
                v.list(match kind.as_str() {
                    "bullet" => Some(ListKind::Bullets),
                    "numbered" => Some(ListKind::Numbers),
                    _ => None,
                });
                serde_json::Value::Null
            }
            Cmd::Indent { delta } => {
                v.indent(delta);
                serde_json::Value::Null
            }
            Cmd::Undo => serde_json::json!(v.undo()),
            Cmd::Redo => serde_json::json!(v.redo()),
            Cmd::BeginGroup => {
                v.begin_group();
                serde_json::Value::Null
            }
            Cmd::EndGroup => {
                v.end_group();
                serde_json::Value::Null
            }
            Cmd::Paste { text, json: j } => {
                v.paste(&text, j.as_deref());
                serde_json::Value::Null
            }
            Cmd::Cut => {
                let (text, j) = v.cut();
                serde_json::json!({ "text": text, "json": j })
            }
            Cmd::Copy => {
                let (text, j) = v.copy();
                serde_json::json!({ "text": text, "json": j })
            }
            Cmd::SetLink { url } => {
                v.set_link(&url);
                serde_json::Value::Null
            }
            Cmd::RemoveLink => {
                v.remove_link();
                serde_json::Value::Null
            }
            Cmd::ClearFind => {
                self.matches.clear();
                v.set_marks("search", Vec::new());
                serde_json::Value::Null
            }
            Cmd::SetMarks { set, marks } => {
                let blocks = v.body().to_vec();
                let marks = marks
                    .into_iter()
                    .filter_map(|(id, path, a, b)| {
                        let path = Path(path);
                        let p = office_engine::pos::para(&blocks, &path)?;
                        let flat = office_engine::text::flat(p);
                        Some((id, Pos::new(path.clone(), byte_offset(&flat.text, a)), Pos::new(path, byte_offset(&flat.text, b))))
                    })
                    .collect();
                v.set_marks(&set, marks);
                serde_json::Value::Null
            }
            Cmd::Find { query, match_case, whole_word } => {
                self.matches = v.find(&query, match_case, whole_word);
                v.set_marks(
                    "search",
                    self.matches.iter().map(|(p, a, b)| (String::new(), Pos::new(p.clone(), *a), Pos::new(p.clone(), *b))).collect(),
                );
                // The first match at or after the caret.
                let caret = v.selection().start().clone();
                let first = self.matches.iter().position(|(p, s, _)| Pos::new(p.clone(), *s) >= caret).unwrap_or(0);
                serde_json::json!({ "count": self.matches.len(), "first": first })
            }
            Cmd::SelectMatch { index } => {
                if let Some((p, a, b)) = self.matches.get(index).cloned() {
                    v.set_selection(Selection { anchor: Pos::new(p.clone(), a), focus: Pos::new(p, b) });
                }
                serde_json::Value::Null
            }
            Cmd::ReplaceMatch { index, text } => {
                if let Some(mt) = self.matches.get(index).cloned() {
                    v.replace(&[mt], &text);
                }
                serde_json::Value::Null
            }
            Cmd::ReplaceAll { text } => {
                let all = std::mem::take(&mut self.matches);
                v.replace(&all, &text);
                serde_json::json!(all.len())
            }
            Cmd::GoTo { path } => {
                v.set_selection(Selection::caret(Pos::new(Path(path), 0)));
                serde_json::Value::Null
            }
            Cmd::InsertTable { rows, cols } => {
                v.insert_table(rows, cols);
                serde_json::Value::Null
            }
            Cmd::Table { op, color } => {
                let op = match op.as_str() {
                    "insertRowAbove" => TableOp::InsertRow { below: false },
                    "insertRowBelow" => TableOp::InsertRow { below: true },
                    "deleteRow" => TableOp::DeleteRow,
                    "insertColumnLeft" => TableOp::InsertColumn { right: false },
                    "insertColumnRight" => TableOp::InsertColumn { right: true },
                    "deleteColumn" => TableOp::DeleteColumn,
                    "fill" => TableOp::Fill(color),
                    "bordersAll" => TableOp::Borders(BorderScope::All),
                    "bordersOutside" => TableOp::Borders(BorderScope::Outside),
                    "bordersNone" => TableOp::Borders(BorderScope::None),
                    "delete" => TableOp::Delete,
                    other => return Err(err(format!("no such table change: {other}"))),
                };
                v.table_op(op);
                serde_json::Value::Null
            }
            Cmd::ImageSize { width_emu, height_emu } => {
                v.set_image_size(width_emu, height_emu);
                serde_json::Value::Null
            }
            Cmd::CropImage { left, top, right, bottom, width_emu, height_emu } => {
                v.crop_image([left, top, right, bottom], width_emu, height_emu);
                serde_json::Value::Null
            }
            Cmd::ResetImageSize => {
                v.reset_image_size();
                serde_json::Value::Null
            }
            Cmd::PageSetup { width_twips, height_twips, landscape, top, bottom, left, right } => {
                use office_core::units::Emu;
                v.set_page_setup(&move |s| {
                    if let Some(w) = width_twips {
                        s.page_width = Emu::from_twips(w);
                    }
                    if let Some(h) = height_twips {
                        s.page_height = Emu::from_twips(h);
                    }
                    if let Some(l) = landscape {
                        s.landscape = l;
                    }
                    if let Some(t) = top {
                        s.margin_top = Emu::from_twips(t);
                    }
                    if let Some(b) = bottom {
                        s.margin_bottom = Emu::from_twips(b);
                    }
                    if let Some(l) = left {
                        s.margin_left = Emu::from_twips(l);
                    }
                    if let Some(r) = right {
                        s.margin_right = Emu::from_twips(r);
                    }
                });
                serde_json::Value::Null
            }
            Cmd::EditHeaderFooter { page, which } => {
                let kind = if which == "footer" { StoryKind::Footer } else { StoryKind::Header };
                serde_json::json!(v.edit_header_footer(page, kind))
            }
            Cmd::ExitHeaderFooter => {
                v.exit_header_footer();
                serde_json::Value::Null
            }
            Cmd::RemoveHeaderFooter => {
                v.remove_header_footer();
                serde_json::Value::Null
            }
            Cmd::PageField { field } => {
                v.insert_page_field(&field);
                serde_json::Value::Null
            }
            Cmd::MarkSaved => {
                v.mark_saved();
                serde_json::Value::Null
            }
            Cmd::Select { anchor, focus } => {
                let blocks = v.current_blocks().to_vec();
                let conv = |(path, units): (Vec<u32>, u32)| {
                    let path = Path(path);
                    let text = office_engine::pos::para(&blocks, &path).map(|q| office_engine::text::flat(q).text).unwrap_or_default();
                    Pos::new(path, byte_offset(&text, units))
                };
                v.set_selection(Selection { anchor: conv(anchor), focus: conv(focus) });
                serde_json::Value::Null
            }
        };
        Ok(json(&out))
    }

    /// The document as a Word file.
    pub fn save(&mut self) -> Result<Vec<u8>, JsError> {
        self.view.save().map_err(err)
    }

    /// A picture's bytes, by media id.
    pub fn media(&self, id: &str) -> Option<Vec<u8>> {
        // Pictures read into the model are media; floating ones are drawn straight from their part.
        self.view.doc.media_bytes(id).ok().map(|b| b.to_vec()).or_else(|| self.view.doc.package.as_ref()?.part(id).ok().map(<[u8]>::to_vec))
    }

    /// A picture's media type.
    #[wasm_bindgen(js_name = mediaType)]
    pub fn media_type(&self, id: &str) -> Option<String> {
        if let Some(m) = self.view.doc.media(id) {
            return Some(m.content_type.clone());
        }
        self.view.doc.package.as_ref().and_then(|p| p.content_type(id)).map(str::to_string)
    }
}
