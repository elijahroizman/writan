use office_core::drawing::{Color, Rgb, Xfrm};
use office_core::model::{Body, Frame, FrameContent, Origin, Paragraph, Placeholder as Ph, Slide, TextBody};
use office_core::opc::Package;
use office_core::units::{Emu, FontSize};
use office_core::{Compiler, Document, Kind};

use super::*;

const LAYOUT: &str = "ppt/slideLayouts/slideLayout1.xml";
const MASTER: &str = "ppt/slideMasters/slideMaster1.xml";

fn compiler() -> Compiler {
    Compiler::new().with_layer(Decks)
}

fn round_trip(doc: &Document) -> Document {
    compiler().read(&compiler().write(doc, Kind::Slides).unwrap(), Some("x.pptx")).unwrap()
}

fn placeholder(id: u32, kind: Option<&str>, index: Option<u32>, text: &str) -> Frame {
    Frame {
        id,
        name: format!("Placeholder {id}"),
        description: None,
        hidden: false,
        shape: Default::default(),
        placeholder: Some(Ph { kind: kind.map(str::to_string), index }),
        link: None,
        content: FrameContent::Shape {
            text: Some(TextBody { paragraphs: vec![Paragraph::text(text)], origin: Origin::default() }),
            connector: false,
        },
        origin: Origin::default(),
    }
}

fn deck(slides: Vec<Slide>) -> Document {
    let mut doc = Document::new(Kind::Slides);
    if let Body::Slides { slides: s, .. } = &mut doc.body {
        *s = slides;
    }
    doc
}

fn slide(frames: Vec<Frame>) -> Slide {
    Slide { frames, ..Default::default() }
}

fn slides_of(doc: &Document) -> &[Slide] {
    super::slides(doc)
}

/// A deck written by the core, with `edit` applied to its parts as PowerPoint would have
/// saved them, read back with the layer.
fn edited(doc: &Document, edit: impl Fn(&str, String) -> String) -> Document {
    let mut pkg = Package::read(&office_core::write(doc, Kind::Slides).unwrap()).unwrap();
    for part in [LAYOUT, MASTER, "ppt/slides/slide1.xml"] {
        let text = String::from_utf8(pkg.part(part).unwrap().to_vec()).unwrap();
        let changed = edit(part, text.clone());
        if changed != text {
            pkg.replace(part, changed.into_bytes()).unwrap();
        }
    }
    compiler().read(&pkg.to_bytes().unwrap(), Some("x.pptx")).unwrap()
}

/// A layout with a title at the top and a body (index 1) whose first level is 20 pt.
const PLACEHOLDERS: &str = r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="100" y="200"/><a:ext cx="3000" cy="400"/></a:xfrm></p:spPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="100" y="900"/><a:ext cx="3000" cy="2000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr><a:defRPr sz="2000" b="1"/></a:lvl1pPr></a:lstStyle><a:p/></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="4" name="Footer 3"/><p:cNvSpPr/><p:nvPr><p:ph type="ftr" idx="11"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp>"#;

