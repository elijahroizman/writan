//! Passes between the model's shapes: a document's body into another kind's, tables to sheets
//! and back, and any document to plain text. [`crate::Compiler::write`] runs [`to_kind`]
//! when it's asked for a kind the body isn't.

use crate::drawing::{ShapeProps, Xfrm};
use crate::model::*;
use crate::units::Emu;
use crate::Result;

/// Whether a document's body is what `kind` holds.
pub fn fits(doc: &Document, kind: Kind) -> bool {
    matches!(
        (&doc.body, kind),
        (Body::Flow { .. }, Kind::Word) | (Body::Slides { .. }, Kind::Slides) | (Body::Workbook { .. }, Kind::Sheets | Kind::Csv)
    )
}

/// The document as `kind`: its body reshaped if need be, its pictures loaded into memory (the
/// package they're in doesn't come along), its theme and properties kept.
pub fn to_kind(doc: &Document, kind: Kind) -> Result<Document> {
    let mut out = Document::new(kind);
    out.properties = doc.properties.clone();
    out.theme = doc.theme.clone();
    out.extensions = doc.extensions.clone();
    for m in &doc.media {
        let data = MediaData::Bytes(doc.media_bytes(&m.id)?);
        out.media.push(Media { id: m.id.clone(), content_type: m.content_type.clone(), data });
    }
    out.body = match (&doc.body, kind) {
        (body, _) if fits(doc, kind) => body.clone(),
        (Body::Flow { blocks }, Kind::Slides) => flow_to_slides(blocks),
        (Body::Flow { blocks }, _) => Body::Workbook { sheets: blocks_to_sheets(blocks) },
        (Body::Slides { slides, .. }, Kind::Word) => Body::Flow { blocks: slides_to_blocks(slides) },
        (Body::Slides { slides, .. }, _) => {
            let blocks = slides_to_blocks(slides);
            Body::Workbook { sheets: blocks_to_sheets(&blocks) }
        }
        (Body::Workbook { sheets }, Kind::Word) => Body::Flow { blocks: sheets_to_blocks(sheets)? },
        (Body::Workbook { sheets }, _) => sheets_to_slides(sheets)?,
    };
    if doc.kind == Kind::Csv && kind == Kind::Sheets {
        if let Body::Workbook { sheets } = &mut out.body {
            let names = crate::formats::xlsx::generated_sheet_names(sheets)?;
            for (sheet, name) in sheets.iter_mut().zip(names) {
                sheet.name = name;
            }
        }
    }
    // A CSV file's dialect only means something to CSV.
    if kind != Kind::Csv {
        out.extensions.remove::<crate::formats::csv::Dialect>();
    }
    Ok(out)
}

/// A table's text as a sheet, its spans as merged ranges.
pub fn table_to_sheet(table: &Table, name: &str) -> Sheet {
    let mut sheet = Sheet::new(name);
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if cell.covered {
                continue;
            }
            let text = cell.plain_text();
            if !text.is_empty() {
                sheet.set(CellRef::new(r as u32, c as u32), text_value(text));
            }
            if cell.col_span > 1 || cell.row_span > 1 {
                let end = CellRef::new(r as u32 + cell.row_span - 1, c as u32 + cell.col_span - 1);
                sheet.merges.push(Range { start: CellRef::new(r as u32, c as u32), end });
            }
        }
    }
    sheet
}

/// Text that reads as a number becomes one, the way a spreadsheet takes typed text.
fn text_value(text: String) -> Value {
    match text.trim().parse::<f64>() {
        Ok(n) if n.is_finite() && format_number(n) == text.trim() => Value::Number(n),
        _ => Value::Text(text),
    }
}

