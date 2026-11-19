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

const CHART_PART: &str = "ppt/charts/chart1.xml";
const CHART_BOOK: &str = "ppt/embeddings/data.xlsx";

fn chart_deck(groups: &str, external: bool) -> Vec<u8> {
    chart_deck_with_properties(groups, external, "")
}

fn chart_deck_with_properties(groups: &str, external: bool, properties: &str) -> Vec<u8> {
    use office_core::opc::Package;
    use office_core::xml::ns;

    let part = "ppt/slides/slide1.xml";
    let mut pkg = Package::read(&deck(&[&["Editable title"]])).unwrap();
    let rid = if external {
        pkg.relate_external(part, "chart", "https://example.com/chart.xml").unwrap()
    } else {
        let xlsx = office_core::write(&Document::new(Kind::Sheets), Kind::Sheets).unwrap();
        pkg.put(CHART_BOOK, xlsx, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");
        let data = pkg.relate(CHART_PART, "package", CHART_BOOK).unwrap();
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{c}" xmlns:a="{a}" xmlns:r="{r}">
              <c:clrMapOvr bg1="lt1"/><c:chart><c:autoTitleDeleted val="1"/><c:plotArea>{groups}</c:plotArea>{properties}</c:chart>
              <c:externalData r:id="{data}"/>
            </c:chartSpace>"#,
            c = ns::C,
            a = ns::A,
            r = ns::R,
        );
        pkg.put(CHART_PART, xml.into_bytes(), office_core::formats::xlsx::chart::CHART_TYPE);
        pkg.relate(part, "chart", CHART_PART).unwrap()
    };
    let frame = format!(
        r#"<p:graphicFrame xmlns:c="{c}" xmlns:r="{r}">
          <p:nvGraphicFramePr><p:cNvPr id="20" name="Imported chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
          <p:xfrm><a:off x="952500" y="1905000"/><a:ext cx="3810000" cy="2857500"/></p:xfrm>
          <a:graphic><a:graphicData uri="{c}"><c:chart r:id="{rid}"/></a:graphicData></a:graphic>
        </p:graphicFrame>"#,
        c = ns::C,
        r = ns::R,
    );
    let slide = String::from_utf8(pkg.part(part).unwrap().to_vec()).unwrap();
    let slide = slide
        .replace("</p:spTree>", &format!("{frame}</p:spTree>"))
        .replace("</p:sld>", r#"<p:clrMapOvr><a:overrideClrMapping bg1="dk1"/></p:clrMapOvr></p:sld>"#);
    pkg.replace(part, slide.into_bytes()).unwrap();
    pkg.to_bytes().unwrap()
}

const CHART_SERIES: &str = r#"<c:ser>
    <c:idx val="0"/><c:order val="0"/>
    <c:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>
    <c:dPt><c:idx val="2"/><c:spPr><a:solidFill><a:schemeClr val="bg1"><a:lumMod val="85000"/></a:schemeClr></a:solidFill></c:spPr></c:dPt>
    <c:cat><c:strRef><c:f>Data!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/>
      <c:pt idx="0"><c:v>April</c:v></c:pt><c:pt idx="1"><c:v>May</c:v></c:pt><c:pt idx="2"><c:v>June</c:v></c:pt>
    </c:strCache></c:strRef></c:cat>
    <c:val><c:numRef><c:f>Data!$B$2:$B$4</c:f><c:numCache><c:ptCount val="3"/>
      <c:pt idx="2"><c:v>5</c:v></c:pt><c:pt idx="0"><c:v>95</c:v></c:pt>
    </c:numCache></c:numRef></c:val>
  </c:ser>"#;

fn chart_items(paint: &SlidePaint) -> &[Item] {
    let frame = paint
        .items
        .iter()
        .find_map(|item| match item {
            Item::Group { id: Some(id), x, y, w, h, items, .. } if id == "20" => {
                assert_eq!((*x, *y, *w, *h), (100.0, 200.0, 400.0, 300.0));
                Some(items)
            }
            _ => None,
        })
        .unwrap();
    match frame.as_slice() {
        [Item::Group { clip: true, items, .. }] => items,
        _ => frame,
    }
}

#[test]
fn imported_doughnut_paints_cached_slices_and_preserves_chart_parts() {
    use office_core::opc::Package;

    let groups = format!(r#"<c:doughnutChart><c:varyColors val="1"/>{CHART_SERIES}<c:holeSize val="60"/></c:doughnutChart>"#);
    let bytes = chart_deck(&groups, false);
    let original = Package::read(&bytes).unwrap();
    let mut view = SlidesView::open(&bytes).unwrap();
    let mut host = FixedMeasurer::default();
    let p = view.paint(&mut host, 0);
    let items = chart_items(&p);
    assert_eq!(items.len(), 2, "missing values aren't plotted");
    for (item, color) in items.iter().zip(["#4472c4", "#d9d9d9"]) {
        let Item::Path { d, fill: Some(fill), dash: "solid", .. } = item else { panic!("{item:?}") };
        assert_eq!(fill.to_ascii_lowercase(), color);
        assert_eq!(d.matches('A').count(), 2, "both arcs of the doughnut slice: {d}");
    }
    view.set_link(2, Some("https://example.com/"));
    let saved = view.save().unwrap();
    let after = Package::read(&saved).unwrap();
    for part in [CHART_PART, "ppt/charts/_rels/chart1.xml.rels", CHART_BOOK] {
        assert_eq!(after.part(part).unwrap(), original.part(part).unwrap());
    }
    let mut reopened = SlidesView::open(&saved).unwrap();
    assert_eq!(chart_items(&reopened.paint(&mut host, 0)), items);
}

#[test]
fn duplicated_chart_slides_keep_their_relationships() {
    let groups = format!("<c:doughnutChart>{CHART_SERIES}</c:doughnutChart>");
    for with_notes in [false, true] {
        let mut view = SlidesView::open(&chart_deck(&groups, false)).unwrap();
        if with_notes {
            crate::set_notes(&mut view.doc, 0, vec![Paragraph::text("Chart notes")]);
        }
        let mut host = FixedMeasurer::default();
        let original = view.paint(&mut host, 0);
        view.duplicate_slides(&[0]);
        assert_eq!(chart_items(&view.paint(&mut host, 1)), chart_items(&original));
        let mut reopened = SlidesView::open(&view.save().unwrap()).unwrap();
        assert_eq!(chart_items(&reopened.paint(&mut host, 1)), chart_items(&original));
        view.delete_slides(&[0]);
        let mut reopened = SlidesView::open(&view.save().unwrap()).unwrap();
        assert_eq!(chart_items(&reopened.paint(&mut host, 0)), chart_items(&original));
        assert_eq!(reopened.slide_infos()[0].notes, if with_notes { "Chart notes" } else { "" });
    }
}

#[test]
fn pasted_chart_remaps_slide_relationship_and_frame_ids() {
    use office_core::formats::pptx::{chart_element, chart_target};
    use office_core::opc::{rels::kind, Package};
    use office_core::xml::ns;

    let groups = format!("<c:doughnutChart>{CHART_SERIES}</c:doughnutChart>");
    let mut view = SlidesView::open(&chart_deck(&groups, false)).unwrap();
    view.add_slide(0, None);
    let mut input = Package::read(&view.save().unwrap()).unwrap();
    let source = "ppt/slides/slide1.xml";
    let target = "ppt/slides/slide2.xml";
    let source_id = input.rels(source).unwrap().first(kind::CHART).unwrap().id.clone();
    let collision = input.relate_external(target, kind::HYPERLINK, "https://example.com/").unwrap();
    assert_eq!(source_id, collision, "the original rId means something else on the destination");
    let mut view = SlidesView::open(&input.to_bytes().unwrap()).unwrap();
    let mut existing = view.slides()[0].frames[0].clone();
    existing.id = 20;
    view.slides_mut()[1].frames.push(existing);
    let legacy = serde_json::to_string(&[view.slides()[0].frames.last().unwrap()]).unwrap();
    view.select_frames(vec![20]);
    let clipboard = view.copy_frames();
    let mut independent = SlidesView::open(&input.to_bytes().unwrap()).unwrap();
    independent.select_frames(vec![20]);
    assert_ne!(independent.copy_frames(), "[]");
    assert!(!independent.paste_frames(&clipboard));
    assert!(!independent.is_dirty());

    view.set_active(1);
    assert!(!view.paste_frames(&legacy), "a bare chart rId is not a portable clipboard reference");
    assert!(view.paste_frames(&clipboard));
    let id = view.selection().frames[0];
    assert_ne!(id, 20);
    let mut host = FixedMeasurer::default();
    let painted =
        |p: SlidePaint| p.items.into_iter().find(|item| matches!(item, Item::Group { id: Some(i), .. } if i == &id.to_string())).unwrap();
    let before = painted(view.paint(&mut host, 1));
    assert!(matches!(&before, Item::Group { items, .. } if matches!(
        items.as_slice(),
        [Item::Group { clip: true, items, .. }] if items.len() == 2
            && items.iter().all(|item| matches!(item, Item::Path { fill: Some(_), .. }))
    )));
    assert!(view.undo());
    assert!(view.redo());
    assert_eq!(painted(view.paint(&mut host, 1)), before);

    // Recopy a pasted chart, which no longer uses the original slide's rId.
    view.select_frames(vec![id]);
    let clipboard = view.copy_frames();
    view.add_slide(1, None);
    assert!(view.paste_frames(&clipboard));

    let saved = view.save().unwrap();
    let output = Package::read(&saved).unwrap();
    let slide_xml = output.xml(target).unwrap();
    let frame = slide_xml.root.descendant(ns::P, "graphicFrame").unwrap();
    assert_eq!(chart_target(&output, target, frame).as_deref(), Some(CHART_PART));
    assert_ne!(chart_element(frame).unwrap().attr_ns(ns::R, "id"), Some(source_id.as_str()));
    assert_eq!(output.rels(target).unwrap().get(&collision).unwrap().target, "https://example.com/");
    for part in [CHART_PART, "ppt/charts/_rels/chart1.xml.rels", CHART_BOOK] {
        assert_eq!(output.part(part).unwrap(), input.part(part).unwrap(), "{part}");
    }
    let mut reopened = SlidesView::open(&saved).unwrap();
    assert_eq!(painted(reopened.paint(&mut host, 1)), before);
    let part = reopened.slides()[2].part.as_deref().unwrap();
    let slide_xml = output.xml(part).unwrap();
    assert_eq!(chart_target(&output, part, slide_xml.root.descendant(ns::P, "graphicFrame").unwrap()).as_deref(), Some(CHART_PART));
}

#[test]
fn chart_move_and_resize_survive_reopening() {
    use office_core::opc::Package;

    let groups = format!("<c:doughnutChart>{CHART_SERIES}</c:doughnutChart>");
    let bytes = chart_deck(&groups, false);
    let mut view = SlidesView::open(&bytes).unwrap();
    let mut host = FixedMeasurer::default();
    let original = Package::read(&bytes).unwrap();
    let part = "ppt/slides/slide1.xml";
    let untouched = Package::read(&view.save().unwrap()).unwrap();
    assert_eq!(untouched.part(part).unwrap(), original.part(part).unwrap());

    view.set_box(&mut host, 20, Boxed { x: 100.0, y: 200.0, w: 320.0, h: 185.0, ..Default::default() });
    view.move_frames(&mut host, &[20], 58.0, -73.0);
    let mut reopened = SlidesView::open(&view.save().unwrap()).unwrap();
    let paint = reopened.paint(&mut host, 0);
    let chart = paint.items.iter().find(|item| matches!(item, Item::Group { id: Some(id), .. } if id == "20")).unwrap();
    let Item::Group { x, y, w, h, items, .. } = chart else { unreachable!() };
    assert_eq!((*x, *y, *w, *h), (158.0, 127.0, 320.0, 185.0));
    assert!(matches!(items.as_slice(), [Item::Group { clip: true, .. }]));
    let output = reopened.doc.package.as_ref().unwrap();
    for part in [CHART_PART, "ppt/charts/_rels/chart1.xml.rels", CHART_BOOK] {
        assert_eq!(output.part(part).unwrap(), original.part(part).unwrap(), "{part}");
    }
}

#[test]
fn duplicating_a_slide_does_not_reuse_its_link_to_a_deleted_slide() {
    use office_core::opc::Package;
    use office_core::xml::{ns, Element};

    let source = "ppt/slides/slide1.xml";
    let deleted = "ppt/slides/slide2.xml";
    let kept = "ppt/slides/slide3.xml";
    let mut pkg = Package::read(&deck(&[&["Source"], &["Deleted"], &["Kept"]])).unwrap();
    let removed_id = pkg.relate(source, "slide", deleted).unwrap();
    let jump = kept.to_ascii_uppercase();
    let kept_id = pkg.relate(source, "slide", &jump).unwrap();
    let missing_id = pkg.relate(source, "slide", "PPT/SLIDES/SLIDE4.XML").unwrap();
    let mut xml = pkg.xml(source).unwrap();
    let nv = xml
        .root
        .child_mut(ns::P, "cSld")
        .unwrap()
        .child_mut(ns::P, "spTree")
        .unwrap()
        .child_mut(ns::P, "sp")
        .unwrap()
        .child_mut(ns::P, "nvSpPr")
        .unwrap()
        .child_mut(ns::P, "cNvPr")
        .unwrap();
    for (name, rid) in [("hlinkClick", &removed_id), ("hlinkMouseOver", &kept_id)] {
        nv.push(Element::new(ns::A, name).with_attr(ns::R, "id", rid).with_attr("", "action", "ppaction://hlinksldjump"));
    }
    pkg.replace(source, xml.to_bytes()).unwrap();
    let mut view = SlidesView::open(&pkg.to_bytes().unwrap()).unwrap();
    view.duplicate_slides(&[0]);
    view.delete_slides(&[2]);
    view.add_slide(2, None);
    crate::set_notes(&mut view.doc, 3, vec![Paragraph::text("New slide notes")]);
    let bytes = view.save().unwrap();
    let saved = Package::read(&bytes).unwrap();
    let reopened = SlidesView::open(&bytes).unwrap();
    let duplicate = reopened.slides()[1].part.as_deref().unwrap();
    let links = saved.rels(duplicate).unwrap();
    assert!(links.get(&removed_id).is_none());
    assert_eq!(reopened.slides()[3].part.as_deref(), Some("ppt/slides/slide4.xml"));
    assert!(links.get(&missing_id).is_none());
    let target = saved.target(duplicate, links.get(&kept_id).unwrap()).unwrap();
    assert_eq!(target, jump);
    assert_eq!(saved.part(&target).unwrap(), saved.part(kept).unwrap());
    let xml = saved.xml(duplicate).unwrap();
    assert!(xml.root.descendant(ns::A, "hlinkClick").is_none());
    assert_eq!(xml.root.descendant(ns::A, "hlinkMouseOver").unwrap().attr_ns(ns::R, "id"), Some(kept_id.as_str()));
}

#[test]
fn duplicated_slides_do_not_share_the_original_comments_part() {
    use office_core::opc::Package;

    let source = "ppt/slides/slide1.xml";
    let comments = "ppt/comments/comment1.xml";
    let mut pkg = Package::read(&deck(&[&["Source"]])).unwrap();
    pkg.put(
        comments,
        br#"<p:cmLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#.to_vec(),
        "application/vnd.openxmlformats-officedocument.presentationml.comments+xml",
    );
    pkg.relate(source, "comments", comments).unwrap();
    let mut view = SlidesView::open(&pkg.to_bytes().unwrap()).unwrap();
    view.duplicate_slides(&[0]);
    let bytes = view.save().unwrap();
    let saved = Package::read(&bytes).unwrap();
    let reopened = SlidesView::open(&bytes).unwrap();
    let duplicate = reopened.slides()[1].part.as_deref().unwrap();
    assert_eq!(saved.related(source, "comments").unwrap()[0].1, comments);
    assert_eq!(saved.part(comments).unwrap(), pkg.part(comments).unwrap());
    assert!(saved.related(duplicate, "comments").unwrap().is_empty());
}

#[test]
fn imported_columns_show_cached_values_and_categories() {
    let groups = format!(r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{CHART_SERIES}</c:barChart>"#);
    let mut view = SlidesView::open(&chart_deck(&groups, false)).unwrap();
    let mut host = FixedMeasurer::default();
    let p = view.paint(&mut host, 0);
    let items = chart_items(&p);
    let bars: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            Item::Rect { h, .. } => Some(*h),
            _ => None,
        })
        .collect();
    assert_eq!(bars.len(), 2);
    assert!((bars[0] / bars[1] - 19.0).abs() < 0.05);
    let labels = texts(items);
    assert!(labels.contains(&"April".to_string()));
    assert!(labels.contains(&"June".to_string()));
}

#[test]
fn imported_scatter_without_x_values_uses_point_numbers() {
    let groups = r#"<c:scatterChart><c:scatterStyle val="marker"/><c:ser><c:yVal><c:numLit><c:ptCount val="3"/>
       <c:pt idx="0"><c:v>4</c:v></c:pt><c:pt idx="2"><c:v>4</c:v></c:pt>
       </c:numLit></c:yVal></c:ser></c:scatterChart>"#;
    let mut view = SlidesView::open(&chart_deck(groups, false)).unwrap();
    let p = view.paint(&mut FixedMeasurer::default(), 0);
    let items = chart_items(&p);
    let xs: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            Item::Path { x, fill: Some(_), .. } => Some(*x),
            _ => None,
        })
        .collect();
    assert_eq!(xs.len(), 2);
    assert!(xs[1] - xs[0] > 300.0);
    let labels = texts(items);
    assert_eq!(&labels[labels.len() - 2..], ["1", "3"]);
}

