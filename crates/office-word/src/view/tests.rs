use office_core::model::{Block, Body, Document, Paragraph, Run, RunProps};
use office_core::{Compiler, Kind};
use office_engine::measure::FixedMeasurer;
use office_engine::pos::{Path, Pos, Selection};

use super::*;
use crate::Words;

fn doc_bytes(paras: &[&str]) -> Vec<u8> {
    let mut doc = Document::new(Kind::Word);
    doc.body = Body::Flow { blocks: paras.iter().map(|t| Block::Paragraph(Paragraph::text(*t))).collect() };
    Compiler::new().with_layer(Words).write(&doc, Kind::Word).unwrap()
}

fn texts(v: &WordView) -> Vec<String> {
    v.blocks(StoryId::Body)
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.plain_text(),
            Block::Table(_) => "[table]".into(),
            Block::Raw { .. } => "[raw]".into(),
        })
        .collect()
}

#[test]
fn opens_lays_out_and_paints() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["Hello world", "Second paragraph"])).unwrap();
    v.layout(&mut host);
    assert_eq!(v.page_count(), 1);
    let (w, h) = v.page_size(0);
    // A4 at 96 px per inch.
    assert!((w - 793.7).abs() < 1.0 && (h - 1122.5).abs() < 1.0, "{w} x {h}");
    let page = v.paint(&mut host, 0);
    let texts: Vec<&str> =
        page.items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.as_str()) } else { None }).collect();
    assert_eq!(texts, vec!["Hello world", "Second paragraph"]);
    assert!(!v.new_styles().is_empty());
}

#[test]
fn many_paragraphs_make_pages() {
    let mut host = FixedMeasurer::default();
    let lines: Vec<String> = (0..200).map(|i| format!("Paragraph number {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut v = WordView::open(&doc_bytes(&refs)).unwrap();
    v.layout(&mut host);
    assert!(v.page_count() > 2, "{}", v.page_count());
    // Every paragraph is drawn on exactly one page.
    let mut seen = 0;
    for i in 0..v.page_count() {
        seen += v.paint(&mut host, i).items.iter().filter(|i| matches!(i, Item::Text { .. })).count();
    }
    assert_eq!(seen, 200);
}

#[test]
fn typing_enter_undo_and_save() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["Hello"])).unwrap();
    v.layout(&mut host);
    v.set_selection(Selection::caret(Pos::new(Path::top(0), 5)));
    for c in [",", " ", "w", "o", "r", "l", "d"] {
        v.insert_text(c);
    }
    assert_eq!(texts(&v), vec!["Hello, world"]);
    v.enter();
    v.insert_text("Next");
    assert_eq!(texts(&v), vec!["Hello, world", "Next"]);
    assert!(v.is_dirty());
    v.undo();
    v.undo();
    assert_eq!(texts(&v), vec!["Hello, world"]);
    v.undo();
    assert_eq!(texts(&v), vec!["Hello,"]);
    v.redo();
    assert_eq!(texts(&v), vec!["Hello, world"]);
    let bytes = v.save().unwrap();
    let back = WordView::open(&bytes).unwrap();
    assert_eq!(texts(&back), vec!["Hello, world"]);
}

#[test]
fn caret_moves_and_overlay() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["abc", "defgh"])).unwrap();
    v.layout(&mut host);
    v.move_caret(&mut host, Move::Right, false);
    v.move_caret(&mut host, Move::Right, false);
    assert_eq!(v.selection().focus, Pos::new(Path::top(0), 2));
    v.move_caret(&mut host, Move::Down, false);
    assert_eq!(v.selection().focus.path, Path::top(1));
    v.move_caret(&mut host, Move::LineEnd, true);
    assert_eq!(v.selection().focus, Pos::new(Path::top(1), 5));
    let ov = v.overlay(&mut host);
    assert!(!ov.rects.is_empty());
    assert!(ov.caret.is_some());
    v.move_caret(&mut host, Move::DocStart, false);
    assert_eq!(v.selection().focus, Pos::new(Path::top(0), 0));
}