/// A sheet's used range as a table of text, its merged ranges as spans. Columns share `width`.
pub fn sheet_to_table(sheet: &Sheet, width: Emu) -> Result<Table> {
    let used = sheet.merges.iter().copied().fold(sheet.used_range(), |used, merge| {
        Some(match used {
            Some(range) => Range {
                start: CellRef::new(range.start.row.min(merge.start.row), range.start.col.min(merge.start.col)),
                end: CellRef::new(range.end.row.max(merge.end.row), range.end.col.max(merge.end.col)),
            },
            None => merge,
        })
    });
    let Some(used) = used else { return Ok(Table::default()) };
    sheet.check_laid_out(used.end)?;
    let (rows, cols) = (used.end.row as usize + 1, used.end.col as usize + 1);
    let text = sheet.to_text_rows()?;
    let mut table = Table::from_text(&(0..rows).map(|r| text.get(r).cloned().unwrap_or_default()).collect::<Vec<_>>(), width);
    for row in &mut table.rows {
        row.cells.resize_with(cols, || Cell::text(""));
    }
    table.columns.resize(cols, Emu::default());
    table.columns.fill(Emu(width.0 / cols.max(1) as i64));
    for m in &sheet.merges {
        let (r0, c0) = (m.start.row as usize, m.start.col as usize);
        if r0 >= rows || c0 >= cols {
            continue;
        }
        let row_end = (m.end.row as usize).min(rows - 1) + 1;
        let col_end = (m.end.col as usize).min(cols - 1) + 1;
        for (r, row) in table.rows.iter_mut().enumerate().take(row_end).skip(r0) {
            for (c, cell) in row.cells.iter_mut().enumerate().take(col_end).skip(c0) {
                cell.covered = (r, c) != (r0, c0);
            }
        }
        let owner = &mut table.rows[r0].cells[c0];
        owner.col_span = m.cols().min((cols - c0) as u32);
        owner.row_span = m.rows().min((rows - r0) as u32);
    }
    Ok(table)
}

/// Every table as a sheet; with no tables, one sheet with a paragraph per row.
fn blocks_to_sheets(blocks: &[Block]) -> Vec<Sheet> {
    let tables: Vec<&Table> = blocks.iter().filter_map(|b| if let Block::Table(t) = b { Some(t) } else { None }).collect();
    if tables.is_empty() {
        let mut sheet = Sheet::new("Sheet1");
        let lines = blocks.iter().filter_map(|b| if let Block::Paragraph(p) = b { Some(p.plain_text()) } else { None });
        for (r, line) in lines.enumerate().filter(|(_, l)| !l.is_empty()) {
            sheet.set(CellRef::new(r as u32, 0), Value::Text(line));
        }
        return vec![sheet];
    }
    tables.iter().enumerate().map(|(i, t)| table_to_sheet(t, &format!("Table {}", i + 1))).collect()
}

/// Each sheet as a table, after a paragraph with its name when there's more than one.
fn sheets_to_blocks(sheets: &[Sheet]) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    for s in sheets.iter().filter(|s| !s.hidden) {
        if sheets.len() > 1 {
            let mut heading = Paragraph::text(&s.name);
            heading.props.style = Some("Heading1".into());
            blocks.push(Block::Paragraph(heading));
        }
        // 6.27 in: an A4 page's text width at Word's default margins.
        blocks.push(Block::Table(sheet_to_table(s, Emu(5_731_510))?));
    }
    if blocks.is_empty() {
        blocks.push(Block::Paragraph(Paragraph::default()));
    }
    Ok(blocks)
}

/// A slide's frames in reading order: top to bottom, then left to right.
fn reading_order(frames: &[Frame]) -> Vec<&Frame> {
    let mut v: Vec<&Frame> = frames.iter().collect();
    let at = |f: &Frame| f.shape.xfrm.as_ref().map(|x| (x.y, x.x)).unwrap_or_default();
    v.sort_by_key(|f| at(f));
    v
}