fn with_placeholders(part: &str, text: String) -> String {
    if part != LAYOUT {
        return text;
    }
    let text = text.replace(r#"type="blank""#, r#"type="obj""#).replace(r#"name="Blank""#, r#"name="Title and Content""#);
    text.replacen("</p:grpSpPr>", &format!("</p:grpSpPr>{PLACEHOLDERS}"), 1).replacen(
        "<p:grpSpPr/>",
        &format!("<p:grpSpPr/>{PLACEHOLDERS}"),
        1,
    )
}

#[test]
fn a_title_takes_the_masters_title_style() {
    let doc = round_trip(&deck(vec![slide(vec![
        placeholder(2, Some("title"), None, "Hello"),
        Frame::text_box(3, Xfrm::default(), vec![Paragraph::text("Loose")]),
    ])]));
    let s = &slides_of(&doc)[0];
    assert_eq!(s.layout.as_deref(), Some(LAYOUT));
    let title = resolve(&doc, s, &s.frames[0]);
    let run = &title.paragraphs[0].1[0];
    assert_eq!(run.size, Some(FontSize(4400)));
    assert_eq!(run.font.as_deref(), Some("Calibri Light"), "the theme's heading font, not +mj-lt");
    let loose = resolve(&doc, s, &s.frames[1]);
    assert_eq!(loose.paragraphs[0].1[0].size, Some(FontSize(1800)), "text outside placeholders takes the default text style");
    assert_eq!(loose.paragraphs[0].1[0].font.as_deref(), Some("Calibri"));
}

#[test]
fn placeholders_take_position_and_levels_from_the_layout() {
    let doc = edited(
        &deck(vec![slide(vec![placeholder(2, Some("title"), None, "Hello"), placeholder(3, None, Some(1), "Point")])]),
        with_placeholders,
    );
    let s = &slides_of(&doc)[0];
    let title = resolve(&doc, s, &s.frames[0]);
    assert_eq!(title.xfrm.as_ref().map(|x| (x.x, x.y, x.width)), Some((Emu(100), Emu(200), Emu(3000))));
    let body = resolve(&doc, s, &s.frames[1]);
    assert_eq!(body.xfrm.as_ref().map(|x| x.y), Some(Emu(900)));
    let run = &body.paragraphs[0].1[0];
    assert_eq!(run.size, Some(FontSize(2000)), "the layout's level overrides the master's body style");
    assert_eq!(run.bold, Some(true));
    assert_eq!(run.font.as_deref(), Some("Calibri"), "the master's body style still supplies the font");

    let l = layout(&doc, "Title and Content").unwrap();
    assert_eq!(layout(&doc, "obj").unwrap().part, l.part);
    assert_eq!(l.placeholders.len(), 3);
}

#[test]
fn slides_added_on_a_layout_get_its_placeholders() {
    let mut doc = edited(&deck(vec![slide(vec![])]), with_placeholders);
    let i = add_slide(&mut doc, LAYOUT).unwrap();
    assert_eq!(i, 1);
    let s = &slides_of(&doc)[1];
    let kinds: Vec<_> = s.frames.iter().map(|f| f.placeholder.clone().unwrap()).collect();
    assert_eq!(kinds, vec![Ph { kind: Some("title".into()), index: None }, Ph { kind: None, index: Some(1) }], "no footer");
    if let FrameContent::Shape { text: Some(body), .. } = &mut doc.body_slides_mut()[1].frames[0].content {
        body.paragraphs = vec![Paragraph::text("Added")];
    }
    let doc = round_trip(&doc);
    let s = &slides_of(&doc)[1];
    assert_eq!(s.layout.as_deref(), Some(LAYOUT));
    assert_eq!(s.part.as_deref(), Some("ppt/slides/slide2.xml"));
    assert_eq!(s.frames[0].paragraphs()[0].plain_text(), "Added");
    assert_eq!(resolve(&doc, s, &s.frames[0]).xfrm.map(|x| x.y), Some(Emu(200)));
}

trait SlidesMut {
    fn body_slides_mut(&mut self) -> &mut Vec<Slide>;
}

impl SlidesMut for Document {
    fn body_slides_mut(&mut self) -> &mut Vec<Slide> {
        match &mut self.body {
            Body::Slides { slides, .. } => slides,
            _ => panic!("not a deck"),
        }
    }
}

#[test]
fn speaker_notes_are_made_read_and_changed() {
    let mut doc = round_trip(&deck(vec![slide(vec![]), slide(vec![])]));
    assert!(notes(&doc, 0).is_empty());
    set_notes(&mut doc, 1, vec![Paragraph::text("Say hello"), Paragraph::text("Then sit down")]);
    let bytes = compiler().write(&doc, Kind::Slides).unwrap();
    let pkg = Package::read(&bytes).unwrap();
    let pres = String::from_utf8(pkg.part("ppt/presentation.xml").unwrap().to_vec()).unwrap();
    assert!(pres.contains("notesMasterIdLst"), "{pres}");
    assert!(pres.find("notesMasterIdLst") < pres.find("sldIdLst"), "the notes master is listed before the slides");
    assert!(pkg.has("ppt/notesMasters/notesMaster1.xml"));

    let mut doc = compiler().read(&bytes, Some("x.pptx")).unwrap();
    let texts = |doc: &Document, i| notes(doc, i).iter().map(|p| p.plain_text()).collect::<Vec<_>>();
    assert_eq!(texts(&doc, 1), ["Say hello", "Then sit down"]);
    assert!(texts(&doc, 0).is_empty());

    set_notes(&mut doc, 1, vec![Paragraph::text("Changed")]);
    let doc = round_trip(&doc);
    assert_eq!(texts(&doc, 1), ["Changed"]);
    let pkg = doc.package.as_ref().unwrap();
    assert_eq!(
        pkg.part_names().filter(|p| p.starts_with("ppt/notesSlides/") && p.ends_with(".xml")).count(),
        1,
        "the same notes slide, rewritten"
    );
    assert_eq!(pkg.part_names().filter(|p| p.starts_with("ppt/notesMasters/") && p.ends_with(".xml")).count(), 1);
}

#[test]
fn unchanged_notes_are_left_alone() {
    let mut doc = round_trip(&deck(vec![slide(vec![])]));
    set_notes(&mut doc, 0, vec![Paragraph::text("Keep")]);
    let first = compiler().write(&doc, Kind::Slides).unwrap();
    let doc = compiler().read(&first, Some("x.pptx")).unwrap();
    let again = compiler().write(&doc, Kind::Slides).unwrap();
    let part = |b: &[u8]| Package::read(b).unwrap().part("ppt/notesSlides/notesSlide1.xml").unwrap().to_vec();
    assert_eq!(part(&first), part(&again));
}

#[test]
fn transitions_round_trip() {
    let mut doc = deck(vec![slide(vec![]), slide(vec![])]);
    let t = Transition { kind: "push".into(), direction: Some("u".into()), duration_ms: Some(1000), on_click: false, after_ms: Some(3000) };
    set_transition(&mut doc.body_slides_mut()[0], Some(&t));
    let mut doc = round_trip(&doc);
    assert_eq!(transition(&slides_of(&doc)[0]), Some(t));
    assert_eq!(transition(&slides_of(&doc)[1]), None);

    set_transition(&mut doc.body_slides_mut()[0], None);
    let doc = round_trip(&doc);
    assert_eq!(transition(&slides_of(&doc)[0]), None);
}

#[test]
fn newer_transitions_and_animations_are_read() {
    let markup = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" Requires="p14"><p:transition spd="slow" p14:dur="1600"><p14:vortex dir="r"/></p:transition></mc:Choice><mc:Fallback><p:transition spd="slow"><p:fade/></p:transition></mc:Fallback></mc:AlternateContent><p:timing><p:tnLst><p:par><p:cTn id="1"><p:childTnLst><p:set><p:cBhvr><p:tgtEl><p:spTgt spid="3"/></p:tgtEl></p:cBhvr></p:set><p:set><p:cBhvr><p:tgtEl><p:spTgt spid="2"/></p:tgtEl></p:cBhvr></p:set></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"#;
    let mut doc =
        edited(&deck(vec![slide(vec![placeholder(2, Some("title"), None, "A"), placeholder(3, None, Some(1), "B")])]), |part, text| {
            if part == "ppt/slides/slide1.xml" {
                text.replacen("</p:sld>", &format!("{markup}</p:sld>"), 1)
            } else {
                text
            }
        });
    let s = &slides_of(&doc)[0];
    let t = transition(s).unwrap();
    assert_eq!((t.kind.as_str(), t.direction.as_deref(), t.duration_ms), ("vortex", Some("r"), Some(1600)));
    assert_eq!(animated_frames(s), [3, 2]);

    clear_animations(&mut doc.body_slides_mut()[0]);
    let doc = round_trip(&doc);
    let s = &slides_of(&doc)[0];
    assert!(animated_frames(s).is_empty());
    assert!(transition(s).is_some(), "the transition stays");
}

#[test]
fn animation_steps_follow_the_main_sequence() {
    // As PowerPoint writes it: title fades in on its own, then a click flies the body in from the
    // bottom with a wipe on the title after it, then a click fades the title out. A shape-click
    // trigger sequence is not a step.
    let effect = |id: u32, class: &str, preset: u32, subtype: u32, node: &str, delay: u32, behaviors: &str| {
        format!(
            r#"<p:par><p:cTn id="{id}" presetID="{preset}" presetClass="{class}" presetSubtype="{subtype}" fill="hold" nodeType="{node}"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst><p:childTnLst>{behaviors}</p:childTnLst></p:cTn></p:par>"#
        )
    };
    let set = |spid: u32| {
        format!(
            r#"<p:set><p:cBhvr><p:cTn id="90" dur="1" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set>"#
        )
    };
    let fade = |spid: u32, dur: u32| {
        format!(
            r#"<p:animEffect transition="in" filter="fade"><p:cBhvr><p:cTn id="91" dur="{dur}"/><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cBhvr></p:animEffect>"#
        )
    };
    let group = |id: u32, delay: &str, inner: &str| {
        format!(
            r#"<p:par><p:cTn id="{id}" fill="hold"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst><p:childTnLst>{inner}</p:childTnLst></p:cTn></p:par>"#
        )
    };
    let step1 = group(3, "indefinite", &group(4, "0", &effect(5, "entr", 10, 0, "afterEffect", 0, &(set(2) + &fade(2, 500)))));
    let step2 = group(
        6,
        "indefinite",
        &(group(7, "0", &effect(8, "entr", 2, 4, "clickEffect", 0, &(set(3) + &fade(3, 500))))
            + &group(9, "500", &effect(10, "emph", 22, 1, "afterEffect", 250, &fade(2, 1000)))),
    );
    let step3 = group(11, "indefinite", &group(12, "0", &effect(13, "exit", 10, 0, "clickEffect", 0, &fade(2, 750))));
    let trigger = format!(
        r#"<p:seq concurrent="1" nextAc="seek"><p:cTn id="14" restart="whenNotActive" fill="hold" nodeType="interactiveSeq"><p:childTnLst>{}</p:childTnLst></p:cTn></p:seq>"#,
        group(15, "indefinite", &effect(16, "entr", 10, 0, "clickEffect", 0, &fade(3, 500)))
    );
    let markup = format!(
        r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{step1}{step2}{step3}</p:childTnLst></p:cTn><p:prevCondLst/><p:nextCondLst/></p:seq>{trigger}</p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"#
    );
    let doc =
        edited(&deck(vec![slide(vec![placeholder(2, Some("title"), None, "A"), placeholder(3, None, Some(1), "B")])]), |part, text| {
            if part == "ppt/slides/slide1.xml" {
                text.replacen("</p:sld>", &format!("{markup}</p:sld>"), 1)
            } else {
                text
            }
        });
    let fx = |frame, class: &str, preset, subtype, delay_ms, duration_ms| Effect {
        frame,
        class: class.into(),
        preset,
        subtype,
        delay_ms,
        duration_ms,
    };
    let steps = animation_steps(&slides_of(&doc)[0]);
    assert_eq!(
        steps,
        [
            AnimationStep { auto: true, effects: vec![fx(2, "entr", 10, 0, 0, 500)] },
            AnimationStep { auto: false, effects: vec![fx(3, "entr", 2, 4, 0, 500), fx(2, "emph", 22, 1, 750, 1000)] },
            AnimationStep { auto: false, effects: vec![fx(2, "exit", 10, 0, 0, 750)] },
        ]
    );
    assert_eq!(steps[1].duration_ms(), 1750);
    assert!(animation_steps(&slides_of(&deck(vec![slide(vec![])]))[0]).is_empty());
}

#[test]
fn theme_colors_go_through_the_color_map() {
    let doc = deck(vec![slide(vec![])]);
    let normal = round_trip(&doc);
    let s = &slides_of(&normal)[0];
    assert_eq!(color(&normal, s, &Color::scheme("tx1")).rgb, Rgb::BLACK);
    assert_eq!(color(&normal, s, &Color::scheme("bg1")).rgb, Rgb::WHITE);

    // A dark master: text is light, the background dark.
    let dark =
        edited(&doc, |part, text| if part == MASTER { text.replace(r#"bg1="lt1" tx1="dk1""#, r#"bg1="dk1" tx1="lt1""#) } else { text });
    let s = &slides_of(&dark)[0];
    assert_eq!(color(&dark, s, &Color::scheme("tx1")).rgb, Rgb::WHITE);
    assert_eq!(color(&dark, s, &Color::scheme("bg1")).rgb, Rgb::BLACK);
}

#[test]
fn without_the_layer_nothing_is_lost() {
    // Notes written with the layer survive a core-only round trip.
    let mut doc = round_trip(&deck(vec![slide(vec![])]));
    set_notes(&mut doc, 0, vec![Paragraph::text("Kept")]);
    let bytes = compiler().write(&doc, Kind::Slides).unwrap();
    let plain = office_core::read(&bytes, Some("x.pptx")).unwrap();
    let doc = compiler().read(&office_core::write(&plain, Kind::Slides).unwrap(), Some("x.pptx")).unwrap();
    assert_eq!(notes(&doc, 0).iter().map(|p| p.plain_text()).collect::<Vec<_>>(), ["Kept"]);
}
