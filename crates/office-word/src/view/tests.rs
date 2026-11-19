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

fn block_control_doc(bytes: &[u8], content: &str) -> office_core::opc::Package {
    let mut pkg = office_core::opc::Package::read(bytes).unwrap();
    let xml = String::from_utf8(pkg.part("word/document.xml").unwrap().to_vec()).unwrap();
    let at = xml.find("</w:p>").unwrap() + "</w:p>".len();
    let control = format!(
        r#"<w:sdt><w:sdtPr><w:alias w:val="Contents"/><w:lock w:val="sdtContentLocked"/></w:sdtPr>
            <w:sdtContent>{content}</w:sdtContent></w:sdt>"#
    );
    let text = format!("{}{control}{}", &xml[..at], &xml[at..]);
    pkg.replace("word/document.xml", text.into_bytes()).unwrap();
    pkg
}

#[test]
fn block_content_control_shows_cached_toc_without_rewriting_xml() {
    let pkg = block_control_doc(
        &doc_bytes(&["Before", "After"]),
        r#"<w:p>
             <w:pPr><w:tabs><w:tab w:val="right" w:pos="6000"/></w:tabs></w:pPr>
             <w:r><w:fldChar w:fldCharType="begin"/></w:r>
             <w:r><w:instrText>TOC</w:instrText></w:r>
             <w:r><w:fldChar w:fldCharType="separate"/></w:r>
             <w:hyperlink w:anchor="_Toc1">
               <w:r><w:t>First chapter</w:t><w:tab/></w:r>
               <w:r><w:fldChar w:fldCharType="begin"/></w:r>
               <w:r><w:instrText>PAGEREF _Toc1</w:instrText></w:r>
               <w:r><w:fldChar w:fldCharType="separate"/></w:r>
               <w:r><w:t>iii</w:t></w:r>
               <w:r><w:fldChar w:fldCharType="end"/></w:r>
             </w:hyperlink>
           </w:p>
           <w:customXml w:element="section">
             <w:sdt><w:sdtContent><w:p><w:r><w:t>Nested control</w:t></w:r></w:p></w:sdtContent></w:sdt>
           </w:customXml>
           <w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
    );
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&pkg.to_bytes().unwrap()).unwrap();
    let page = v.paint(&mut host, 0);
    let drawn: Vec<_> = page
        .items
        .iter()
        .filter_map(|i| if let Item::Text { text, x, y, .. } = i { Some((text.as_str(), *x, *y)) } else { None })
        .collect();
    assert_eq!(drawn.iter().map(|(text, ..)| *text).collect::<Vec<_>>(), ["Before", "First chapter", "iii", "Nested control", "After"]);
    let (_, title_x, title_y) = drawn[1];
    let (_, number_x, number_y) = drawn[2];
    assert_eq!(number_y, title_y);
    assert!(number_x - title_x > 300.0, "the cached page number should use the TOC tab stop");
    assert!(v.hit(&mut host, 0, title_x, title_y).is_none(), "the preview is not editable");

    v.set_selection(Selection::caret(Pos::start()));
    v.enter();
    v.insert_text("Edited ");
    let saved_page = v.paint(&mut host, 0);
    assert!(saved_page.items.iter().any(|i| matches!(i, Item::Text {text, ..} if text == "First chapter")));
    assert_eq!(texts(&v), ["", "Edited Before", "[raw]", "After"]);
    let saved = office_core::opc::Package::read(&v.save().unwrap()).unwrap();
    let original_xml = pkg.xml("word/document.xml").unwrap();
    let saved_xml = saved.xml("word/document.xml").unwrap();
    assert_eq!(original_xml.root.path(ns::W, &["body", "sdt"]), saved_xml.root.path(ns::W, &["body", "sdt"]));
}