fn slides_to_blocks(slides: &[Slide]) -> Vec<Block> {
    fn frame_blocks(f: &Frame, out: &mut Vec<Block>) {
        match &f.content {
            FrameContent::Shape { text: Some(body), .. } => out.extend(body.paragraphs.iter().cloned().map(Block::Paragraph)),
            FrameContent::Table { table } => out.push(Block::Table(table.clone())),
            FrameContent::Picture { image } => {
                out.push(Block::Paragraph(Paragraph { content: vec![Inline::Image(image.clone())], ..Default::default() }))
            }
            FrameContent::Group { frames } => reading_order(frames).into_iter().for_each(|f| frame_blocks(f, out)),
            _ => {}
        }
    }
    let mut blocks = Vec::new();
    for (i, slide) in slides.iter().filter(|s| !s.hidden).enumerate() {
        if i > 0 {
            blocks.push(Block::Paragraph(Paragraph {
                content: vec![Inline::Break { kind: BreakKind::Page, origin: Default::default() }],
                ..Default::default()
            }));
        }
        for f in reading_order(&slide.frames) {
            frame_blocks(f, &mut blocks);
        }
    }
    blocks
}

/// Margins on a 16:9 slide, and a box filling the rest.
const MARGIN: i64 = 457_200;

fn body_box(size: SlideSize) -> Xfrm {
    Xfrm {
        x: Emu(MARGIN),
        y: Emu(MARGIN),
        width: Emu(size.width.0 - 2 * MARGIN),
        height: Emu(size.height.0 - 2 * MARGIN),
        ..Default::default()
    }
}

/// The height a row of a converted table gets: PowerPoint's default.
const ROW: i64 = 370_840;

/// About how tall a paragraph is in a text box: each line as tall as its largest text (18 pt
/// where a run doesn't say), with its line and paragraph spacing.
fn paragraph_height(p: &Paragraph) -> i64 {
    let largest = p.runs().map(|r| r.props.size.map(|s| s.points()).unwrap_or(18.0)).fold(f64::NAN, f64::max);
    // An empty paragraph is as tall as its mark.
    let size = if largest.is_nan() { p.mark.as_ref().and_then(|m| m.size).map(|s| s.points()).unwrap_or(18.0) } else { largest };
    let spacing = match p.props.line_spacing {
        Some(LineSpacing::Multiple(m)) => m.fraction().max(1.0),
        _ => 1.0,
    };
    let lines = 1.0 + p.runs().map(|r| r.text.bytes().filter(|&b| b == b'\n').count()).sum::<usize>() as f64;
    let around = p.props.space_before.unwrap_or_default().0 + p.props.space_after.unwrap_or_default().0;
    Emu::from_points(size * 1.2 * spacing * lines).0 + around
}

/// Splits explicit lines while preserving run formatting and hyperlinks in each fragment.
fn split_paragraph(p: Paragraph, max_height: i64) -> impl Iterator<Item = Paragraph> {
    let height = paragraph_height(&p);
    if height <= max_height {
        return Some(p).into_iter().chain(Vec::new());
    }
    let lines = 1 + p.runs().map(|r| r.text.bytes().filter(|&b| b == b'\n').count()).sum::<usize>() as i64;
    let around = p.props.space_before.unwrap_or_default().0 + p.props.space_after.unwrap_or_default().0;
    let line_height = ((height - around + lines - 1) / lines).max(1);
    let per_part = ((max_height - around) / line_height).max(1);
    let mut part = Paragraph { props: p.props.clone(), mark: p.mark.clone(), ..Default::default() };
    let mut parts = Vec::new();
    let mut count = 1;
    for inline in p.content {
        match inline {
            Inline::Text(run) => {
                let mut start = 0;
                for (at, _) in run.text.match_indices('\n') {
                    if count == per_part {
                        if at > start {
                            part.content.push(Inline::Text(Run {
                                text: run.text[start..at].into(),
                                props: run.props.clone(),
                                link: run.link.clone(),
                                origin: Origin::default(),
                            }));
                        }
                        parts.push(std::mem::replace(
                            &mut part,
                            Paragraph { props: p.props.clone(), mark: p.mark.clone(), ..Default::default() },
                        ));
                        count = 1;
                        start = at + 1;
                    } else {
                        count += 1;
                    }
                }
                if start < run.text.len() {
                    part.content.push(Inline::Text(Run { text: run.text[start..].into(), origin: Origin::default(), ..run }));
                }
            }
            inline => part.content.push(inline),
        }
    }
    parts.push(part);
    None.into_iter().chain(parts)
}