#[test]
fn focus_band_follows_the_caret_paragraph_across_pages() {
    let mut host = FixedMeasurer::default();
    let long = "A paragraph long enough to cross a page. ".repeat(400);
    let mut v = WordView::open(&doc_bytes(&[&long, "Last paragraph"])).unwrap();
    v.layout(&mut host);
    let bands = v.overlay(&mut host).paragraph;
    assert!(bands.len() > 1);
    assert!(bands.iter().all(|b| b.3 > 0.0 && b.4 > 0.0));
    v.set_selection(Selection::caret(Pos::new(Path::top(1), 0)));
    let ov = v.overlay(&mut host);
    assert_eq!(ov.paragraph.len(), 1);
    let (page, _, y, _, h) = ov.paragraph[0];
    let (caret_page, _, caret_y, _) = ov.caret.unwrap();
    assert_eq!(page, caret_page);
    assert!(caret_y >= y && caret_y <= y + h);
}

#[test]
fn clicking_places_the_caret() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["Hello world"])).unwrap();
    v.layout(&mut host);
    let page = v.pages[0].clone();
    // Calibri 11 pt (default) is about 14.67 px: 7.33 px a character here.
    let x = page.body_x + 7.33 * 4.0 + 1.0;
    assert!(v.click(&mut host, 0, x, page.body_y + 5.0, false, 1));
    assert_eq!(v.selection().focus, Pos::new(Path::top(0), 4));
    v.click(&mut host, 0, x, page.body_y + 5.0, false, 2);
    assert_eq!(v.selection().ordered(), (Pos::new(Path::top(0), 0), Pos::new(Path::top(0), 5)));
}

#[test]
fn formatting_and_state() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["Hello world"])).unwrap();
    v.layout(&mut host);
    v.set_selection(Selection { anchor: Pos::new(Path::top(0), 0), focus: Pos::new(Path::top(0), 5) });
    assert!(!v.state().bold);
    v.toggle(Toggle::Bold);
    assert!(v.state().bold);
    v.set_size(28);
    assert_eq!(v.state().size, Some(28));
    v.set_color("FF0000");
    assert_eq!(v.state().color.as_deref(), Some("FF0000"));
    v.align("center");
    assert_eq!(v.state().align, Some("center"));
    v.toggle(Toggle::Bold);
    assert!(!v.state().bold);
    // With a caret, formatting applies to what's typed next.
    v.set_selection(Selection::caret(Pos::new(Path::top(0), 11)));
    v.toggle(Toggle::Italic);
    v.insert_text("!");
    let Block::Paragraph(p) = &v.blocks(StoryId::Body)[0] else { panic!() };
    assert_eq!(p.runs().last().unwrap().text, "!");
    assert_eq!(p.runs().last().unwrap().props.italic, Some(true));
}

#[test]
fn page_breaks_tables_and_lists() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["one", "two"])).unwrap();
    v.layout(&mut host);
    v.set_selection(Selection::caret(Pos::new(Path::top(0), 3)));
    v.page_break();
    v.layout(&mut host);
    assert_eq!(v.page_count(), 2);
    v.undo();
    v.layout(&mut host);
    assert_eq!(v.page_count(), 1);

    v.set_selection(Selection::caret(Pos::new(Path::top(1), 3)));
    v.insert_table(2, 3);
    assert_eq!(texts(&v), vec!["one", "two", "[table]", ""]);
    assert_eq!(v.state().table.as_ref().map(|t| (t.rows, t.columns)), Some((2, 3)));
    v.insert_text("cell");
    v.tab(false);
    v.insert_text("next");
    v.table_op(super::commands::TableOp::InsertRow { below: true });
    assert_eq!(v.state().table.as_ref().map(|t| t.rows), Some(3));
    v.layout(&mut host);
    let items = v.paint(&mut host, 0).items;
    assert!(items.iter().any(|i| matches!(i, Item::Line { .. })), "table borders drawn");

    v.set_selection(Selection::caret(Pos::new(Path::top(0), 0)));
    v.list(Some(crate::ListKind::Numbers));
    assert_eq!(v.state().list.as_deref(), Some("numbered"));
    v.layout(&mut host);
    let items = v.paint(&mut host, 0).items;
    assert!(items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "1.")), "{items:?}");
    let bytes = v.save().unwrap();
    let mut back = WordView::open(&bytes).unwrap();
    back.layout(&mut host);
    assert_eq!(back.state().list.as_deref(), Some("numbered"));
    assert!(back.paint(&mut host, 0).items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "1.")));
}

