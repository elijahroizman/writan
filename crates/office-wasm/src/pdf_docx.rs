//! Rebuilds an editable Word document from a PDF, the way Word opens one: the text PDFium reads
//! from each page, with its fonts, sizes, colours and links, joined into paragraphs in reading
//! order, with lists, tables and pictures, on the PDF's page size and margins.
//!
//! The app's PDF worker reads the pages (`layout` in `src/editors/pdf/engine/worker.ts`) into
//! fragments: runs of characters on one baseline that touch, split where their formatting
//! changes. Everything here is geometry on those fragments, in points from the page's top left.

use std::collections::HashMap;

use office_core::drawing::{Color, Rgb};
use office_core::model::{
    Align, Block, Body, Cell, Document, Image, Inline, Kind, LineSpacing, Link, Paragraph, Row, Run, RunProps, Table,
};
use office_core::model::{Baseline, Origin};
use office_core::units::{Emu, FontSize, Percent};
use office_core::xml::{ns, Element};
use office_core::Compiler;
use office_engine::para::first_strong_rtl;
use office_word::{Section, Words};
use serde::Deserialize;

use crate::ocr_docx::{
    column_cut, has_rtl, list_marker, list_numbering, median, reading_order, xml_text, ListMarker, OcrBBox as BBox, OcrNumbering,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    #[serde(default)]
    title: Option<String>,
    pages: Vec<PageIn>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageIn {
    width: f64,
    height: f64,
    #[serde(default)]
    fragments: Vec<FragmentIn>,
    #[serde(default)]
    pictures: Vec<PictureIn>,
    /// Thin horizontal and vertical lines drawn on the page: underlines and table borders.
    #[serde(default)]
    rules: Vec<BBox>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FragmentIn {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    baseline: f64,
    size: f64,
    #[serde(default)]
    rtl: bool,
    /// Read off a picture: recognized, or a scan's invisible text layer.
    #[serde(default)]
    ocr: bool,
    spans: Vec<SpanIn>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpanIn {
    text: String,
    #[serde(default)]
    baseline: Option<f64>,
    size: f64,
    #[serde(default)]
    font: Option<String>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    /// `RRGGBB`.
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    w: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PictureIn {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// Where the encoded picture sits in the media buffer passed alongside the JSON.
    offset: usize,
    length: usize,
    content_type: String,
    /// A drawing (a chart or diagram) taken as a picture, labels and all.
    #[serde(default)]
    figure: bool,
}

#[derive(Clone, Debug)]
struct Span {
    text: String,
    baseline: f64,
    size: f64,
    font: Option<String>,
    bold: bool,
    italic: bool,
    color: Option<Rgb>,
    link: Option<String>,
    underline: bool,
}

/// A run of text on one line: a fragment, or several that sat close together.
#[derive(Clone, Debug)]
struct Piece {
    b: BBox,
    baseline: f64,
    size: f64,
    rtl: bool,
    spans: Vec<Span>,
    /// Where the text after a separate list marker or label starts (its left edge, or right
    /// edge in right-to-left text).
    text_at: Option<f64>,
    /// Read off a picture (see [`FragmentIn::ocr`]).
    ocr: bool,
    /// The left and right edges of the column the piece sits in.
    col: (f64, f64),
    /// Tab stops the text jumps to (page positions; `true` for one that aligns text on its right).
    tabs: Vec<(f64, bool)>,
    page: usize,
}

impl Piece {
    fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn chars(&self) -> usize {
        self.spans.iter().map(|s| s.text.chars().filter(|c| !c.is_whitespace()).count()).sum()
    }

    fn all_bold(&self) -> bool {
        let (bold, all) = self.spans.iter().fold((0, 0), |(b, a), s| {
            let n = s.text.chars().filter(|c| !c.is_whitespace()).count();
            (b + if s.bold { n } else { 0 }, a + n)
        });
        all > 0 && bold * 10 >= all * 9
    }

    /// The left edge in left-to-right text, the right edge in right-to-left text.
    fn start(&self) -> f64 {
        if self.rtl {
            self.b.right()
        } else {
            self.b.x
        }
    }

    /// The width of the first word, roughly: whether it would have fit on the line before.
    fn first_word_width(&self) -> f64 {
        let text = self.text();
        let trimmed = text.trim();
        let chars = trimmed.chars().count().max(1) as f64;
        let word = trimmed.split_whitespace().next().map_or(0, |w| w.chars().count()) as f64;
        self.b.w * word / chars
    }
}

fn union(a: BBox, b: BBox) -> BBox {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    BBox { x, y, w: a.right().max(b.right()) - x, h: a.bottom().max(b.bottom()) - y }
}

fn overlap_x(a: BBox, b: BBox) -> f64 {
    a.right().min(b.right()) - a.x.max(b.x)
}

fn inside(b: BBox, x: f64, y: f64) -> bool {
    x >= b.x && x <= b.right() && y >= b.y && y <= b.bottom()
}

/// Private-use bullets (Symbol and Wingdings glyphs without a Unicode mapping) and other marks
/// that start a list item.
fn normalize_bullet(text: &str) -> Option<String> {
    let first = text.trim_start().chars().next()?;
    let bullet = matches!(
        first,
        '\u{F0B7}'
            | '\u{F0A7}'
            | '\u{F076}'
            | '\u{F0D8}'
            | '\u{F0FC}'
            | '\u{F06E}'
            | '\u{F0A8}'
            | '●'
            | '○'
            | '■'
            | '□'
            | '►'
            | '➢'
            | '✓'
            | '✔'
            | '·'
            | '∙'
            | '⁃'
    );
    bullet.then(|| {
        let rest = &text.trim_start()[first.len_utf8()..];
        format!("•{rest}")
    })
}

/// A marker on its own, set apart from its item's text: `•`, `1.`, `(a)`, `iv)`.
fn is_marker(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 5 {
        return false;
    }
    if t.chars().count() == 1 {
        let c = t.chars().next().unwrap();
        return matches!(c, '•' | '-' | '–' | '—' | '*' | '▪' | '◦' | '‣') || normalize_bullet(t).is_some();
    }
    let body = t.trim_start_matches('(');
    let Some(label) = body.strip_suffix(['.', ')']) else { return false };
    !label.is_empty()
        && (label.chars().all(|c| c.is_ascii_digit())
            || (label.chars().count() == 1 && label.chars().all(char::is_alphabetic))
            || label.chars().all(|c| matches!(c, 'i' | 'v' | 'x' | 'I' | 'V' | 'X')))
}

/// A section's number on its own: `4`, `2.1`, `A.3`, `IV`, `§ 12`.
fn is_section_number(text: &str) -> bool {
    let t = text.trim().trim_end_matches('.');
    let t = t.strip_prefix('§').map(str::trim_start).unwrap_or(t);
    !t.is_empty()
        && t.chars().count() <= 8
        && (t.split('.').all(|part| {
            !part.is_empty()
                && (part.chars().all(|c| c.is_ascii_digit()) || (part.len() == 1 && part.chars().all(|c| c.is_ascii_uppercase())))
        }) || t.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L')))
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}')
}

pub fn pdf_document_to_docx(json: &str, media: &[u8]) -> Result<Vec<u8>, String> {
    let input: Input = serde_json::from_str(json).map_err(|e| format!("Invalid PDF layout JSON: {e}"))?;
    write_document(input, media).map_err(|e| format!("Could not write the Word document: {e}"))
}

struct Page {
    width: f64,
    height: f64,
    pieces: Vec<Piece>,
    pictures: Vec<usize>,
    rules: Vec<BBox>,
}

fn write_document(input: Input, media: &[u8]) -> office_core::Result<Vec<u8>> {
    let hidden = running_heads(&input.pages);
    let mut pictures_in = Vec::new();
    let mut pages = Vec::new();
    for (index, page) in input.pages.into_iter().enumerate() {
        let figures: Vec<BBox> = page.pictures.iter().filter(|p| p.figure).map(|p| BBox { x: p.x, y: p.y, w: p.w, h: p.h }).collect();
        let mut pieces = Vec::new();
        for (f, fragment) in page.fragments.into_iter().enumerate() {
            if hidden.contains(&(index, f)) {
                continue;
            }
            let (cx, cy) = (fragment.x + fragment.w / 2.0, fragment.y + fragment.h / 2.0);
            if figures.iter().any(|b| inside(*b, cx, cy)) {
                continue;
            }
            if let Some(piece) = piece_from(fragment, index, &page.rules) {
                pieces.push(piece);
            }
        }
        let area = page.width * page.height;
        let text_chars: usize = pieces.iter().map(Piece::chars).sum();
        let mut pictures = Vec::new();
        for picture in page.pictures {
            let b = BBox { x: picture.x, y: picture.y, w: picture.w, h: picture.h };
            if b.w < 2.0 || b.h < 2.0 || picture.offset.saturating_add(picture.length) > media.len() || picture.length == 0 {
                continue;
            }
            let within = |p: &Piece| inside(b, p.b.x + p.b.w / 2.0, p.b.y + p.b.h / 2.0);
            // Recognized words over a picture: the picture is the scan they were read from when
            // they cover much of it, and a photo with some words in it otherwise.
            let read: f64 = pieces.iter().filter(|p| p.ocr && within(p)).map(|p| p.b.w * p.b.h).sum();
            if read > 0.0 {
                let page_sized = b.w * b.h >= area * 0.7;
                if read >= b.w * b.h * if page_sized { 0.02 } else { 0.08 } {
                    continue;
                }
                // A page-sized picture with a few words on it may still be a sparse scan: its
                // words stay, as well as the picture.
                if !page_sized {
                    pieces.retain(|p| !(p.ocr && within(p)));
                }
            }
            // A page-sized background behind a page of text.
            let covered: usize = pieces.iter().filter(|p| within(p)).map(Piece::chars).sum();
            if b.w * b.h > area * 0.5 && covered > 200 && covered * 2 > text_chars {
                continue;
            }
            pictures.push(pictures_in.len());
            pictures_in.push(picture);
        }
        pages.push(Page { width: page.width, height: page.height, pieces, pictures, rules: page.rules });
    }

    let body_size = body_size(&pages);
    let words = vocabulary(&pages);
    let geometry = Geometry::of(&pages, &pictures_in);
    let mut items: Vec<Item> = Vec::new();
    for page in &pages {
        let page_items = page_items(page, &pictures_in, body_size, &geometry);
        if page_items.is_empty() {
            continue;
        }
        let first = items.len();
        items.extend(page_items);
        if first > 0 {
            let continued = continues_across_pages(&items[first - 1], &items[first], body_size);
            if continued {
                let next = items.remove(first);
                if let (ItemKind::Para(a), ItemKind::Para(b)) = (&mut items[first - 1].kind, next.kind) {
                    a.extend(b);
                }
            } else {
                items[first].page_break = true;
            }
        }
    }

    let mut document = Document::new(Kind::Word);
    document.properties.title = input.title.map(|t| xml_text(&t)).filter(|t| !t.is_empty());
    let mut blocks = Vec::new();
    let mut lists: Vec<(usize, ListMarker)> = Vec::new();
    let mut picture_count = 0;
    let mut headings: Vec<f64> =
        items.iter().filter_map(|i| if let ItemKind::Para(lines) = &i.kind { heading_size(lines, body_size) } else { None }).collect();
    headings.sort_by(|a, b| b.total_cmp(a));
    headings.dedup();
    for item in &items {
        match &item.kind {
            ItemKind::Para(lines) => {
                let (paragraph, marker) = build_paragraph(lines, item, body_size, &headings, &words, &geometry);
                if let Some(marker) = marker {
                    lists.push((blocks.len(), marker));
                }
                blocks.push(Block::Paragraph(paragraph));
            }
            ItemKind::Table(table) => {
                if item.page_break {
                    blocks.push(Block::Paragraph(break_paragraph()));
                }
                blocks.push(Block::Table(build_table(table, &words, &geometry)));
            }
            ItemKind::Picture(index) => {
                let picture = &pictures_in[*index];
                let bytes = media[picture.offset..picture.offset + picture.length].to_vec();
                let id = document.add_media(&picture.content_type, bytes);
                picture_count += 1;
                blocks.push(Block::Paragraph(build_picture(item, id, picture_count, &geometry)));
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(Block::Paragraph(Paragraph::default()));
    }
    document.body = Body::Flow { blocks };
    office_word::set_section(&mut document, 0, &geometry.section());

    let compiler = Compiler::new().with_layer(Words);
    if lists.is_empty() {
        return compiler.write(&document, Kind::Word);
    }
    let numbering = list_numbering(&mut document, lists);
    compiler.with_layer(OcrNumbering(numbering)).write(&document, Kind::Word)
}

fn piece_from(fragment: FragmentIn, page: usize, rules: &[BBox]) -> Option<Piece> {
    let mut spans = Vec::new();
    let mut at = fragment.x;
    let total: usize = fragment.spans.iter().map(|s| s.text.chars().count()).sum::<usize>().max(1);
    for span in fragment.spans {
        let text = xml_text(&span.text.replace(['\r', '\n'], " "));
        if text.is_empty() {
            continue;
        }
        // Without its own box, a span's share of the fragment by characters.
        let w = span.w.unwrap_or(fragment.w * text.chars().count() as f64 / total as f64);
        let x = span.x.unwrap_or(at);
        at = x + w;
        let baseline = span.baseline.unwrap_or(fragment.baseline);
        let size = if span.size.is_finite() && span.size > 0.5 { span.size } else { fragment.size };
        let underline = !text.trim().is_empty()
            && rules.iter().any(|r| {
                r.h <= size * 0.15 + 0.5
                    && r.y >= baseline - size * 0.05
                    && r.y <= baseline + size * 0.3
                    && overlap_x(*r, BBox { x, y: 0.0, w, h: 0.0 }) >= w * 0.8
                    && r.w <= w + size * 3.0
            });
        spans.push(Span {
            text,
            baseline,
            size,
            font: span.font.filter(|f| !f.trim().is_empty()),
            bold: span.bold,
            italic: span.italic,
            color: span.color.as_deref().and_then(Rgb::parse),
            link: span.link.filter(|l| l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:")),
            underline,
        });
    }
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    if text.trim().is_empty() {
        return None;
    }
    let size = if fragment.size.is_finite() && fragment.size > 0.5 { fragment.size } else { fragment.h.max(1.0) * 0.8 };
    let rtl = fragment.rtl || first_strong_rtl(&text) == Some(true);
    let ocr = fragment.ocr;
    Some(Piece {
        b: BBox { x: fragment.x, y: fragment.y, w: fragment.w, h: fragment.h },
        baseline: fragment.baseline,
        size,
        rtl,
        spans,
        text_at: None,
        ocr,
        col: (0.0, 0.0),
        tabs: Vec::new(),
        page,
    })
}

/// Fragments repeated at the top or bottom of most pages (running heads, page numbers), which
/// a reflowed document would otherwise carry into the middle of its text.
fn running_heads(pages: &[PageIn]) -> std::collections::HashSet<(usize, usize)> {
    let mut hidden = std::collections::HashSet::new();
    if pages.len() < 2 {
        return hidden;
    }
    let key = |text: &str| -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .map(|c| if c.is_ascii_digit() { '#' } else { c.to_ascii_lowercase() })
            .collect()
    };
    let mut seen: HashMap<(bool, String), std::collections::HashSet<usize>> = HashMap::new();
    let mut candidates = Vec::new();
    for (p, page) in pages.iter().enumerate() {
        // Lines in the top and bottom bands, read whole: a heading's "1:" isn't a page number.
        let mut lines: Vec<(bool, f64, Vec<usize>)> = Vec::new();
        for (f, fragment) in page.fragments.iter().enumerate() {
            let top = fragment.y + fragment.h < page.height * 0.08;
            let bottom = fragment.y > page.height * 0.92;
            if !top && !bottom {
                continue;
            }
            match lines.iter_mut().find(|(t, baseline, _)| *t == top && (baseline - fragment.baseline).abs() < fragment.size * 0.5) {
                Some(line) => line.2.push(f),
                None => lines.push((top, fragment.baseline, vec![f])),
            }
        }
        for (top, _, mut fragments) in lines {
            fragments.sort_by(|&a, &b| page.fragments[a].x.total_cmp(&page.fragments[b].x));
            let text: Vec<String> = fragments.iter().map(|&f| page.fragments[f].spans.iter().map(|s| s.text.as_str()).collect()).collect();
            let k = key(&text.join(" "));
            if k.trim().is_empty() {
                continue;
            }
            let page_number = {
                let t = k.trim().trim_matches(|c: char| !c.is_alphanumeric() && c != '#');
                let t = t.strip_prefix("page").map(str::trim).unwrap_or(t);
                !t.is_empty() && t.split([' ', '/']).filter(|w| !w.is_empty()).all(|w| w.chars().all(|c| c == '#') || w == "of")
            };
            seen.entry((top, k.clone())).or_default().insert(p);
            candidates.push((p, fragments, top, k, page_number));
        }
    }
    let needed = pages.len().div_ceil(2).max(2);
    for (p, fragments, top, k, page_number) in candidates {
        if page_number || seen.get(&(top, k)).is_some_and(|s| s.len() >= needed) {
            hidden.extend(fragments.into_iter().map(|f| (p, f)));
        }
    }
    hidden
}

/// The size most of the text is set in.
fn body_size(pages: &[Page]) -> f64 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for span in pages.iter().flat_map(|p| &p.pieces).flat_map(|p| &p.spans) {
        *counts.entry((span.size * 2.0).round() as u32).or_default() += span.text.chars().count();
    }
    counts.into_iter().max_by_key(|&(size, n)| (n, size)).map_or(11.0, |(size, _)| size as f64 / 2.0).clamp(4.0, 72.0)
}

/// The document's page: the most common page size, and margins from where the text sits.
struct Geometry {
    width: f64,
    height: f64,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

impl Geometry {
    fn of(pages: &[Page], pictures: &[PictureIn]) -> Geometry {
        let mut sizes: HashMap<(i64, i64), (usize, f64, f64)> = HashMap::new();
        for page in pages {
            sizes.entry((page.width.round() as i64, page.height.round() as i64)).or_insert((0, page.width, page.height)).0 += 1;
        }
        let (width, height) = sizes.into_values().max_by_key(|&(n, _, _)| n).map_or((612.0, 792.0), |(_, w, h)| (w, h));
        let (mut left, mut right, mut top, mut bottom) = (f64::MAX, f64::MAX, f64::MAX, f64::MAX);
        for page in pages {
            let boxes = page.pieces.iter().map(|p| p.b).chain(page.pictures.iter().map(|&i| {
                let p = &pictures[i];
                BBox { x: p.x, y: p.y, w: p.w, h: p.h }
            }));
            for b in boxes {
                left = left.min(b.x);
                right = right.min(page.width - b.right());
                top = top.min(b.y);
                bottom = bottom.min(page.height - b.bottom());
            }
        }
        // Margins of at least 18 points (less on a label-sized page), leaving room for text.
        let (least_x, least_y) = (18.0f64.min(width * 0.1), 18.0f64.min(height * 0.1));
        let fit = |v: f64, least: f64, most: f64| if v == f64::MAX { 72.0f64.min(most) } else { v.min(most).max(least) };
        let mut left = fit(left, least_x, width * 0.35);
        let mut right = fit(right, least_x, width * 0.35);
        if width - left - right < 144.0f64.min(width * 0.6) {
            left = least_x;
            right = least_x;
        }
        let top = fit(top, least_y, height * 0.3);
        // Some room below the last line, in case the fonts on this computer set the text a little
        // longer than the PDF's, and no deeper than the top margin where the text simply stops
        // short of the page's foot.
        let bottom = if bottom == f64::MAX { top } else { fit(bottom - 24.0, least_y, top.max(72.0).min(height * 0.3)) };
        Geometry { width, height, left, right, top, bottom }
    }

    fn text_width(&self) -> f64 {
        self.width - self.left - self.right
    }

    fn section(&self) -> Section {
        let pt = Emu::from_points;
        Section {
            page_width: pt(self.width),
            page_height: pt(self.height),
            landscape: self.width > self.height,
            margin_top: pt(self.top),
            margin_right: pt(self.right),
            margin_bottom: pt(self.bottom),
            margin_left: pt(self.left),
            header: pt((self.top * 0.5).min(36.0)),
            footer: pt((self.bottom * 0.5).min(36.0)),
            ..Section::default()
        }
    }
}

struct TableBuild {
    /// Each column's left and right edge.
    columns: Vec<(f64, f64)>,
    /// Rows of cells, each cell the pieces in it, top to bottom.
    rows: Vec<Vec<Vec<Piece>>>,
    ruled_h: bool,
    ruled_v: bool,
}

/// A column's left and right edges.
type Column = (f64, f64);

enum ItemKind {
    Para(Vec<Piece>),
    Table(TableBuild),
    Picture(usize),
}

struct Item {
    b: BBox,
    kind: ItemKind,
    /// Points between this and what's above it on the page.
    space_before: f64,
    /// The column this sits in: its left and right edges.
    column: (f64, f64),
    page_break: bool,
}

/// One page's paragraphs, tables and pictures, in reading order.
fn page_items(page: &Page, pictures: &[PictureIn], body: f64, geometry: &Geometry) -> Vec<Item> {
    let mut lines = lines(page.pieces.clone());
    let picture_boxes: Vec<BBox> =
        page.pictures.iter().map(|&i| BBox { x: pictures[i].x, y: pictures[i].y, w: pictures[i].w, h: pictures[i].h }).collect();
    let area = (geometry.left, page.width - geometry.right);
    let text_width = (page.width - geometry.left - geometry.right).max(144.0);
    let tables = find_tables(&mut lines, &page.rules, text_width);
    rejoin_prose(&mut lines);
    // Tables take part as wholes: their cells side by side aren't columns of text.
    let table_boxes: Vec<BBox> = tables.iter().map(|t| t.rows.iter().flatten().flatten().map(|p| p.b).reduce(union).unwrap()).collect();
    let mut boxes = picture_boxes.clone();
    boxes.extend(&table_boxes);
    let mut columns = assign_columns(&mut lines, &boxes, body * 0.9, area);
    let table_columns = columns.split_off(picture_boxes.len());
    let picture_columns = columns;
    let rest: Vec<Piece> = lines.into_iter().flatten().collect();
    let paragraphs = group_paragraphs(rest);

    let span = |cols: &mut dyn Iterator<Item = (f64, f64)>| cols.fold((f64::MAX, f64::MIN), |(l, r), (a, b)| (l.min(a), r.max(b)));
    let mut positioned: Vec<(BBox, (ItemKind, Column))> = Vec::new();
    for paragraph in paragraphs {
        let b = paragraph.iter().map(|p| p.b).reduce(union).unwrap();
        let column = span(&mut paragraph.iter().map(|p| p.col));
        positioned.push((b, (ItemKind::Para(paragraph), column)));
    }
    for (n, table) in tables.into_iter().enumerate() {
        positioned.push((table_boxes[n], (ItemKind::Table(table), table_columns[n])));
    }
    for (n, &picture) in page.pictures.iter().enumerate() {
        positioned.push((picture_boxes[n], (ItemKind::Picture(picture), picture_columns[n])));
    }
    let rtl = {
        let rtl: usize = page.pieces.iter().filter(|p| p.rtl).map(Piece::chars).sum();
        rtl * 2 > page.pieces.iter().map(Piece::chars).sum::<usize>()
    };
    let ordered = reading_order(positioned, rtl, body * 0.9, 0);
    let mut items: Vec<Item> = Vec::new();
    for (b, (kind, mut column)) in ordered {
        // Text that fills most of the page's width sits between the margins.
        if column.1 - column.0 >= text_width * 0.75 {
            column = (column.0.min(geometry.left), column.1.max(page.width - geometry.right));
        }
        let space_before = match items.last() {
            Some(prev) if prev.b.bottom() <= b.y + 1.0 => b.y - prev.b.bottom(),
            Some(_) => 0.0,
            None => b.y - geometry.top,
        };
        let item = Item { b, kind, space_before: space_before.max(0.0), column, page_break: false };
        // A paragraph that runs on from the foot of one column to the top of the next.
        if let Some(prev) = items.last_mut() {
            if b.y < prev.b.bottom() - 1.0 && column.0 >= prev.column.1 - 1.0 && continues_across_pages(prev, &item, body) {
                if let (ItemKind::Para(a), ItemKind::Para(more)) = (&mut prev.kind, item.kind) {
                    a.extend(more);
                }
                continue;
            }
        }
        items.push(item);
    }
    items
}

/// Gives each line piece the edges of the column it sits in (and returns the pictures'), by
/// splitting the page where it splits into side-by-side columns.
fn assign_columns(lines: &mut [Vec<Piece>], pictures: &[BBox], min_gap: f64, area: (f64, f64)) -> Vec<(f64, f64)> {
    let mut boxes: Vec<(BBox, usize)> =
        lines.iter().flatten().map(|p| p.b).chain(pictures.iter().copied()).enumerate().map(|(i, b)| (b, i)).collect();
    let mut out = vec![(0.0, 0.0); boxes.len()];
    split_columns(&mut boxes, min_gap, 0, area, &mut out);
    let mut at = 0;
    for piece in lines.iter_mut().flatten() {
        piece.col = out[at];
        at += 1;
    }
    out[at..].to_vec()
}

/// `area` is the page's text area: what a page with no columns, or text across them, sits in.
fn split_columns(boxes: &mut [(BBox, usize)], min_gap: f64, depth: u8, area: (f64, f64), out: &mut [(f64, f64)]) {
    let extent = |set: &[(BBox, usize)]| set.iter().fold((f64::MAX, f64::MIN), |(l, r), (b, _)| (l.min(b.x), r.max(b.right())));
    let mut whole = extent(boxes);
    if depth == 0 {
        whole = (whole.0.min(area.0), whole.1.max(area.1));
    }
    let split = (boxes.len() > 1 && depth < 8).then(|| column_cut(boxes, min_gap)).flatten();
    if let Some(split) = split {
        boxes.sort_by_key(|(b, _)| {
            if b.right() <= split {
                0
            } else if b.x >= split {
                2
            } else {
                1
            }
        });
        let left = boxes.iter().take_while(|(b, _)| b.right() <= split).count();
        let right = boxes.iter().rev().take_while(|(b, _)| b.x >= split).count();
        // Side-by-side columns have some length; a label beside a line of text doesn't.
        if left >= 3 && right >= 3 {
            let n = boxes.len();
            for (_, i) in &boxes[left..n - right] {
                out[*i] = whole;
            }
            let (head, tail) = boxes.split_at_mut(n - right);
            split_columns(&mut head[..left], min_gap, depth + 1, area, out);
            split_columns(tail, min_gap, depth + 1, area, out);
            return;
        }
    }
    for (_, i) in boxes.iter() {
        out[*i] = whole;
    }
}

/// Pieces on the same baseline, left to right, with those that sit close together joined.
fn lines(mut pieces: Vec<Piece>) -> Vec<Vec<Piece>> {
    pieces.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.b.x.total_cmp(&b.b.x)));
    let mut lines: Vec<Vec<Piece>> = Vec::new();
    for piece in pieces {
        if let Some(line) = lines.last_mut() {
            let first = &line[0];
            let same = (piece.baseline - first.baseline).abs() < 0.35 * piece.size.min(first.size)
                || (piece.size < first.size * 0.85
                    && piece.b.y + piece.b.h * 0.5 > first.b.y
                    && piece.b.y + piece.b.h * 0.5 < first.b.bottom());
            if same && !line.iter().any(|p| overlap_x(p.b, piece.b) > piece.b.w.min(p.b.w) * 0.5) {
                line.push(piece);
                continue;
            }
        }
        lines.push(vec![piece]);
    }
    lines.into_iter().map(join_line).collect()
}

fn join_line(mut line: Vec<Piece>) -> Vec<Piece> {
    let rtl = {
        let rtl: usize = line.iter().filter(|p| p.rtl).map(Piece::chars).sum();
        rtl * 2 > line.iter().map(Piece::chars).sum::<usize>()
    };
    if rtl {
        line.sort_by(|a, b| b.b.right().total_cmp(&a.b.right()));
    } else {
        line.sort_by(|a, b| a.b.x.total_cmp(&b.b.x));
    }
    let count = line.len();
    let mut out: Vec<Piece> = Vec::new();
    for mut piece in line {
        if let Some(first) = piece.spans.first_mut() {
            if let Some(bullet) = normalize_bullet(&first.text) {
                first.text = bullet;
            }
        }
        let only = out.len() == 1;
        if let Some(prev) = out.last_mut() {
            let gap = if rtl { prev.b.x - piece.b.right() } else { piece.b.x - prev.b.right() };
            let s = prev.size.max(piece.size);
            // Recognized text reads a bullet as a letter (`e`, `o`): one character on its own,
            // in no font, set apart before the text.
            let read_bullet = only
                && prev.spans.iter().all(|sp| sp.font.is_none())
                && prev.text().trim().chars().count() == 1
                && prev.text().trim().chars().all(|c| matches!(c, 'e' | 'o' | '°' | '«' | '»' | '©' | '®' | '*' | '+'))
                && gap > s * 0.5;
            if read_bullet {
                if let Some(first) = prev.spans.first_mut() {
                    first.text = "•".into();
                }
            }
            let marker = only && prev.text_at.is_none() && is_marker(&prev.text()) && gap < s * 8.0;
            // A heading's number, set a space apart from its title: `2.1   Methods`.
            let number = only && count == 2 && is_section_number(&prev.text()) && gap < s * 4.0 && piece.size >= prev.size * 0.9;
            // Wider gaps separate table cells and columns; `rejoin_prose` closes those that aren't.
            if number {
                absorb(prev, piece, gap, false);
                continue;
            }
            if gap < s * 0.8 || marker {
                if marker {
                    prev.text_at = Some(piece.start());
                }
                absorb(prev, piece, gap, marker);
                continue;
            }
        }
        out.push(piece);
    }
    if !rtl {
        return out;
    }
    out.sort_by(|a, b| a.b.x.total_cmp(&b.b.x));
    out
}

/// Appends `piece`, which follows `prev` on its line `gap` points further on.
fn absorb(prev: &mut Piece, piece: Piece, gap: f64, marker: bool) {
    let s = prev.size.max(piece.size);
    let separator = if marker && gap > s * 0.6 { "\t" } else { " " };
    let needs_space = !prev.text().ends_with(char::is_whitespace) && !piece.text().starts_with(char::is_whitespace);
    if needs_space && gap > s * 0.1 || marker {
        if let Some(last) = prev.spans.last_mut() {
            last.text = last.text.trim_end().to_string();
            last.text.push_str(separator);
        }
    }
    prev.b = union(prev.b, piece.b);
    if piece.size > prev.size {
        prev.size = piece.size;
        prev.baseline = piece.baseline;
    }
    prev.spans.extend(piece.spans);
}

/// Joins the parts of a line split at a wide gap, unless the lines around it leave the same gap
/// (a gutter between columns, or a tab stop in a form).
fn rejoin_prose(lines: &mut [Vec<Piece>]) {
    for i in 0..lines.len() {
        if lines[i].len() < 2 {
            continue;
        }
        let rtl = lines[i].iter().filter(|p| p.rtl).count() * 2 > lines[i].len();
        let baseline = lines[i][0].baseline;
        let size = lines[i].iter().map(|p| p.size).fold(0.0, f64::max);
        let neighbors: Vec<usize> = (i.saturating_sub(3)..(i + 4).min(lines.len()))
            .filter(|&j| j != i && (lines[j][0].baseline - baseline).abs() <= size * 5.0)
            .collect();
        let mut pieces = std::mem::take(&mut lines[i]);
        let count = pieces.len();
        if rtl {
            pieces.reverse();
        }
        let mut out: Vec<Piece> = Vec::new();
        for piece in pieces {
            if let Some(prev) = out.last_mut() {
                let (left, right) = if rtl { (piece.b.right(), prev.b.x) } else { (prev.b.right(), piece.b.x) };
                let gap = right - left;
                // Text runs across the whole gap in the lines around: a wide space in justified text.
                let covered =
                    neighbors.iter().filter(|&&j| lines[j].iter().any(|p| p.b.x < left + 1.0 && p.b.right() > right - 1.0)).count();
                if gap < size * 3.0 && !neighbors.is_empty() && covered * 2 >= neighbors.len() && covered >= 1 {
                    absorb(prev, piece, gap, false);
                    continue;
                }
                // Two parts of a line on their own, such as a heading and its marks set to the
                // right: one paragraph with a tab between, unless the lines around share the gap.
                let gutter =
                    neighbors.iter().any(|&j| lines[j].windows(2).any(|w| w[0].b.right() < right && (w[1].b.x - right).abs() < size));
                if count == 2 && !rtl && !gutter {
                    let to_right = piece.b.right() >= piece.col.1.max(prev.col.1) - size;
                    prev.tabs.push(if to_right { (piece.b.right(), true) } else { (piece.b.x, false) });
                    if let Some(last) = prev.spans.last_mut() {
                        last.text = last.text.trim_end().to_string();
                        last.text.push('\t');
                    }
                    prev.b = union(prev.b, piece.b);
                    prev.spans.extend(piece.spans);
                    continue;
                }
            }
            out.push(piece);
        }
        out.sort_by(|a, b| a.b.x.total_cmp(&b.b.x));
        lines[i] = out;
    }
}

/// Takes rows of text set in aligned columns out of `lines` as tables.
fn find_tables(lines: &mut Vec<Vec<Piece>>, rules: &[BBox], text_width: f64) -> Vec<TableBuild> {
    let mut tables = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].len() < 2 {
            i += 1;
            continue;
        }
        let mut columns: Vec<(f64, f64)> = lines[i].iter().map(|p| (p.b.x, p.b.right())).collect();
        let mut rows: Vec<Vec<Vec<Piece>>> = vec![lines[i].iter().map(|p| vec![p.clone()]).collect()];
        let mut end = i + 1;
        while end < lines.len() {
            let line = &lines[end];
            let prev = &lines[end - 1];
            let size = line.iter().chain(prev.iter()).map(|p| p.size).fold(0.0, f64::max);
            let step = line[0].baseline - prev[0].baseline;
            if step > size * 3.0 || step <= 0.0 {
                break;
            }
            let Some(slots) = fit_columns(&columns, line, size) else { break };
            if line.len() == 1 {
                // A wrapped cell's next line, close under the row before.
                if step > size * 1.6 || end == i + 1 && rows[0].iter().filter(|c| !c.is_empty()).count() < 2 {
                    break;
                }
                let row = rows.last_mut().unwrap();
                match slots[0] {
                    Slot::Column(c) if !row[c].is_empty() => row[c].push(line[0].clone()),
                    _ => break,
                }
                end += 1;
                continue;
            }
            for (piece, slot) in line.iter().zip(&slots) {
                if let Slot::New(at) = *slot {
                    columns.insert(at, (piece.b.x, piece.b.right()));
                    for r in rows.iter_mut() {
                        r.insert(at, Vec::new());
                    }
                }
            }
            let mut row: Vec<Vec<Piece>> = vec![Vec::new(); columns.len()];
            for (piece, slot) in line.iter().zip(&slots) {
                let c = match *slot {
                    Slot::Column(c) | Slot::New(c) => c,
                };
                columns[c].0 = columns[c].0.min(piece.b.x);
                columns[c].1 = columns[c].1.max(piece.b.right());
                row[c].push(piece.clone());
            }
            rows.push(row);
            end += 1;
        }
        let row_count = rows.len();
        let top = lines[i][0].b.y;
        let bottom = lines[end - 1].iter().map(|p| p.b.bottom()).fold(0.0, f64::max);
        let left = columns.first().map_or(0.0, |c| c.0);
        let right = columns.last().map_or(0.0, |c| c.1);
        let near = |r: &&BBox| {
            r.y >= top - 12.0 && r.bottom() <= bottom + 12.0 && overlap_x(**r, BBox { x: left, y: 0.0, w: right - left, h: 0.0 }) > 0.0
        };
        let ruled_h = rules.iter().filter(near).filter(|r| r.w > r.h && r.w >= (right - left) * 0.3).count() >= 2;
        let ruled_v = rules.iter().filter(near).filter(|r| r.h > r.w).count() >= 2;
        let pieces: Vec<&Piece> = rows.iter().flatten().flatten().collect();
        let is_wide = |p: &Piece| p.b.w > text_width * 0.28;
        let wide = pieces.iter().filter(|p| is_wide(p)).count();
        // Columns of running text: two or more columns whose lines are mostly long.
        let wide_columns = (0..columns.len())
            .filter(|&c| {
                let cells: Vec<&Piece> = rows.iter().flat_map(|r| r[c].iter()).collect();
                !cells.is_empty() && cells.iter().filter(|p| is_wide(p)).count() * 2 >= cells.len()
            })
            .count();
        let sizes = pieces.iter().map(|p| p.size);
        let (small, large) = sizes.fold((f64::MAX, 0.0f64), |(a, b), s| (a.min(s), b.max(s)));
        let prose = wide * 10 > pieces.len() * 6 || wide_columns >= 2 || large > small * 1.35;
        let ok = columns.len() >= 2 && !prose && (row_count >= 3 || (row_count >= 2 && (columns.len() >= 3 || ruled_h || ruled_v)));
        if ok {
            lines.drain(i..end);
            tables.push(TableBuild { columns, rows, ruled_h, ruled_v });
        } else {
            i += 1;
        }
    }
    tables
}

enum Slot {
    Column(usize),
    New(usize),
}

/// Which column each piece of a line falls in, or `None` when a piece spans columns.
fn fit_columns(columns: &[(f64, f64)], line: &[Piece], size: f64) -> Option<Vec<Slot>> {
    let mut used = vec![false; columns.len()];
    let mut slots = Vec::new();
    for piece in line {
        let hits: Vec<usize> =
            columns.iter().enumerate().filter(|(_, c)| piece.b.x < c.1 && piece.b.right() > c.0).map(|(i, _)| i).collect();
        match hits.as_slice() {
            [c] if !used[*c] => {
                used[*c] = true;
                slots.push(Slot::Column(*c));
            }
            [] => {
                // A new column in a gap between the others (or past them), clear of both sides.
                let at = columns.iter().position(|c| c.0 > piece.b.right()).unwrap_or(columns.len());
                let clear_left = at == 0 || columns[at - 1].1 + size * 0.5 < piece.b.x;
                let clear_right = at == columns.len() || piece.b.right() + size * 0.5 < columns[at].0;
                if !clear_left || !clear_right || line.len() == 1 {
                    return None;
                }
                slots.push(Slot::New(at));
            }
            _ => return None,
        }
    }
    // One new column at most per row; the columns after it move along one.
    let mut news = slots.iter().filter_map(|s| if let Slot::New(at) = s { Some(*at) } else { None });
    let Some(at) = news.next() else { return Some(slots) };
    if news.next().is_some() {
        return None;
    }
    for slot in slots.iter_mut() {
        if let Slot::Column(c) = slot {
            if *c >= at {
                *c += 1;
            }
        }
    }
    Some(slots)
}

/// Gathers lines into paragraphs: each line goes under the one above it when it continues it.
fn group_paragraphs(mut pieces: Vec<Piece>) -> Vec<Vec<Piece>> {
    pieces.sort_by(|a, b| a.b.y.total_cmp(&b.b.y).then(a.b.x.total_cmp(&b.b.x)));
    let mut paragraphs: Vec<Vec<Piece>> = Vec::new();
    for piece in pieces {
        let best = paragraphs
            .iter()
            .enumerate()
            .filter(|(_, p)| continues(p, &piece))
            .min_by(|(_, a), (_, b)| {
                let ga = piece.baseline - a.last().unwrap().baseline;
                let gb = piece.baseline - b.last().unwrap().baseline;
                ga.total_cmp(&gb)
            })
            .map(|(i, _)| i);
        match best {
            Some(i) => paragraphs[i].push(piece),
            None => paragraphs.push(vec![piece]),
        }
    }
    paragraphs
}

fn continues(paragraph: &[Piece], q: &Piece) -> bool {
    let last = paragraph.last().unwrap();
    let first = &paragraph[0];
    let s = last.size.max(q.size);
    let gap = q.baseline - last.baseline;
    if gap <= last.size * 0.5 || q.b.y < last.b.y {
        return false;
    }
    if paragraph.len() >= 2 {
        let pitch = last.baseline - paragraph[paragraph.len() - 2].baseline;
        if (gap - pitch).abs() > pitch * 0.25 + 0.5 {
            return false;
        }
    } else if gap > last.size.max(q.size) * 1.75 {
        return false;
    }
    let ratio = q.size / last.size;
    if !(0.87..=1.15).contains(&ratio) || last.rtl != q.rtl || last.all_bold() != q.all_bold() {
        return false;
    }
    if overlap_x(last.b, q.b) <= 0.0 {
        return false;
    }
    if list_marker(q.text().trim()).is_some() || is_marker(q.text().split_whitespace().next().unwrap_or("")) && q.text_at.is_some() {
        return false;
    }
    // The line broke where the text didn't fill it: the next word would have fit. A paragraph's
    // own lines show how wide it runs; a first line alone only has its column to go by.
    let (col_left, col_right) = if paragraph.len() >= 2 { (f64::MAX, f64::MIN) } else { last.col };
    let right_edge = paragraph.iter().map(|p| p.b.right()).fold(q.b.right().max(col_right), f64::max);
    let left_edge = paragraph.iter().map(|p| p.b.x).fold(q.b.x.min(col_left), f64::min);
    let centered = ((last.b.x + last.b.w / 2.0) - (q.b.x + q.b.w / 2.0)).abs() <= s && (last.b.x - q.b.x).abs() > s;
    let room = if centered {
        (right_edge - left_edge) - last.b.w
    } else if q.rtl {
        last.b.x - left_edge
    } else {
        right_edge - last.b.right()
    };
    // A list item's next line, lined up under its text after the marker, continues it.
    let indent = if q.rtl { first.start() - q.start() } else { q.start() - first.start() };
    let hanging = paragraph.len() == 1
        && match first.text_at {
            Some(at) => (q.start() - at).abs() <= s * 0.8,
            None => list_marker(first.text().trim()).is_some() && indent > s * 0.5 && indent < s * 4.0,
        };
    if !hanging && room > q.first_word_width() + s * 0.6 {
        return false;
    }
    let start_gap = (q.start() - last.start()).abs();
    let aligned = start_gap <= s * 0.8;
    let indented_first = paragraph.len() == 1 && {
        let indent = if q.rtl { q.start() - first.start() } else { first.start() - q.start() };
        indent > s * 0.8 && indent <= s * 6.0
    };
    let right_aligned = !q.rtl && (last.b.right() - q.b.right()).abs() <= s * 0.8 && (last.b.x - q.b.x).abs() > s;
    aligned || indented_first || hanging || centered || right_aligned
}

/// Whether the last paragraph on a page runs on into the first one on the next.
fn continues_across_pages(prev: &Item, next: &Item, body: f64) -> bool {
    let (ItemKind::Para(a), ItemKind::Para(b)) = (&prev.kind, &next.kind) else { return false };
    let (last, first) = (a.last().unwrap(), &b[0]);
    let text = last.text();
    let text = text.trim_end();
    let ends = text.ends_with(['.', '!', '?', ':', ';', '。', '"', '”', ')']);
    let full = if last.rtl { last.b.x <= prev.column.0 + body * 2.0 } else { last.b.right() >= prev.column.1 - body * 2.0 };
    let ratio = first.size / last.size;
    let indent = (first.start() - if first.rtl { next.column.1 } else { next.column.0 }).abs();
    !ends
        && full
        && (0.9..=1.1).contains(&ratio)
        && indent < body * 0.5
        && last.all_bold() == first.all_bold()
        && list_marker(first.text().trim()).is_none()
}

fn break_paragraph() -> Paragraph {
    let mut paragraph = Paragraph { origin: page_break_origin(), ..Default::default() };
    paragraph.props.line_spacing = Some(LineSpacing::Exactly(Emu::from_points(1.0)));
    paragraph.props.space_before = Some(Emu(0));
    paragraph.props.space_after = Some(Emu(0));
    paragraph.mark = Some(RunProps { size: Some(FontSize::from_points(1.0)), ..Default::default() });
    paragraph
}

fn page_break_origin() -> Origin {
    paragraph_origin(true, &[])
}

/// A new paragraph's markup for what the model doesn't hold: starting a page, and tab stops
/// (points from the margin; `true` for a right tab).
fn paragraph_origin(page_break: bool, tabs: &[(f64, bool)]) -> Origin {
    let mut ppr = Element::new(ns::W, "pPr");
    if page_break {
        ppr.push(Element::new(ns::W, "pageBreakBefore"));
    }
    if !tabs.is_empty() {
        let mut list = Element::new(ns::W, "tabs");
        for &(pos, right) in tabs {
            list.push(Element::new(ns::W, "tab").with_attr(ns::W, "val", if right { "right" } else { "left" }).with_attr(
                ns::W,
                "pos",
                Emu::from_points(pos.max(0.0)).twips().to_string(),
            ));
        }
        ppr.push(list);
    }
    Origin::new(Element::new(ns::W, "p").with_child(ppr))
}

fn round_half(points: f64) -> f64 {
    (points * 2.0).round() / 2.0
}

fn run_props(span: &Span, line: &Piece, heading: bool) -> RunProps {
    let raised = span.size < line.size * 0.85 && span.baseline < line.baseline - line.size * 0.2;
    let lowered = span.size < line.size * 0.85 && span.baseline > line.baseline + line.size * 0.1;
    let shifted = raised || lowered;
    // Word sets super- and subscripts smaller itself, from the size of the text around them.
    let size = round_half(if shifted { line.size } else { span.size }).max(1.0);
    let black = span.color.is_none_or(|c| c == Rgb::BLACK);
    let mut props = RunProps {
        font: span.font.clone(),
        size: Some(FontSize::from_points(size)),
        bold: if span.bold {
            Some(true)
        } else if heading {
            Some(false)
        } else {
            None
        },
        italic: if span.italic {
            Some(true)
        } else if heading {
            Some(false)
        } else {
            None
        },
        color: if !black {
            span.color.map(Color::rgb)
        } else if heading {
            Some(Color::rgb(Rgb::BLACK))
        } else {
            None
        },
        baseline: if raised {
            Some(Baseline::Superscript)
        } else if lowered {
            Some(Baseline::Subscript)
        } else {
            None
        },
        underline: span.underline.then(|| "single".to_string()),
        ..Default::default()
    };
    if has_rtl(&span.text) {
        props.font_complex = span.font.clone();
    }
    if span.text.chars().any(is_cjk) {
        props.font_east_asian = span.font.clone();
    }
    props
}

fn push_spans(content: &mut Vec<Inline>, line: &Piece, heading: bool) {
    for span in &line.spans {
        let run = Run {
            text: span.text.clone(),
            props: run_props(span, line, heading),
            link: span.link.clone().map(|url| Link { url: Some(url), ..Default::default() }),
            ..Default::default()
        };
        if let Some(Inline::Text(prev)) = content.last_mut() {
            if prev.props == run.props && prev.link == run.link {
                prev.text.push_str(&run.text);
                continue;
            }
        }
        content.push(Inline::Text(run));
    }
}

fn line_spacing(lines: &[Piece], size: f64) -> (Option<LineSpacing>, f64) {
    let pitch = median(lines.windows(2).filter(|w| w[0].page == w[1].page).map(|w| w[1].baseline - w[0].baseline));
    let Some(pitch) = pitch else { return (Some(LineSpacing::Multiple(Percent(Percent::WHOLE))), 0.0) };
    let natural = size * 1.15;
    let ratio = pitch / natural;
    if ratio < 0.92 {
        (Some(LineSpacing::Exactly(Emu::from_points(pitch))), 0.0)
    } else if ratio < 1.08 {
        (Some(LineSpacing::Multiple(Percent(Percent::WHOLE))), 0.0)
    } else {
        let multiple = ((ratio * 20.0).round() / 20.0).min(3.0);
        (Some(LineSpacing::Multiple(Percent((multiple * Percent::WHOLE as f64) as i32))), pitch - natural)
    }
}

/// The size of a paragraph set larger than the body text, short enough to be a heading.
fn heading_size(lines: &[Piece], body: f64) -> Option<f64> {
    let chars: usize = lines.iter().map(Piece::chars).sum();
    let size = round_half(median(lines.iter().map(|l| l.size))?);
    (lines.len() <= 3 && chars <= 200 && size >= body * 1.15 && list_marker(lines[0].text().trim()).is_none()).then_some(size)
}

/// `headings` are the heading sizes in the document, largest first: the largest is Heading 1.
fn build_paragraph(
    lines: &[Piece],
    item: &Item,
    body: f64,
    headings: &[f64],
    words: &Vocabulary,
    geometry: &Geometry,
) -> (Paragraph, Option<ListMarker>) {
    let first = &lines[0];
    let rtl = lines.iter().filter(|l| l.rtl).count() * 2 > lines.len();
    let size = median(lines.iter().map(|l| l.size)).unwrap_or(body);
    let heading_level = heading_size(lines, body).map(|s| headings.iter().position(|&h| h <= s).unwrap_or(headings.len()).min(2) as u8 + 1);
    let heading = heading_level.is_some();

    let mut content: Vec<Inline> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            join_lines(&mut content, line, words);
        }
        push_spans(&mut content, line, heading);
    }
    // A list marker of the kinds Word numbers becomes a real list; its text goes.
    let mut marker = None;
    let text: String = content.iter().map(|i| if let Inline::Text(r) = i { r.text.as_str() } else { "" }).collect();
    let lead = text.len() - text.trim_start().len();
    if let Some((m, marks, rest)) = list_marker(&text[lead..]) {
        marker = Some(m);
        // Drop the label (it may be a run of its own, in bold) and the space after it.
        let mut cut = text.len() - rest.len() - marks.len();
        for inline in content.iter_mut() {
            let Inline::Text(run) = inline else { continue };
            let take = cut.min(run.text.len());
            run.text.replace_range(..take, "");
            cut -= take;
            if cut == 0 {
                break;
            }
        }
        if let Some(Inline::Text(run)) = content.iter_mut().find(|i| matches!(i, Inline::Text(r) if !r.text.is_empty())) {
            run.text.insert_str(0, marks);
        }
        content.retain(|i| !matches!(i, Inline::Text(r) if r.text.is_empty()));
    }
    // A line to write on, drawn with underscores to the margin: another font's wider underscores
    // would wrap it onto a second line, so it's left a little short.
    let reaches_margin = lines.iter().all(|l| l.b.right() >= item.column.1 - size * 2.0);
    if reaches_margin
        && lines.len() == 1
        && first.text().trim().len() >= 8
        && first.text().trim().chars().all(|c| matches!(c, '_' | '.' | '…'))
    {
        for inline in content.iter_mut() {
            if let Inline::Text(run) = inline {
                let keep = run.text.chars().count() * 4 / 5;
                run.text = run.text.chars().take(keep).collect();
            }
        }
    }
    let mut paragraph = Paragraph { content, ..Default::default() };
    let tabs: Vec<(f64, bool)> = lines.iter().flat_map(|l| &l.tabs).map(|&(at, right)| (at - item.column.0, right)).collect();
    if item.page_break || !tabs.is_empty() {
        paragraph.origin = paragraph_origin(item.page_break, &tabs);
    }
    if rtl {
        paragraph.props.rtl = Some(true);
    }
    if let Some(level) = heading_level {
        paragraph.props.style = Some(format!("Heading{level}"));
    }
    let (spacing, extra) = line_spacing(lines, size);
    paragraph.props.line_spacing = spacing;
    paragraph.props.space_before = Some(Emu::from_points((item.space_before - extra).clamp(0.0, 400.0).round()));
    paragraph.props.space_after = Some(Emu(0));

    let (left, right) = item.column;
    let s = size;
    let width = right - left;
    // Lines that ran on into the next column, moved back into the first one's place.
    let moved: Vec<Piece> = lines
        .iter()
        .map(|l| {
            let mut l = l.clone();
            if (l.col.0 - first.col.0).abs() > 1.0 {
                l.b.x += first.col.0 - l.col.0;
            }
            l
        })
        .collect();
    let lines = &moved[..];
    let centers_aligned = lines.iter().all(|l| ((l.b.x + l.b.w / 2.0) - (left + width / 2.0)).abs() <= s * 1.2);
    let lefts_aligned = lines.iter().skip(1).all(|l| (l.b.x - lines[1.min(lines.len() - 1)].b.x).abs() <= s * 0.5);
    let rights_aligned = |set: &[Piece]| set.iter().all(|l| (l.b.right() - set[0].b.right()).abs() <= s * 0.5);
    let start_gap = if rtl {
        right - lines.iter().map(|l| l.b.right()).fold(f64::MIN, f64::max)
    } else {
        lines.iter().map(|l| l.b.x).fold(f64::MAX, f64::min) - left
    };
    let end_gap = if rtl {
        lines.iter().map(|l| l.b.x).fold(f64::MAX, f64::min) - left
    } else {
        right - lines.iter().map(|l| l.b.right()).fold(f64::MIN, f64::max)
    };
    let align = if lines.len() >= 3 && rights_aligned(&lines[..lines.len() - 1]) && (lefts_aligned || rtl) && end_gap < s * 2.0 {
        Some(Align::Justify)
    } else if centers_aligned && start_gap > s * 1.5 && end_gap > s * 1.5 && (start_gap - end_gap).abs() <= s * 2.4 {
        Some(Align::Center)
    } else if !rtl && end_gap <= s * 2.5 && start_gap > width * 0.3 && (lines.len() == 1 || rights_aligned(lines)) {
        Some(Align::Right)
    } else if !rtl && lines.len() == 1 && start_gap > width * 0.5 && end_gap < start_gap * 0.5 {
        // A short line set far over to the right, short of the margin: `[2 marks]`.
        paragraph.props.indent_end = Some(Emu::from_points(end_gap.round()));
        Some(Align::Right)
    } else if rtl
        && start_gap > width * 0.3
        && end_gap <= s * 1.0
        && (lines.len() == 1 || lines.iter().all(|l| (l.b.x - lines[0].b.x).abs() <= s * 0.5))
    {
        Some(Align::Left)
    } else if rtl {
        Some(Align::Right)
    } else {
        None
    };
    paragraph.props.align = align;

    if matches!(align, None | Some(Align::Justify)) || (rtl && align == Some(Align::Right)) {
        let edge = |p: &Piece| if rtl { right - p.b.right() } else { p.b.x - left };
        let body_start = if lines.len() >= 2 { lines[1..].iter().map(edge).fold(f64::MAX, f64::min) } else { edge(first) };
        let first_start = edge(first);
        let pt = |v: f64| Emu::from_points(v.round());
        // A marker in the same run of text as the item: its next line shows where the text starts.
        let text_at = first.text_at.or((marker.is_some() && lines.len() >= 2).then(|| lines[1].start()));
        if let Some(at) = text_at {
            let text_start = if rtl { right - at } else { at - left };
            paragraph.props.indent_start = Some(pt(text_start.clamp(0.0, geometry.text_width() * 0.8)));
            paragraph.props.indent_first_line = Some(pt(-(text_start - first_start).max(0.0)));
        } else if marker.is_some() {
            paragraph.props.indent_start = Some(pt((first_start + 18.0).clamp(0.0, geometry.text_width() * 0.8)));
            paragraph.props.indent_first_line = Some(pt(-18.0));
        } else {
            if body_start > s * 0.3 {
                paragraph.props.indent_start = Some(pt(body_start.clamp(0.0, geometry.text_width() * 0.8)));
            }
            let first_line = first_start - body_start;
            if lines.len() >= 2 && first_line.abs() > s * 0.3 {
                paragraph.props.indent_first_line = Some(pt(first_line));
            }
        }
    }
    (paragraph, marker)
}

