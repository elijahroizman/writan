use office_core::drawing::Xfrm;
use office_core::model::{Body, Document, Frame, Paragraph, Slide};
use office_core::units::Emu;
use office_core::{Compiler, Kind};
use office_engine::display::Item;
use office_engine::measure::FixedMeasurer;

use super::*;
use crate::Decks;

fn deck(texts: &[&[&str]]) -> Vec<u8> {
    let mut doc = Document::new(Kind::Slides);
    let slides = texts
        .iter()
        .map(|frames| Slide {
            frames: frames
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    Frame::text_box(
                        i as u32 + 2,
                        Xfrm {
                            x: Emu::from_inches(1.0),
                            y: Emu::from_inches(1.0 + i as f64 * 1.5),
                            width: Emu::from_inches(6.0),
                            height: Emu::from_inches(1.0),
                            ..Default::default()
                        },
                        vec![Paragraph::text(*t)],
                    )
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    doc.body = Body::Slides { size: Default::default(), slides };
    Compiler::new().with_layer(Decks).write(&doc, Kind::Slides).unwrap()
}

fn texts(items: &[Item]) -> Vec<String> {
    let mut out = Vec::new();
    for i in items {
        match i {
            Item::Text { text, .. } => out.push(text.clone()),
            Item::Group { items, .. } => out.extend(texts(items)),
            _ => {}
        }
    }
    out
}

#[test]
fn draws_slides() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["Hello", "World"], &["Second"]])).unwrap();
    assert_eq!(v.slides().len(), 2);
    let (w, h) = v.size();
    assert!((w - 1280.0).abs() < 1.0 && (h - 720.0).abs() < 1.0);
    let p = v.paint(&mut host, 0);
    assert_eq!(texts(&p.items), vec!["Hello", "World"]);
    assert!(!v.new_styles().is_empty());
    let els = v.elements(&mut host, 0);
    assert_eq!(els.len(), 2);
    assert_eq!(els[0].x, 96.0);
    assert_eq!(els[0].kind, "text");
    assert_eq!(v.slide_infos().len(), 2);
}

#[test]
fn blank_deck() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::blank();
    assert_eq!(v.slides().len(), 2);
    assert!(texts(&v.paint(&mut host, 0).items).contains(&"Presentation title".to_string()));
}

// A red 1x1 PNG. Clipboard tests compare the actual bytes after a save and reopen.
const CLIPBOARD_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63,
    0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

// A different, green 1x1 PNG for destination media with a conflicting id.
const EXISTING_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63,
    0x60, 0xF8, 0xCF, 0xF0, 0x1F, 0x00, 0x04, 0x01, 0x01, 0xFF, 0x71, 0xEB, 0x47, 0xE5, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

