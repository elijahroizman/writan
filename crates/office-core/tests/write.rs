//! The back ends: documents built in code survive a write and a read, edits land where they
//! should, and nothing else in the package moves.

mod common;

use common::*;
use office_core::drawing::Xfrm;
use office_core::model::*;
use office_core::opc::Package;
use office_core::units::Emu;
use office_core::{Document, Kind};

fn body_blocks(doc: &Document) -> &[Block] {
    match &doc.body {
        Body::Flow { blocks } => blocks,
        _ => panic!("not a flow document"),
    }
}

/// Asserts two parts of the model are equal, naming the first place they aren't.
fn same<T: serde::Serialize + PartialEq>(a: &T, b: &T) {
    fn first(a: &serde_json::Value, b: &serde_json::Value, path: String) -> Option<String> {
        use serde_json::Value::*;
        match (a, b) {
            (Object(x), Object(y)) => {
                x.keys().chain(y.keys()).find_map(|k| first(x.get(k).unwrap_or(&Null), y.get(k).unwrap_or(&Null), format!("{path}.{k}")))
            }
            (Array(x), Array(y)) if x.len() == y.len() => {
                x.iter().zip(y).enumerate().find_map(|(i, (x, y))| first(x, y, format!("{path}[{i}]")))
            }
            (x, y) if x == y => None,
            (x, y) => Some(format!("{path}: {x} vs {y}")),
        }
    }
    if a != b {
        let (x, y) = (serde_json::to_value(a).unwrap(), serde_json::to_value(b).unwrap());
        panic!("differ at {}", first(&x, &y, String::new()).unwrap_or_else(|| "(only in what JSON leaves out)".into()));
    }
}

/// The model in a form that doesn't depend on how a format happens to split things: runs
/// joined, covered cells' (invisible) content dropped, where things were read from forgotten,
/// and Word's complex-script size, bold and italic left out where they're the others' (Word
/// writes both).
fn canon(mut doc: Document) -> Body {
    fn blocks(blocks: &mut [Block]) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => {
                    for i in &mut p.content {
                        if let Inline::Text(r) = i {
                            let r = &mut r.props;
                            if r.size_complex == r.size {
                                r.size_complex = None;
                            }
                            if r.bold_complex == r.bold {
                                r.bold_complex = None;
                            }
                            if r.italic_complex == r.italic {
                                r.italic_complex = None;
                            }
                        }
                    }
                    p.merge_runs();
                    for i in &mut p.content {
                        if let Inline::Image(img) = i {
                            img.media.clear();
                        }
                    }
                }
                Block::Table(t) => table(t),
                Block::Raw { .. } => {}
            }
        }
    }
    fn table(t: &mut Table) {
        for cell in t.rows.iter_mut().flat_map(|r| &mut r.cells) {
            if cell.covered {
                cell.blocks.clear();
            }
            blocks(&mut cell.blocks);
        }
    }
    fn frames(list: &mut [Frame]) {
        for f in list {
            match &mut f.content {
                FrameContent::Shape { text: Some(body), .. } => body.paragraphs.iter_mut().for_each(Paragraph::merge_runs),
                FrameContent::Table { table: t } => table(t),
                FrameContent::Group { frames: inner } => frames(inner),
                // Media ids name where the bytes are, which changes on saving.
                FrameContent::Picture { image } => image.media.clear(),
                _ => {}
            }
        }
    }
    match &mut doc.body {
        Body::Flow { blocks: b } => blocks(b),
        Body::Slides { slides, .. } => slides.iter_mut().for_each(|s| {
            s.part = None;
            s.layout = None;
            frames(&mut s.frames)
        }),
        Body::Workbook { sheets } => sheets.iter_mut().for_each(|s| s.part = None),
    }
    doc.body
}

fn round_trip(doc: &Document, kind: Kind) -> Document {
    let bytes = office_core::write(doc, kind).unwrap();
    office_core::read(&bytes, Some(&format!("x.{}", kind.extension()))).unwrap()
}

#[test]
fn word_from_scratch() {
    let doc = word();
    let back = round_trip(&doc, Kind::Word);
    same(&canon(back.clone()), &canon(doc.clone()));
    assert_eq!(back.properties.title.as_deref(), Some("From scratch"));
    let Block::Paragraph(p) = &body_blocks(&back)[3] else { panic!() };
    let Inline::Image(img) = &p.content[0] else { panic!() };
    assert_eq!(&*back.media_bytes(&img.media).unwrap(), PNG);
    assert!(back.theme.is_some(), "a new document gets the Office theme");
}