#[test]
fn headers_with_page_numbers() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["body"])).unwrap();
    v.layout(&mut host);
    assert!(v.edit_header_footer(0, crate::StoryKind::Footer));
    v.insert_text("Page ");
    v.insert_page_field("PAGE_X_OF_Y");
    v.exit_header_footer();
    v.layout(&mut host);
    let items = v.paint(&mut host, 0).items;
    let footer: String = items
        .iter()
        .filter_map(|i| if let Item::Group { items, .. } = i { Some(items) } else { None })
        .flatten()
        .filter_map(|i| if let Item::Text { text, .. } = i { Some(text.clone()) } else { None })
        .collect();
    assert_eq!(footer, "Page 1 of 1");
    let bytes = v.save().unwrap();
    let mut back = WordView::open(&bytes).unwrap();
    back.layout(&mut host);
    assert!(back.pages[0].footer.is_some());
}

#[test]
fn find_and_replace() {
    let mut v = WordView::open(&doc_bytes(&["the cat sat", "a cat"])).unwrap();
    let m = v.find("cat", false, false);
    assert_eq!(m.len(), 2);
    v.replace(&m, "dog");
    assert_eq!(texts(&v), vec!["the dog sat", "a dog"]);
    v.undo();
    assert_eq!(texts(&v), vec!["the cat sat", "a cat"]);
    assert_eq!(v.find("CAT", true, false).len(), 0);
    assert_eq!(v.find("ca", false, true).len(), 0);
}

#[test]
fn copy_and_paste_keep_formatting() {
    let mut doc = Document::new(Kind::Word);
    let mut p = Paragraph::text("plain ");
    p.content.push(office_core::model::Inline::Text(Run::new("bold").with_props(RunProps { bold: Some(true), ..Default::default() })));
    doc.body = Body::Flow { blocks: vec![Block::Paragraph(p), Block::Paragraph(Paragraph::text("end"))] };
    let bytes = Compiler::new().with_layer(Words).write(&doc, Kind::Word).unwrap();
    let mut v = WordView::open(&bytes).unwrap();
    v.set_selection(Selection { anchor: Pos::new(Path::top(0), 0), focus: Pos::new(Path::top(0), 10) });
    let (text, json) = v.copy();
    assert_eq!(text, "plain bold");
    v.set_selection(Selection::caret(Pos::new(Path::top(1), 3)));
    v.paste(&text, Some(&json));
    assert_eq!(texts(&v)[1], "endplain bold");
    let Block::Paragraph(p) = &v.blocks(StoryId::Body)[1] else { panic!() };
    assert_eq!(p.runs().last().unwrap().props.bold, Some(true));
}