/// Joins a paragraph's next line onto what came before: a space, or nothing where a word was
/// hyphenated across the break or the script doesn't use spaces.
/// Every word in the document, lowercased: evidence for joining a word hyphenated across lines.
type Vocabulary = std::collections::HashSet<String>;

fn word_key(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn vocabulary(pages: &[Page]) -> Vocabulary {
    pages
        .iter()
        .flat_map(|p| &p.pieces)
        .flat_map(|p| &p.spans)
        .flat_map(|s| s.text.split_whitespace())
        .map(word_key)
        .filter(|w| !w.is_empty())
        .collect()
}

/// Joins a paragraph's next line onto what came before: a space, or nothing where a word was
/// hyphenated across the break or the script doesn't use spaces.
fn join_lines(content: &mut [Inline], next: &Piece, words: &Vocabulary) {
    let next_text = next.text();
    let next_first = next_text.trim_start().chars().next();
    let Some(Inline::Text(run)) = content.iter_mut().rev().find(|i| matches!(i, Inline::Text(_))) else { return };
    let trimmed = run.text.trim_end().len();
    run.text.truncate(trimmed);
    let before = run.text.chars().rev().nth(1);
    let last = run.text.chars().next_back();
    if last == Some('-') && before.is_some_and(char::is_alphabetic) && next_first.is_some_and(char::is_lowercase) {
        // A hyphen at the end of a line either breaks a word (`recon-` `struction`) or belongs to
        // it (`well-` `known`). It goes only when the document has the word without it.
        let word = run.text.rsplit(char::is_whitespace).next().unwrap_or("");
        let joined = word_key(&format!("{}{}", word.trim_end_matches('-'), next_text.split_whitespace().next().unwrap_or("")));
        if words.contains(&joined) {
            run.text.pop();
        }
    } else if last.is_some_and(is_cjk) && next_first.is_some_and(is_cjk) {
    } else if last == Some('\u{AD}') {
        run.text.pop();
    } else {
        run.text.push(' ');
    }
}

fn cell_paragraph(pieces: &[Piece], align: Option<Align>, words: &Vocabulary) -> Paragraph {
    let mut content = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 {
            join_lines(&mut content, piece, words);
        }
        push_spans(&mut content, piece, false);
    }
    let mut paragraph = Paragraph { content, ..Default::default() };
    let rtl = pieces.iter().any(|p| p.rtl);
    if rtl {
        paragraph.props.rtl = Some(true);
    }
    paragraph.props.align = align.or(rtl.then_some(Align::Right));
    paragraph.props.space_before = Some(Emu(0));
    paragraph.props.space_after = Some(Emu(0));
    paragraph.props.line_spacing = Some(LineSpacing::Multiple(Percent(Percent::WHOLE)));
    paragraph
}