#[test]
fn slides_from_scratch() {
    let doc = slides();
    let back = round_trip(&doc, Kind::Slides);
    same(&canon(back), &canon(doc));
}

#[test]
fn sheets_from_scratch() {
    let doc = sheets();
    let back = round_trip(&doc, Kind::Sheets);
    same(&canon(back), &canon(doc));
}

#[test]
fn edits_touch_only_what_changed() {
    let mut doc = Document::new(Kind::Word);
    let png = doc.add_media("image/png", PNG.to_vec());
    let pic = Paragraph {
        content: vec![Inline::Image(Image {
            media: png,
            width: Emu(1000),
            height: Emu(1000),
            name: "Dot".into(),
            description: None,
            origin: Default::default(),
        })],
        ..Default::default()
    };
    doc.body = Body::Flow {
        blocks: vec![Block::Paragraph(rich_paragraph()), Block::Paragraph(Paragraph::text("untouched")), Block::Paragraph(pic)],
    };
    let first = office_core::write(&doc, Kind::Word).unwrap();

    // Saving what was read, unchanged, changes nothing.
    let mut read = office_core::read(&first, Some("x.docx")).unwrap();
    let again = office_core::write(&read, Kind::Word).unwrap();
    let (a, b) = (Package::read(&first).unwrap(), Package::read(&again).unwrap());
    for name in a.part_names() {
        assert_eq!(a.part(name).unwrap(), b.part(name).unwrap(), "{name} changed");
    }

    // Make one run italic and change the other paragraph's text.
    let Body::Flow { blocks } = &mut read.body else { panic!() };
    let Block::Paragraph(p) = &mut blocks[0] else { panic!() };
    p.runs_mut().next().unwrap().props.italic = Some(true);
    let Block::Paragraph(p) = &mut blocks[1] else { panic!() };
    p.runs_mut().next().unwrap().text = "edited".into();
    let edited = office_core::write(&read, Kind::Word).unwrap();
    let c = Package::read(&edited).unwrap();
    for name in a.part_names() {
        let same = a.part(name).unwrap() == c.part(name).unwrap();
        assert_eq!(same, name != "word/document.xml", "{name}");
    }
    let back = office_core::read(&edited, Some("x.docx")).unwrap();
    let blocks = body_blocks(&back);
    let Block::Paragraph(p0) = &blocks[0] else { panic!() };
    assert_eq!(p0.runs().next().unwrap().props.italic, Some(true));
    assert_eq!(p0.runs().nth(1).unwrap().props.bold, Some(true), "the other runs keep their formatting");
    let Block::Paragraph(p1) = &blocks[1] else { panic!() };
    assert_eq!(p1.plain_text(), "edited");
}

#[test]
fn removing_slides_and_sheets() {
    let mut doc = Document::new(Kind::Slides);
    let slide = |t: &str| Slide {
        frames: vec![Frame::text_box(2, Xfrm { width: Emu(100), height: Emu(100), ..Default::default() }, vec![Paragraph::text(t)])],
        ..Default::default()
    };
    doc.body = Body::Slides { size: SlideSize::default(), slides: vec![slide("one"), slide("two"), slide("three")] };
    let mut read = round_trip(&doc, Kind::Slides);
    let Body::Slides { slides, .. } = &mut read.body else { panic!() };
    slides.remove(1);
    slides.swap(0, 1);
    let back = round_trip(&read, Kind::Slides);
    let Body::Slides { slides, .. } = &back.body else { panic!() };
    let texts: Vec<String> = slides.iter().map(|s| s.frames[0].paragraphs()[0].plain_text()).collect();
    assert_eq!(texts, ["three", "one"]);
    let pkg = back.package.as_ref().unwrap();
    assert_eq!(pkg.part_names().filter(|n| n.starts_with("ppt/slides/slide")).count(), 2, "the removed slide's part is gone");

    let mut doc = Document::new(Kind::Sheets);
    let Body::Workbook { sheets } = &mut doc.body else { panic!() };
    sheets.push(Sheet::new("Two"));
    let mut read = round_trip(&doc, Kind::Sheets);
    let Body::Workbook { sheets } = &mut read.body else { panic!() };
    sheets.remove(0);
    let back = round_trip(&read, Kind::Sheets);
    let Body::Workbook { sheets } = &back.body else { panic!() };
    assert_eq!(sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Two"]);
}

#[test]
fn copied_slides_get_their_own_part() {
    let mut read = round_trip(&slides(), Kind::Slides);
    let Body::Slides { slides, .. } = &mut read.body else { panic!() };
    let mut copy = slides[0].clone();
    if let FrameContent::Shape { text: Some(body), .. } = &mut copy.frames[0].content {
        body.paragraphs = vec![Paragraph::text("the copy")];
    }
    slides.insert(1, copy);
    let back = round_trip(&read, Kind::Slides);
    let Body::Slides { slides: after, .. } = &back.body else { panic!() };
    let Body::Slides { slides: before, .. } = &read.body else { panic!() };
    assert_eq!(after.len(), before.len());
    assert_eq!(after[1].frames[0].paragraphs()[0].plain_text(), "the copy");
    assert_eq!(
        after[0].frames[0].paragraphs()[0].plain_text(),
        before[0].frames[0].paragraphs()[0].plain_text(),
        "the original is untouched"
    );
    assert_ne!(after[0].part, after[1].part);
    assert!(matches!(after[1].frames[1].content, FrameContent::Picture { .. }), "the copy's picture still resolves");
    let pkg = back.package.as_ref().unwrap();
    let pres = pkg.xml("ppt/presentation.xml").unwrap();
    let ids: Vec<&str> =
        pres.root.descendant(office_core::xml::ns::P, "sldIdLst").unwrap().elements().filter_map(|e| e.attr("id")).collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), after.len());
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
}

