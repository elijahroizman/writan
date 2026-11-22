//! One-click document conversion between Office formats, clean LLM-friendly Markdown, and plain text.

use std::collections::HashMap;

use office_core::convert::{plain_text, to_kind};
use office_core::model::{
    Block, Body, BreakKind, Document, Frame, FrameContent, Inline, Kind, Link, ParaProps, Paragraph, Run, RunProps, Slide, Strike, Table,
    Value,
};
use office_core::units::{Emu, FontSize};
use office_core::xml::{ns, Element};
use office_core::Compiler;
use office_sheets::Sheets;
use office_slides::Decks;
use office_word::stories::NoteKind;
use office_word::view::math;
use office_word::{ListKind, Words};
use wasm_bindgen::prelude::*;

use crate::err;
use crate::slides::{is_compound_file, legacy_ppt_to_pptx};

/// Converts a document (`docx`, `pptx`, `ppt`, `xlsx`, `xlsm`, `csv`, `tsv`, `md`, `txt`) into
/// `target` (`docx`, `pptx`, `xlsx`, `csv`, `md`, `txt`).
#[wasm_bindgen(js_name = convertDocument)]
pub fn convert_document(bytes: &[u8], name: &str, target: &str) -> Result<Vec<u8>, JsError> {
    convert_inner(bytes, name, target).map_err(err)
}

/// Exports a document (`docx`, `pptx`, `ppt`, `xlsx`, `xlsm`, `csv`, `tsv`, `md`, `txt`) as clean
/// Markdown suitable for feeding to LLMs.
#[wasm_bindgen(js_name = toMarkdown)]
pub fn to_markdown(bytes: &[u8], name: &str) -> Result<String, JsError> {
    to_markdown_inner(bytes, name).map_err(err)
}

fn compiler() -> Compiler {
    Compiler::new().with_layer(Words).with_layer(Decks).with_layer(Sheets)
}