#[test]
fn block_content_control_flows_across_pages() {
    let content: String = (0..140).map(|i| format!("<w:p><w:r><w:t>Entry {i:03}</w:t></w:r></w:p>")).collect();
    let mut v = WordView::open(&block_control_doc(&doc_bytes(&["Before", "After"]), &content).to_bytes().unwrap()).unwrap();
    let mut host = FixedMeasurer::default();
    v.layout(&mut host);
    assert!(v.page_count() > 1);
    let entries: Vec<_> = (0..v.page_count())
        .flat_map(|page| v.paint(&mut host, page).items)
        .filter_map(|i| match i {
            Item::Text { text, .. } if text.starts_with("Entry ") => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(entries, (0..140).map(|i| format!("Entry {i:03}")).collect::<Vec<_>>());
}

#[test]
fn block_content_control_continues_the_surrounding_list() {
    let mut original = WordView::open(&doc_bytes(&["Before", "After"])).unwrap();
    let list = crate::add_list(&mut original.doc, crate::ListKind::Numbers);
    crate::set_list(&mut original.doc, 0, Some((&list, 0)));
    crate::set_list(&mut original.doc, 1, Some((&list, 0)));
    let pkg = block_control_doc(
        &original.save().unwrap(),
        &format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="{list}"/></w:numPr></w:pPr>
                <w:r><w:t>Inside</w:t></w:r></w:p>"#
        ),
    );
    let mut v = WordView::open(&pkg.to_bytes().unwrap()).unwrap();
    let mut host = FixedMeasurer::default();
    let page = v.paint(&mut host, 0);
    let drawn: Vec<_> =
        page.items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.as_str()) } else { None }).collect();
    assert_eq!(drawn, ["1.", "Before", "2.", "Inside", "3.", "After"]);
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
fn paragraph_styles_render_without_reopening() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::blank();
    v.insert_text("Title");
    v.enter();
    v.insert_text("Body");
    v.set_selection(Selection::caret(Pos::start()));
    v.layout(&mut host);
    assert!(v.outline().is_empty());
    for (id, size, headings) in [("Heading1", 1600, 1), ("Heading2", 1300, 1), ("Title", 2800, 0), ("Normal", 1100, 0)] {
        v.set_style(id);
        v.layout(&mut host);
        let p = pos::para(v.current_blocks(), &Path::top(0)).unwrap();
        assert_eq!(crate::resolved_run(&v.doc, p, p.runs().next().unwrap()).size.unwrap().0, size, "{id}");
        assert_eq!(v.outline().len(), headings, "{id}");
        let mut page = v.paint(&mut host, 0);
        let mut reopened = WordView::open(&v.save().unwrap()).unwrap();
        reopened.layout(&mut host);
        let mut back = reopened.paint(&mut host, 0);
        assert_eq!(page.items.len(), back.items.len());
        for (a, b) in page.items.iter_mut().zip(&mut back.items) {
            if let (Item::Text { s: sa, .. }, Item::Text { s: sb, .. }) = (&mut *a, &mut *b) {
                assert_eq!(v.styles.get(office_engine::para::StyleId(*sa)), reopened.styles.get(office_engine::para::StyleId(*sb)), "{id}");
                *sa = 0;
                *sb = 0;
            }
            assert_eq!(a, b, "{id}");
        }
    }
}

#[test]
fn paragraph_styles_on_empty_paragraphs_and_unknown_styles() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::blank();
    v.layout(&mut host);
    v.set_style("Heading1");
    v.layout(&mut host);
    assert_eq!(v.outline(), vec![(String::new(), 0, Path::top(0))]);
    assert_eq!(v.extras.para_styles["Heading1"].keep_next, Some(true));
    v.undo();
    v.layout(&mut host);
    assert!(v.outline().is_empty());
    v.redo();
    v.layout(&mut host);
    assert_eq!(v.outline().len(), 1);
    let before = word(&v.doc).unwrap().styles.styles.clone();
    v.set_style("UnknownStyle");
    v.layout(&mut host);
    assert!(v.outline().is_empty());
    assert_eq!(word(&v.doc).unwrap().styles.styles, before);
    v.set_style("Heading1");
    v.layout(&mut host);
    assert_eq!(word(&v.doc).unwrap().styles.styles, before, "existing styles are not replaced or duplicated");
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
fn list_enter_continues_labels_and_undoes() {
    for (kind, labels) in [(crate::ListKind::Numbers, vec!["1.", "2.", "3."]), (crate::ListKind::Bullets, vec!["•", "•", "•"])] {
        let mut host = FixedMeasurer::default();
        let mut v = WordView::open(&doc_bytes(&["one"])).unwrap();
        v.set_selection(Selection::caret(Pos::new(Path::top(0), 3)));
        v.list(Some(kind));
        v = WordView::open(&v.save().unwrap()).unwrap();
        v.set_selection(Selection::caret(Pos::new(Path::top(0), 3)));
        for text in ["two", "three"] {
            v.enter();
            v.insert_text(text);
        }
        v.layout(&mut host);
        for (i, label) in labels.iter().enumerate() {
            assert_eq!(v.labels.get(&Path::top(i as u32)).map(String::as_str), Some(*label));
        }
        let expected: Vec<_> = labels.iter().map(|s| Some(s.to_string())).collect();
        assert_eq!(crate::list_labels(&v.doc), expected);
        let back = WordView::open(&v.save().unwrap()).unwrap();
        assert_eq!(crate::list_labels(&back.doc), expected);
        v.undo();
        v.undo();
        assert_eq!(texts(&v), vec!["one", "two"]);
        v.redo();
        assert_eq!(crate::list_labels(&v.doc), expected);
    }
}