/// A text box's insets, top and bottom.
const INSETS: i64 = 2 * 91_440;

/// Lays frames out down slides, starting a new slide when the next frame doesn't fit.
struct Deck {
    area: Xfrm,
    slides: Vec<Slide>,
    frames: Vec<Frame>,
    y: i64,
    id: u32,
}

impl Deck {
    fn new(size: SlideSize) -> Deck {
        let area = body_box(size);
        Deck { y: area.y.0, area, slides: Vec::new(), frames: Vec::new(), id: 2 }
    }

    fn room(&self) -> i64 {
        self.area.y.0 + self.area.height.0 - self.y
    }

    fn new_slide(&mut self) {
        if !self.frames.is_empty() {
            self.slides.push(Slide { frames: std::mem::take(&mut self.frames), ..Default::default() });
        }
        self.y = self.area.y.0;
        self.id = 2;
    }

    /// Puts a frame `height` tall at the next free place, on a new slide if it doesn't fit here.
    fn place(&mut self, width: i64, height: i64, make: impl FnOnce(u32, Xfrm) -> Frame) {
        if height > self.room() && !self.frames.is_empty() {
            self.new_slide();
        }
        let xfrm = Xfrm { x: self.area.x, y: Emu(self.y), width: Emu(width), height: Emu(height), ..Default::default() };
        self.frames.push(make(self.id, xfrm));
        self.id += 1;
        self.y += height;
    }

    /// Paragraphs in text boxes, as many to a box as fit, the rest on the next slides.
    fn text(&mut self, paragraphs: Vec<Paragraph>) {
        if paragraphs.iter().all(|p| p.plain_text().trim().is_empty()) {
            return;
        }
        let mut chunk: Vec<Paragraph> = Vec::new();
        let mut height = INSETS;
        let max_height = self.area.height.0 - INSETS;
        for p in paragraphs.into_iter().flat_map(|p| split_paragraph(p, max_height)) {
            let h = paragraph_height(&p);
            if !chunk.is_empty() && height + h > self.room() {
                self.flush_text(std::mem::take(&mut chunk), height);
                self.new_slide();
                height = INSETS;
            }
            height += h;
            chunk.push(p);
        }
        self.flush_text(chunk, height);
    }

    fn flush_text(&mut self, chunk: Vec<Paragraph>, height: i64) {
        if chunk.is_empty() {
            return;
        }
        let width = self.area.width.0;
        self.place(width, height, |id, xfrm| Frame::text_box(id, xfrm, chunk));
    }

