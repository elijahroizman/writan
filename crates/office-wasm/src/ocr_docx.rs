//! Reconstruct an editable Word document from the OCR worker's logical text and bounding boxes.

use office_core::model::{Align, Block, Body, BreakKind, Cell, Document, Inline, Kind, Paragraph, Row, Table};
use office_core::opc::{rels::kind, Package};
use office_core::units::Emu;
use office_core::xml::{ns, Element, XmlDoc};
use office_core::{Compiler, Layer};
use office_engine::para::first_strong_rtl;
use office_word::{numbering, Words};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OcrDocumentInput {
    #[serde(default)]
    title: Option<String>,
    pages: Vec<OcrPageResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code, reason = "Keep the OCR wire schema, including metadata unused by Word reconstruction")]
struct OcrPageResult {
    page_index: usize,
    width_px: f64,
    height_px: f64,
    #[serde(default)]
    width_pt: Option<f64>,
    #[serde(default)]
    height_pt: Option<f64>,
    text: String,
    mean_confidence: f64,
    detected_rotation: u16,
    elapsed_ms: f64,
    blocks: Vec<OcrBlock>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code, reason = "Confidence is OCR metadata")]
struct OcrBlock {
    bbox: OcrBBox,
    confidence: f64,
    paragraphs: Vec<OcrParagraph>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code, reason = "Paragraph geometry and confidence are OCR metadata")]
struct OcrParagraph {
    text: String,
    bbox: OcrBBox,
    confidence: f64,
    rtl: bool,
    lines: Vec<OcrLine>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code, reason = "Baseline, confidence and direction are already reflected in the line box and text")]
struct OcrLine {
    text: String,
    bbox: OcrBBox,
    #[serde(default)]
    baseline: Option<f64>,
    confidence: f64,
    rtl: bool,
    words: Vec<OcrWord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code, reason = "Font size, confidence and baseline are OCR metadata")]
struct OcrWord {
    text: String,
    bbox: OcrBBox,
    #[serde(default)]
    baseline: Option<f64>,
    confidence: f64,
    rtl: bool,
    #[serde(default)]
    font_size_px: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct OcrBBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl OcrBBox {
    pub(crate) fn right(self) -> f64 {
        self.x + self.w
    }

    pub(crate) fn bottom(self) -> f64 {
        self.y + self.h
    }
}

impl OcrPageResult {
    // The worker maps its boxes back to the source image. Apply the correction for reading order.
    fn upright(&self, b: OcrBBox) -> OcrBBox {
        match self.detected_rotation {
            90 => OcrBBox { x: self.height_px - b.bottom(), y: b.x, w: b.h, h: b.w },
            180 => OcrBBox { x: self.width_px - b.right(), y: self.height_px - b.bottom(), ..b },
            270 => OcrBBox { x: b.y, y: self.width_px - b.right(), w: b.h, h: b.w },
            _ => b,
        }
    }