fn picture_ids(frames: &[Frame]) -> Vec<&str> {
    frames
        .iter()
        .flat_map(|f| match &f.content {
            FrameContent::Picture { image } => vec![image.media.as_str()],
            FrameContent::Group { frames } => picture_ids(frames),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn clipboard_pictures_transfer_to_an_independent_deck() {
    let mut source = SlidesView::open(&deck(&[&[]])).unwrap();
    source.insert_image(CLIPBOARD_PNG.to_vec(), "image/png", 1.0, 1.0, "Copied PNG");
    let clipboard = source.copy_frames();
    drop(source);
    let mut target = SlidesView::open(&deck(&[&[]])).unwrap();
    assert!(target.paste_frames(&clipboard));
    let reopened = SlidesView::open(&target.save().unwrap()).unwrap();
    let ids = picture_ids(&reopened.slides()[0].frames);
    assert_eq!(ids.len(), 1);
    assert_eq!(reopened.media(ids[0]).unwrap(), CLIPBOARD_PNG);
    assert_eq!(reopened.media_type(ids[0]).as_deref(), Some("image/png"));
}

#[test]
fn clipboard_pictures_remap_conflicts_and_nested_groups() {
    let source_svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><path fill="#f00" d="M0 0h1v1H0z"/></svg>"##;
    let target_svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><path fill="#0f0" d="M0 0h1v1H0z"/></svg>"##;
    let mut host = FixedMeasurer::default();
    for packaged_source in [false, true] {
        for packaged_target in [false, true] {
            let mut source = SlidesView::open(&deck(&[&[]])).unwrap();
            let png = source.insert_image(CLIPBOARD_PNG.to_vec(), "image/png", 1.0, 1.0, "Copied PNG");
            let svg = source.insert_image(source_svg.to_vec(), "image/svg+xml", 1.0, 1.0, "Copied SVG");
            source.select_frames(vec![png]);
            source.duplicate_frames();
            let duplicate = source.selection().frames[0];
            source.select_frames(vec![duplicate, svg]);
            source.group(&mut host);
            let inner = source.selection().frames[0];
            let shape = source.insert_shape("rect");
            source.select_frames(vec![inner, shape]);
            source.group(&mut host);
            let outer = source.selection().frames[0];
            if packaged_source {
                source = SlidesView::open(&source.save().unwrap()).unwrap();
            }
            source.select_frames(vec![png, outer]);
            let clipboard = source.copy_frames();
            drop(source);

            let mut target = SlidesView::open(&deck(&[&[]])).unwrap();
            target.insert_image(EXISTING_PNG.to_vec(), "image/png", 1.0, 1.0, "Existing PNG");
            target.insert_image(target_svg.to_vec(), "image/svg+xml", 1.0, 1.0, "Existing SVG");
            if packaged_target {
                target = SlidesView::open(&target.save().unwrap()).unwrap();
            }
            let existing = target.slides()[0].frames.clone();
            let old_ids = picture_ids(&existing);
            assert!(target.paste_frames(&clipboard));
            let pasted_ids = picture_ids(&target.slides()[0].frames[existing.len()..]);
            assert_eq!(pasted_ids.len(), 3);
            assert!(pasted_ids.iter().all(|id| !old_ids.contains(id)));
            assert_eq!(pasted_ids[0], pasted_ids[2], "pictures sharing source media share imported media");
            assert_eq!(target.doc.media.len(), 4, "each source image imports once");
            assert!(target.undo());
            assert_eq!(target.slides()[0].frames, existing);
            assert!(target.redo());
            let reopened = SlidesView::open(&target.save().unwrap()).unwrap();
            let ids = picture_ids(&reopened.slides()[0].frames);
            assert_eq!(ids.len(), 5);
            for (id, bytes, content_type) in [
                (ids[0], EXISTING_PNG, "image/png"),
                (ids[1], target_svg.as_slice(), "image/svg+xml"),
                (ids[2], CLIPBOARD_PNG, "image/png"),
                (ids[3], source_svg.as_slice(), "image/svg+xml"),
                (ids[4], CLIPBOARD_PNG, "image/png"),
            ] {
                assert_eq!(reopened.media(id).unwrap(), bytes);
                assert_eq!(reopened.media_type(id).as_deref(), Some(content_type));
            }
        }
    }
}

#[test]
fn clipboard_with_missing_picture_media_is_rejected_without_changes() {
    let mut source = SlidesView::open(&deck(&[&[]])).unwrap();
    let first = source.insert_image(CLIPBOARD_PNG.to_vec(), "image/png", 1.0, 1.0, "First");
    let second = source.insert_image(EXISTING_PNG.to_vec(), "image/png", 1.0, 1.0, "Second");
    source.select_frames(vec![first, second]);
    let mut clipboard: serde_json::Value = serde_json::from_str(&source.copy_frames()).unwrap();
    clipboard["media"].as_object_mut().unwrap().remove("new:2");

    let mut target = SlidesView::open(&deck(&[&["Existing text"]])).unwrap();
    target.insert_image(EXISTING_PNG.to_vec(), "image/png", 1.0, 1.0, "Existing picture");
    target.mark_saved();
    let frames = target.slides()[0].frames.clone();
    let media = target.doc.media.clone();
    let selection = target.selection().clone();
    assert!(!target.paste_frames(&clipboard.to_string()));
    assert_eq!(target.slides()[0].frames, frames);
    assert_eq!(target.doc.media, media);
    assert_eq!(*target.selection(), selection);
    assert!(!target.is_dirty());
    assert!(target.save().is_ok());
}

#[test]
fn legacy_frame_clipboards_accept_text_but_require_media_for_pictures() {
    let mut source = SlidesView::open(&deck(&[&["Copied text"]])).unwrap();
    let text = serde_json::to_string(&source.slides()[0].frames).unwrap();
    let mut target = SlidesView::open(&deck(&[&[]])).unwrap();
    assert!(target.paste_frames(&text));
    let reopened = SlidesView::open(&target.save().unwrap()).unwrap();
    assert_eq!(reopened.slides()[0].frames[0].paragraphs()[0].plain_text(), "Copied text");

    source.insert_image(CLIPBOARD_PNG.to_vec(), "image/png", 1.0, 1.0, "Copied picture");
    let picture = serde_json::to_string(&source.slides()[0].frames[1..]).unwrap();
    target.insert_image(EXISTING_PNG.to_vec(), "image/png", 1.0, 1.0, "Unrelated picture with same media id");
    assert!(!target.paste_frames(&picture));
    assert_eq!(target.slides()[0].frames.len(), 2);
}

#[test]
fn select_move_and_undo() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["Hello", "World"]])).unwrap();
    assert_eq!(v.press(&mut host, 100.0, 100.0, false), Some(2));
    v.move_frames(&mut host, &[2], 50.0, 20.0);
    assert_eq!(v.elements(&mut host, 0)[0].x, 146.0);
    assert!(v.is_dirty());
    v.undo();
    assert_eq!(v.elements(&mut host, 0)[0].x, 96.0);
    v.redo();
    v.set_box(&mut host, 2, Boxed { x: 10.0, y: 10.0, w: 300.0, h: 50.0, rot: 45.0, ..Default::default() });
    let e = &v.elements(&mut host, 0)[0];
    assert_eq!((e.x, e.width, e.rotation), (10.0, 300.0, 45.0));
    v.delete_frames();
    assert_eq!(v.elements(&mut host, 0).len(), 1);
}

