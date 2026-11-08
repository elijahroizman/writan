//! Presentations.

use office_slides::view::{Boxed, SlidesView, TextPatch};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::{err, json, Host, JsMeasurer};

#[wasm_bindgen]
pub struct SlidesDoc {
    view: SlidesView,
    host: Host,
    /// The last find's matches: (slide, frame id).
    matches: Vec<(usize, u32)>,
    query: String,
}

/// A command from the editor.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Cmd {
    SetActive { index: usize },
    Press { x: f64, y: f64, additive: bool },
    SelectFrames { ids: Vec<u32> },
    SelectIn { x0: f64, y0: f64, x1: f64, y1: f64, additive: bool },
    MoveFrames { ids: Vec<u32>, dx: f64, dy: f64 },
    SetBox { id: u32, x: f64, y: f64, w: f64, h: f64, rot: f64 },
    Flip { horizontal: bool },
    Crop { id: u32, left: f64, top: f64, right: f64, bottom: f64, x: f64, y: f64, w: f64, h: f64 },
    Delete,
    Duplicate,
    PasteFrames { json: String },
    Restack { how: String },
    Align { how: String },
    Distribute { horizontal: bool },
    Group,
    Ungroup,
    TextStyle { patch: TextPatchIn },
    TableTextStyle { patch: TextPatchIn },
    Link { id: u32, url: Option<String> },
    Bullets { kind: String },
    Fill { hex: Option<String> },
    Line { hex: Option<String>, width: Option<f64> },
    InsertTextBox,
    InsertShape { preset: String },
    InsertTable { rows: u32, cols: u32 },
    Table { op: String, value: Option<String> },
    AddSlide { after: usize, layout: Option<String> },
    AddTemplate { after: usize, template: String },
    DuplicateSlides { indexes: Vec<usize> },
    DeleteSlides { indexes: Vec<usize> },
    MoveSlide { from: usize, to: usize },
    ToggleHidden { indexes: Vec<usize> },
    ApplyLayout { layout: String },
    Notes { index: usize, text: String },
    Transition { indexes: Vec<usize>, transition: Option<office_slides::Transition> },
    BeginEdit { id: u32, x: Option<f64>, y: Option<f64> },
    EndEdit,
    TextClick { x: f64, y: f64, extend: bool, count: u32 },
    InsertText { text: String },
    Backspace { word: bool },
    DeleteText { word: bool },
    Enter,
    LineBreak,
    Tab { shift: bool },
    SelectAllText,
    MoveCaret { to: String, extend: bool },
    Undo,
    Redo,
    BeginGroup,
    EndGroup,
    MarkSaved,
    Find { query: String },
    ReplaceMatch { index: usize, text: String },
    ReplaceAll { text: String },
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TextPatchIn {
    font_family: Option<String>,
    font_size: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strikethrough: Option<bool>,
    color: Option<String>,
    align: Option<String>,
}

fn open_view(bytes: &[u8]) -> office_core::Result<SlidesView> {
    if is_compound_file(bytes) {
        let pptx = legacy_ppt_to_pptx(bytes)?;
        SlidesView::open(&pptx)
    } else {
        SlidesView::open(bytes)
    }
}

fn is_compound_file(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0])
}

fn legacy_ppt_to_pptx(bytes: &[u8]) -> office_core::Result<Vec<u8>> {
    let doc = office_oxide::Document::from_reader(std::io::Cursor::new(bytes.to_vec()), office_oxide::DocumentFormat::Ppt)
        .map_err(|e| office_core::Error::Invalid(format!("couldn't read this PowerPoint file: {e}")))?;
    if doc.format() != office_oxide::DocumentFormat::Ppt {
        return Err(office_core::Error::Invalid("this isn't a PowerPoint file".into()));
    }
    let ir = doc.to_ir();
    let mut out = std::io::Cursor::new(Vec::new());
    office_oxide::create::create_from_ir_to_writer(&ir, office_oxide::DocumentFormat::Pptx, &mut out)
        .map_err(|e| office_core::Error::Invalid(format!("couldn't convert this PowerPoint file to .pptx: {e}")))?;
    Ok(out.into_inner())
}