    fn points_per_pixel(&self) -> f64 {
        let x_scale = self.width_pt.map(|pt| pt / self.width_px);
        let y_scale = self.height_pt.map(|pt| pt / self.height_px);
        let scales = if matches!(self.detected_rotation, 90 | 270) { [x_scale, y_scale] } else { [y_scale, x_scale] };
        scales.into_iter().flatten().find(|s| s.is_finite() && *s > 0.0).unwrap_or(0.75)
    }
}

pub fn ocr_document_to_docx(json: &str) -> Result<Vec<u8>, String> {
    let mut input: OcrDocumentInput = serde_json::from_str(json).map_err(|e| format!("Invalid OCR document JSON: {e}"))?;
    input.pages.sort_by_key(|page| page.page_index);
    write_document(input).map_err(|e| format!("Could not write OCR Word document: {e}"))
}

fn write_document(input: OcrDocumentInput) -> office_core::Result<Vec<u8>> {
    let typical_height = median(input.pages.iter().flat_map(|page| {
        let scale = page.points_per_pixel();
        page.blocks.iter().flat_map(|b| &b.paragraphs).flat_map(|p| &p.lines).map(move |line| page.upright(line.bbox).h * scale)
    }));
    let mut document = Document::new(Kind::Word);
    document.properties.title = input.title.map(|title| xml_text(&title));
    let mut blocks = Vec::new();
    let mut lists = Vec::new();
    for page in &input.pages {
        let mut content = Vec::new();
        let rtl = page_is_rtl(page);
        let positioned: Vec<PositionedBlock<'_>> = page.blocks.iter().map(|block| (page.upright(block.bbox), block)).collect();
        let min_gap = typical_height.map_or(8.0, |height| height / page.points_per_pixel() * 0.6);
        let ordered = reading_order(positioned, rtl, min_gap, 0);
        let paragraphs: Vec<_> = ordered.iter().flat_map(|(_, block)| &block.paragraphs).collect();
        reconstruct_paragraphs(page, &paragraphs, typical_height, &mut content);
        if content.is_empty() {
            push_paragraphs(page.text.lines().map(|line| (line, false)), false, None, &mut content);
        }
        if content.is_empty() {
            continue;
        }
        if !blocks.is_empty() {
            blocks.push(Block::Paragraph(Paragraph {
                content: vec![Inline::Break { kind: BreakKind::Page, origin: Default::default() }],
                ..Default::default()
            }));
        }
        for (block, marker) in content {
            if let Some(marker) = marker {
                lists.push((blocks.len(), marker));
            }
            blocks.push(block);
        }
    }
    if blocks.is_empty() {
        blocks.push(Block::Paragraph(Paragraph::default()));
    }
    document.body = Body::Flow { blocks };

    let compiler = Compiler::new().with_layer(Words);
    if lists.is_empty() {
        return compiler.write(&document, Kind::Word);
    }
    let numbering = list_numbering(&mut document, lists);
    compiler.with_layer(OcrNumbering(numbering)).write(&document, Kind::Word)
}

pub(crate) fn median(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<_> = values.into_iter().filter(|h| h.is_finite() && *h > 0.0).collect();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    let upper = *values.get(middle)?;
    Some(if values.len() % 2 == 0 { values[middle - 1] * 0.5 + upper * 0.5 } else { upper })
}

fn page_is_rtl(page: &OcrPageResult) -> bool {
    let (mut rtl, mut ltr) = (0, 0);
    for paragraph in page.blocks.iter().flat_map(|block| &block.paragraphs) {
        let direction = paragraph.lines.iter().find_map(|line| first_strong_rtl(&line.text));
        match first_strong_rtl(&paragraph.text).or(direction) {
            Some(true) => rtl += 1,
            Some(false) if !paragraph.rtl => ltr += 1,
            _ if paragraph.rtl => rtl += 1,
            _ => {}
        }
    }
    if rtl == ltr {
        first_strong_rtl(&page.text).unwrap_or(false)
    } else {
        rtl > ltr
    }
}

type PositionedBlock<'a> = (OcrBBox, &'a OcrBlock);

/// Orders boxes for reading: top to bottom, with side-by-side columns one after the other
/// (right first in right-to-left pages), and a box that spans columns between the bands.
pub(crate) fn reading_order<T>(mut blocks: Vec<(OcrBBox, T)>, rtl: bool, min_gap: f64, depth: u8) -> Vec<(OcrBBox, T)> {
    blocks.sort_by(|a, b| {
        a.0.y.total_cmp(&b.0.y).then_with(|| if rtl { b.0.right().total_cmp(&a.0.right()) } else { a.0.x.total_cmp(&b.0.x) })
    });
    // A column break needs evidence from side-by-side blocks. Limit recursive splits
    // on very fragmented scans, where the OCR engine may return hundreds of tiny blocks.
    let Some(split) = (blocks.len() > 1 && depth < 8).then(|| column_cut(&blocks, min_gap)).flatten() else { return blocks };
    let (mut left, mut right, mut ordered) = (Vec::new(), Vec::new(), Vec::with_capacity(blocks.len()));
    // A spanning block (such as a page heading) separates bands of columns.
    // Process each band in column order before emitting the spanning block.
    for block in blocks {
        if block.0.right() <= split {
            left.push(block);
        } else if block.0.x >= split {
            right.push(block);
        } else {
            append_columns(&mut ordered, &mut left, &mut right, rtl, min_gap, depth);
            ordered.push(block);
        }
    }
    append_columns(&mut ordered, &mut left, &mut right, rtl, min_gap, depth);
    ordered
}