#[test]
fn typing_in_a_frame() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["Hello"]])).unwrap();
    assert!(v.begin_edit(&mut host, 2, None));
    v.move_caret(&mut host, "end", false);
    for c in [",", " ", "y", "o", "u"] {
        v.insert_text(c);
    }
    v.enter();
    v.insert_text("Next");
    assert_eq!(v.elements(&mut host, 0)[0].text, "Hello, you\nNext");
    assert!(v.text_overlay(&mut host).unwrap().caret.is_some());
    v.undo();
    v.undo();
    assert_eq!(v.elements(&mut host, 0)[0].text, "Hello, you");
    v.end_edit();
    let bytes = v.save().unwrap();
    let mut back = SlidesView::open(&bytes).unwrap();
    assert_eq!(back.elements(&mut host, 0)[0].text, "Hello, you");
}

#[test]
fn slide_operations() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["One"], &["Two"], &["Three"]])).unwrap();
    v.move_slide(0, 2);
    assert_eq!(texts(&v.paint(&mut host, 2).items), vec!["One"]);
    v.duplicate_slides(&[0]);
    assert_eq!(v.slides().len(), 4);
    v.delete_slides(&[0, 1]);
    assert_eq!(v.slides().len(), 2);
    v.toggle_hidden(&[0]);
    assert!(v.slides()[0].hidden);
    v.set_notes(0, "Speaker notes");
    assert_eq!(v.slide_infos()[0].notes, "Speaker notes");
    v.set_transition(
        &[0],
        Some(crate::Transition { kind: "fade".into(), direction: None, duration_ms: Some(700), on_click: true, after_ms: None }),
    );
    assert_eq!(v.slide_infos()[0].transition.as_ref().map(|t| t.kind.as_str()), Some("fade"));
    let bytes = v.save().unwrap();
    let back = SlidesView::open(&bytes).unwrap();
    assert_eq!(back.slides().len(), 2);
    assert_eq!(back.slide_infos()[0].notes, "Speaker notes");
    assert!(back.slides()[0].hidden);
}