/// Laying out only what changed must give the same pages as laying out everything.
#[test]
fn incremental_layout_matches_full_layout() {
    let mut host = FixedMeasurer::default();
    let lines: Vec<String> =
        (0..120).map(|i| format!("Paragraph {i} with some words to wrap across the line when it gets long enough")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut v = WordView::open(&doc_bytes(&refs)).unwrap();
    v.layout(&mut host);
    // A fixed sequence of edits of every kind: typing, Enter, deleting across paragraphs,
    // a table, a page break, a list, undo and redo.
    let mut seed = 7u64;
    let mut rand = |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n.max(1)
    };
    for step in 0..60 {
        let n = v.blocks(StoryId::Body).len() as u64;
        let b = rand(n) as u32;
        let len = match &v.blocks(StoryId::Body)[b as usize] {
            Block::Paragraph(p) => office_engine::text::len(p),
            _ => 0,
        };
        let at = Pos::new(Path::top(b), rand(len as u64 + 1) as u32);
        if !matches!(v.blocks(StoryId::Body)[b as usize], Block::Paragraph(_)) {
            continue;
        }
        v.set_selection(Selection::caret(at.clone()));
        match rand(9) {
            0..=2 => v.insert_text("typed text "),
            3 => v.enter(),
            4 => v.backspace(false),
            5 => {
                let to = Pos::new(Path::top((b + 2).min(n as u32 - 1)), 0);
                if matches!(v.blocks(StoryId::Body)[to.path.block() as usize], Block::Paragraph(_)) {
                    v.set_selection(Selection { anchor: at, focus: to });
                    v.backspace(false);
                }
            }
            6 => v.page_break(),
            7 => {
                v.undo();
            }
            _ => {
                v.redo();
            }
        }
        if step == 20 {
            v.insert_table(2, 2);
        }
        if step == 30 {
            v.list(Some(crate::ListKind::Numbers));
        }
        v.layout(&mut host);
        let mut fresh = WordView::from_document(v.doc.clone());
        fresh.layout(&mut host);
        assert_eq!(v.page_count(), fresh.page_count(), "page count after step {step}");
        for page in 0..v.page_count() {
            let a = serde_json::to_string(&v.paint(&mut host, page).items).unwrap();
            let b = serde_json::to_string(&fresh.paint(&mut host, page).items).unwrap();
            assert!(a == b, "page {page} differs after step {step}");
        }
    }
}

#[test]
fn typing_a_list_marker_starts_a_list() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&["", ""])).unwrap();
    v.layout(&mut host);
    v.set_selection(Selection::caret(Pos::new(Path::top(0), 0)));
    for c in ["-", " ", "a"] {
        v.insert_text(c);
    }
    assert_eq!(texts(&v)[0], "a");
    assert_eq!(v.state().list.as_deref(), Some("bullet"));
    v.undo();
    v.undo();
    assert_eq!(texts(&v)[0], "- ");
    assert_eq!(v.state().list, None);
    v.set_selection(Selection::caret(Pos::new(Path::top(1), 0)));
    for c in ["1", ")", " "] {
        v.insert_text(c);
    }
    assert_eq!(texts(&v)[1], "");
    assert_eq!(v.state().list.as_deref(), Some("numbered"));
    // Mid-paragraph, a dash and a space are only text.
    v.insert_text("x");
    v.insert_text("-");
    v.insert_text(" ");
    assert_eq!(texts(&v)[1], "x- ");
}

#[test]
fn a_workbook_named_docx_is_an_error_not_a_crash() {
    // The app picks the editor by extension; the view once panicked on a workbook's body.
    let book = Compiler::new().write(&Document::new(Kind::Sheets), Kind::Sheets).unwrap();
    let e = WordView::open(&book).err().expect("a workbook isn't a Word document");
    assert_eq!(e.to_string(), "this is an Excel workbook, not a Word document");
}

/// A Word file whose body is `body` (Word markup), as Word would have saved it.
fn markup_bytes(body: &str) -> Vec<u8> {
    let bytes = doc_bytes(&[]);
    let mut pkg = office_core::opc::Package::read(&bytes).unwrap();
    let text = String::from_utf8(pkg.part("word/document.xml").unwrap().to_vec()).unwrap();
    let text = text.replacen("<w:body>", &format!("<w:body>{body}"), 1);
    pkg.replace("word/document.xml", text.into_bytes()).unwrap();
    pkg.to_bytes().unwrap()
}

fn saved_body(v: &mut WordView) -> String {
    let bytes = v.save().unwrap();
    let pkg = office_core::opc::Package::read(&bytes).unwrap();
    String::from_utf8(pkg.part("word/document.xml").unwrap().to_vec()).unwrap()
}

/// The page's text items: (text, x, width, right to left).
fn text_items(v: &mut WordView, host: &mut FixedMeasurer) -> Vec<(String, f64, f64, bool)> {
    v.paint(host, 0)
        .items
        .into_iter()
        .filter_map(|i| if let Item::Text { text, x, w, rtl, .. } = i { Some((text, x, w, rtl)) } else { None })
        .collect()
}