/// Word's default cell margin, each side.
const CELL_MARGIN: f64 = 5.4;

fn build_table(table: &TableBuild, words: &Vocabulary, geometry: &Geometry) -> Table {
    let n = table.columns.len();
    let mut edges = vec![table.columns[0].0 - CELL_MARGIN];
    for i in 1..n {
        edges.push((table.columns[i - 1].1 + table.columns[i].0) / 2.0);
    }
    edges.push(table.columns[n - 1].1 + CELL_MARGIN);
    let columns: Vec<Emu> = edges.windows(2).map(|w| Emu::from_points((w[1] - w[0]).max(12.0))).collect();
    // How each column's cells line up: on the right (numbers), in the middle, or on the left.
    let aligns: Vec<Option<Align>> = (0..n)
        .map(|c| {
            let cells: Vec<&Piece> = table.rows.iter().filter_map(|r| r[c].first()).collect();
            if cells.len() < 2 {
                return None;
            }
            let spread = |f: &dyn Fn(&Piece) -> f64| {
                let v: Vec<f64> = cells.iter().map(|p| f(p)).collect();
                v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
            };
            let size = cells.iter().map(|p| p.size).fold(0.0, f64::max);
            let (l, r, m) = (spread(&|p| p.b.x), spread(&|p| p.b.right()), spread(&|p| p.b.x + p.b.w / 2.0));
            if r <= size * 0.4 && l > size {
                Some(Align::Right)
            } else if m <= size * 0.4 && l > size && r > size {
                Some(Align::Center)
            } else {
                None
            }
        })
        .collect();
    let rows = table
        .rows
        .iter()
        .map(|row| Row {
            cells: row
                .iter()
                .enumerate()
                .map(|(c, pieces)| Cell { blocks: vec![Block::Paragraph(cell_paragraph(pieces, aligns[c], words))], ..Default::default() })
                .collect(),
            ..Default::default()
        })
        .collect();
    let border = |side: &str| {
        Element::new(ns::W, side).with_attr(ns::W, "val", "single").with_attr(ns::W, "sz", "4").with_attr(ns::W, "space", "0").with_attr(
            ns::W,
            "color",
            "auto",
        )
    };
    let mut sides = Vec::new();
    if table.ruled_h {
        sides.extend(["top", "bottom", "insideH"]);
    }
    if table.ruled_v {
        sides.extend(["left", "right", "insideV"]);
    }
    let mut pr =
        Element::new(ns::W, "tblPr").with_child(Element::new(ns::W, "tblW").with_attr(ns::W, "w", "0").with_attr(ns::W, "type", "auto"));
    // From the margin to the table's left border, so the first column's text starts where it did.
    let indent = edges[0] - geometry.left;
    if indent > 0.5 {
        pr.push(Element::new(ns::W, "tblInd").with_attr(ns::W, "w", Emu::from_points(indent).twips().to_string()).with_attr(
            ns::W,
            "type",
            "dxa",
        ));
    }
    if !sides.is_empty() {
        let mut borders = Element::new(ns::W, "tblBorders");
        for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            if sides.contains(&side) {
                borders.push(border(side));
            }
        }
        pr.push(borders);
    }
    pr.push(Element::new(ns::W, "tblLayout").with_attr(ns::W, "type", "fixed"));
    Table { columns, rows, style: None, origin: Origin::new(Element::new(ns::W, "tbl").with_child(pr)) }
}