#[test]
fn leaving_a_saved_list_restores_its_base_style() {
    let mut v = WordView::open(&doc_bytes(&[""])).unwrap();
    v.list(Some(crate::ListKind::Numbers));
    v = WordView::open(&v.save().unwrap()).unwrap();
    v.enter();
    let p = pos::para(v.blocks(StoryId::Body), &Path::top(0)).unwrap();
    assert_eq!(p.props.style.as_deref(), Some("Normal"));
    let props = word(&v.doc).unwrap().styles.resolve_para(&p.props);
    assert!(props.indent_start.is_none_or(|e| e.0 == 0));
    assert!(props.indent_first_line.is_none_or(|e| e.0 == 0));
    v.undo();
    assert_eq!(v.state().list.as_deref(), Some("numbered"));
    v.redo();
    v.enter();
    let back = WordView::open(&v.save().unwrap()).unwrap();
    assert_eq!(crate::list_labels(&back.doc), vec![None, None]);
}

#[test]
fn leaving_lists_resets_indent_and_is_undoable() {
    use office_core::units::Emu;
    use office_core::xml::{ns, Element};
    for kind in [crate::ListKind::Numbers, crate::ListKind::Bullets] {
        for exit in 0..3 {
            let mut v = WordView::open(&doc_bytes(&[if exit == 0 { "" } else { "item" }])).unwrap();
            v.list(Some(kind));
            let path = Path::top(0);
            let mut listed = pos::para(v.blocks(StoryId::Body), &path).unwrap().clone();
            listed.props.indent_start = Some(Emu::from_twips(720));
            listed.props.indent_first_line = Some(Emu::from_twips(-360));
            let mut origin = listed.origin.get().unwrap().clone();
            origin.child_or_insert(ns::W, "pPr").push(Element::new(ns::W, "ind").with_attr(ns::W, "left", "720").with_attr(
                ns::W,
                "hanging",
                "360",
            ));
            listed.origin = office_core::model::Origin::new(origin);
            v.blocks_mut(StoryId::Body)[0] = Block::Paragraph(listed.clone());
            match exit {
                0 => v.enter(),
                1 => v.list(Some(kind)),
                _ => v.backspace(false),
            }
            let normal = pos::para(v.blocks(StoryId::Body), &path).unwrap().clone();
            assert_eq!(crate::list_labels(&v.doc), vec![None]);
            assert_eq!(normal.props.indent_start, None);
            assert_eq!(normal.props.indent_first_line, None);
            assert_eq!(normal.props.style, None);
            v.undo();
            let restored = pos::para(v.blocks(StoryId::Body), &path).unwrap();
            assert_eq!(restored.props, listed.props);
            assert_eq!(crate::list_of(&v.doc, restored), crate::list_of(&v.doc, &listed));
            v.redo();
            v.set_selection(Selection::caret(Pos::new(path, if exit == 0 { 0 } else { 4 })));
            v.enter();
            v.insert_text("normal");
            let back = WordView::open(&v.save().unwrap()).unwrap();
            assert_eq!(crate::list_labels(&back.doc), vec![None, None]);
            for block in back.blocks(StoryId::Body) {
                let Block::Paragraph(p) = block else { panic!("paragraph") };
                assert_eq!(p.props.indent_start, None);
                assert_eq!(p.props.indent_first_line, None);
            }
        }
    }
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

#[test]
fn a_runs_own_complex_script_size_beats_its_styles() {
    let mut host = FixedMeasurer::default();
    // A character style with a big complex-script size; one run sets its own, equal to its
    // other size (10 pt), as Word writes it.
    let style = r#"<w:style w:type="character" w:styleId="Big"><w:name w:val="Big"/><w:rPr><w:sz w:val="20"/><w:szCs w:val="40"/></w:rPr></w:style>"#;
    let body = r#"<w:p><w:r><w:rPr><w:rStyle w:val="Big"/></w:rPr><w:t>אב</w:t></w:r></w:p><w:p><w:r><w:rPr><w:rStyle w:val="Big"/><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr><w:t>גד</w:t></w:r></w:p>"#;
    let mut pkg = office_core::opc::Package::read(&markup_bytes(body)).unwrap();
    let styles = String::from_utf8(pkg.part("word/styles.xml").unwrap().to_vec()).unwrap();
    pkg.replace("word/styles.xml", styles.replacen("</w:styles>", &format!("{style}</w:styles>"), 1).into_bytes()).unwrap();
    let mut v = WordView::open(&pkg.to_bytes().unwrap()).unwrap();
    v.layout(&mut host);
    let items = text_items(&mut v, &mut host);
    let width = |t: &str| items.iter().find(|i| i.0 == t).unwrap_or_else(|| panic!("{t} in {items:?}")).2;
    // 20 pt and 10 pt: two letters of 26.67 and 13.33 px.
    assert!((width("אב") - 2.0 * width("גד")).abs() < 0.05, "{items:?}");
}

#[test]
fn pasting_lines_into_an_empty_paragraph_leaves_their_direction() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&doc_bytes(&[""])).unwrap();
    v.layout(&mut host);
    v.set_selection(Selection::caret(Pos::new(Path::top(0), 0)));
    v.paste("שלום\nworld wide", None);
    let rtl: Vec<Option<bool>> = v
        .blocks(StoryId::Body)
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.props.rtl,
            _ => None,
        })
        .collect();
    assert_eq!(rtl, vec![None, None]);
}