    /// A table, split across slides by rows when it's too tall, the header row on each part.
    fn table(&mut self, t: Table) {
        let total: i64 = t.columns.iter().map(|c| c.0).sum();
        let mut t = t;
        // Fit the table to the slide's width.
        if total > self.area.width.0 && total > 0 {
            for c in &mut t.columns {
                c.0 = c.0 * self.area.width.0 / total;
            }
        }
        let width: i64 = t.columns.iter().map(|c| c.0).sum();
        let header = t.rows.first().is_some_and(|r| r.header);
        let per_slide = (self.area.height.0 / ROW).max(2) as usize;
        let mut start = 0;
        while start < t.rows.len() {
            let room = ((self.room() / ROW) as usize).max(if self.frames.is_empty() { per_slide } else { 0 });
            // Too little room for more than the header: begin on a new slide.
            let fits = room.saturating_sub(if header && start > 0 { 1 } else { 0 });
            if fits == 0 {
                self.new_slide();
                continue;
            }
            let end = (start + fits).min(t.rows.len());
            let mut part = slice_table(&t, start, end);
            if header && start > 0 {
                part.rows.insert(0, t.rows[0].clone());
            }
            let height = part.rows.len() as i64 * ROW;
            self.place(width, height, |id, xfrm| Frame {
                id,
                name: format!("Table {id}"),
                description: None,
                hidden: false,
                shape: ShapeProps { xfrm: Some(xfrm), ..Default::default() },
                placeholder: None,
                link: None,
                content: FrameContent::Table { table: part },
                origin: Origin::default(),
            });
            start = end;
            if start < t.rows.len() {
                self.new_slide();
            }
        }
    }

    /// A picture, shrunk to fit the slide if it's bigger, keeping its shape.
    fn picture(&mut self, image: Image) {
        let (w, h) = (image.width.0.max(1), image.height.0.max(1));
        let scale = (self.area.width.0 as f64 / w as f64).min(self.area.height.0 as f64 / h as f64).min(1.0);
        let (w, h) = ((w as f64 * scale) as i64, (h as f64 * scale) as i64);
        self.place(w, h, |id, xfrm| Frame {
            id,
            name: format!("Picture {id}"),
            description: image.description.clone(),
            hidden: false,
            shape: ShapeProps { xfrm: Some(xfrm), preset: Some("rect".into()), ..Default::default() },
            placeholder: None,
            link: None,
            content: FrameContent::Picture { image: Image { width: Emu(w), height: Emu(h), ..image } },
            origin: Origin::default(),
        });
    }

    fn finish(mut self) -> Vec<Slide> {
        self.new_slide();
        self.slides
    }
}

/// Rows `start..end` of a table, with spans that cross the cut ended there: a row span from
/// above gives its first cell in the slice back, and one running past the end is cut short.
fn slice_table(t: &Table, start: usize, end: usize) -> Table {
    let mut part =
        Table { columns: t.columns.clone(), rows: t.rows[start..end].to_vec(), style: t.style.clone(), origin: Origin::default() };
    for (i, row) in part.rows.iter_mut().enumerate() {
        row.origin = Origin::default();
        let r = start + i;
        for (c, cell) in row.cells.iter_mut().enumerate() {
            cell.origin = Origin::default();
            if cell.covered {
                if let Some((or, oc)) = t.owner(r, c) {
                    // The owner is above the slice: the top row of what it covers here takes over.
                    if or < start && r == start {
                        let owner = &t.rows[or].cells[oc];
                        if c == oc {
                            cell.covered = false;
                            cell.col_span = owner.col_span;
                            cell.row_span = (or + owner.row_span as usize - start) as u32;
                        }
                    }
                }
            }
            if !cell.covered {
                cell.row_span = cell.row_span.min((end - r) as u32).max(1);
            }
        }
    }
    part
}