fn append_columns<T>(
    ordered: &mut Vec<(OcrBBox, T)>,
    left: &mut Vec<(OcrBBox, T)>,
    right: &mut Vec<(OcrBBox, T)>,
    rtl: bool,
    min_gap: f64,
    depth: u8,
) {
    let (first, second) = if rtl { (right, left) } else { (left, right) };
    ordered.extend(reading_order(std::mem::take(first), rtl, min_gap, depth + 1));
    ordered.extend(reading_order(std::mem::take(second), rtl, min_gap, depth + 1));
}

pub(crate) fn column_cut<T>(blocks: &[(OcrBBox, T)], min_gap: f64) -> Option<f64> {
    let mut best = None;
    for &(left, _) in blocks.iter().filter(|(b, _)| b.w > 0.0 && b.h > 0.0) {
        let right = blocks
            .iter()
            .map(|(b, _)| *b)
            .filter(|b| b.w > 0.0 && b.x >= left.right() + min_gap && b.y < left.bottom() && b.bottom() > left.y)
            .min_by(|a, b| a.x.total_cmp(&b.x));
        let Some(right) = right else { continue };
        let gap = right.x - left.right();
        let split = left.right() + gap * 0.5;
        let crossing = blocks.iter().filter(|(b, _)| b.x < split && b.right() > split).count();
        if best.is_none_or(|(count, width, _)| crossing < count || (crossing == count && gap > width)) {
            best = Some((crossing, gap, split));
        }
    }
    best.map(|(_, _, split)| split)
}

type Content = Vec<(Block, Option<ListMarker>)>;

fn reconstruct_paragraphs(page: &OcrPageResult, paragraphs: &[&OcrParagraph], typical_height: Option<f64>, out: &mut Content) {
    // Keep paragraph boundaries while looking across consecutive OCR lines for table rows.
    let lines: Vec<_> =
        paragraphs.iter().enumerate().flat_map(|(p, paragraph)| paragraph.lines.iter().map(move |line| (p, line))).collect();
    let row_candidates: Vec<_> = lines.iter().map(|(_, line)| table_row(page, line)).collect();
    let mut at = 0;
    for (p, paragraph) in paragraphs.iter().enumerate() {
        if paragraph.lines.is_empty() {
            push_paragraphs(paragraph.text.lines().map(|line| (line, false)), paragraph.rtl, None, out);
            continue;
        }
        while lines.get(at).is_some_and(|(index, _)| *index == p) {
            if let Some(end) = table_end(&row_candidates, &lines, at) {
                let rows = row_candidates[at..end].iter().flatten();
                out.push((Block::Table(build_table(rows)), None));
                at = end;
                continue;
            }
            let first = at;
            at += 1;
            while lines.get(at).is_some_and(|(index, _)| *index == p) && table_end(&row_candidates, &lines, at).is_none() {
                at += 1;
            }
            let segment = &lines[first..at];
            let entire_paragraph = segment.len() == paragraph.lines.len();
            if entire_paragraph && segment.iter().all(|(_, line)| line.text.trim().is_empty()) {
                push_paragraphs(paragraph.text.lines().map(|line| (line, false)), paragraph.rtl, None, out);
            } else {
                let heading = if entire_paragraph && segment.len() <= 2 {
                    typical_height
                        .zip(median(segment.iter().map(|(_, line)| page.upright(line.bbox).h * page.points_per_pixel())))
                        .and_then(|(typical, height)| heading_level(height / typical))
                } else {
                    None
                };
                push_paragraphs(segment.iter().map(|(_, line)| (line.text.as_str(), line.rtl)), paragraph.rtl, heading, out);
            }
        }
    }
}

fn heading_level(ratio: f64) -> Option<u8> {
    [(1.65, 1), (1.35, 2), (1.18, 3)].into_iter().find_map(|(threshold, level)| (ratio >= threshold).then_some(level))
}

fn push_paragraphs<'a>(lines: impl Iterator<Item = (&'a str, bool)>, rtl: bool, heading: Option<u8>, out: &mut Content) {
    let mut group = Vec::new();
    for (line, line_rtl) in lines {
        let line = line.trim();
        if line.is_empty() || (!group.is_empty() && list_marker(line).is_some()) {
            push_joined(&group, rtl, None, out);
            group.clear();
        }
        if !line.is_empty() {
            group.push((line, line_rtl));
        }
    }
    push_joined(&group, rtl, heading, out);
}