/// The accents from LibreOffice's `math-accents.docx` (CORPUS-008), in one inline equation.
const MATH_ACCENTS: &str = r#"<w:p><w:r><w:rPr></w:rPr></w:r><m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:acc><m:accPr><m:chr m:val="&#x301;"/></m:accPr><m:e><m:r><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/></w:rPr><m:t xml:space="preserve">a</m:t></m:r></m:e></m:acc><m:acc><m:accPr><m:chr m:val="&#x2D9;"/></m:accPr><m:e><m:r><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/></w:rPr><m:t xml:space="preserve">a</m:t></m:r></m:e></m:acc><m:bar><m:barPr><m:pos m:val="bot"/></m:barPr><m:e><m:r><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/></w:rPr><m:t xml:space="preserve">a</m:t></m:r></m:e></m:bar></m:oMath></w:p>"#;

fn painted_text(v: &mut WordView, host: &mut FixedMeasurer) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for page in 0..v.page_count() {
        for i in v.paint(host, page).items {
            if let Item::Text { text, s, .. } = i {
                out.push((text, s));
            }
        }
    }
    out
}

#[test]
fn office_math_equations_are_drawn_and_survive_saving() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&markup_bytes(MATH_ACCENTS)).unwrap();
    for _ in 0..2 {
        v.layout(&mut host);
        let painted = painted_text(&mut v, &mut host);
        assert_eq!(painted.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["a\u{301}a\u{307}a\u{332}"]);
        let style = v.styles.get(office_engine::para::StyleId(painted[0].1));
        assert!(style.font.family.starts_with("\"Cambria Math\""), "{}", style.font.family);
        let body = saved_body(&mut v);
        assert_eq!(body.matches("<m:acc>").count(), 2, "{body}");
        assert!(body.contains("<m:bar>"), "{body}");
        v = WordView::open(&v.save().unwrap()).unwrap();
    }
}