/// A slide for each page (split at page breaks and top-level headings), with the paragraphs in
/// text boxes and each table and picture as its own frame below them, carried over onto more
/// slides when a page holds more than one slide does.
fn flow_to_slides(blocks: &[Block]) -> Body {
    let size = SlideSize::default();
    let mut deck = Deck::new(size);
    let mut text: Vec<Paragraph> = Vec::new();
    for b in blocks {
        match b {
            Block::Paragraph(p) => {
                if matches!(p.props.style.as_deref(), Some("Title" | "Heading1")) {
                    deck.text(std::mem::take(&mut text));
                    deck.new_slide();
                }
                let mut part = Paragraph { props: p.props.clone(), mark: p.mark.clone(), ..Default::default() };
                for inline in &p.content {
                    match inline {
                        Inline::Break { kind: BreakKind::Page, .. } => {
                            if !part.content.is_empty() {
                                text.push(part.clone());
                                part.content.clear();
                            }
                            deck.text(std::mem::take(&mut text));
                            deck.new_slide();
                        }
                        Inline::Image(image) => {
                            if !part.content.is_empty() {
                                text.push(part.clone());
                                part.content.clear();
                            }
                            deck.text(std::mem::take(&mut text));
                            deck.picture(image.clone());
                        }
                        Inline::Break { .. } => part.content.push(Inline::Text(Run::new("\n"))),
                        inline => part.content.push(inline.clone()),
                    }
                }
                if !part.content.is_empty() {
                    text.push(part);
                }
            }
            Block::Table(t) => {
                deck.text(std::mem::take(&mut text));
                deck.table(t.clone());
            }
            Block::Raw { .. } => {}
        }
    }
    deck.text(text);
    Body::Slides { size, slides: deck.finish() }
}

/// Each visible sheet as a table, on as many slides as it takes.
fn sheets_to_slides(sheets: &[Sheet]) -> Result<Body> {
    let size = SlideSize::default();
    let mut deck = Deck::new(size);
    for s in sheets.iter().filter(|s| !s.hidden) {
        deck.new_slide();
        let table = sheet_to_table(s, deck.area.width)?;
        if !table.rows.is_empty() {
            deck.table(table);
        }
    }
    Ok(Body::Slides { size, slides: deck.finish() })
}