fn push_joined(lines: &[(&str, bool)], rtl: bool, heading: Option<u8>, out: &mut Content) {
    let Some(&(first, _)) = lines.first() else { return };
    let item = list_marker(first);
    let mut text = match item {
        Some((_, marks, rest)) => [marks, rest].concat(),
        None => first.to_string(),
    };
    for &(next, _) in &lines[1..] {
        let wrap_hyphen = text.strip_suffix('-').and_then(|s| s.chars().next_back()).is_some_and(|c| c.is_ascii_alphabetic())
            && next.starts_with(|c: char| c.is_ascii_lowercase());
        if wrap_hyphen {
            text.pop();
        } else {
            text.push(' ');
        }
        text.push_str(next);
    }
    let rtl = rtl || lines.iter().any(|(_, rtl)| *rtl) || has_rtl(first) || has_rtl(&text);
    let mut paragraph = word_paragraph(&text, rtl);
    if let Some(level) = heading.filter(|_| item.is_none() && text.chars().count() <= 120) {
        // The Words layer installs the built-in style, including its font size.
        paragraph.props.style = Some(format!("Heading{level}"));
    }
    out.push((Block::Paragraph(paragraph), item.map(|(marker, _, _)| marker)));
}

pub(crate) fn has_rtl(text: &str) -> bool {
    text.char_indices().any(|(at, c)| first_strong_rtl(&text[at..at + c.len_utf8()]) == Some(true))
}