#[test]
fn display_equations_and_equations_in_content_controls_are_drawn() {
    let mut host = FixedMeasurer::default();
    let m = "http://schemas.openxmlformats.org/officeDocument/2006/math";
    let body = format!(
        r#"<w:p><m:oMathPara xmlns:m="{m}"><m:oMath><m:r><m:t>E=m</m:t></m:r><m:sSup><m:e><m:r><m:t>c</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath></m:oMathPara></w:p><w:p><w:sdt><w:sdtContent><m:oMath xmlns:m="{m}"><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>x</m:t></m:r></m:den></m:f></m:oMath></w:sdtContent></w:sdt></w:p>"#
    );
    let mut v = WordView::open(&markup_bytes(&body)).unwrap();
    v.layout(&mut host);
    let painted: Vec<String> = painted_text(&mut v, &mut host).into_iter().map(|(t, _)| t).collect();
    assert_eq!(painted, ["E=mc²", "1/x"]);
}

#[test]
fn original_corpus_math_accents_are_drawn() {
    let Some(dir) = std::env::var_os("CORPUS_FILES") else { return };
    let bytes = std::fs::read(std::path::Path::new(&dir).join("lo-math-accents.docx")).unwrap();
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&bytes).unwrap();
    v.layout(&mut host);
    let text: String = painted_text(&mut v, &mut host).into_iter().map(|(t, _)| t).collect();
    assert_eq!(text.chars().filter(|c| *c == 'a').count(), 13, "{text:?}");
    assert!(text.contains("a\u{301}") && text.contains("a\u{20D7}") && text.contains("a\u{332}"), "{text:?}");
}

#[test]
fn accepting_and_rejecting_tracked_changes_undoes_as_one_step() {
    let body = r#"<w:p><w:r><w:t xml:space="preserve">Keep </w:t></w:r><w:ins w:id="1" w:author="Ann"><w:r><w:t>new</w:t></w:r></w:ins><w:del w:id="2" w:author="Bob"><w:r><w:delText>old</w:delText></w:r></w:del></w:p>"#;
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&markup_bytes(body)).unwrap();
    v.layout(&mut host);
    let kinds: Vec<_> = v.revisions().iter().map(|r| (r.kind, r.author.clone(), r.text.clone())).collect();
    assert_eq!(
        kinds,
        vec![(crate::RevisionKind::Insertion, "Ann".into(), "new".into()), (crate::RevisionKind::Deletion, "Bob".into(), "old".into())]
    );

    assert_eq!(v.resolve_revisions(true), 2);
    v.layout(&mut host);
    assert!(v.revisions().is_empty());
    assert_eq!(texts(&v)[0], "Keep new");
    assert!(v.is_dirty());
    let saved = saved_body(&mut v);
    assert!(!saved.contains("w:ins") && !saved.contains("w:del") && saved.contains("new") && !saved.contains("old"), "{saved}");

    assert!(v.undo());
    v.layout(&mut host);
    assert_eq!(v.revisions().len(), 2);
    assert!(v.redo());
    assert!(v.revisions().is_empty());
    assert!(v.undo());

    assert_eq!(v.resolve_revisions(false), 2);
    v.layout(&mut host);
    assert!(v.revisions().is_empty());
    assert_eq!(texts(&v)[0], "Keep old");
    // Nothing left to resolve: no change is recorded.
    assert_eq!(v.resolve_revisions(true), 0);
    assert!(v.undo());
    assert_eq!(v.revisions().len(), 2);
}

#[test]
fn resolving_only_paragraph_marks_or_rows_takes_effect() {
    let mut host = FixedMeasurer::default();
    let marks = r#"<w:p><w:pPr><w:rPr><w:ins w:id="1" w:author="Ann"/></w:rPr></w:pPr><w:r><w:t>One</w:t></w:r></w:p><w:p><w:r><w:t>Two</w:t></w:r></w:p>"#;
    let mut v = WordView::open(&markup_bytes(marks)).unwrap();
    v.layout(&mut host);
    assert_eq!(v.revisions().len(), 1);
    assert_eq!(v.resolve_revisions(true), 1);
    assert!(v.revisions().is_empty());
    assert_eq!(texts(&v), vec!["One", "Two"]);
    assert!(v.undo());
    assert_eq!(v.revisions().len(), 1);

    // A table whose only row was deleted: accepting removes the table and leaves somewhere to type.
    let rows = r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:trPr><w:del w:id="1" w:author="Bob"/></w:trPr><w:tc><w:p><w:r><w:t>gone</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    let mut v = WordView::open(&markup_bytes(rows)).unwrap();
    v.layout(&mut host);
    let kinds: Vec<_> = v.revisions().iter().map(|r| r.kind).collect();
    assert_eq!(kinds, vec![crate::RevisionKind::Row]);
    assert_eq!(v.resolve_revisions(true), 1);
    v.layout(&mut host);
    assert!(v.revisions().is_empty());
    v.insert_text("typed");
    assert_eq!(texts(&v), vec!["typed"]);
}