/// A workbook of sheets A, B and C as Excel might have saved it: a chart sheet between A and
/// B, names scoped to B and C, C the active tab, and a date written as ISO text in A.
fn workbook_with_chart_sheet() -> Document {
    let mut doc = Document::new(Kind::Sheets);
    let Body::Workbook { sheets } = &mut doc.body else { panic!() };
    sheets[0].name = "A".into();
    sheets[0].set(CellRef::new(0, 0), Value::Number(1.0));
    sheets.push(Sheet::new("B"));
    sheets.push(Sheet::new("C"));
    let mut pkg = Package::read(&office_core::write(&doc, Kind::Sheets).unwrap()).unwrap();
    let main = "xl/workbook.xml";
    let chart = "xl/chartsheets/sheet1.xml";
    pkg.put(
        chart,
        br#"<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"/></sheetViews></chartsheet>"#.to_vec(),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml",
    );
    let rid = pkg.relate(main, "chartsheet", chart).unwrap();
    let text = String::from_utf8(pkg.part(main).unwrap().to_vec()).unwrap();
    let sheets_at = text.find("<sheet ").unwrap();
    let after_a = sheets_at + text[sheets_at..].find("/>").unwrap() + 2;
    let mut text = format!("{}<sheet name=\"Chart\" sheetId=\"9\" r:id=\"{rid}\"/>{}", &text[..after_a], &text[after_a..]);
    text = text.replace("<workbookView/>", r#"<workbookView activeTab="3"/>"#);
    text = text.replace("</sheets>", r#"</sheets><definedNames><definedName name="inB" localSheetId="2">B!$A$1</definedName><definedName name="inC" localSheetId="3">C!$A$1</definedName><definedName name="all">A!$A$1</definedName></definedNames>"#);
    pkg.replace(main, text.into_bytes()).unwrap();
    let sheet = "xl/worksheets/sheet1.xml";
    let text = String::from_utf8(pkg.part(sheet).unwrap().to_vec()).unwrap();
    let text = text.replace("</row>", r#"<c r="B1" t="d"><v>2024-03-01T00:00:00</v></c></row>"#);
    pkg.replace(sheet, text.into_bytes()).unwrap();
    office_core::read(&pkg.to_bytes().unwrap(), Some("x.xlsx")).unwrap()
}

fn workbook_xml(doc: &Document) -> String {
    let bytes = office_core::write(doc, Kind::Sheets).unwrap();
    String::from_utf8(Package::read(&bytes).unwrap().part("xl/workbook.xml").unwrap().to_vec()).unwrap()
}

#[test]
fn chart_sheets_keep_their_place() {
    let doc = workbook_with_chart_sheet();
    let before = String::from_utf8(doc.package.as_ref().unwrap().part("xl/workbook.xml").unwrap().to_vec()).unwrap();
    assert_eq!(workbook_xml(&doc), before, "an unchanged workbook is written as it was");

    // Without B, C moves up to where B was: its name and the active tab follow it, and B's
    // own name goes with B.
    let mut doc = doc;
    let Body::Workbook { sheets } = &mut doc.body else { panic!() };
    sheets.remove(1);
    let xml = workbook_xml(&doc);
    let names: Vec<&str> = xml.match_indices("<sheet name=\"").map(|(i, m)| xml[i + m.len()..].split('"').next().unwrap()).collect();
    assert_eq!(names, ["A", "Chart", "C"]);
    assert!(xml.contains(r#"<definedName name="inC" localSheetId="2">"#), "{xml}");
    assert!(!xml.contains("inB"), "{xml}");
    assert!(xml.contains(r#"<definedName name="all">"#));
    assert!(xml.contains(r#"activeTab="2""#), "{xml}");
}

#[test]
fn iso_date_cells_stay_dates() {
    let mut doc = workbook_with_chart_sheet();
    let Body::Workbook { sheets } = &mut doc.body else { panic!() };
    sheets[0].set(CellRef::new(1, 0), Value::Number(2.0));
    let bytes = office_core::write(&doc, Kind::Sheets).unwrap();
    let sheet = String::from_utf8(Package::read(&bytes).unwrap().part("xl/worksheets/sheet1.xml").unwrap().to_vec()).unwrap();
    assert!(sheet.contains(r#"<c r="B1" t="d"><v>2024-03-01T00:00:00</v></c>"#), "{sheet}");
}

#[test]
fn underline_off_stays_off_in_excel() {
    let mut doc = Document::new(Kind::Sheets);
    let mut run = Run::new("plain");
    run.props.underline = Some("none".into());
    let Body::Workbook { sheets } = &mut doc.body else { panic!() };
    sheets[0].set(CellRef::new(0, 0), Value::Rich(vec![run, Run::new(" and more")]));
    let bytes = office_core::write(&doc, Kind::Sheets).unwrap();
    let pkg = Package::read(&bytes).unwrap();
    let strings = String::from_utf8(pkg.part("xl/sharedStrings.xml").unwrap().to_vec()).unwrap();
    assert!(strings.contains(r#"<u val="none"/>"#), "{strings}");
}

/// `part`'s XML text with `edit` applied, read back.
fn patched(doc: &Document, kind: Kind, part: &str, edit: impl Fn(String) -> String) -> office_core::Result<Document> {
    let mut pkg = Package::read(&office_core::write(doc, kind).unwrap()).unwrap();
    let text = String::from_utf8(pkg.part(part).unwrap().to_vec()).unwrap();
    let changed = edit(text.clone());
    assert_ne!(changed, text, "the edit should change {part}");
    pkg.replace(part, changed.into_bytes()).unwrap();
    office_core::read(&pkg.to_bytes().unwrap(), Some(&format!("x.{}", kind.extension())))
}

#[test]
fn damaged_spans_and_sizes_are_contained() {
    // Word cells claiming four billion columns, in a grid of 10 000, fill no more than Word's
    // 63 columns a row.
    let doc = patched(&word(), Kind::Word, "word/document.xml", |t| {
        let t = t.replace("<w:tcPr>", r#"<w:tcPr><w:gridSpan w:val="4000000000"/>"#);
        t.replacen("<w:tblGrid>", &format!("<w:tblGrid>{}", r#"<w:gridCol w:w="10"/>"#.repeat(10_000)), 1)
    })
    .unwrap();
    let table = body_blocks(&doc).iter().find_map(|b| if let Block::Table(t) = b { Some(t) } else { None }).unwrap();
    for row in &table.rows {
        // 63 slots of spans, then one for each of the row's own cells past them.
        assert!(row.cells.len() <= 63 + 3, "{}", row.cells.len());
    }

    // A PowerPoint cell spanning no rows or columns spans itself, and converts.
    let doc = patched(&slides(), Kind::Slides, "ppt/slides/slide1.xml", |t| t.replacen("<a:tc>", r#"<a:tc rowSpan="0" gridSpan="0">"#, 1))
        .unwrap();
    office_core::convert::to_kind(&doc, Kind::Sheets).unwrap();

    // A sheet with cells at opposite corners isn't laid out as a 17-billion-cell table.
    let mut doc = Document::new(Kind::Sheets);
    let Body::Workbook { sheets } = &mut doc.body else { panic!() };
    sheets[0].set(CellRef::new(0, 0), Value::Text("A1".into()));
    sheets[0].set(CellRef::new(1_048_575, 16_383), Value::Text("XFD1048576".into()));
    assert!(office_core::convert::to_kind(&doc, Kind::Word).is_err());
    assert!(office_core::convert::plain_text(&doc).is_err());
}