pub(crate) fn xml_text(text: &str) -> String {
    text.chars()
        .filter(|&c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'))
        .collect()
}

fn word_paragraph(text: &str, rtl: bool) -> Paragraph {
    let mut paragraph = Paragraph::text(xml_text(text));
    if rtl {
        paragraph.props.rtl = Some(true);
        paragraph.props.align = Some(Align::Right);
    }
    paragraph
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListFormat {
    Bullet(char),
    Decimal(&'static str),
    Hebrew,
}

#[derive(Clone, Copy)]
pub(crate) struct ListMarker {
    format: ListFormat,
    number: u32,
}

pub(crate) fn list_marker(text: &str) -> Option<(ListMarker, &str, &str)> {
    let content =
        text.trim_start_matches(|c| matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'));
    let marks = &text[..text.len() - content.len()];
    let text = content;
    let first = text.chars().next()?;
    if matches!(first, '•' | '-' | '*' | '▪' | '◦' | '‣') {
        let rest = &text[first.len_utf8()..];
        if rest.starts_with(char::is_whitespace) || !matches!(first, '-' | '*') {
            let rest = rest.trim_start();
            return (!rest.is_empty()).then_some((ListMarker { format: ListFormat::Bullet(first), number: 1 }, marks, rest));
        }
    }
    let (label, rest) = text.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    let (digits, template) = if let Some(n) = label.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        (n, "(%1)")
    } else if let Some(n) = label.strip_suffix(')') {
        (n, "%1)")
    } else {
        let n = label.strip_suffix('.')?;
        if let Some(c) = n.chars().next().filter(|c| ('א'..='ת').contains(c) && n.chars().count() == 1) {
            let number = "אבגדהוזחטיכלמנסעפצקרשת".chars().position(|letter| letter == c)? as u32 + 1;
            return Some((ListMarker { format: ListFormat::Hebrew, number }, marks, rest));
        }
        (n, "%1.")
    };
    let number = digits.bytes().all(|b| b.is_ascii_digit()).then(|| digits.parse::<u32>().ok()).flatten()?;
    Some((ListMarker { format: ListFormat::Decimal(template), number }, marks, rest))
}

pub(crate) struct OcrNumbering(pub XmlDoc);

impl Layer for OcrNumbering {
    fn name(&self) -> &'static str {
        "OCR numbering"
    }

    fn kinds(&self) -> &[Kind] {
        &[Kind::Word]
    }

    fn edits_before_write(&self, _to: Kind) -> bool {
        false
    }

    fn write_parts(&self, _doc: &Document, pkg: &mut Package, _to: Kind) -> office_core::Result<()> {
        let part = "word/numbering.xml";
        pkg.put_xml(part, &self.0, "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml");
        pkg.relate(&pkg.main_part()?, kind::NUMBERING, part)?;
        Ok(())
    }
}

// Like Word's list tools, set numPr through office-word. Supply OCR-specific list
// definitions so an original counter and punctuation survive in the editable list.
pub(crate) fn list_numbering(doc: &mut Document, lists: Vec<(usize, ListMarker)>) -> XmlDoc {
    let mut xml = numbering::empty();
    let mut id = 0;
    let mut nums = Vec::new();
    let mut previous: Option<(usize, ListMarker)> = None;
    let val = |name, value: &str| Element::new(ns::W, name).with_attr(ns::W, "val", value);
    for (index, marker) in lists {
        let continues = previous.is_some_and(|(prev_index, prev)| {
            index == prev_index + 1
                && marker.format == prev.format
                && (matches!(marker.format, ListFormat::Bullet(_)) || prev.number.checked_add(1) == Some(marker.number))
        });
        if !continues {
            id += 1;
            let id_str = id.to_string();
            let (format, label) = match marker.format {
                ListFormat::Bullet(c) => ("bullet", c.to_string()),
                ListFormat::Decimal(template) => ("decimal", template.into()),
                ListFormat::Hebrew => ("hebrew2", "%1.".into()),
            };
            let align = if matches!(marker.format, ListFormat::Hebrew) { "right" } else { "left" };
            let indent = Element::new(ns::W, "ind").with_attr(ns::W, "start", "720").with_attr(ns::W, "hanging", "360");
            let level = Element::new(ns::W, "lvl")
                .with_attr(ns::W, "ilvl", "0")
                .with_child(val("start", &marker.number.to_string()))
                .with_child(val("numFmt", format))
                .with_child(val("lvlText", &label))
                .with_child(val("lvlJc", align))
                .with_child(Element::new(ns::W, "pPr").with_child(indent));
            xml.root.push(
                Element::new(ns::W, "abstractNum")
                    .with_attr(ns::W, "abstractNumId", &id_str)
                    .with_child(val("multiLevelType", "singleLevel"))
                    .with_child(level),
            );
            nums.push(Element::new(ns::W, "num").with_attr(ns::W, "numId", &id_str).with_child(val("abstractNumId", &id_str)));
        }
        office_word::set_list(doc, index, Some((&id.to_string(), 0)));
        previous = Some((index, marker));
    }
    for num in nums {
        xml.root.push(num);
    }
    xml
}

struct TableRow<'a> {
    bbox: OcrBBox,
    space: f64,
    cells: Vec<TableCell<'a>>,
}

struct TableCell<'a> {
    left: f64,
    right: f64,
    words: Vec<(usize, &'a OcrWord)>,
}

fn table_row<'a>(page: &OcrPageResult, line: &'a OcrLine) -> Option<TableRow<'a>> {
    // A list's separated marker (or a bullet followed by a hanging indent) isn't a table cell.
    if list_marker(line.text.trim()).is_some() {
        return None;
    }
    let mut words: Vec<_> = line
        .words
        .iter()
        .enumerate()
        .filter_map(|(index, word)| {
            let bbox = page.upright(word.bbox);
            (bbox.w > 0.0 && bbox.right().is_finite() && !word.text.trim().is_empty()).then_some((bbox, index, word))
        })
        .collect();
    words.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    let glyph_width = words.iter().map(|(b, _, _)| b.w).sum::<f64>()
        / words.iter().map(|(_, _, word)| word.text.trim().chars().count()).sum::<usize>().max(1) as f64;
    // Fall back to a fraction of the average glyph width when the line only has wide gaps.
    let space =
        median(words.windows(2).map(|pair| pair[1].0.x - pair[0].0.right()).filter(|gap| *gap <= glyph_width)).unwrap_or(glyph_width * 0.6);
    let mut cells: Vec<TableCell<'_>> = Vec::new();
    for (bbox, index, word) in words {
        if let Some(cell) = cells.last_mut().filter(|cell| bbox.x - cell.right <= 2.2 * space) {
            cell.right = cell.right.max(bbox.right());
            cell.words.push((index, word));
        } else {
            cells.push(TableCell { left: bbox.x, right: bbox.right(), words: vec![(index, word)] });
        }
    }
    (cells.len() >= 2).then_some(TableRow { bbox: page.upright(line.bbox), space, cells })
}

fn aligned_rows(a: &TableRow<'_>, b: &TableRow<'_>) -> bool {
    let row_height = a.bbox.h.max(b.bbox.h);
    let y_step = b.bbox.y - a.bbox.y;
    a.cells.len() == b.cells.len()
        && row_height > 0.0
        && y_step >= row_height * 0.5
        && y_step <= row_height * 2.5
        && aligned_columns(a, b)
        && a.cells.windows(2).zip(b.cells.windows(2)).all(|(a, b)| a[1].left.min(b[1].left) > a[0].right.max(b[0].right))
}