fn build_picture(item: &Item, media: String, n: usize, geometry: &Geometry) -> Paragraph {
    let max = geometry.text_width();
    let scale = if item.b.w > max { max / item.b.w } else { 1.0 };
    let (w, h) = (item.b.w * scale, item.b.h * scale);
    let image = Image {
        media,
        width: Emu::from_points(w),
        height: Emu::from_points(h),
        name: format!("Picture {n}"),
        description: None,
        origin: Origin::default(),
    };
    let mut paragraph = Paragraph { content: vec![Inline::Image(image)], ..Default::default() };
    if item.page_break {
        paragraph.origin = page_break_origin();
    }
    let (left, right) = item.column;
    let middle = item.b.x + item.b.w / 2.0;
    if scale < 1.0 {
    } else if (middle - (left + right) / 2.0).abs() <= (right - left) * 0.05 && item.b.x - left > 12.0 {
        paragraph.props.align = Some(Align::Center);
    } else if right - item.b.right() <= 6.0 && item.b.x - left > (right - left) * 0.3 {
        paragraph.props.align = Some(Align::Right);
    } else if item.b.x - geometry.left > 2.0 {
        paragraph.props.indent_start = Some(Emu::from_points((item.b.x - geometry.left).min(max - w).max(0.0).round()));
    }
    paragraph.props.space_before = Some(Emu::from_points(item.space_before.clamp(0.0, 400.0).round()));
    paragraph.props.space_after = Some(Emu(0));
    paragraph.props.line_spacing = Some(LineSpacing::Multiple(Percent(Percent::WHOLE)));
    paragraph
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn span(text: &str, size: f64) -> Value {
        json!({ "text": text, "size": size, "font": "Arial" })
    }

    /// A line of text starting at `x` whose baseline is `y`, `width` points wide.
    fn frag(x: f64, y: f64, width: f64, size: f64, spans: Vec<Value>) -> Value {
        json!({ "x": x, "y": y - size * 0.8, "w": width, "h": size * 1.1, "baseline": y, "size": size, "spans": spans })
    }

    fn text(x: f64, y: f64, width: f64, t: &str) -> Value {
        frag(x, y, width, 11.0, vec![span(t, 11.0)])
    }

    fn page(fragments: Vec<Value>) -> Value {
        json!({ "width": 612, "height": 792, "fragments": fragments })
    }

    fn convert(pages: Vec<Value>) -> Document {
        convert_with(json!({ "title": "Converted", "pages": pages }), &[])
    }

    fn convert_with(input: Value, media: &[u8]) -> Document {
        let bytes = pdf_document_to_docx(&input.to_string(), media).unwrap();
        Compiler::new().with_layer(Words).read(&bytes, Some("out.docx")).unwrap()
    }

    fn texts(doc: &Document) -> Vec<String> {
        office_word::paragraphs(doc).into_iter().map(Paragraph::plain_text).collect()
    }

    #[test]
    fn joins_wrapped_lines_into_paragraphs_and_keeps_page_setup() {
        let doc = convert(vec![page(vec![
            text(72.0, 100.0, 460.0, "The first paragraph wraps over two lines because it"),
            text(72.0, 114.0, 200.0, "is long enough."),
            text(72.0, 142.0, 466.0, "A second paragraph starts after a gap and goes on to"),
            text(72.0, 156.0, 120.0, "its end."),
        ])]);
        assert_eq!(
            texts(&doc),
            [
                "The first paragraph wraps over two lines because it is long enough.",
                "A second paragraph starts after a gap and goes on to its end."
            ]
        );
        let section = &office_word::sections(&doc)[0];
        assert_eq!(section.page_width, Emu::from_points(612.0));
        assert_eq!(section.page_height, Emu::from_points(792.0));
        assert_eq!(section.margin_left, Emu::from_points(72.0));
        assert_eq!(doc.properties.title.as_deref(), Some("Converted"));
        let p = office_word::paragraphs(&doc)[1];
        assert_eq!(p.props.space_after, Some(Emu(0)));
        assert!(p.props.space_before.unwrap().points() > 8.0);
    }

    #[test]
    fn keeps_short_lines_that_were_broken_on_purpose() {
        let doc = convert(vec![page(vec![
            text(72.0, 100.0, 70.0, "Jane Doe"),
            text(72.0, 114.0, 90.0, "1 Main Street"),
            text(72.0, 128.0, 80.0, "Springfield"),
            text(72.0, 160.0, 468.0, "Dear Jane, a letter's text that reaches the right margin of the page"),
        ])]);
        assert_eq!(texts(&doc)[..3], ["Jane Doe", "1 Main Street", "Springfield"]);
    }

    #[test]
    fn keeps_fonts_sizes_styles_colors_and_links() {
        let doc = convert(vec![page(vec![
            frag(
                72.0,
                80.0,
                200.0,
                24.0,
                vec![json!({ "text": "Big Title", "size": 24, "font": "Georgia", "bold": true, "color": "1F4E79" })],
            ),
            frag(
                72.0,
                120.0,
                300.0,
                11.0,
                vec![
                    span("Plain, ", 11.0),
                    json!({ "text": "italic", "size": 11, "font": "Arial", "italic": true }),
                    span(" and ", 11.0),
                    json!({ "text": "a link", "size": 11, "font": "Arial", "link": "https://example.com", "color": "0000FF" }),
                ],
            ),
        ])]);
        let paragraphs = office_word::paragraphs(&doc);
        assert_eq!(paragraphs[0].props.style.as_deref(), Some("Heading1"));
        let title = paragraphs[0].runs().next().unwrap();
        assert_eq!(title.props.font.as_deref(), Some("Georgia"));
        assert_eq!(title.props.size, Some(FontSize::from_points(24.0)));
        assert_eq!(title.props.bold, Some(true));
        assert_eq!(title.props.color, Some(Color::rgb(Rgb(0x1F, 0x4E, 0x79))));
        let runs: Vec<_> = paragraphs[1].runs().collect();
        assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<String>(), "Plain, italic and a link");
        assert!(runs.iter().any(|r| r.text == "italic" && r.props.italic == Some(true)));
        let link = runs.iter().find(|r| r.text == "a link").unwrap();
        assert_eq!(link.link.as_ref().and_then(|l| l.url.as_deref()), Some("https://example.com"));
    }

    #[test]
    fn rebuilds_lists_from_markers_set_apart_from_their_text() {
        let doc = convert(vec![page(vec![
            text(72.0, 100.0, 5.0, "\u{F0B7}"),
            text(90.0, 100.0, 200.0, "First item"),
            text(72.0, 114.0, 5.0, "\u{F0B7}"),
            text(90.0, 114.0, 200.0, "Second item"),
            text(72.0, 140.0, 10.0, "1."),
            text(90.0, 140.0, 200.0, "Numbered"),
        ])]);
        assert_eq!(texts(&doc), ["First item", "Second item", "Numbered"]);
        let labels = office_word::list_labels(&doc);
        assert_eq!(labels, [Some("•".into()), Some("•".into()), Some("1.".into())]);
        let p = office_word::paragraphs(&doc)[0];
        assert_eq!(p.props.indent_start, Some(Emu::from_points(18.0)));
        assert_eq!(p.props.indent_first_line, Some(Emu::from_points(-18.0)));
    }

    #[test]
    fn finds_tables_in_aligned_columns_but_not_in_two_column_text() {
        let row = |y: f64, cells: [&str; 3]| -> Vec<Value> {
            cells.iter().enumerate().map(|(i, t)| text(72.0 + i as f64 * 150.0, y, 6.0 * t.len() as f64, t)).collect()
        };
        let mut fragments = vec![text(72.0, 80.0, 100.0, "Before")];
        fragments.extend(row(110.0, ["Name", "City", "Score"]));
        fragments.extend(row(126.0, ["Ann", "Rome", "12"]));
        fragments.extend(row(142.0, ["Bob", "Paris", "7"]));
        let doc = convert(vec![page(fragments)]);
        let Body::Flow { blocks } = &doc.body else { panic!() };
        let table = blocks.iter().find_map(|b| if let Block::Table(t) = b { Some(t) } else { None }).expect("a table");
        let cells: Vec<Vec<String>> = table.rows.iter().map(|r| r.cells.iter().map(Cell::plain_text).collect()).collect();
        assert_eq!(cells, [["Name", "City", "Score"], ["Ann", "Rome", "12"], ["Bob", "Paris", "7"]]);

        // Two columns of running text read down the left column, then the right.
        let mut columns = Vec::new();
        for i in 0..6 {
            let y = 100.0 + i as f64 * 14.0;
            let end = if i == 5 { "." } else { "" };
            columns.push(text(72.0, y, 220.0, &format!("left column line {i} with words that fill{end}")));
            columns.push(text(320.0, y, 220.0, &format!("right column line {i} with words that fill")));
        }
        let doc = convert(vec![page(columns)]);
        let Body::Flow { blocks } = &doc.body else { panic!() };
        assert!(blocks.iter().all(|b| matches!(b, Block::Paragraph(_))));
        let all = texts(&doc);
        assert_eq!(all.len(), 2);
        assert!(all[0].starts_with("left column line 0") && all[0].ends_with("line 5 with words that fill."));
        assert!(all[1].starts_with("right column line 0"));
    }

    #[test]
    fn breaks_pages_drops_running_heads_and_joins_paragraphs_across_pages() {
        let head = |n: usize| text(72.0, 40.0, 100.0, &format!("Annual report {n}"));
        let number = |n: usize| text(300.0, 760.0, 10.0, &n.to_string());
        let doc = convert(vec![
            page(vec![head(1), text(72.0, 100.0, 468.0, "This sentence runs to the end of the page and"), number(1)]),
            page(vec![head(2), text(72.0, 100.0, 200.0, "carries on here."), number(2)]),
            page(vec![head(3), text(72.0, 100.0, 200.0, "A new page."), number(3)]),
        ]);
        assert_eq!(texts(&doc), ["This sentence runs to the end of the page and carries on here.", "A new page."]);
        let p = office_word::paragraphs(&doc)[1];
        assert!(p.origin.get().unwrap().path(ns::W, &["pPr", "pageBreakBefore"]).is_some());
    }

    #[test]
    fn places_pictures_in_reading_order_and_skips_scans_behind_text() {
        let png = b"\x89PNG\r\n\x1a\nfake";
        let input = json!({
            "pages": [{
                "width": 612, "height": 792,
                "fragments": [text(72.0, 100.0, 468.0, "Above, a line of text from margin to margin"), text(72.0, 400.0, 100.0, "Below")],
                "pictures": [{ "x": 206, "y": 150, "w": 200, "h": 150, "offset": 0, "length": png.len(), "contentType": "image/png" }],
            }, {
                "width": 612, "height": 792,
                "fragments": (0..12).map(|i| {
                    let mut line = text(72.0, 100.0 + i as f64 * 14.0, 468.0, "Recognized words in a line of a scanned page that reaches the margin");
                    line["ocr"] = json!(true);
                    line
                }).collect::<Vec<_>>(),
                "pictures": [{ "x": 0, "y": 0, "w": 612, "h": 792, "offset": 0, "length": png.len(), "contentType": "image/png" }],
            }]
        });
        let doc = convert_with(input, png);
        let paragraphs = office_word::paragraphs(&doc);
        assert!(matches!(paragraphs[1].content[0], Inline::Image(ref i) if i.width == Emu::from_points(200.0)));
        assert_eq!(paragraphs[1].props.align, Some(Align::Center));
        let all = texts(&doc);
        assert_eq!(all[..3], ["Above, a line of text from margin to margin", "", "Below"]);
        assert!(all[3].starts_with("Recognized words in a line"), "{all:?}");
        assert_eq!(doc.media.len(), 1);
    }

    #[test]
    fn right_to_left_text_and_centered_lines() {
        let doc = convert(vec![page(vec![
            json!({ "x": 380, "y": 92, "w": 160, "h": 12, "baseline": 100, "size": 11, "rtl": true, "spans": [span("שלום עולם", 11.0)] }),
            text(256.0, 130.0, 100.0, "Centered"),
            text(72.0, 160.0, 468.0, "A full line of ordinary text that reaches the right margin"),
        ])]);
        let paragraphs = office_word::paragraphs(&doc);
        assert_eq!(paragraphs[0].props.rtl, Some(true));
        assert_eq!(paragraphs[1].props.align, Some(Align::Center));
    }

    #[test]
    fn heading_numbers_tabs_and_justified_gaps_stay_on_their_line() {
        let doc = convert(vec![page(vec![
            // A section number a space apart from its title.
            frag(72.0, 80.0, 10.0, 14.0, vec![span("4", 14.0)]),
            frag(94.0, 80.0, 90.0, 14.0, vec![span("Next steps", 14.0)]),
            // A heading with its marks set against the right margin.
            text(72.0, 110.0, 150.0, "Section 1: Analysis"),
            text(480.0, 110.0, 60.0, "(12 marks)"),
            // Justified text with one wide space, in lines that run across it.
            text(72.0, 140.0, 468.0, "A justified paragraph whose lines reach the right margin of"),
            text(72.0, 154.0, 200.0, "the page and in which"),
            text(285.0, 154.0, 255.0, "one space was set much wider than the rest"),
            text(72.0, 168.0, 468.0, "so the line is in two parts that belong together in one line"),
            text(72.0, 182.0, 100.0, "of text."),
        ])]);
        let all = texts(&doc);
        assert_eq!(all[0], "4 Next steps");
        assert_eq!(all[1], "Section 1: Analysis\t(12 marks)");
        assert!(all[2].contains("in which one space was set much wider"), "{all:?}");
        let tabs = office_word::paragraphs(&doc)[1].origin.get().unwrap().path(ns::W, &["pPr", "tabs"]).cloned().unwrap();
        assert_eq!(tabs.child(ns::W, "tab").unwrap().attr_ns(ns::W, "val"), Some("right"));
    }

    #[test]
    fn reads_two_columns_in_order_and_runs_a_paragraph_on_across_them() {
        let mut fragments = vec![frag(72.0, 80.0, 120.0, 14.0, vec![span("Next steps", 14.0)])];
        let left = ["The first column starts here and its", "lines run down the left side of the", "page until the column ends, and the"];
        let right = ["sentence carries on at the top of the", "right column, where it finally ends.", "Short."];
        for i in 0..3 {
            let y = 110.0 + i as f64 * 14.0;
            fragments.push(text(72.0, y, 229.0, left[i]));
            fragments.push(text(311.0, y, if i == 2 { 40.0 } else { 229.0 }, right[i]));
        }
        let doc = convert(vec![page(fragments)]);
        assert_eq!(
            texts(&doc),
            [
                "Next steps",
                "The first column starts here and its lines run down the left side of the page until the column ends, and the sentence carries on at the top of the right column, where it finally ends. Short."
            ]
        );
    }

    #[test]
    fn keeps_numbers_in_a_heading_at_the_top_of_each_page() {
        let heading = |n: u32| frag(72.0, 60.0, 300.0, 16.0, vec![span(&format!("Section {n}: Part"), 16.0)]);
        let doc = convert(vec![
            page(vec![heading(1), text(72.0, 200.0, 100.0, "Body one.")]),
            page(vec![heading(2), text(72.0, 200.0, 100.0, "Body two.")]),
        ]);
        assert_eq!(texts(&doc), ["Section 1: Part", "Body one.", "Section 2: Part", "Body two."]);
    }

    #[test]
    fn a_photo_keeps_its_picture_and_a_scan_keeps_its_words() {
        let png = b"\x89PNG\r\n\x1a\nfake";
        let word = |x: f64, y: f64, t: &str| {
            let mut w = text(x, y, 60.0, t);
            w["ocr"] = json!(true);
            w
        };
        let mut fragments = vec![text(72.0, 80.0, 468.0, "A page with a photo of an advert and a scan of a letter beside it")];
        // A photo with two words in it.
        fragments.extend([word(100.0, 200.0, "HEINZ"), word(100.0, 230.0, "KETCHUP")]);
        // A scan, covering 40% of the page, full of recognized words.
        for i in 0..20 {
            fragments.push(word(330.0, 160.0 + i as f64 * 14.0, "Dear"));
            fragments.push(word(394.0, 160.0 + i as f64 * 14.0, "reader"));
            fragments.push(word(458.0, 160.0 + i as f64 * 14.0, "letter"));
        }
        let picture =
            |x: f64, w: f64| json!({ "x": x, "y": 140, "w": w, "h": 400, "offset": 0, "length": png.len(), "contentType": "image/png" });
        let doc = convert_with(
            json!({ "pages": [{ "width": 612, "height": 792, "fragments": fragments, "pictures": [picture(80.0, 200.0), picture(320.0, 230.0)] }] }),
            png,
        );
        let all = texts(&doc).join("|");
        assert_eq!(doc.media.len(), 1, "the photo stays, the scan goes: {all}");
        assert!(!all.contains("HEINZ"), "{all}");
        assert!(all.contains("Dear reader letter"), "{all}");
    }

    #[test]
    fn a_sparse_full_page_scan_keeps_its_word() {
        let png = b"\x89PNG\r\n\x1a\nfake";
        let mut word = text(72.0, 100.0, 60.0, "Approved");
        word["ocr"] = json!(true);
        let picture = json!({ "x": 0, "y": 0, "w": 612, "h": 792, "offset": 0, "length": png.len(), "contentType": "image/png" });
        let doc = convert_with(json!({ "pages": [{ "width": 612, "height": 792, "fragments": [word], "pictures": [picture] }] }), png);
        assert!(texts(&doc).contains(&"Approved".to_string()));
    }

    #[test]
    fn keeps_hyphenated_words_and_joins_broken_ones() {
        let doc = convert(vec![page(vec![
            text(72.0, 100.0, 468.0, "This paragraph about the well-"),
            text(72.0, 114.0, 468.0, "known state-of-the-"),
            text(72.0, 128.0, 468.0, "art methods and their recon-"),
            text(72.0, 142.0, 200.0, "struction goes on."),
            text(72.0, 170.0, 300.0, "A reconstruction is said once more."),
        ])]);
        assert_eq!(texts(&doc)[0], "This paragraph about the well-known state-of-the-art methods and their reconstruction goes on.");
    }

    #[test]
    fn label_sized_pages_get_margins_that_fit() {
        let doc = convert_with(
            json!({ "pages": [{ "width": 50, "height": 80, "fragments": [frag(4.0, 20.0, 40.0, 6.0, vec![span("Label", 6.0)])] }] }),
            &[],
        );
        let section = &office_word::sections(&doc)[0];
        assert!(section.text_width().0 > 0);
        assert!(section.margin_top.points() + section.margin_bottom.points() < 80.0);
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(pdf_document_to_docx("{", &[]).unwrap_err().starts_with("Invalid PDF layout JSON"));
        assert_eq!(texts(&convert(vec![])), [""]);
    }
}