#[test]
fn hebrew_typed_into_an_empty_paragraph_makes_it_right_to_left() {
    use office_core::model::Align;
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&[""])).unwrap();
    v.layout(&mut host);
    v.set_selection(Selection::caret(Pos::new(Path::top(0), 0)));
    for c in ["ש", "ל", "ו", "ם"] {
        v.insert_text(c);
    }
    let para = |v: &WordView| match &v.blocks(StoryId::Body)[0] {
        Block::Paragraph(p) => p.clone(),
        _ => unreachable!(),
    };
    assert_eq!(para(&v).props.rtl, Some(true));
    let st = v.state();
    assert!(st.rtl);
    assert_eq!(st.align, Some("right"));
    // Drawn right to left against the right margin.
    let (w, _) = v.page_size(0);
    let items = text_items(&mut v, &mut host);
    assert_eq!(items.len(), 1, "{items:?}");
    let (text, x, width, rtl) = &items[0];
    assert!(text == "שלום" && *rtl && x + width > w / 2.0 && (w - (x + width) - 96.0).abs() < 2.0, "{items:?} on {w}");

    // The arrows swap: left goes on through the text.
    v.set_selection(Selection::caret(Pos::new(Path::top(0), 0)));
    v.move_caret(&mut host, Move::Left, false);
    assert_eq!(v.selection().focus.offset, 2);
    v.move_caret(&mut host, Move::Right, false);
    assert_eq!(v.selection().focus.offset, 0);

    // Word's left and right mean start and end: aligned right on the page is "left".
    v.align("left");
    assert_eq!((para(&v).props.align, v.state().align), (Some(Align::Right), Some("left")));
    let body = saved_body(&mut v);
    assert!(body.contains("<w:bidi/>") && body.contains(r#"<w:jc w:val="right"/>"#), "{body}");
    v.align("right");
    assert_eq!((para(&v).props.align, v.state().align), (Some(Align::Left), Some("right")));

    // Back to left to right: the direction goes, and the paragraph's start is the left again.
    v.set_direction(false);
    assert_eq!(para(&v).props.rtl, None);
    assert_eq!(v.state().align, Some("left"));
    // Typed English doesn't turn an empty paragraph right to left.
    v.enter();
    v.insert_text("a");
    assert_eq!(para(&v).props.rtl, None);
}

#[test]
fn right_to_left_documents_from_word() {
    let mut host = FixedMeasurer::default();
    // Word's markup: a right-to-left paragraph of a Hebrew run with complex-script size and bold,
    // an English run, and a right-to-left table.
    let body = r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:rtl/><w:sz w:val="20"/><w:szCs w:val="40"/><w:bCs/></w:rPr><w:t xml:space="preserve">אבג </w:t></w:r><w:r><w:rPr><w:sz w:val="20"/><w:szCs w:val="40"/></w:rPr><w:t>abc</w:t></w:r></w:p><w:tbl><w:tblPr><w:bidiVisual/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>first</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>second</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let mut v = WordView::open(&markup_bytes(body)).unwrap();
    v.layout(&mut host);
    let items = text_items(&mut v, &mut host);
    let find = |t: &str| items.iter().find(|i| i.0 == t).unwrap_or_else(|| panic!("{t} in {items:?}")).clone();
    let (hebrew, english) = (find("אבג "), find("abc"));
    // The Hebrew is twice the size (20 pt against 10) and starts the line at the right; the
    // English follows to its left.
    assert!(hebrew.3 && !english.3);
    // (Three letters and a space, which is half a letter wide.)
    assert!((hebrew.2 / 3.5 - 2.0 * (english.2 / 3.0)).abs() < 0.01 * hebrew.2, "{hebrew:?} {english:?}");
    assert!((english.1 + english.2 - hebrew.1).abs() < 0.05, "{english:?} left of {hebrew:?}");
    // The table's first column is at the right.
    let (first, second) = (find("first"), find("second"));
    assert!(first.1 > second.1, "{first:?} {second:?}");
    // Saving leaves the Hebrew run's formatting as it was.
    let body = saved_body(&mut v);
    assert!(body.contains(r#"<w:szCs w:val="40"/>"#) && body.contains("<w:rtl/>") && body.contains("<w:bidiVisual/>"), "{body}");
}