fn ext_lower(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn open_document(bytes: &[u8], name: &str) -> office_core::Result<Document> {
    let ext = ext_lower(name);
    if matches!(ext.as_str(), "md" | "markdown") {
        let text = String::from_utf8_lossy(bytes);
        return Ok(markdown_to_word(&text));
    }
    if ext == "txt" {
        let text = String::from_utf8_lossy(bytes);
        return Ok(plain_text_to_word(&text));
    }
    let comp = compiler();
    let mut doc = if is_compound_file(bytes) {
        let pptx = legacy_ppt_to_pptx(bytes)?;
        comp.read(&pptx, Some("presentation.pptx"))?
    } else {
        comp.read(bytes, Some(name))?
    };
    if doc.kind == Kind::Sheets {
        recalculate_missing_formulas(&mut doc);
    }
    Ok(doc)
}

fn cell_has_formula(c: &office_core::model::SheetCell) -> bool {
    c.formula.is_some() || c.origin.get().is_some_and(|el| el.child(ns::X, "f").is_some())
}

fn recalculate_missing_formulas(doc: &mut Document) {
    let has_uncached = match &doc.body {
        Body::Workbook { sheets } => {
            sheets.iter().any(|s| s.rows.iter().any(|r| r.cells.iter().any(|c| cell_has_formula(c) && matches!(c.value, Value::Empty))))
        }
        _ => false,
    };
    if !has_uncached {
        return;
    }
    let saved: Vec<(usize, u32, u32, Value)> = match &doc.body {
        Body::Workbook { sheets } => sheets
            .iter()
            .enumerate()
            .flat_map(|(si, s)| {
                s.rows.iter().flat_map(move |r| {
                    let row_idx = r.index;
                    r.cells
                        .iter()
                        .filter(|c| cell_has_formula(c) && !matches!(c.value, Value::Empty))
                        .map(move |c| (si, row_idx, c.col, c.value.clone()))
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    office_sheets::recalculate(doc, None);
    if let Body::Workbook { sheets } = &mut doc.body {
        for (si, row_idx, col, val) in saved {
            if let Some(cell) = sheets
                .get_mut(si)
                .and_then(|s| s.rows.iter_mut().find(|r| r.index == row_idx))
                .and_then(|r| r.cells.iter_mut().find(|c| c.col == col))
            {
                if matches!(cell.value, Value::Error(_) | Value::Empty) {
                    cell.value = val;
                }
            }
        }
    }
}

fn convert_inner(bytes: &[u8], name: &str, target: &str) -> office_core::Result<Vec<u8>> {
    match target {
        "md" | "markdown" => Ok(to_markdown_inner(bytes, name)?.into_bytes()),
        "txt" => {
            let ext = ext_lower(name);
            if matches!(ext.as_str(), "txt" | "md" | "markdown") {
                return Ok(bytes.to_vec());
            }
            let doc = open_document(bytes, name)?;
            Ok(plain_text(&doc)?.into_bytes())
        }
        "docx" | "pptx" | "xlsx" | "csv" => {
            let kind = match target {
                "docx" => Kind::Word,
                "pptx" => Kind::Slides,
                "xlsx" => Kind::Sheets,
                "csv" => Kind::Csv,
                _ => unreachable!(),
            };
            let doc = open_document(bytes, name)?;
            let converted = if doc.kind == kind { doc } else { to_kind(&doc, kind)? };
            compiler().write(&converted, kind)
        }
        other => Err(office_core::Error::Invalid(format!("unsupported target format: {other}"))),
    }
}

fn to_markdown_inner(bytes: &[u8], name: &str) -> office_core::Result<String> {
    let ext = ext_lower(name);
    if matches!(ext.as_str(), "md" | "markdown") {
        return Ok(normalize_markdown(&String::from_utf8_lossy(bytes)));
    }
    if ext == "txt" {
        return Ok(normalize_markdown(&String::from_utf8_lossy(bytes)));
    }
    let doc = open_document(bytes, name)?;
    document_to_markdown(&doc)
}

/// Converts any parsed Office [`Document`] to clean, LLM-friendly Markdown.
pub fn document_to_markdown(doc: &Document) -> office_core::Result<String> {
    let raw = match &doc.body {
        Body::Flow { blocks } => word_to_markdown(doc, blocks),
        Body::Slides { slides, .. } => slides_to_markdown(doc, slides),
        Body::Workbook { sheets } => sheets_to_markdown(sheets)?,
    };
    Ok(normalize_markdown(&raw))
}

// ── Word → Markdown

fn word_to_markdown(doc: &Document, blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut list_counters: HashMap<(String, u8), usize> = HashMap::new();
    let mut prev_was_list = false;
    let mut in_code_block = false;
    let mut referenced_footnotes: Vec<String> = Vec::new();
    let mut referenced_endnotes: Vec<String> = Vec::new();

    for block in blocks {
        match block {
            Block::Paragraph(p) => {
                let heading = heading_level(doc, p);
                let list = if heading.is_none() { office_word::list_of(doc, p) } else { None };
                let is_code = heading.is_none() && list.is_none() && is_code_paragraph(doc, p);

                if is_code {
                    if prev_was_list {
                        out.push('\n');
                        prev_was_list = false;
                    }
                    if !in_code_block {
                        if !out.is_empty() && !out.ends_with("\n\n") {
                            out.push('\n');
                        }
                        out.push_str("```\n");
                        in_code_block = true;
                    }
                    let line = visible_paragraph_text(doc, p).replace('\u{c}', "");
                    out.push_str(line.trim_end());
                    out.push('\n');
                    continue;
                }

                if in_code_block {
                    out.push_str("```\n\n");
                    in_code_block = false;
                }

                let (text, has_page_break) =
                    render_paragraph_inlines(doc, p, heading.is_some(), false, &mut referenced_footnotes, &mut referenced_endnotes);
                let trimmed = text.trim();

                if trimmed.is_empty() {
                    if has_page_break && !out.is_empty() {
                        if prev_was_list {
                            out.push('\n');
                            prev_was_list = false;
                        }
                        out.push_str("---\n\n");
                    }
                    continue;
                }

                if let Some(level) = heading {
                    if prev_was_list {
                        out.push('\n');
                        prev_was_list = false;
                    }
                    let hashes = "#".repeat(level.clamp(1, 6) as usize);
                    out.push_str(&hashes);
                    out.push(' ');
                    out.push_str(trimmed);
                    out.push_str("\n\n");
                } else if let Some((num_id, level)) = list {
                    let ordered = is_ordered_list(doc, &num_id, level);
                    let indent = "  ".repeat(level.min(6) as usize);
                    let marker = if ordered {
                        let n = {
                            let counter = list_counters.entry((num_id.clone(), level)).or_insert(0);
                            *counter += 1;
                            *counter
                        };
                        // Reset deeper levels of the same list.
                        list_counters.retain(|(id, l), _| id != &num_id || *l <= level);
                        format!("{n}. ")
                    } else {
                        "- ".to_string()
                    };
                    out.push_str(&indent);
                    out.push_str(&marker);
                    out.push_str(&trimmed.replace('\n', &format!("\n{}  ", indent)));
                    out.push('\n');
                    prev_was_list = true;
                } else if is_quote_paragraph(doc, p) {
                    if prev_was_list {
                        out.push('\n');
                        prev_was_list = false;
                    }
                    for line in trimmed.lines() {
                        out.push_str("> ");
                        out.push_str(line.trim_end());
                        out.push('\n');
                    }
                    out.push('\n');
                } else {
                    if prev_was_list {
                        out.push('\n');
                        prev_was_list = false;
                    }
                    out.push_str(trimmed);
                    out.push_str("\n\n");
                }

                if has_page_break {
                    if prev_was_list {
                        out.push('\n');
                        prev_was_list = false;
                    }
                    out.push_str("---\n\n");
                }
            }
            Block::Table(table) => {
                if in_code_block {
                    out.push_str("```\n\n");
                    in_code_block = false;
                }
                if prev_was_list {
                    out.push('\n');
                    prev_was_list = false;
                }
                let md = table_to_markdown(Some(doc), table);
                if !md.is_empty() {
                    out.push_str(&md);
                    out.push_str("\n\n");
                }
            }
            Block::Raw { xml, .. } => {
                if in_code_block {
                    out.push_str("```\n\n");
                    in_code_block = false;
                }
                if let Some(extra) = raw_block_markdown(xml, &mut referenced_footnotes, &mut referenced_endnotes) {
                    let trimmed = extra.trim();
                    if !trimmed.is_empty() {
                        if prev_was_list {
                            out.push('\n');
                            prev_was_list = false;
                        }
                        out.push_str(trimmed);
                        out.push_str("\n\n");
                    }
                }
            }
        }
    }

    if in_code_block {
        out.push_str("```\n");
    }

    // Append any referenced footnotes / endnotes cleanly at the bottom.
    if let Some(w) = office_word::word(doc) {
        let mut notes_out = Vec::new();
        for id in &referenced_footnotes {
            if let Some(note) = w.notes.iter().find(|n| n.kind == NoteKind::Footnote && &n.id == id) {
                let body = note_blocks_text(doc, &note.blocks);
                if !body.is_empty() {
                    notes_out.push(format!("[^{id}]: {body}"));
                }
            }
        }
        for id in &referenced_endnotes {
            if let Some(note) = w.notes.iter().find(|n| n.kind == NoteKind::Endnote && &n.id == id) {
                let body = note_blocks_text(doc, &note.blocks);
                if !body.is_empty() {
                    notes_out.push(format!("[^e{id}]: {body}"));
                }
            }
        }
        if !notes_out.is_empty() {
            out.push('\n');
            out.push_str(&notes_out.join("\n"));
            out.push('\n');
        }
    }

    out
}

fn note_blocks_text(doc: &Document, blocks: &[Block]) -> String {
    let mut dummy_fn = Vec::new();
    let mut dummy_en = Vec::new();
    let mut parts = Vec::new();
    for b in blocks {
        if let Block::Paragraph(p) = b {
            let (t, _) = render_paragraph_inlines(doc, p, false, true, &mut dummy_fn, &mut dummy_en);
            let t = t.trim();
            if !t.is_empty() {
                parts.push(t.to_string());
            }
        }
    }
    parts.join(" ")
}

fn heading_level(doc: &Document, p: &Paragraph) -> Option<u8> {
    if let Some(style_id) = p.props.style.as_deref() {
        if let Some(lvl) = style_name_to_heading(style_id) {
            return Some(lvl);
        }
        if let Some(w) = office_word::word(doc) {
            for s in w.styles.chain(style_id) {
                if let Some(lvl) = style_name_to_heading(&s.id).or_else(|| style_name_to_heading(&s.name)) {
                    return Some(lvl);
                }
            }
        }
    }
    // Fall back to explicit w:outlineLvl (0-based, 0..=5 -> H1..H6).
    let lvl = p
        .origin
        .get()
        .and_then(|o| o.path(ns::W, &["pPr", "outlineLvl"]))
        .and_then(office_core::formats::docx::val)
        .and_then(|v| v.parse::<u8>().ok());
    lvl.filter(|&l| l < 6).map(|l| l + 1)
}

fn style_name_to_heading(name: &str) -> Option<u8> {
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "title" => Some(1),
        "subtitle" => Some(2),
        "heading1" | "heading 1" => Some(1),
        "heading2" | "heading 2" => Some(2),
        "heading3" | "heading 3" => Some(3),
        "heading4" | "heading 4" => Some(4),
        "heading5" | "heading 5" => Some(5),
        "heading6" | "heading 6" => Some(6),
        _ => None,
    }
}

fn is_ordered_list(doc: &Document, num_id: &str, level: u8) -> bool {
    let Some(w) = office_word::word(doc) else { return false };
    let Some(levels) = w.numbering.levels(num_id) else { return false };
    let Some(lvl) = levels.get(level as usize) else { return false };
    !matches!(lvl.format.as_str(), "bullet" | "none" | "")
}

fn is_quote_paragraph(doc: &Document, p: &Paragraph) -> bool {
    let check = |s: &str| {
        let l = s.to_ascii_lowercase();
        l == "quote" || l == "intensequote" || l == "intense quote" || l == "blocktext" || l == "block text"
    };
    let Some(style_id) = p.props.style.as_deref() else { return false };
    if check(style_id) {
        return true;
    }
    office_word::word(doc).is_some_and(|w| w.styles.chain(style_id).iter().any(|s| check(&s.id) || check(&s.name)))
}

fn is_monospace_font(font: Option<&str>) -> bool {
    let Some(f) = font else { return false };
    let l = f.to_ascii_lowercase();
    l.contains("mono")
        || l.contains("courier")
        || l.contains("consolas")
        || l.contains("menlo")
        || l.contains("fira code")
        || l.contains("source code")
        || l.contains("inconsolata")
        || l.contains("lucida console")
}

fn visible_paragraph_text(doc: &Document, p: &Paragraph) -> String {
    let mut out = String::new();
    for inline in &p.content {
        match inline {
            Inline::Text(r) => {
                if office_word::resolved_word_run(doc, p, r).toggles.vanish == Some(true) {
                    continue;
                }
                out.push_str(&r.text);
            }
            Inline::Break { .. } | Inline::Image(_) | Inline::Raw { .. } => {}
        }
    }
    out
}

fn is_code_paragraph(doc: &Document, p: &Paragraph) -> bool {
    if let Some(style_id) = p.props.style.as_deref() {
        let l = style_id.to_ascii_lowercase();
        if l == "code" || l == "codeblock" || l == "htmlpreformatted" || l == "sourcetext" {
            return true;
        }
    }
    let mut has_text = false;
    for r in p.runs() {
        if r.text.trim().is_empty() {
            continue;
        }
        has_text = true;
        let resolved = office_word::resolved_word_run(doc, p, r);
        if !is_monospace_font(resolved.props.font.as_deref()) {
            return false;
        }
    }
    has_text
}

#[derive(Clone, PartialEq, Eq)]
struct InlineFmt {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

fn render_paragraph_inlines(
    doc: &Document,
    p: &Paragraph,
    in_heading: bool,
    in_table: bool,
    footnotes: &mut Vec<String>,
    endnotes: &mut Vec<String>,
) -> (String, bool) {
    let mut out = String::new();
    let mut has_page_break = false;

    // Coalesce adjacent text runs with identical Markdown formatting.
    let mut cur_fmt: Option<InlineFmt> = None;
    let mut cur_buf = String::new();

    let flush = |out: &mut String, cur_fmt: &mut Option<InlineFmt>, cur_buf: &mut String| {
        if cur_buf.is_empty() {
            *cur_fmt = None;
            return;
        }
        if let Some(fmt) = cur_fmt.take() {
            out.push_str(&format_inline_span(cur_buf, &fmt));
        } else {
            out.push_str(cur_buf);
        }
        cur_buf.clear();
    };

    for inline in &p.content {
        match inline {
            Inline::Text(r) => {
                let resolved = office_word::resolved_word_run(doc, p, r);
                if resolved.toggles.vanish == Some(true) {
                    continue;
                }
                let bold = !in_heading && resolved.props.bold == Some(true);
                let italic = resolved.props.italic == Some(true);
                let strike = matches!(resolved.props.strike, Some(Strike::Single | Strike::Double))
                    || resolved.toggles.strike == Some(true)
                    || resolved.toggles.double_strike == Some(true);
                let code = is_monospace_font(resolved.props.font.as_deref());
                let link = r.link.as_ref().and_then(|l| l.url.clone()).filter(|u| !u.is_empty());
                let fmt = InlineFmt { bold, italic, strike, code, link };

                let text = if in_table { r.text.replace('\n', "<br>").replace('\t', " ") } else { r.text.replace('\t', " ") };

                if cur_fmt.as_ref() == Some(&fmt) {
                    cur_buf.push_str(&text);
                } else {
                    flush(&mut out, &mut cur_fmt, &mut cur_buf);
                    cur_fmt = Some(fmt);
                    cur_buf.push_str(&text);
                }
            }
            Inline::Break { kind: BreakKind::Page, .. } => {
                flush(&mut out, &mut cur_fmt, &mut cur_buf);
                has_page_break = true;
            }
            Inline::Break { kind: BreakKind::Column, .. } => {
                flush(&mut out, &mut cur_fmt, &mut cur_buf);
                out.push(if in_table { ' ' } else { '\n' });
            }
            Inline::Image(img) => {
                if let Some(desc) = img.description.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    flush(&mut out, &mut cur_fmt, &mut cur_buf);
                    if let Some(math_md) = latex_tag_markdown(desc) {
                        out.push_str(&math_md);
                    } else {
                        out.push_str(&format!("![{}]()", desc.replace(['[', ']'], "")));
                    }
                }
            }
            Inline::Raw { xml } => {
                if let Some(extra) = raw_inline_markdown(xml, footnotes, endnotes) {
                    flush(&mut out, &mut cur_fmt, &mut cur_buf);
                    out.push_str(&extra);
                }
            }
        }
    }
    flush(&mut out, &mut cur_fmt, &mut cur_buf);
    (out, has_page_break)
}

fn latex_tag_markdown(desc: &str) -> Option<String> {
    let raw = desc.strip_prefix("writan:latex:")?.trim();
    let val: serde_json::Value = serde_json::from_str(raw).ok()?;
    let latex = val.get("latex")?.as_str()?.trim();
    if latex.is_empty() {
        return None;
    }
    let display = val.get("display").and_then(serde_json::Value::as_bool).unwrap_or(false);
    Some(if display { format!("$${latex}$$") } else { format!("${latex}$") })
}

fn raw_block_markdown(el: &Element, footnotes: &mut Vec<String>, endnotes: &mut Vec<String>) -> Option<String> {
    if el.is(ns::W, "del") || el.is(ns::W, "moveFrom") {
        return None;
    }
    if el.is(ns::W, "sdt") {
        if let Some(content) = el.child(ns::W, "sdtContent") {
            let mut paras = Vec::new();
            for c in content.elements() {
                if let Some(s) = raw_block_markdown(c, footnotes, endnotes) {
                    let t = s.trim();
                    if !t.is_empty() {
                        paras.push(t.to_string());
                    }
                }
            }
            return (!paras.is_empty()).then(|| paras.join("\n\n"));
        }
        return None;
    }
    if el.is(ns::W, "p") {
        let mut line = String::new();
        for c in el.elements() {
            if let Some(s) = raw_inline_markdown(c, footnotes, endnotes) {
                line.push_str(&s);
            }
        }
        return (!line.trim().is_empty()).then_some(line);
    }
    raw_inline_markdown(el, footnotes, endnotes)
}

fn raw_run_is_vanished(run_el: &Element) -> bool {
    run_el
        .child(ns::W, "rPr")
        .and_then(|rpr| rpr.child(ns::W, "vanish"))
        .is_some_and(|v| !matches!(v.attr_ns(ns::W, "val"), Some("0" | "false" | "off")))
}

fn raw_inline_markdown(el: &Element, footnotes: &mut Vec<String>, endnotes: &mut Vec<String>) -> Option<String> {
    if el.is(ns::W, "del") || el.is(ns::W, "moveFrom") {
        return None;
    }
    if math::is_math(el) {
        let eq = math::linear(el);
        let eq = eq.trim();
        if eq.is_empty() {
            return None;
        }
        return Some(if el.is(ns::M, "oMathPara") { format!("$${eq}$$") } else { format!("${eq}$") });
    }
    if let Some(r) = el.child(ns::W, "footnoteReference").or_else(|| el.is(ns::W, "footnoteReference").then_some(el)) {
        if let Some(id) = r.attr_ns(ns::W, "id") {
            if !footnotes.iter().any(|x| x == id) {
                footnotes.push(id.to_string());
            }
            return Some(format!("[^{id}]"));
        }
    }
    if let Some(r) = el.child(ns::W, "endnoteReference").or_else(|| el.is(ns::W, "endnoteReference").then_some(el)) {
        if let Some(id) = r.attr_ns(ns::W, "id") {
            if !endnotes.iter().any(|x| x == id) {
                endnotes.push(id.to_string());
            }
            return Some(format!("[^e{id}]"));
        }
    }
    if el.is(ns::W, "r") {
        if raw_run_is_vanished(el) {
            return None;
        }
        let mut text = String::new();
        for c in el.elements() {
            if c.is(ns::W, "t") {
                text.push_str(&c.text());
            } else if c.is(ns::W, "tab") {
                text.push(' ');
            } else if c.is(ns::W, "br") || c.is(ns::W, "cr") {
                text.push('\n');
            }
        }
        return (!text.is_empty()).then_some(text);
    }
    // Check inside content controls (w:sdt/w:sdtContent) for equations and visible text.
    if el.is(ns::W, "sdt") {
        if let Some(content) = el.child(ns::W, "sdtContent") {
            let mut out = String::new();
            for c in content.elements() {
                if let Some(s) = raw_inline_markdown(c, footnotes, endnotes) {
                    out.push_str(&s);
                }
            }
            return (!out.is_empty()).then_some(out);
        }
        return None;
    }
    if el.is(ns::W, "ins") || el.is(ns::W, "smartTag") || el.is(ns::W, "hyperlink") || el.is(ns::W, "moveTo") {
        let mut out = String::new();
        for c in el.elements() {
            if let Some(s) = raw_inline_markdown(c, footnotes, endnotes) {
                out.push_str(&s);
            }
        }
        return (!out.is_empty()).then_some(out);
    }
    None
}

fn format_inline_span(raw: &str, fmt: &InlineFmt) -> String {
    if raw.trim().is_empty() {
        return raw.to_string();
    }
    let lead_len = raw.len() - raw.trim_start().len();
    let trail_len = raw.len() - raw.trim_end().len();
    let lead = &raw[..lead_len];
    let trail = &raw[raw.len() - trail_len..];
    let core = &raw[lead_len..raw.len() - trail_len];

    let mut inner = if fmt.code {
        let ticks = if core.contains('`') { "``" } else { "`" };
        let pad = if core.starts_with('`') || core.ends_with('`') { " " } else { "" };
        format!("{ticks}{pad}{core}{pad}{ticks}")
    } else {
        let mut s = core.to_string();
        if fmt.strike {
            s = format!("~~{s}~~");
        }
        if fmt.bold && fmt.italic {
            s = format!("***{s}***");
        } else if fmt.bold {
            s = format!("**{s}**");
        } else if fmt.italic {
            s = format!("*{s}*");
        }
        s
    };

    if let Some(url) = &fmt.link {
        inner = format!("[{inner}]({url})");
    }

    format!("{lead}{inner}{trail}")
}

fn table_to_markdown(doc: Option<&Document>, table: &Table) -> String {
    if table.rows.is_empty() {
        return String::new();
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut max_cols = 0usize;
    let mut dummy_fn = Vec::new();
    let mut dummy_en = Vec::new();

    for row in &table.rows {
        let mut cells = Vec::new();
        for cell in &row.cells {
            if cell.covered {
                cells.push(String::new());
                continue;
            }
            let text = if let Some(doc) = doc {
                cell.blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Paragraph(p) => {
                            let (t, _) = render_paragraph_inlines(doc, p, false, true, &mut dummy_fn, &mut dummy_en);
                            let t = t.trim().to_string();
                            (!t.is_empty()).then_some(t)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("<br>")
            } else {
                cell.plain_text().replace('\n', "<br>")
            };
            cells.push(escape_pipe_cell(&text));
        }
        max_cols = max_cols.max(cells.len());
        rows.push(cells);
    }

    if max_cols == 0 {
        return String::new();
    }
    for r in &mut rows {
        r.resize(max_cols, String::new());
    }

    let mut out = String::new();
    let write_row = |out: &mut String, cells: &[String]| {
        out.push('|');
        for c in cells {
            out.push(' ');
            out.push_str(c);
            out.push_str(" |");
        }
        out.push('\n');
    };

    write_row(&mut out, &rows[0]);
    out.push('|');
    for _ in 0..max_cols {
        out.push_str(" --- |");
    }
    out.push('\n');
    for r in rows.iter().skip(1) {
        write_row(&mut out, r);
    }
    out.trim_end().to_string()
}

fn escape_pipe_cell(s: &str) -> String {
    s.trim().replace('|', "\\|").replace('\r', "").replace('\n', "<br>")
}

// ── Slides → Markdown

fn slides_to_markdown(doc: &Document, slides: &[Slide]) -> String {
    let mut out = String::new();
    let mut visible_num = 0usize;
    for (idx, slide) in slides.iter().enumerate() {
        if slide.hidden {
            continue;
        }
        visible_num += 1;
        let ordered = slide_reading_order(&slide.frames);
        let title = ordered.iter().find_map(|f| {
            if matches!(f.placeholder.as_ref().and_then(|p| p.kind.as_deref()), Some("title" | "ctrTitle")) {
                if let FrameContent::Shape { text: Some(body), .. } = &f.content {
                    let t = body.paragraphs.iter().map(Paragraph::plain_text).collect::<Vec<_>>().join(" ");
                    let t = t.trim().to_string();
                    return (!t.is_empty()).then_some(t);
                }
            }
            None
        });
        if let Some(ref t) = title {
            out.push_str(&format!("## Slide {visible_num}: {t}\n\n"));
        } else {
            out.push_str(&format!("## Slide {visible_num}\n\n"));
        }

        let mut skipped_title = false;
        for f in ordered {
            if !skipped_title
                && title.is_some()
                && matches!(f.placeholder.as_ref().and_then(|p| p.kind.as_deref()), Some("title" | "ctrTitle"))
            {
                skipped_title = true;
                continue;
            }
            slide_frame_markdown(f, &mut out);
        }

        let notes = office_slides::notes(doc, idx);
        let note_text: Vec<String> = notes.iter().map(|p| p.plain_text().trim().to_string()).filter(|s| !s.is_empty()).collect();
        if !note_text.is_empty() {
            out.push_str("> **Notes:** ");
            out.push_str(&note_text.join(" "));
            out.push_str("\n\n");
        }
    }
    out
}

fn slide_reading_order(frames: &[Frame]) -> Vec<&Frame> {
    let mut v: Vec<&Frame> = frames.iter().filter(|f| !f.hidden).collect();
    v.sort_by_key(|f| f.shape.xfrm.as_ref().map(|x| (x.y, x.x)).unwrap_or_default());
    v
}

fn slide_frame_markdown(f: &Frame, out: &mut String) {
    match &f.content {
        FrameContent::Shape { text: Some(body), .. } => {
            for p in &body.paragraphs {
                let text = slide_para_inline_markdown(p);
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(lvl) = p.props.level {
                    out.push_str(&"  ".repeat(lvl.min(6) as usize));
                    out.push_str("- ");
                    out.push_str(trimmed);
                    out.push('\n');
                } else {
                    out.push_str(trimmed);
                    out.push_str("\n\n");
                }
            }
            if !out.ends_with("\n\n") && !out.is_empty() {
                out.push('\n');
            }
        }
        FrameContent::Table { table } => {
            let md = table_to_markdown(None, table);
            if !md.is_empty() {
                out.push_str(&md);
                out.push_str("\n\n");
            }
        }
        FrameContent::Picture { image } => {
            if let Some(desc) = image.description.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                if let Some(math_md) = latex_tag_markdown(desc) {
                    out.push_str(&math_md);
                    out.push_str("\n\n");
                } else {
                    out.push_str(&format!("![{}]()\n\n", desc.replace(['[', ']'], "")));
                }
            }
        }
        FrameContent::Group { frames } => {
            for child in slide_reading_order(frames) {
                slide_frame_markdown(child, out);
            }
        }
        _ => {}
    }
}

fn slide_para_inline_markdown(p: &Paragraph) -> String {
    let mut out = String::new();
    for inline in &p.content {
        if let Inline::Text(r) = inline {
            let fmt = InlineFmt {
                bold: r.props.bold == Some(true),
                italic: r.props.italic == Some(true),
                strike: matches!(r.props.strike, Some(Strike::Single | Strike::Double)),
                code: is_monospace_font(r.props.font.as_deref()),
                link: r.link.as_ref().and_then(|l| l.url.clone()).filter(|u| !u.is_empty()),
            };
            out.push_str(&format_inline_span(&r.text, &fmt));
        }
    }
    out
}

// ── Sheets → Markdown

fn sheets_to_markdown(sheets: &[office_core::model::Sheet]) -> office_core::Result<String> {
    let visible: Vec<_> = sheets.iter().filter(|s| !s.hidden).collect();
    let multi = visible.len() > 1;
    let mut out = String::new();
    for sheet in visible {
        let rows = sheet.to_text_rows()?;
        if multi {
            out.push_str(&format!("## {}\n\n", sheet.name));
        }
        if rows.is_empty() {
            continue;
        }
        let max_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if max_cols == 0 {
            continue;
        }
        let write_row = |out: &mut String, row: &[String]| {
            out.push('|');
            for c in 0..max_cols {
                let cell = row.get(c).map(|s| escape_pipe_cell(s)).unwrap_or_default();
                out.push(' ');
                out.push_str(&cell);
                out.push_str(" |");
            }
            out.push('\n');
        };
        write_row(&mut out, &rows[0]);
        out.push('|');
        for _ in 0..max_cols {
            out.push_str(" --- |");
        }
        out.push('\n');
        for r in rows.iter().skip(1) {
            write_row(&mut out, r);
        }
        out.push('\n');
    }
    Ok(out)
}

fn normalize_markdown(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0usize;
    for raw_line in s.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    let trimmed = out.trim_end_matches('\n');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

// ── Markdown / Plain Text → Word

fn plain_text_to_word(text: &str) -> Document {
    let mut doc = Document::new(Kind::Word);
    let blocks: Vec<Block> = text.replace("\r\n", "\n").split('\n').map(|line| Block::Paragraph(Paragraph::text(line))).collect();
    doc.body = Body::Flow { blocks: if blocks.is_empty() { vec![Block::Paragraph(Paragraph::default())] } else { blocks } };
    doc
}

/// Converts Markdown text into a structured Word [`Document`] (headings, lists, code blocks,
/// tables, blockquotes, and inline bold/italic/code/strikethrough/links).
pub fn markdown_to_word(md: &str) -> Document {
    let mut doc = Document::new(Kind::Word);
    // Attach default Word styles so Heading1..Heading6 and lists serialize properly.
    let mut blocks: Vec<Block> = Vec::new();
    let mut list_items: Vec<(usize, ListKind, u8)> = Vec::new();

    let owned_lines: Vec<String> = md.replace("\r\n", "\n").replace('\r', "\n").lines().map(String::from).collect();
    let mut i = 0;

    while i < owned_lines.len() {
        let line = &owned_lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence = if trimmed.starts_with("```") { "```" } else { "~~~" };
            i += 1;
            while i < owned_lines.len() && !owned_lines[i].trim().starts_with(fence) {
                let run = Run::new(&owned_lines[i]).with_props(RunProps {
                    font: Some("Consolas".into()),
                    size: Some(FontSize(20)),
                    ..Default::default()
                });
                blocks.push(Block::Paragraph(Paragraph { content: vec![Inline::Text(run)], ..Default::default() }));
                i += 1;
            }
            if i < owned_lines.len() {
                i += 1;
            }
            continue;
        }

        // ATX Heading (# .. ######).
        if let Some((lvl, rest)) = parse_heading_line(trimmed) {
            let mut p = parse_inline_markdown(rest);
            p.props.style = Some(format!("Heading{lvl}"));
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }

        // Thematic break (---, ***, ___).
        if is_thematic_break(trimmed) {
            blocks.push(Block::Paragraph(Paragraph {
                content: vec![Inline::Break { kind: BreakKind::Page, origin: Default::default() }],
                ..Default::default()
            }));
            i += 1;
            continue;
        }

        // GFM pipe table: header row + separator row.
        if trimmed.contains('|') && i + 1 < owned_lines.len() && is_table_separator(&owned_lines[i + 1]) {
            let mut table_rows: Vec<Vec<String>> = vec![split_pipe_row(trimmed)];
            i += 2;
            while i < owned_lines.len() && owned_lines[i].contains('|') && !owned_lines[i].trim().is_empty() {
                table_rows.push(split_pipe_row(&owned_lines[i]));
                i += 1;
            }
            let cols = table_rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
            for r in &mut table_rows {
                r.resize(cols, String::new());
            }
            let mut t = Table::from_text(&table_rows, Emu(5_731_510));
            if let Some(first) = t.rows.first_mut() {
                first.header = true;
            }
            blocks.push(Block::Table(t));
            continue;
        }

        // Blockquote.
        if let Some(rest) = trimmed.strip_prefix('>') {
            let mut p = parse_inline_markdown(rest.trim_start());
            p.props.style = Some("Quote".into());
            p.props.indent_start = Some(Emu(457_200));
            blocks.push(Block::Paragraph(p));
            i += 1;
            continue;
        }

        // List item (unordered or ordered).
        if let Some((kind, level, rest)) = parse_list_line(line) {
            let idx = blocks.len();
            let p = parse_inline_markdown(rest);
            blocks.push(Block::Paragraph(p));
            list_items.push((idx, kind, level));
            i += 1;
            continue;
        }

        // Normal paragraph (join consecutive plain lines).
        let mut para_lines = vec![trimmed.to_string()];
        i += 1;
        while i < owned_lines.len() {
            let next = owned_lines[i].trim();
            if next.is_empty()
                || next.starts_with("```")
                || next.starts_with("~~~")
                || parse_heading_line(next).is_some()
                || is_thematic_break(next)
                || next.starts_with('>')
                || parse_list_line(&owned_lines[i]).is_some()
                || (next.contains('|') && i + 1 < owned_lines.len() && is_table_separator(&owned_lines[i + 1]))
            {
                break;
            }
            para_lines.push(next.to_string());
            i += 1;
        }
        blocks.push(Block::Paragraph(parse_inline_markdown(&para_lines.join(" "))));
    }

    if blocks.is_empty() {
        blocks.push(Block::Paragraph(Paragraph::default()));
    }
    doc.body = Body::Flow { blocks };

    if !list_items.is_empty() {
        let bullet_id = office_word::add_list(&mut doc, ListKind::Bullets);
        let number_id = office_word::add_list(&mut doc, ListKind::Numbers);
        for (idx, kind, level) in list_items {
            let id = match kind {
                ListKind::Bullets => bullet_id.as_str(),
                ListKind::Numbers => number_id.as_str(),
            };
            office_word::set_list(&mut doc, idx, Some((id, level)));
        }
    }

    doc
}

fn parse_heading_line(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes as u8, rest.trim().trim_end_matches('#').trim_end()))
}

fn is_thematic_break(line: &str) -> bool {
    let chars: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    chars.len() >= 3 && (chars.iter().all(|&c| c == '-') || chars.iter().all(|&c| c == '*') || chars.iter().all(|&c| c == '_'))
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('|') || !t.contains('-') {
        return false;
    }
    t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn split_pipe_row(line: &str) -> Vec<String> {
    let t = line.trim().trim_start_matches('|').trim_end_matches('|');
    t.split('|').map(|c| c.trim().replace("\\|", "|")).collect()
}

fn parse_list_line(line: &str) -> Option<(ListKind, u8, &str)> {
    let spaces = line.bytes().take_while(|&b| b == b' ').count();
    let level = ((spaces / 2).min(6)) as u8;
    let rest = &line[spaces..];
    if let Some(after) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")).or_else(|| rest.strip_prefix("+ ")) {
        return Some((ListKind::Bullets, level, after.trim()));
    }
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 {
        if let Some(after) = rest[digits..].strip_prefix(". ").or_else(|| rest[digits..].strip_prefix(") ")) {
            return Some((ListKind::Numbers, level, after.trim()));
        }
    }
    None
}

fn parse_inline_markdown(input: &str) -> Paragraph {
    let mut content: Vec<Inline> = Vec::new();
    let mut plain = String::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    let flush_plain = |plain: &mut String, content: &mut Vec<Inline>| {
        if !plain.is_empty() {
            content.push(Inline::Text(Run::new(std::mem::take(plain))));
        }
    };

    while i < bytes.len() {
        // Inline code: `...`
        if bytes[i] == b'`' {
            if let Some(end) = input[i + 1..].find('`') {
                flush_plain(&mut plain, &mut content);
                let code = &input[i + 1..i + 1 + end];
                content.push(Inline::Text(Run::new(code).with_props(RunProps { font: Some("Consolas".into()), ..Default::default() })));
                i += end + 2;
                continue;
            }
        }
        // Link: [label](url)
        if bytes[i] == b'[' {
            if let Some(close_bracket) = input[i + 1..].find("](") {
                let after_mid = i + 1 + close_bracket + 2;
                if let Some(close_paren) = input[after_mid..].find(')') {
                    flush_plain(&mut plain, &mut content);
                    let label = &input[i + 1..i + 1 + close_bracket];
                    let url = &input[after_mid..after_mid + close_paren];
                    let mut run = Run::new(label);
                    run.link = Some(Link { url: Some(url.to_string()), ..Default::default() });
                    content.push(Inline::Text(run));
                    i = after_mid + close_paren + 1;
                    continue;
                }
            }
        }
        // Bold + italic: ***...***
        if input[i..].starts_with("***") {
            if let Some(end) = input[i + 3..].find("***") {
                flush_plain(&mut plain, &mut content);
                content.push(Inline::Text(Run::new(&input[i + 3..i + 3 + end]).with_props(RunProps {
                    bold: Some(true),
                    italic: Some(true),
                    ..Default::default()
                })));
                i += end + 6;
                continue;
            }
        }
        // Bold: **...**
        if input[i..].starts_with("**") {
            if let Some(end) = input[i + 2..].find("**") {
                flush_plain(&mut plain, &mut content);
                content.push(Inline::Text(
                    Run::new(&input[i + 2..i + 2 + end]).with_props(RunProps { bold: Some(true), ..Default::default() }),
                ));
                i += end + 4;
                continue;
            }
        }
        // Strikethrough: ~~...~~
        if input[i..].starts_with("~~") {
            if let Some(end) = input[i + 2..].find("~~") {
                flush_plain(&mut plain, &mut content);
                content.push(Inline::Text(
                    Run::new(&input[i + 2..i + 2 + end]).with_props(RunProps { strike: Some(Strike::Single), ..Default::default() }),
                ));
                i += end + 4;
                continue;
            }
        }
        // Italic: *...*
        if bytes[i] == b'*' && !input[i..].starts_with("**") {
            if let Some(end) = input[i + 1..].find('*') {
                if end > 0 {
                    flush_plain(&mut plain, &mut content);
                    content.push(Inline::Text(
                        Run::new(&input[i + 1..i + 1 + end]).with_props(RunProps { italic: Some(true), ..Default::default() }),
                    ));
                    i += end + 2;
                    continue;
                }
            }
        }
        let ch = input[i..].chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }

    flush_plain(&mut plain, &mut content);
    Paragraph { props: ParaProps::default(), content, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use office_core::model::{CellRef, Sheet, Value};

    #[test]
    fn docx_to_markdown_preserves_headings_lists_tables_and_formatting() {
        let md_in = "# Project Spec\n\nIntro with **bold**, *italic*, `code`, ~~old~~, and [Writan](https://writan.app).\n\n## Goals\n\n- Fast export\n- Clean output\n\n| Name | Value |\n| --- | --- |\n| Alpha | 1 |\n| Beta | 2 |\n";
        let doc = markdown_to_word(md_in);
        let docx_bytes = compiler().write(&doc, Kind::Word).unwrap();

        let md_out = to_markdown_inner(&docx_bytes, "spec.docx").unwrap();
        assert!(md_out.contains("# Project Spec"), "{md_out}");
        assert!(md_out.contains("**bold**"), "{md_out}");
        assert!(md_out.contains("*italic*"), "{md_out}");
        assert!(md_out.contains("`code`"), "{md_out}");
        assert!(md_out.contains("~~old~~"), "{md_out}");
        assert!(md_out.contains("[Writan](https://writan.app)"), "{md_out}");
        assert!(md_out.contains("## Goals"), "{md_out}");
        assert!(md_out.contains("- Fast export"), "{md_out}");
        assert!(md_out.contains("- Clean output"), "{md_out}");
        assert!(md_out.contains("| Name | Value |"), "{md_out}");
        assert!(md_out.contains("| Alpha | 1 |"), "{md_out}");
    }

    #[test]
    fn workbook_to_markdown_renders_gfm_tables_per_sheet() {
        let mut doc = Document::new(Kind::Sheets);
        let mut s1 = Sheet::new("Summary");
        s1.set(CellRef::new(0, 0), Value::Text("Item".into()));
        s1.set(CellRef::new(0, 1), Value::Text("Qty".into()));
        s1.set(CellRef::new(1, 0), Value::Text("Apples".into()));
        s1.set(CellRef::new(1, 1), Value::Number(4.0));
        let mut s2 = Sheet::new("Notes");
        s2.set(CellRef::new(0, 0), Value::Text("Note".into()));
        s2.set(CellRef::new(1, 0), Value::Text("Fresh".into()));
        doc.body = Body::Workbook { sheets: vec![s1, s2] };

        let bytes = compiler().write(&doc, Kind::Sheets).unwrap();
        let md = to_markdown_inner(&bytes, "data.xlsx").unwrap();
        assert!(md.contains("## Summary"), "{md}");
        assert!(md.contains("| Item | Qty |"), "{md}");
        assert!(md.contains("| Apples | 4 |"), "{md}");
        assert!(md.contains("## Notes"), "{md}");
    }

    #[test]
    fn convert_between_office_and_markdown_formats() {
        let md = "# Hello\n\nWorld\n";
        let docx = convert_inner(md.as_bytes(), "note.md", "docx").unwrap();
        let pptx = convert_inner(&docx, "note.docx", "pptx").unwrap();
        let slides_md = to_markdown_inner(&pptx, "note.pptx").unwrap();
        assert!(slides_md.contains("Hello"), "{slides_md}");
        assert!(slides_md.contains("World"), "{slides_md}");
    }
}