#[test]
fn shapes_tables_and_styles() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["Title"]])).unwrap();
    let shape = v.insert_shape("star5");
    let p = v.paint(&mut host, 0);
    assert!(format!("{:?}", p.items).contains("Path"), "the star is drawn");
    v.set_fill(Some("FF0000"));
    let el = v.elements(&mut host, 0).into_iter().find(|e| e.id == shape.to_string()).unwrap();
    assert_eq!(el.shape_style.unwrap().fill_color.as_deref(), Some("#ff0000"));
    let t = v.insert_table(3, 2);
    let el = v.elements(&mut host, 0).into_iter().find(|e| e.id == t.to_string()).unwrap();
    assert_eq!(el.table.as_ref().map(|t| (t.rows, t.columns, t.header)), Some((3, 2, true)));
    v.table_op("rows", Some("4"));
    assert_eq!(v.elements(&mut host, 0).into_iter().find(|e| e.id == t.to_string()).unwrap().table.unwrap().rows, 4);
    assert!(v.begin_edit(&mut host, t, None));
    v.insert_text("cell");
    v.end_edit();
    v.select_frames(vec![2]);
    v.text_style(&TextPatch { bold: Some(true), font_size: Some(40.0), ..Default::default() });
    let ts = v.elements(&mut host, 0)[0].text_style.clone().unwrap();
    assert!(ts.bold);
    assert_eq!(ts.font_size, Some(40.0));
    v.select_frames(vec![2, shape]);
    v.group(&mut host);
    assert_eq!(v.elements(&mut host, 0).iter().filter(|e| e.kind == "group").count(), 1);
    v.ungroup();
    assert_eq!(v.elements(&mut host, 0).iter().filter(|e| e.kind == "group").count(), 0);
    let before: Vec<String> = v.elements(&mut host, 0).iter().map(|e| format!("{}:{}", e.kind, e.text)).collect();
    let bytes = v.save().unwrap();
    let mut back = SlidesView::open(&bytes).unwrap();
    let after: Vec<String> = back.elements(&mut host, 0).iter().map(|e| format!("{}:{}", e.kind, e.text)).collect();
    assert_eq!(before, after);
    let p = back.paint(&mut host, 0);
    assert!(texts(&p.items).contains(&"cell".to_string()), "{:?}", texts(&p.items));
}

#[test]
fn template_slides_are_scaled_and_undo_as_one_change() {
    let mut host = FixedMeasurer::default();
    for (id, count) in [
        ("titleAndContent", 3),
        ("title", 3),
        ("sectionHeader", 4),
        ("twoContent", 4),
        ("comparison", 6),
        ("titleOnly", 2),
        ("quote", 3),
        ("closing", 3),
        ("blank", 0),
    ] {
        let mut v = SlidesView::open(&deck(&[&["Original"]])).unwrap();
        v.add_template(0, id);
        assert_eq!(v.selection().active, 1);
        let els = v.elements(&mut host, 1);
        assert_eq!(els.len(), count, "{id}");
        assert!(els.iter().all(|e| e.x >= 0.0 && e.x + e.width <= v.size().0 + 1.0));
        let bytes = v.save().unwrap();
        let reopened = SlidesView::open(&bytes).unwrap();
        assert_eq!(reopened.slides().len(), 2);
        assert!(v.undo());
        assert_eq!(v.slides().len(), 1);
        assert!(!v.can_undo());
        assert!(v.redo());
        assert_eq!(v.elements(&mut host, 1).len(), count);
    }
}

#[test]
fn frame_links_round_trip_and_leave_unchanged_parts_alone() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["Linked object"]])).unwrap();
    v.set_link(2, Some("https://example.com/watch?v=123"));
    assert_eq!(v.elements(&mut host, 0)[0].link.as_ref().unwrap().url.as_deref(), Some("https://example.com/watch?v=123"));
    let bytes = v.save().unwrap();
    let mut pkg = office_core::opc::Package::read(&bytes).unwrap();
    let part = "ppt/slides/slide1.xml";
    let mut xml = pkg.xml(part).unwrap();
    let click = xml
        .root
        .child_mut(office_core::xml::ns::P, "cSld")
        .unwrap()
        .child_mut(office_core::xml::ns::P, "spTree")
        .unwrap()
        .child_mut(office_core::xml::ns::P, "sp")
        .unwrap()
        .child_mut(office_core::xml::ns::P, "nvSpPr")
        .unwrap()
        .child_mut(office_core::xml::ns::P, "cNvPr")
        .unwrap()
        .child_mut(office_core::xml::ns::A, "hlinkClick")
        .unwrap();
    click.set_attr("", "tooltip", "Watch this presentation");
    click.set_attr("", "history", "0");
    click.set_attr("", "highlightClick", "1");
    pkg.replace(part, xml.to_bytes()).unwrap();
    let bytes = pkg.to_bytes().unwrap();
    let mut reopened = SlidesView::open(&bytes).unwrap();
    let stable = reopened.save().unwrap();
    let a = office_core::opc::Package::read(&bytes).unwrap();
    let b = office_core::opc::Package::read(&stable).unwrap();
    for name in a.part_names() {
        assert_eq!(a.part(name).unwrap(), b.part(name).unwrap(), "{name}");
    }
    assert_eq!(reopened.elements(&mut host, 0)[0].link.as_ref().unwrap().url.as_deref(), Some("https://example.com/watch?v=123"));
    reopened.set_link(2, None);
    let mut removed = SlidesView::open(&reopened.save().unwrap()).unwrap();
    assert!(removed.elements(&mut host, 0)[0].link.is_none());
    reopened.undo();
    assert!(reopened.elements(&mut host, 0)[0].link.is_some());
}