fn table_end(rows: &[Option<TableRow<'_>>], lines: &[(usize, &OcrLine)], start: usize) -> Option<usize> {
    let first = rows.get(start)?.as_ref()?;
    let mut previous = first;
    let mut end = start + 1;
    for row in rows[end..].iter().map_while(Option::as_ref) {
        if lines[end].0 > lines[end - 1].0 + 1 || !aligned_rows(previous, row) || (end > start + 1 && !aligned_columns(first, row)) {
            break;
        }
        previous = row;
        end += 1;
    }
    (end > start + 1).then_some(end)
}

fn aligned_columns(a: &TableRow<'_>, b: &TableRow<'_>) -> bool {
    let tolerance = a.space.max(b.space) * 2.0;
    a.cells.iter().zip(&b.cells).all(|(a, b)| (a.left - b.left).abs().min((a.right - b.right).abs()) <= tolerance)
}

fn build_table<'a, 'b>(rows: impl Iterator<Item = &'b TableRow<'a>>) -> Table
where
    'a: 'b,
{
    let rows: Vec<_> = rows
        .map(|row| Row {
            cells: row
                .cells
                .iter()
                .map(|cell| {
                    // Restore the OCR iterator's logical order inside each visually detected cell.
                    let mut words = cell.words.clone();
                    words.sort_by_key(|&(index, _)| index);
                    let text = words.iter().map(|(_, word)| word.text.trim()).collect::<Vec<_>>().join(" ");
                    let rtl = words.iter().any(|(_, word)| word.rtl) || has_rtl(&text);
                    Cell { blocks: vec![Block::Paragraph(word_paragraph(&text, rtl))], ..Default::default() }
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    let count = rows.first().map_or(1, |row| row.cells.len());
    // Word's writer gives new tables a thin single-line grid.
    Table { columns: vec![Emu::from_inches(6.5 / count as f64); count], rows, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use office_core::units::FontSize;
    use serde_json::{json, Value};

    fn bbox(x: i32, y: i32, w: i32, h: i32) -> Value {
        json!({ "x": x, "y": y, "w": w, "h": h })
    }

    fn line(text: &str, y: i32, height: i32) -> Value {
        json!({ "text": text, "bbox": bbox(0, y, 100, height), "confidence": 99, "rtl": false, "words": [] })
    }

    fn paragraph(lines: Vec<Value>) -> Value {
        let text: Vec<_> = lines.iter().filter_map(|l| l["text"].as_str()).collect();
        json!({ "text": text.join("\n"), "bbox": bbox(0, 0, 100, 20), "confidence": 99, "rtl": false, "lines": lines })
    }

    fn block(bbox: Value, paragraphs: Vec<Value>) -> Value {
        json!({ "bbox": bbox, "confidence": 99, "paragraphs": paragraphs })
    }

    fn page(index: usize, blocks: Vec<Value>) -> Value {
        json!({
            "pageIndex": index, "widthPx": 600, "heightPx": 800, "text": "", "meanConfidence": 99,
            "detectedRotation": 0, "elapsedMs": 1, "blocks": blocks,
        })
    }

    fn round_trip(pages: Vec<Value>) -> Document {
        let input = json!({ "title": "OCR test", "pages": pages });
        let bytes = ocr_document_to_docx(&input.to_string()).unwrap();
        let doc = Compiler::new().with_layer(Words).read(&bytes, Some("ocr.docx")).unwrap();
        assert_eq!(doc.properties.title.as_deref(), Some("OCR test"));
        doc
    }

    fn single_block(paragraphs: Vec<Value>) -> Document {
        round_trip(vec![page(0, vec![block(bbox(0, 0, 600, 800), paragraphs)])])
    }

    fn texts(doc: &Document) -> Vec<String> {
        office_word::paragraphs(doc).into_iter().map(Paragraph::plain_text).collect()
    }

    #[test]
    fn uses_built_in_heading_styles_and_sizes() {
        let doc = single_block(vec![
            paragraph(vec![line("Main", 0, 36)]),
            paragraph(vec![line("Section", 45, 28)]),
            paragraph(vec![line("Topic", 80, 24)]),
            paragraph((0..5).map(|i| line("Body text", 120 + i * 23, 20)).collect()),
        ]);
        let paragraphs = office_word::paragraphs(&doc);
        for (p, (id, size)) in paragraphs.iter().zip([("Heading1", 16.0), ("Heading2", 13.0), ("Heading3", 12.0)]) {
            assert_eq!(p.props.style.as_deref(), Some(id));
            assert_eq!(office_word::resolved_run(&doc, p, p.runs().next().unwrap()).size, Some(FontSize::from_points(size)));
        }
        assert_eq!(paragraphs.len(), 4);
        assert!(paragraphs[3].props.style.is_none());
    }

    #[test]
    fn joins_wraps_without_reversing_text_or_removing_other_hyphens() {
        let doc = single_block(vec![
            paragraph(vec![line("A reconstruc-", 0, 20), line("ted paragraph", 22, 20), line("with wrapping.", 44, 20)]),
            paragraph(vec![line("A retained-", 66, 20), line("Upper-case start", 88, 20)]),
            paragraph(vec![line("שלום-", 110, 20), line("world", 132, 20)]),
        ]);
        assert_eq!(texts(&doc), ["A reconstructed paragraph with wrapping.", "A retained- Upper-case start", "שלום- world"]);
    }

    #[test]
    fn writes_native_lists_and_preserves_restarts_and_labels() {
        let doc = single_block(vec![
            paragraph(vec![line("• Alpha", 0, 20), line("• Beta", 22, 20), line("continued", 44, 20)]),
            paragraph(vec![line("(3) Gamma", 66, 20), line("(4) Delta", 88, 20)]),
            paragraph(vec![line("7) Epsilon", 110, 20)]),
            paragraph(vec![line("א. אחת", 132, 20), line("\u{200f}ב. שתיים", 154, 20)]),
        ]);
        assert_eq!(texts(&doc), ["Alpha", "Beta continued", "Gamma", "Delta", "Epsilon", "אחת", "\u{200f}שתיים"]);
        let labels = office_word::list_labels(&doc);
        assert_eq!(labels[..5], ["•", "•", "(3)", "(4)", "7)"].map(|label| Some(label.into())));
        let paragraphs = office_word::paragraphs(&doc);
        let (id, level) = office_word::list_of(&doc, paragraphs[5]).unwrap();
        assert_eq!(office_word::list_of(&doc, paragraphs[6]), Some((id.clone(), level)));
        let definition = &office_word::word(&doc).unwrap().numbering.levels(&id).unwrap()[level as usize];
        assert_eq!(definition.format, "hebrew2");
        assert_eq!(paragraphs[5].props.rtl, Some(true));
        for (i, prefix) in ["-", "*", "▪", "◦", "‣", "1."].into_iter().enumerate() {
            let edited = format!("{prefix} Item {i}");
            let (_, _, rest) = list_marker(&edited).unwrap();
            assert_eq!(rest, format!("Item {i}"));
        }
    }

    fn table_line(names: [&str; 3], y: i32) -> Value {
        let mut row = line(&names.join(" "), y, 20);
        row["words"] = Value::Array(names.into_iter().enumerate().map(|(column, text)| {
            json!({ "text": text, "bbox": bbox(20 + column as i32 * 130, y, 35, 20), "confidence": 99, "rtl": false })
        }).collect());
        row
    }

    #[test]
    fn reconstructs_aligned_rows_across_paragraphs_as_a_bordered_table() {
        let doc = round_trip(vec![page(
            0,
            vec![
                block(
                    bbox(0, 0, 320, 50),
                    vec![paragraph(vec![line("Before", 0, 20)]), paragraph(vec![table_line(["Name", "City", "Score"], 30)])],
                ),
                block(
                    bbox(0, 54, 320, 80),
                    vec![
                        paragraph(vec![table_line(["Noah", "Roma", "123"], 54), table_line(["ليلى", "Paris", "456"], 78)]),
                        paragraph(vec![line("After", 110, 20)]),
                    ],
                ),
            ],
        )]);
        let Body::Flow { blocks } = &doc.body else { panic!("expected a Word body") };
        assert_eq!(blocks.len(), 3);
        let Block::Table(table) = &blocks[1] else { panic!("expected a table") };
        assert_eq!(table.columns.len(), 3);
        let rows: Vec<_> = table.rows.iter().map(|r| r.cells.iter().map(Cell::plain_text).collect::<Vec<_>>()).collect();
        assert_eq!(rows, [["Name", "City", "Score"], ["Noah", "Roma", "123"], ["ليلى", "Paris", "456"]]);
        let first_cell = &table.rows[2].cells[0].blocks[0];
        assert!(matches!(first_cell, Block::Paragraph(p) if p.props.rtl == Some(true) && p.props.align == Some(Align::Right)));
        let borders = table.origin.get().unwrap().path(ns::W, &["tblPr", "tblBorders"]).unwrap();
        assert_eq!(borders.child(ns::W, "insideV").unwrap().attr_ns(ns::W, "sz"), Some("4"));
    }

    #[test]
    fn preserves_hebrew_arabic_mixed_text_and_explicit_rtl() {
        let mut explicit_rtl = paragraph(vec![line("123", 66, 20)]);
        explicit_rtl["rtl"] = json!(true);
        let doc = single_block(vec![
            paragraph(vec![line("שלום עולם", 0, 20)]),
            paragraph(vec![line("مرحبا بالعالم", 22, 20)]),
            paragraph(vec![line("English ואז עברית", 44, 20)]),
            explicit_rtl,
            paragraph(vec![line("Plain English", 88, 20)]),
        ]);
        assert_eq!(texts(&doc), ["שלום עולם", "مرحبا بالعالم", "English ואז עברית", "123", "Plain English"]);
        let paragraphs = office_word::paragraphs(&doc);
        for p in &paragraphs[..4] {
            assert_eq!(p.props.rtl, Some(true));
            assert_eq!(p.props.align, Some(Align::Right));
            assert!(p.origin.get().unwrap().path(ns::W, &["pPr", "bidi"]).is_some());
        }
        assert_eq!(paragraphs[4].props.rtl, None);
    }

    #[test]
    fn reads_columns_in_page_direction_and_handles_spanning_blocks() {
        let make_block = |label, x, y, w| block(bbox(x, y, w, 50), vec![paragraph(vec![line(label, y, 20)])]);
        let ltr = page(
            0,
            vec![
                make_block("Right two", 320, 105, 240),
                make_block("Left two", 20, 115, 240),
                make_block("Right one", 320, 50, 240),
                make_block("Spanning title", 10, 0, 550),
                make_block("Left one", 20, 60, 240),
            ],
        );
        let rtl = page(
            1,
            vec![
                make_block("ימין שני", 320, 105, 240),
                make_block("שמאל שני", 20, 115, 240),
                make_block("ימין ראשון", 320, 50, 240),
                make_block("שמאל ראשון", 20, 60, 240),
            ],
        );
        assert_eq!(
            texts(&round_trip(vec![rtl, ltr])),
            [
                "Spanning title",
                "Left one",
                "Left two",
                "Right one",
                "Right two",
                "\u{c}",
                "ימין ראשון",
                "ימין שני",
                "שמאל ראשון",
                "שמאל שני"
            ]
        );
    }

    #[test]
    fn sorts_rotated_single_column_blocks_and_skips_empty_pages() {
        let mut rotated = page(
            0,
            vec![
                block(bbox(80, 60, 25, 140), vec![paragraph(vec![line("Second", 0, 20)])]),
                block(bbox(20, 60, 25, 140), vec![paragraph(vec![line("First", 0, 20)])]),
            ],
        );
        rotated["detectedRotation"] = json!(90);
        let mut fallback = page(2, vec![]);
        fallback["text"] = json!("Another page");
        let doc = round_trip(vec![fallback, page(1, vec![]), rotated]);
        assert_eq!(texts(&doc), ["First", "Second", "\u{c}", "Another page"]);
        assert!(matches!(office_word::paragraphs(&doc)[2].content[0], Inline::Break { kind: BreakKind::Page, .. }));
    }

    #[test]
    fn handles_empty_documents_and_invalid_json() {
        for doc in [round_trip(vec![]), round_trip(vec![page(0, vec![]), page(1, vec![])])] {
            assert_eq!(texts(&doc), [""]);
        }
        for bad in ["{", "{}", r#"{"pages":[{"pageIndex":0}]}"#, r#"{"pages":"invalid"}"#] {
            assert!(ocr_document_to_docx(bad).unwrap_err().starts_with("Invalid OCR document JSON:"));
        }
    }
}