#[test]
fn smartart_in_text_is_drawn_and_takes_its_room() {
    let mut host = FixedMeasurer::default();
    let bytes = office_core::formats::diagram::test_diagram(&doc_bytes(&["After"]), "word/document.xml", true, |xml, data| {
        let run = format!(
            r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="{}"><wp:extent cx="3048000" cy="1524000"/><wp:docPr id="1" name="Diagram 1"/><a:graphic xmlns:a="{}">{data}</a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
            ns::WP,
            ns::A
        );
        xml.replacen("<w:body>", &format!("<w:body>{run}"), 1)
    });
    let mut v = WordView::open(&bytes).unwrap();
    let page = v.paint(&mut host, 0);
    fn walk(items: &[Item], dy: f64, out: &mut Vec<(String, f64)>) {
        for i in items {
            match i {
                Item::Text { text, y, .. } => out.push((text.clone(), dy + y)),
                Item::Group { items, y, .. } => walk(items, dy + y, out),
                _ => {}
            }
        }
    }
    let mut texts = Vec::new();
    walk(&page.items, 0.0, &mut texts);
    let names: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(names, vec!["Cached", "After"]);
    // The diagram is 160 px tall, so the next paragraph starts below it.
    assert!(texts[1].1 - texts[0].1 > 80.0, "{texts:?}");
    assert!(office_core::opc::Package::read(&v.save().unwrap()).unwrap().has("word/diagrams/drawing1.xml"));
}

#[test]
fn smartart_in_a_block_content_control_is_drawn() {
    let mut host = FixedMeasurer::default();
    let bytes = office_core::formats::diagram::test_diagram(&doc_bytes(&["After"]), "word/document.xml", true, |xml, data| {
        let control = format!(
            r#"<w:sdt><w:sdtPr/><w:sdtContent><w:p><w:r><w:drawing><wp:inline xmlns:wp="{}"><wp:extent cx="3048000" cy="1524000"/><wp:docPr id="1" name="Diagram 1"/><a:graphic xmlns:a="{}">{data}</a:graphic></wp:inline></w:drawing></w:r></w:p></w:sdtContent></w:sdt>"#,
            ns::WP,
            ns::A
        );
        xml.replacen("<w:body>", &format!("<w:body>{control}"), 1)
    });
    let mut v = WordView::open(&bytes).unwrap();
    let page = v.paint(&mut host, 0);
    fn walk(items: &[Item], out: &mut Vec<String>) {
        for i in items {
            match i {
                Item::Text { text, .. } => out.push(text.clone()),
                Item::Group { items, .. } => walk(items, out),
                _ => {}
            }
        }
    }
    let mut texts = Vec::new();
    walk(&page.items, &mut texts);
    assert_eq!(texts, vec!["Cached", "After"]);
}

/// A document whose body is `body` (Word markup) with `notes` (footnote bodies, by id) in
/// `word/footnotes.xml`, as Word saves them: separators first, each note starting with its mark.
fn footnoted_doc(body: &str, notes: &[(&str, &str)]) -> Vec<u8> {
    noted_doc(body, "footnote", notes)
}