/// The document's text: paragraphs on lines, table cells split by tabs, slides and sheets one
/// after another.
pub fn plain_text(doc: &Document) -> Result<String> {
    Ok(match &doc.body {
        Body::Flow { blocks } => blocks_text(blocks),
        Body::Slides { slides, .. } => blocks_text(&slides_to_blocks(slides)).replace('\u{c}', ""),
        Body::Workbook { sheets } => sheets
            .iter()
            .map(|s| Ok(s.to_text_rows()?.into_iter().map(|r| r.join("\t")).collect::<Vec<_>>().join("\n")))
            .collect::<Result<Vec<_>>>()?
            .join("\n\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_and_sheets_keep_spans() {
        let mut t = Table::from_text(&[vec!["a".into(), "".into(), "3".into()], vec!["".into(), "".into(), "x".into()]], Emu(3000));
        t.rows[0].cells[0].col_span = 2;
        t.rows[0].cells[0].row_span = 2;
        for (r, c) in [(0, 1), (1, 0), (1, 1)] {
            t.rows[r].cells[c].covered = true;
        }
        let sheet = table_to_sheet(&t, "T");
        assert_eq!(sheet.merges, vec![Range::parse("A1:B2").unwrap()]);
        assert_eq!(sheet.value(CellRef::new(0, 2)), &Value::Number(3.0));
        let back = sheet_to_table(&sheet, Emu(3000)).unwrap();
        assert_eq!((back.rows[0].cells[0].col_span, back.rows[0].cells[0].row_span), (2, 2));
        assert!(back.rows[1].cells[1].covered);
        assert_eq!(back.rows[1].cells[2].plain_text(), "x");
    }

    #[test]
    fn every_kind_converts_to_every_other() {
        let mut word = Document::new(Kind::Word);
        word.body = Body::Flow {
            blocks: vec![
                Block::Paragraph(Paragraph {
                    props: ParaProps { style: Some("Title".into()), ..Default::default() },
                    ..Paragraph::text("Report")
                }),
                Block::Paragraph(Paragraph::text("Intro")),
                Block::Table(Table::from_text(&[vec!["k".into(), "v".into()], vec!["n".into(), "2".into()]], Emu(1_000_000))),
                Block::Paragraph(Paragraph {
                    content: vec![Inline::Break { kind: BreakKind::Page, origin: Default::default() }, Inline::Text(Run::new("Page two"))],
                    ..Default::default()
                }),
            ],
        };
        let slides = to_kind(&word, Kind::Slides).unwrap();
        let Body::Slides { slides: s, .. } = &slides.body else { panic!() };
        assert_eq!(s.len(), 2, "a slide per page");
        assert!(matches!(s[0].frames[1].content, FrameContent::Table { .. }));
        let sheets = to_kind(&word, Kind::Sheets).unwrap();
        assert_eq!(sheets.sheet_value(CellRef::new(1, 1)), Some(&Value::Number(2.0)));
        let csv = to_kind(&sheets, Kind::Csv).unwrap();
        assert_eq!(crate::formats::csv::write(&csv).unwrap(), b"\xEF\xBB\xBFk,v\r\nn,2\r\n");
        let back = to_kind(&sheets, Kind::Word).unwrap();
        assert!(plain_text(&back).unwrap().contains("k\tv\nn\t2"));
        let from_slides = to_kind(&slides, Kind::Word).unwrap();
        assert_eq!(plain_text(&from_slides).unwrap().replace('\u{c}', ""), "Report\nIntro\nk\tv\nn\t2\n\nPage two");
        assert!(plain_text(&to_kind(&sheets, Kind::Slides).unwrap()).unwrap().contains("n\t2"));
    }

    #[test]
    fn long_pages_carry_onto_more_slides() {
        let mut word = Document::new(Kind::Word);
        let mut blocks: Vec<Block> = (0..40).map(|i| Block::Paragraph(Paragraph::text(format!("Line {i}")))).collect();
        let mut t = Table::from_text(&(0..30).map(|i| vec![format!("r{i}"), "x".into()]).collect::<Vec<_>>(), Emu(20_000_000));
        t.rows[0].header = true;
        // A row span across where the table will be cut.
        t.rows[10].cells[1].row_span = 4;
        for row in &mut t.rows[11..14] {
            row.cells[1].covered = true;
        }
        blocks.push(Block::Table(t));
        word.body = Body::Flow { blocks };
        let doc = to_kind(&word, Kind::Slides).unwrap();
        let Body::Slides { slides, size } = &doc.body else { panic!() };
        assert!(slides.len() >= 4, "{} slides", slides.len());
        let bottom = size.height.0 - MARGIN;
        for s in slides {
            for f in &s.frames {
                let x = f.shape.xfrm.as_ref().unwrap();
                assert!(x.y.0 + x.height.0 <= bottom, "a frame runs off the slide");
                if let FrameContent::Shape { text: Some(body), .. } = &f.content {
                    let needed = INSETS + body.paragraphs.iter().map(paragraph_height).sum::<i64>();
                    assert!(needed <= x.height.0, "text needs {needed} EMU, frame has {}", x.height.0);
                }
                assert!(x.x.0 + x.width.0 <= size.width.0 - MARGIN, "or off its side");
            }
        }
        // Every line and every row made it, and later parts of the table start with its header.
        let text = plain_text(&doc).unwrap();
        assert!((0..40).all(|i| text.contains(&format!("Line {i}\n")) || text.ends_with(&format!("Line {i}"))));
        // (Rows whose second cell a span covers have only the first.)
        assert!((0..30).all(|i| text.lines().any(|l| l == format!("r{i}\tx") || l == format!("r{i}"))));
        let tables: Vec<&Table> = slides
            .iter()
            .flat_map(|s| &s.frames)
            .filter_map(|f| if let FrameContent::Table { table } = &f.content { Some(table) } else { None })
            .collect();
        assert!(tables.len() >= 2);
        assert!(tables[1..].iter().all(|t| t.rows[0].cells[0].plain_text() == "r0"));
        // Spans never run past a part's last row, and no covered cell is left without its owner.
        for t in tables {
            for (r, row) in t.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    assert!(r + cell.row_span as usize <= t.rows.len());
                    if cell.covered {
                        assert!(t.owner(r, c).is_some());
                    }
                }
            }
        }
    }
}