#[test]
fn table_text_style_applies_to_all_cells_while_editing() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["Original"]])).unwrap();
    let id = v.insert_table(2, 3);
    v.begin_edit(&mut host, id, None);
    v.table_text_style(&TextPatch {
        font_family: Some("Georgia".into()),
        font_size: Some(24.0),
        align: Some("center".into()),
        ..Default::default()
    });
    let frame = v.slides()[0].frames.iter().find(|f| f.id == id).unwrap();
    for p in frame.paragraphs() {
        assert_eq!(p.mark.as_ref().unwrap().font.as_deref(), Some("Georgia"));
        assert_eq!(p.props.align, Some(office_core::model::Align::Center));
    }
    let table = v.elements(&mut host, 0).into_iter().find(|f| f.id == id.to_string()).unwrap().table.unwrap();
    assert_eq!(table.text_style.unwrap().font_size, Some(24.0));
    v.undo();
    let frame = v.slides()[0].frames.iter().find(|f| f.id == id).unwrap();
    assert!(frame.paragraphs().iter().all(|p| p.mark.as_ref().and_then(|m| m.font.as_ref()).is_none()));
}

#[test]
fn transition_changes_in_an_outer_group_undo_together_and_keep_timing_without_an_effect() {
    let mut v = SlidesView::open(&deck(&[&["First"], &["Second"]])).unwrap();
    let timed = crate::Transition { kind: String::new(), direction: None, duration_ms: None, on_click: false, after_ms: Some(1500) };
    v.begin_group();
    v.set_transition(&[0], Some(timed.clone()));
    v.set_transition(&[1], Some(timed.clone()));
    v.end_group();
    assert!(v.slide_infos().iter().all(|s| s.transition.as_ref() == Some(&timed)));
    v.undo();
    assert!(v.slide_infos().iter().all(|s| s.transition.is_none()));
    assert!(!v.can_undo());
}

#[test]
fn a_document_named_pptx_is_an_error_not_a_crash() {
    let doc = Compiler::new().write(&Document::new(Kind::Word), Kind::Word).unwrap();
    let e = SlidesView::open(&doc).err().expect("a Word file isn't a deck");
    assert_eq!(e.to_string(), "this is a Word document, not a PowerPoint presentation");
}

#[test]
fn spelling_reads_text_and_places_ranges() {
    use office_engine::pos::{Path, Pos};
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck(&[&["Teh title", "A secnod line"]])).unwrap();
    let text = v.slide_text();
    let words: Vec<_> = text.iter().map(|(f, p, t)| (*f, p.0.clone(), t.as_str())).collect();
    assert_eq!(words, vec![(2, vec![0], "Teh title"), (3, vec![0], "A secnod line")]);
    // "Teh" in the first box and "secnod" in the second.
    let ranges = [(2, Pos::new(Path(vec![0]), 0), Pos::new(Path(vec![0]), 3)), (3, Pos::new(Path(vec![0]), 2), Pos::new(Path(vec![0]), 8))];
    let boxes = v.text_ranges(&mut host, &ranges);
    assert_eq!(boxes.len(), 2);
    assert_eq!(boxes[0].frame, "2");
    assert_eq!(boxes[0].rects.len(), 1);
    assert_eq!(boxes[0].rects[0].0, 0);
    assert_eq!(boxes[1].rects[0].0, 1);
    // The second range starts two characters in, so further right than the first.
    let (_, x0, _, w0, _) = boxes[0].rects[0];
    let (_, x1, _, w1, _) = boxes[1].rects[0];
    assert!(x1 > x0 && w0 > 0.0 && w1 > w0);
    // Picking a correction selects the word in its box, ready to type over.
    assert!(v.select_text(3, Selection { anchor: Pos::new(Path(vec![0]), 2), focus: Pos::new(Path(vec![0]), 8) }));
    v.insert_text("second");
    assert_eq!(v.slide_text()[1].2, "A second line");
    assert!(!v.select_text(99, Selection::caret(Pos::new(Path(vec![0]), 0))));
}