/// [`footnoted_doc`] for `kind` notes (`footnote` or `endnote`).
fn noted_doc(body: &str, kind: &str, notes: &[(&str, &str)]) -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let mut pkg = office_core::opc::Package::read(&doc_bytes(&[])).unwrap();
    let text = String::from_utf8(pkg.part("word/document.xml").unwrap().to_vec()).unwrap();
    pkg.replace("word/document.xml", text.replacen("<w:body>", &format!("<w:body>{body}"), 1).into_bytes()).unwrap();
    let mut xml = format!(
        r#"<w:{kind}s xmlns:w="{W}"><w:{kind} w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:{kind}><w:{kind} w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{kind}>"#
    );
    for (id, note) in notes {
        xml += &format!(
            r#"<w:{kind} w:id="{id}"><w:p><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:{kind}Ref/></w:r><w:r><w:t xml:space="preserve"> {note}</w:t></w:r></w:p></w:{kind}>"#
        );
    }
    xml += &format!("</w:{kind}s>");
    let part = format!("word/{kind}s.xml");
    pkg.put(&part, xml.into_bytes(), &format!("application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}s+xml"));
    pkg.relate("word/document.xml", &format!("{kind}s"), &part).unwrap();
    pkg.to_bytes().unwrap()
}

fn endnote_para(text: &str, id: &str) -> String {
    format!(
        r#"<w:p><w:r><w:t>{text}</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:endnoteReference w:id="{id}"/></w:r></w:p>"#
    )
}

fn footnote_para(text: &str, id: &str) -> String {
    format!(
        r#"<w:p><w:r><w:t>{text}</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="{id}"/></w:r></w:p>"#
    )
}

fn page_texts(items: &[Item]) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for i in items {
        match i {
            Item::Text { text, y, .. } => out.push((text.clone(), *y)),
            Item::Group { items, .. } => out.extend(page_texts(items)),
            _ => {}
        }
    }
    out
}

#[test]
fn footnote_bodies_paint_at_the_bottom_of_their_page() {
    let mut host = FixedMeasurer::default();
    let mut v = WordView::open(&footnoted_doc(&footnote_para("Foo", "1"), &[("1", "bar")])).unwrap();
    v.layout(&mut host);
    assert_eq!(v.page_count(), 1);
    let page = v.paint(&mut host, 0);
    let texts = page_texts(&page.items);
    let y = |t: &str| texts.iter().find(|(s, _)| s.trim() == t).map(|(_, y)| *y).unwrap_or_else(|| panic!("{t:?} in {texts:?}"));
    // The body's reference mark, then the note's own mark and its text.
    assert_eq!(texts.iter().filter(|(s, _)| s == "1").count(), 2, "{texts:?}");
    let (foo, bar) = (y("Foo"), y("bar"));
    assert!(bar > page.height * 0.75 && bar < page.height, "the note sits at the bottom of the page: {bar} of {}", page.height);
    assert!(bar > foo);
    let separator = page.items.iter().find_map(|i| match i {
        Item::Line { y1, y2, .. } if y1 == y2 => Some(*y1),
        _ => None,
    });
    assert!(separator.is_some_and(|s| s > foo && s < bar), "a separator line above the note: {separator:?}");
}

#[test]
fn footnotes_take_space_from_the_body_and_stay_on_their_page() {
    let mut host = FixedMeasurer::default();
    let body: String = (1..=40).map(|i| footnote_para(&format!("Paragraph {i}"), &i.to_string())).collect();
    let notes: Vec<(String, String)> = (1..=40).map(|i| (i.to_string(), format!("Note {i} text"))).collect();
    let notes: Vec<(&str, &str)> = notes.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut v = WordView::open(&footnoted_doc(&body, &notes)).unwrap();
    let mut plain = WordView::open(&doc_bytes(
        &(1..=40).map(|i| format!("Paragraph {i}")).collect::<Vec<_>>().iter().map(|s| s.as_str()).collect::<Vec<_>>(),
    ))
    .unwrap();
    plain.layout(&mut host);
    v.layout(&mut host);
    assert!(v.page_count() > plain.page_count(), "{} pages with notes, {} without", v.page_count(), plain.page_count());
    let mut seen = Vec::new();
    for p in 0..v.page_count() {
        let texts = page_texts(&v.paint(&mut host, p).items);
        let paras: Vec<(u32, f64)> =
            texts.iter().filter_map(|(t, y)| Some((t.strip_prefix("Paragraph ")?.trim().parse().ok()?, *y))).collect();
        let notes: Vec<(u32, f64)> =
            texts.iter().filter_map(|(t, y)| Some((t.trim().strip_prefix("Note ")?.strip_suffix(" text")?.parse().ok()?, *y))).collect();
        let mut on_page: Vec<u32> = paras.iter().map(|(n, _)| *n).collect();
        on_page.dedup();
        assert_eq!(notes.iter().map(|(n, _)| *n).collect::<Vec<_>>(), on_page, "page {p}: each note with its reference");
        let lowest = paras.iter().map(|(_, y)| *y).fold(f64::MIN, f64::max);
        assert!(notes.iter().all(|(_, y)| *y > lowest), "page {p}: notes below the body");
        seen.extend(notes.iter().map(|(n, _)| *n));
    }
    assert_eq!(seen, (1..=40).collect::<Vec<_>>());
}