#[wasm_bindgen]
impl SlidesDoc {
    /// Opens a PowerPoint file, or makes a new deck when `bytes` is empty.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], host: Host) -> Result<SlidesDoc, JsError> {
        let view = if bytes.is_empty() { SlidesView::blank() } else { open_view(bytes).map_err(err)? };
        Ok(SlidesDoc { view, host, matches: Vec::new(), query: String::new() })
    }

    /// `{ width, height }` of a slide, in pixels.
    pub fn size(&self) -> String {
        let (w, h) = self.view.size();
        json(&serde_json::json!({ "width": w, "height": h }))
    }

    /// Every slide: id, hidden, layout, notes, transition, revision.
    pub fn slides(&self) -> String {
        json(&self.view.slide_infos())
    }

    pub fn theme(&self) -> String {
        let theme = self.view.doc.theme.clone().unwrap_or_else(office_core::drawing::Theme::office);
        let colors: std::collections::BTreeMap<_, _> = office_core::drawing::theme::SLOTS
            .iter()
            .map(|slot| (*slot, format!("#{}", theme.scheme_color(slot).unwrap_or(office_core::drawing::Rgb::BLACK).hex())))
            .collect();
        json(&serde_json::json!({ "heading": theme.fonts.major.latin, "body": theme.fonts.minor.latin, "colors": colors }))
    }

    pub fn layouts(&self) -> String {
        json(&self.view.layout_infos())
    }

    /// Draws a slide: `{ width, height, items, styles }` (styles: paint styles not sent before).
    pub fn paint(&mut self, index: usize) -> String {
        let mut m = JsMeasurer(&self.host);
        let p = self.view.paint(&mut m, index);
        let styles = self.view.new_styles();
        json(&serde_json::json!({ "width": p.width, "height": p.height, "items": p.items, "styles": styles }))
    }

    /// The frames on a slide, as the inspector reads them.
    pub fn elements(&mut self, index: usize) -> String {
        let mut m = JsMeasurer(&self.host);
        json(&self.view.elements(&mut m, index))
    }

    /// `{ active, frames, editing, canUndo, canRedo, dirty, boxes, text }`: what's selected, the
    /// selected frames' boxes, and the caret and selection while text is edited.
    pub fn state(&mut self) -> String {
        let mut m = JsMeasurer(&self.host);
        let boxes: Vec<_> = self
            .view
            .selected_boxes(&mut m)
            .into_iter()
            .map(|(id, b)| serde_json::json!({ "id": id.to_string(), "x": b.x, "y": b.y, "w": b.w, "h": b.h, "rot": b.rot }))
            .collect();
        let text = self.view.text_overlay(&mut m);
        let sel = self.view.selection();
        let restack: Vec<&str> = ["front", "forward", "backward", "back"].into_iter().filter(|h| self.view.can_restack(h)).collect();
        json(&serde_json::json!({
            "active": sel.active,
            "frames": sel.frames.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
            "editing": sel.editing.as_ref().map(|e| e.frame.to_string()),
            "canUndo": self.view.can_undo(),
            "canRedo": self.view.can_redo(),
            "dirty": self.view.is_dirty(),
            "boxes": boxes,
            "text": text,
            "restack": restack,
        }))
    }

    /// The topmost frame at a slide point, if any.
    #[wasm_bindgen(js_name = frameAt)]
    pub fn frame_at(&mut self, x: f64, y: f64) -> Option<String> {
        let mut m = JsMeasurer(&self.host);
        self.view.frame_at(&mut m, x, y).map(|id| id.to_string())
    }

    /// The selected frames, for the clipboard.
    #[wasm_bindgen(js_name = copyFrames)]
    pub fn copy_frames(&self) -> String {
        self.view.copy_frames()
    }

    /// The selected text while editing.
    #[wasm_bindgen(js_name = copyText)]
    pub fn copy_text(&self) -> String {
        self.view.copy_text()
    }

    /// Puts a picture in the middle of the active slide.
    #[wasm_bindgen(js_name = insertImage)]
    pub fn insert_image(&mut self, bytes: Vec<u8>, content_type: &str, width: f64, height: f64, name: &str) -> String {
        self.view.insert_image(bytes, content_type, width, height, name).to_string()
    }

    pub fn dirty(&self) -> bool {
        self.view.is_dirty()
    }

    #[wasm_bindgen(js_name = fontsChanged)]
    pub fn fonts_changed(&mut self) {
        self.view.fonts_changed();
    }

    /// Runs an editing command (JSON); returns its result as JSON.
    pub fn command(&mut self, cmd: &str) -> Result<String, JsError> {
        let cmd: Cmd = serde_json::from_str(cmd).map_err(err)?;
        let mut m = JsMeasurer(&self.host);
        let v = &mut self.view;
        let null = serde_json::Value::Null;
        let out = match cmd {
            Cmd::SetActive { index } => {
                v.set_active(index);
                null
            }
            Cmd::Press { x, y, additive } => serde_json::json!(v.press(&mut m, x, y, additive).map(|id| id.to_string())),
            Cmd::SelectFrames { ids } => {
                v.select_frames(ids);
                null
            }
            Cmd::SelectIn { x0, y0, x1, y1, additive } => {
                v.select_in(&mut m, x0, y0, x1, y1, additive);
                null
            }
            Cmd::MoveFrames { ids, dx, dy } => {
                v.move_frames(&mut m, &ids, dx, dy);
                null
            }
            Cmd::SetBox { id, x, y, w, h, rot } => {
                v.set_box(&mut m, id, Boxed { x, y, w, h, rot, ..Default::default() });
                null
            }
            Cmd::Crop { id, left, top, right, bottom, x, y, w, h } => {
                v.crop_picture(id, [left, top, right, bottom], Boxed { x, y, w, h, ..Default::default() });
                null
            }
            Cmd::Flip { horizontal } => {
                v.flip(&mut m, horizontal);
                null
            }
            Cmd::Delete => {
                v.delete_frames();
                null
            }
            Cmd::Duplicate => {
                v.duplicate_frames();
                null
            }
            Cmd::PasteFrames { json: j } => serde_json::json!(v.paste_frames(&j)),
            Cmd::Restack { how } => {
                v.restack(&how);
                null
            }
            Cmd::Align { how } => {
                v.align_frames(&mut m, &how);
                null
            }
            Cmd::Distribute { horizontal } => {
                v.distribute(&mut m, horizontal);
                null
            }
            Cmd::Group => {
                v.group(&mut m);
                null
            }
            Cmd::Ungroup => {
                v.ungroup();
                null
            }
            Cmd::TextStyle { patch: p } => {
                v.text_style(&TextPatch {
                    font_family: p.font_family,
                    font_size: p.font_size,
                    bold: p.bold,
                    italic: p.italic,
                    underline: p.underline,
                    strikethrough: p.strikethrough,
                    color: p.color,
                    align: p.align,
                });
                null
            }
            Cmd::TableTextStyle { patch: p } => {
                v.table_text_style(&TextPatch {
                    font_family: p.font_family,
                    font_size: p.font_size,
                    bold: p.bold,
                    italic: p.italic,
                    underline: p.underline,
                    strikethrough: p.strikethrough,
                    color: p.color,
                    align: p.align,
                });
                null
            }
            Cmd::Link { id, url } => {
                v.set_link(id, url.as_deref());
                null
            }
            Cmd::Bullets { kind } => {
                v.toggle_bullets(&kind);
                null
            }
            Cmd::Fill { hex } => {
                v.set_fill(hex.as_deref());
                null
            }
            Cmd::Line { hex, width } => {
                v.set_line(hex.as_deref(), width);
                null
            }
            Cmd::InsertTextBox => serde_json::json!(v.insert_text_box().to_string()),
            Cmd::InsertShape { preset } => serde_json::json!(v.insert_shape(&preset).to_string()),
            Cmd::InsertTable { rows, cols } => serde_json::json!(v.insert_table(rows, cols).to_string()),
            Cmd::Table { op, value } => {
                v.table_op(&op, value.as_deref());
                null
            }
            Cmd::AddSlide { after, layout } => {
                v.add_slide(after, layout.as_deref());
                null
            }
            Cmd::AddTemplate { after, template } => {
                v.add_template(after, &template);
                null
            }
            Cmd::DuplicateSlides { indexes } => {
                v.duplicate_slides(&indexes);
                null
            }
            Cmd::DeleteSlides { indexes } => {
                v.delete_slides(&indexes);
                null
            }
            Cmd::MoveSlide { from, to } => {
                v.move_slide(from, to);
                null
            }
            Cmd::ToggleHidden { indexes } => {
                v.toggle_hidden(&indexes);
                null
            }
            Cmd::ApplyLayout { layout } => {
                v.apply_layout(&layout);
                null
            }
            Cmd::Notes { index, text } => {
                v.set_notes(index, &text);
                null
            }
            Cmd::Transition { indexes, transition } => {
                v.set_transition(&indexes, transition);
                null
            }
            Cmd::BeginEdit { id, x, y } => serde_json::json!(v.begin_edit(&mut m, id, x.zip(y))),
            Cmd::EndEdit => {
                v.end_edit();
                null
            }
            Cmd::TextClick { x, y, extend, count } => {
                v.text_click(&mut m, x, y, extend, count);
                null
            }
            Cmd::InsertText { text } => {
                v.insert_text(&text);
                null
            }
            Cmd::Backspace { word } => {
                v.backspace(word);
                null
            }
            Cmd::DeleteText { word } => {
                v.delete_forward(word);
                null
            }
            Cmd::Enter => {
                v.enter();
                null
            }
            Cmd::LineBreak => {
                v.line_break();
                null
            }
            Cmd::Tab { shift } => {
                v.tab(shift);
                null
            }
            Cmd::SelectAllText => {
                v.select_all_text();
                null
            }
            Cmd::MoveCaret { to, extend } => {
                v.move_caret(&mut m, &to, extend);
                null
            }
            Cmd::Undo => serde_json::json!(v.undo()),
            Cmd::Redo => serde_json::json!(v.redo()),
            Cmd::BeginGroup => {
                v.begin_group();
                null
            }
            Cmd::EndGroup => {
                v.end_group();
                null
            }
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
                    .map(|(s, f)| serde_json::json!({ "slide": s, "id": f.to_string() }))
                    .collect::<Vec<_>>())
            }
            Cmd::ReplaceMatch { index, text } => {
                if let Some(mt) = self.matches.get(index).copied() {
                    v.replace(&[mt], &self.query, &text);
                }
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

    pub fn save(&mut self) -> Result<Vec<u8>, JsError> {
        self.view.save().map_err(err)
    }

    pub fn media(&self, id: &str) -> Option<Vec<u8>> {
        self.view.media(id)
    }

    #[wasm_bindgen(js_name = mediaType)]
    pub fn media_type(&self, id: &str) -> Option<String> {
        self.view.media_type(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Entry {
        name: &'static str,
        kind: u8,
        right: u32,
        child: u32,
        data: Vec<u8>,
    }

    const NO_ENTRY: u32 = 0xFFFF_FFFF;

    fn rec(rec_type: u16, ver: u16, data: &[u8]) -> Vec<u8> {
        let mut b = ver.to_le_bytes().to_vec();
        b.extend_from_slice(&rec_type.to_le_bytes());
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
        b
    }

    fn slide_stream(text: &str) -> Vec<u8> {
        let mut textbox = rec(0x0F9F, 0, &0u32.to_le_bytes());
        textbox.extend(rec(0x0FA8, 0, text.as_bytes()));
        let textbox = rec(0xF00D, 0x000F, &textbox);
        rec(0x03EE, 0x000F, &textbox)
    }

    fn build_cfb(entries: &[Entry]) -> Vec<u8> {
        const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
        const FAT_SECT: u32 = 0xFFFF_FFFD;
        const FREE_SECT: u32 = 0xFFFF_FFFF;
        let streams: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| e.kind == 2).map(|(i, _)| i).collect();
        let sectors = 2 + streams.len();
        let mut file = vec![0u8; 512 * (1 + sectors)];
        file[0..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
        file[0x18..0x1A].copy_from_slice(&0x003Eu16.to_le_bytes());
        file[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
        file[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
        file[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
        file[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
        file[0x2C..0x30].copy_from_slice(&1u32.to_le_bytes());
        file[0x38..0x3C].copy_from_slice(&4096u32.to_le_bytes());
        file[0x3C..0x40].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        file[0x44..0x48].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        file[0x4C..0x50].copy_from_slice(&1u32.to_le_bytes());
        for i in 1..109 {
            file[0x4C + i * 4..0x50 + i * 4].copy_from_slice(&FREE_SECT.to_le_bytes());
        }
        let fat = 1024;
        let mut fat_entries = vec![END_OF_CHAIN, FAT_SECT];
        for (n, e) in entries.iter().enumerate() {
            let buf = &mut file[512 + n * 128..512 + (n + 1) * 128];
            for (i, unit) in e.name.encode_utf16().enumerate() {
                buf[i * 2..i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
            }
            let name_size = ((e.name.encode_utf16().count() + 1) * 2) as u16;
            buf[0x40..0x42].copy_from_slice(&name_size.to_le_bytes());
            buf[0x42] = if n == 0 { 5 } else { e.kind };
            buf[0x43] = 1;
            buf[0x44..0x48].copy_from_slice(&NO_ENTRY.to_le_bytes());
            buf[0x48..0x4C].copy_from_slice(&e.right.to_le_bytes());
            buf[0x4C..0x50].copy_from_slice(&e.child.to_le_bytes());
            let (start, size) = if e.kind == 2 {
                let sector = 2 + streams.iter().position(|&s| s == n).unwrap();
                let off = 512 + sector * 512;
                file[off..off + e.data.len()].copy_from_slice(&e.data);
                fat_entries.push(END_OF_CHAIN);
                (sector as u32, e.data.len() as u32)
            } else {
                (END_OF_CHAIN, 0)
            };
            let buf = &mut file[512 + n * 128..512 + (n + 1) * 128];
            buf[0x74..0x78].copy_from_slice(&start.to_le_bytes());
            buf[0x78..0x7C].copy_from_slice(&size.to_le_bytes());
        }
        for i in 0..128 {
            let v = fat_entries.get(i).copied().unwrap_or(FREE_SECT);
            file[fat + i * 4..fat + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        file
    }

    fn minimal_ppt(text: &str) -> Vec<u8> {
        build_cfb(&[
            Entry { name: "Root Entry", kind: 5, right: NO_ENTRY, child: 1, data: vec![] },
            Entry { name: "PowerPoint Document", kind: 2, right: NO_ENTRY, child: NO_ENTRY, data: slide_stream(text) },
        ])
    }

    #[test]
    fn legacy_ppt_converts_to_a_readable_pptx() {
        let ppt = minimal_ppt("Legacy slide");
        let pptx = legacy_ppt_to_pptx(&ppt).unwrap();
        assert!(pptx.starts_with(b"PK"));
        let mut view = open_view(&ppt).unwrap();
        assert_eq!(view.slide_infos().len(), 1);
        let saved = view.save().unwrap();
        assert!(saved.starts_with(b"PK"));
        assert_eq!(SlidesView::open(&saved).unwrap().slide_infos().len(), 1);
    }
}