#[test]
fn imported_scatter_plots_blank_y_as_zero_when_requested() {
    let group = r#"<c:scatterChart><c:ser>
        <c:xVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>100</c:v></c:pt></c:numLit></c:xVal>
        <c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>4</c:v></c:pt></c:numLit></c:yVal>
        </c:ser></c:scatterChart>"#;
    for (mode, count) in [("gap", 1), ("zero", 2)] {
        let props = format!(r#"<c:dispBlanksAs val="{mode}"/>"#);
        let mut view = SlidesView::open(&chart_deck_with_properties(group, false, &props)).unwrap();
        let p = view.paint(&mut FixedMeasurer::default(), 0);
        let items = chart_items(&p);
        let markers: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Path { x, y, fill: Some(_), .. } => Some((*x, *y)),
                _ => None,
            })
            .collect();
        assert_eq!(markers.len(), count);
        assert_eq!(texts(items).iter().any(|t| t == "100"), mode == "zero");
        if mode == "zero" {
            assert!(markers[1].0 > markers[0].0 && markers[1].1 > markers[0].1);
        }
    }
}

#[test]
fn imported_line_uses_the_saved_blank_cell_mode() {
    let group = format!("<c:lineChart>{CHART_SERIES}</c:lineChart>");
    for (mode, segments) in [("gap", 0), ("span", 1), ("zero", 2)] {
        let props = format!(r#"<c:dispBlanksAs val="{mode}"/>"#);
        let mut view = SlidesView::open(&chart_deck_with_properties(&group, false, &props)).unwrap();
        let p = view.paint(&mut FixedMeasurer::default(), 0);
        let lines: String = chart_items(&p)
            .iter()
            .filter_map(|i| match i {
                Item::Path { d, stroke: Some(_), .. } => Some(d.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(lines.matches('L').count(), segments, "{mode}: {lines}");
    }
}

#[test]
fn unsupported_or_unavailable_charts_keep_the_placeholder() {
    let malformed = CHART_SERIES.replace(r#"idx="2""#, r#"idx="18446744073709551615""#);
    for (groups, external) in [
        ("<c:radarChart/>".to_string(), false),
        (format!(r#"<c:lineChart>{CHART_SERIES}</c:lineChart><c:valAx><c:scaling><c:logBase val="10"/></c:scaling></c:valAx>"#), false),
        (format!(r#"<c:lineChart>{CHART_SERIES}</c:lineChart><c:valAx><c:scaling><c:max val="4"/></c:scaling></c:valAx>"#), false),
        (format!("<c:doughnutChart>{malformed}</c:doughnutChart>"), false),
        (String::new(), true),
    ] {
        let mut view = SlidesView::open(&chart_deck(&groups, external)).unwrap();
        let mut host = FixedMeasurer::default();
        let p = view.paint(&mut host, 0);
        assert!(matches!(chart_items(&p), [Item::Path { dash: "dash", .. }]));
    }
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
fn clearing_a_title_keeps_its_text_style() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::blank();
    let title = v.elements(&mut host, 0).remove(0);
    let style = serde_json::to_value(title.text_style).unwrap();

    assert!(v.begin_edit(&mut host, 2, None));
    v.select_all_text();
    v.backspace(false);
    let empty = v.elements(&mut host, 0).remove(0);
    assert_eq!(empty.text, "");
    assert_eq!(serde_json::to_value(empty.text_style).unwrap(), style);

    v.undo();
    assert_eq!(v.elements(&mut host, 0)[0].text, title.text);
    v.redo();
    assert_eq!(v.elements(&mut host, 0)[0].text, "");

    // The style must survive both typing immediately and reopening a saved, empty title.
    let mut reopened = SlidesView::open(&v.save().unwrap()).unwrap();
    assert!(reopened.begin_edit(&mut host, 2, None));
    for view in [&mut v, &mut reopened] {
        view.insert_text("Epic Games vs Apple");
        let new_title = view.elements(&mut host, 0).remove(0);
        assert_eq!(new_title.text, "Epic Games vs Apple");
        assert_eq!(serde_json::to_value(new_title.text_style).unwrap(), style);
    }
}

#[test]
fn replacing_all_slide_text_keeps_its_text_style() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::blank();
    assert!(v.begin_edit(&mut host, 2, None));
    v.select_all_text();
    v.insert_text("New title");
    let title = v.elements(&mut host, 0).remove(0);
    assert_eq!(title.text, "New title");
    let style = title.text_style.unwrap();
    assert_eq!(style.font_family.as_deref(), Some("Helvetica Neue"));
    assert_eq!(style.font_size, Some(48.0));
    assert!(style.bold);
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

// A rounded rectangle as PowerPoint writes one: its outline comes from the theme through
// `p:style`, and its text is centered by `a:bodyPr`, neither of which the model holds.
const ROUNDED_RECT: &str = r#"<p:sp><p:nvSpPr><p:cNvPr id="9" name="Rounded Rectangle 8"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="2743200"/><a:ext cx="3657600" cy="1828800"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom></p:spPr><p:style><a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></p:style><p:txBody><a:bodyPr rtlCol="0" anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Rounded</a:t></a:r></a:p></p:txBody></p:sp>"#;

/// A deck as if made in PowerPoint: slide 1 has a text box, the rounded rectangle and notes.
fn imported_deck() -> Vec<u8> {
    let mut v = SlidesView::open(&deck(&[&["Intro"], &["Other"]])).unwrap();
    v.set_notes(0, "Remember the demo");
    let mut pkg = office_core::opc::Package::read(&v.save().unwrap()).unwrap();
    let part = "ppt/slides/slide1.xml";
    let xml = String::from_utf8(pkg.part(part).unwrap().to_vec()).unwrap();
    assert!(xml.contains("</p:spTree>"));
    pkg.replace(part, xml.replace("</p:spTree>", &format!("{ROUNDED_RECT}</p:spTree>")).into_bytes()).unwrap();
    pkg.to_bytes().unwrap()
}

fn slide_xml(bytes: &[u8], index: usize) -> String {
    let v = SlidesView::open(bytes).unwrap();
    let part = v.slides()[index].part.clone().unwrap();
    let pkg = office_core::opc::Package::read(bytes).unwrap();
    String::from_utf8(pkg.part(&part).unwrap().to_vec()).unwrap()
}

#[test]
fn duplicating_an_imported_slide_keeps_its_notes() {
    let mut v = SlidesView::open(&imported_deck()).unwrap();
    assert_eq!(v.slide_infos()[0].notes, "Remember the demo");
    v.duplicate_slides(&[0]);
    assert_eq!(v.slides().len(), 3);
    assert_eq!(v.slide_infos()[1].notes, "Remember the demo");
    // The copy's notes are its own.
    v.set_notes(1, "Copy notes");
    assert_eq!(v.slide_infos()[0].notes, "Remember the demo");
    let back = SlidesView::open(&v.save().unwrap()).unwrap();
    let notes: Vec<String> = back.slide_infos().into_iter().map(|s| s.notes).collect();
    assert_eq!(notes, vec!["Remember the demo", "Copy notes", ""]);
    v.undo();
    v.undo();
    assert_eq!(v.slides().len(), 2);
    v.redo();
    assert_eq!(v.slide_infos()[1].notes, "Remember the demo");
}

#[test]
fn duplicated_notes_stay_with_their_copies() {
    // A copy without notes comes before one with them, and must not take its name.
    let mut v = SlidesView::open(&imported_deck()).unwrap();
    v.move_slide(1, 0);
    v.duplicate_slides(&[0, 1]);
    let notes = |v: &SlidesView| v.slide_infos().into_iter().map(|s| s.notes).collect::<Vec<_>>();
    assert_eq!(notes(&v), vec!["", "Remember the demo", "", "Remember the demo"]);
    let back = SlidesView::open(&v.save().unwrap()).unwrap();
    assert_eq!(notes(&back), notes(&v));
}

#[test]
fn a_new_slide_does_not_take_a_deleted_copys_notes() {
    let mut v = SlidesView::open(&imported_deck()).unwrap();
    v.duplicate_slides(&[0]);
    v.delete_slides(&[1]);
    v.add_slide(1, None);
    assert_eq!(v.slide_infos()[2].notes, "");
    let back = SlidesView::open(&v.save().unwrap()).unwrap();
    assert_eq!(back.slide_infos()[2].notes, "");
    v.undo();
    v.undo();
    assert_eq!(v.slide_infos()[1].notes, "Remember the demo");
}

#[test]
fn a_new_slide_saved_under_a_deleted_copys_name_has_no_notes() {
    let mut v = SlidesView::open(&imported_deck()).unwrap();
    v.duplicate_slides(&[0]);
    v.delete_slides(&[1]);
    v.add_slide(1, None);
    // The blank slide's copy is unnamed, so saving gives it the first free part name.
    v.duplicate_slides(&[2]);
    assert_eq!(v.slide_infos()[3].notes, "");
    let back = SlidesView::open(&v.save().unwrap()).unwrap();
    let notes: Vec<String> = back.slide_infos().into_iter().map(|s| s.notes).collect();
    assert_eq!(notes, vec!["Remember the demo", "", "", ""]);
}

#[test]
fn a_duplicated_link_in_notes_points_at_its_url() {
    let mut pkg = office_core::opc::Package::read(&imported_deck()).unwrap();
    let notes_of = |pkg: &office_core::opc::Package, slide: &str| pkg.related(slide, "notesSlide").unwrap()[0].1.clone();
    let notes = notes_of(&pkg, "ppt/slides/slide1.xml");
    let rid = pkg.relate_external(&notes, "hyperlink", "https://example.com/demo").unwrap();
    let xml = String::from_utf8(pkg.part(&notes).unwrap().to_vec()).unwrap();
    let link = format!(
        r#"<a:r><a:rPr lang="en-US"><a:hlinkClick r:id="{rid}"/><a:hlinkMouseOver r:id="{rid}"/></a:rPr><a:t> link</a:t></a:r><a:fld id="{{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}}" type="slidenum"><a:rPr lang="en-US"><a:hlinkClick r:id="{rid}"/></a:rPr><a:t>7</a:t></a:fld></a:p><a:p><a:pPr><a:buBlip><a:blip r:embed="{rid}"/></a:buBlip></a:pPr><a:r><a:t>Pic</a:t></a:r>"#
    );
    assert!(xml.contains("Remember the demo</a:t></a:r>"), "{xml}");
    pkg.replace(&notes, xml.replace("Remember the demo</a:t></a:r>", &format!("Remember the demo</a:t></a:r>{link}")).into_bytes())
        .unwrap();

    let mut v = SlidesView::open(&pkg.to_bytes().unwrap()).unwrap();
    v.duplicate_slides(&[0]);
    let saved = office_core::opc::Package::read(&v.save().unwrap()).unwrap();
    let copy = SlidesView::open(&saved.to_bytes().unwrap()).unwrap().slides()[1].part.clone().unwrap();
    let notes = notes_of(&saved, &copy);
    let xml = String::from_utf8(saved.part(&notes).unwrap().to_vec()).unwrap();
    let rid = xml.split("hlinkClick r:id=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or_else(|| panic!("{xml}"));
    let rels = saved.rels(&notes).unwrap();
    assert_eq!(rels.get(rid).map(|r| r.target.as_str()), Some("https://example.com/demo"));
    // Every relationship id the copy's notes use is its own, and the field's text stays.
    for id in xml.split("r:id=\"").skip(1).map(|r| r.split('"').next().unwrap()) {
        assert!(rels.get(id).is_some(), "{id} in {xml}");
    }
    assert!(xml.contains(">7</a:t></a:fld>"), "{xml}");
    assert!(xml.contains(">Pic<") && !xml.contains("buBlip"), "{xml}");
}

#[test]
fn pasting_an_imported_shape_keeps_its_outline_and_text_anchor() {
    let bytes = imported_deck();
    let mut source = SlidesView::open(&bytes).unwrap();
    let rounded = source.slides()[0].frames.iter().find(|f| f.name == "Rounded Rectangle 8").unwrap().id;
    source.sel.frames = vec![rounded];
    let clipboard = source.copy_frames();
    for same_deck in [true, false] {
        let mut target = if same_deck { SlidesView::open(&bytes).unwrap() } else { SlidesView::open(&deck(&[&[]])).unwrap() };
        let at = if same_deck { 1 } else { 0 };
        target.sel.active = at;
        assert!(target.paste_frames(&clipboard));
        let saved = target.save().unwrap();
        let xml = slide_xml(&saved, at);
        assert!(xml.contains("prst=\"roundRect\""), "{xml}");
        assert!(xml.contains("<a:lnRef idx=\"2\""), "outline: {xml}");
        assert!(xml.contains("anchor=\"ctr\""), "anchor: {xml}");
        assert!(xml.contains(">Rounded<"), "{xml}");
        assert!(!xml.contains("<a:lnRef idx=\"2\" xmlns"), "redeclared namespaces: {xml}");
    }
}

fn deck_with_diagram(cached: bool) -> Vec<u8> {
    office_core::formats::diagram::test_diagram(&deck(&[&["Title"]]), "ppt/slides/slide1.xml", cached, |xml, data| {
        let frame = format!(
            r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="9" name="Diagram 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="914400" y="2286000"/><a:ext cx="6096000" cy="3048000"/></p:xfrm><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{data}</a:graphic></p:graphicFrame></p:spTree>"#
        );
        xml.replacen("</p:spTree>", &frame, 1)
    })
}

#[test]
fn smartart_draws_its_cached_drawing() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck_with_diagram(true)).unwrap();
    let p = v.paint(&mut host, 0);
    assert_eq!(texts(&p.items), vec!["Title", "Cached"]);
    // The diagram's frame holds the cached shape, filled, scaled into the 640 x 320 px frame.
    let Some(Item::Group { items, x, w, .. }) = p.items.iter().find(|i| matches!(i, Item::Group { id: Some(id), .. } if id == "9")) else {
        panic!("{:?}", p.items)
    };
    assert_eq!((*x, *w), (96.0, 640.0));
    assert!(
        matches!(&items[0], Item::Group { w, items, .. } if (*w - 320.0).abs() < 0.01 && items.iter().any(|i| matches!(i, Item::Path { fill: Some(f), .. } if f == "#4472c4"))),
        "{items:?}"
    );
    // Its original parts are saved as they were.
    let saved = v.save().unwrap();
    assert!(office_core::opc::Package::read(&saved).unwrap().has("ppt/diagrams/drawing1.xml"));
}

#[test]
fn smartart_without_a_cached_drawing_shows_its_text() {
    let mut host = FixedMeasurer::default();
    let mut v = SlidesView::open(&deck_with_diagram(false)).unwrap();
    let p = v.paint(&mut host, 0);
    assert_eq!(texts(&p.items), vec!["Title", "Plan", "Scope", "Build"]);
}

#[test]
fn duplicated_smartart_keeps_its_diagram() {
    use office_core::opc::Package;

    for with_notes in [false, true] {
        let mut view = SlidesView::open(&deck_with_diagram(true)).unwrap();
        if with_notes {
            crate::set_notes(&mut view.doc, 0, vec![Paragraph::text("Diagram notes")]);
        }
        let mut host = FixedMeasurer::default();
        view.duplicate_slides(&[0]);
        assert_eq!(texts(&view.paint(&mut host, 1).items), vec!["Title", "Cached"], "notes: {with_notes}");
        let bytes = view.save().unwrap();
        let mut reopened = SlidesView::open(&bytes).unwrap();
        assert_eq!(texts(&reopened.paint(&mut host, 1).items), vec!["Title", "Cached"], "notes: {with_notes}");
        let saved = Package::read(&bytes).unwrap();
        let duplicate = reopened.slides()[1].part.clone().unwrap();
        let data = saved.related(&duplicate, "diagramData").unwrap();
        assert_eq!(data.len(), 1, "notes: {with_notes}");
        // Deleting the original leaves the copy's diagram whole.
        view.delete_slides(&[0]);
        let mut reopened = SlidesView::open(&view.save().unwrap()).unwrap();
        assert_eq!(texts(&reopened.paint(&mut host, 0).items), vec!["Title", "Cached"], "notes: {with_notes}");
    }
}

fn filled_paths(items: &[Item], out: &mut Vec<String>) {
    for i in items {
        match i {
            Item::Path { fill: Some(fill), .. } => out.push(fill.clone()),
            Item::Group { items, .. } => filled_paths(items, out),
            _ => {}
        }
    }
}

#[test]
fn office_smartart_draws_its_cached_shapes() {
    // Apache POI's SmartArt.pptx: five theme-coloured boxes, and a copy recoloured with srgbClr.
    for bytes in [
        &include_bytes!("../../../office-core/src/formats/test_data/poi-SmartArt.pptx")[..],
        &include_bytes!("../../../office-core/src/formats/test_data/poi-SmartArt-srgb-valid.pptx")[..],
    ] {
        let mut host = FixedMeasurer::default();
        let mut v = SlidesView::open(bytes).unwrap();
        let mut fills = Vec::new();
        filled_paths(&v.paint(&mut host, 0).items, &mut fills);
        assert_eq!(fills, vec!["#4472c4"; 5]);
    }
}