#[test]
fn footnotes_referenced_in_content_controls_are_painted() {
    let mut host = FixedMeasurer::default();
    let body = format!("<w:sdt><w:sdtContent>{}</w:sdtContent></w:sdt>", footnote_para("Foo", "1"));
    let mut v = WordView::open(&footnoted_doc(&body, &[("1", "bar")])).unwrap();
    v.layout(&mut host);
    let texts = page_texts(&v.paint(&mut host, 0).items);
    assert!(texts.iter().any(|(t, _)| t.trim() == "Foo"), "{texts:?}");
    assert!(texts.iter().any(|(t, _)| t.trim() == "bar"), "{texts:?}");
}

#[test]
fn endnotes_follow_the_last_line_with_roman_marks() {
    let mut host = FixedMeasurer::default();
    let body = format!("{}<w:p><w:r><w:t>Last line</w:t></w:r></w:p>", endnote_para("Foo", "1"));
    let mut v = WordView::open(&noted_doc(&body, "endnote", &[("1", "bar")])).unwrap();
    v.layout(&mut host);
    assert_eq!(v.page_count(), 1);
    let page = v.paint(&mut host, 0);
    let texts = page_texts(&page.items);
    let y = |t: &str| texts.iter().find(|(s, _)| s.trim() == t).map(|(_, y)| *y).unwrap_or_else(|| panic!("{t:?} in {texts:?}"));
    // Word numbers endnotes i, ii, iii: the body's mark and the note's own.
    assert_eq!(texts.iter().filter(|(s, _)| s == "i").count(), 2, "{texts:?}");
    let (last, bar) = (y("Last line"), y("bar"));
    assert!(bar > last && bar < page.height * 0.5, "the note follows the last line, not the page bottom: {bar}");
    let separator = page.items.iter().find_map(|i| match i {
        Item::Line { y1, y2, .. } if y1 == y2 => Some(*y1),
        _ => None,
    });
    assert!(separator.is_some_and(|s| s > last && s < bar), "a separator line above the note: {separator:?}");
}

#[test]
fn endnotes_that_dont_fit_go_on_new_pages_in_order() {
    let mut host = FixedMeasurer::default();
    let body: String = (1..=20).map(|i| endnote_para(&format!("Paragraph {i}"), &i.to_string())).collect();
    let long = "word ".repeat(60);
    let notes: Vec<(String, String)> = (1..=20).map(|i| (i.to_string(), format!("Note {i} {long}"))).collect();
    let notes: Vec<(&str, &str)> = notes.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut v = WordView::open(&noted_doc(&body, "endnote", &notes)).unwrap();
    v.layout(&mut host);
    assert!(v.page_count() > 1, "{} pages", v.page_count());
    let mut seen = Vec::new();
    let mut last_body_page = 0;
    for p in 0..v.page_count() {
        let texts = page_texts(&v.paint(&mut host, p).items);
        if texts.iter().any(|(t, _)| t.starts_with("Paragraph ")) {
            last_body_page = p;
        }
        for (t, _) in &texts {
            if let Some(n) = t.trim().strip_prefix("Note ").and_then(|t| t.split_whitespace().next()).and_then(|n| n.parse::<u32>().ok()) {
                assert!(p >= last_body_page, "note {n} on page {p}, before the body ends");
                seen.push(n);
            }
        }
    }
    assert_eq!(seen, (1..=20).collect::<Vec<_>>());
    assert!(v.page_count() > last_body_page + 1, "endnotes continue on pages of their own");
}
