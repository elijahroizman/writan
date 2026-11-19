//! The office compiler's PowerPoint layer: what only presentations have, on top of
//! `office-core`'s slides of frames.
//!
//! - **Layouts and masters** ([`templates`]): what a placeholder frame inherits (its position,
//!   its text's formatting at each outline level) and what the theme's colors map to, put
//!   together in [`resolve`] and [`color`]; adding slides on a layout ([`add_slide`]).
//! - **Speaker notes** ([`notes`], [`set_notes`]).
//! - **Transitions** ([`transition`], [`set_transition`]), which frames are animated, and the
//!   steps the slideshow plays them in ([`animation_steps`]).
//!
//! Add [`Decks`] to a compiler to have it read and write all this.

pub mod templates;
pub mod view;

use office_core::drawing::{Color, Rgba, Theme, Xfrm};
use office_core::formats::common::{PartCtx, PartRels};
use office_core::formats::dml::{read_paragraph, write_paragraph};
use office_core::model::{
    Body, Document, Frame, FrameContent, Inline, Kind, Origin, ParaProps, Paragraph, Placeholder as Ph, RunProps, Slide, TextBody,
};
use office_core::opc::{rels::kind, Package};
use office_core::xml::{ns, Element, Node, XmlDoc};
use office_core::{Layer, Result};
use serde::{Deserialize, Serialize};

pub use templates::{Levels, Template};

/// The layer: add it to a [`office_core::Compiler`].
pub struct Decks;

/// What the layer keeps on a presentation (in its extensions).
#[derive(Clone, Debug, Default)]
pub struct Ppt {
    /// Every master and layout, masters first.
    pub templates: Vec<Template>,
    /// The presentation's `defaultTextStyle`, for text outside placeholders.
    pub default_text: Levels,
    /// Speaker notes by slide part.
    notes: Vec<(String, Vec<Paragraph>)>,
    notes_read: Vec<(String, Vec<Paragraph>)>,
}

const NOTES_SLIDE: &str = "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";
const NOTES_MASTER: &str = "application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml";
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";

impl Layer for Decks {
    fn name(&self) -> &'static str {
        "PowerPoint"
    }

    fn kinds(&self) -> &[Kind] {
        &[Kind::Slides]
    }

    fn after_read(&self, doc: &mut Document) -> Result<()> {
        let mut ppt = Ppt {
            default_text: templates::levels(doc.origin.get().and_then(|o| o.child(ns::P, "defaultTextStyle"))),
            ..Default::default()
        };
        if let Some(pkg) = &doc.package {
            let main = pkg.main_part()?;
            for (_, master) in pkg.related(&main, kind::SLIDE_MASTER)? {
                if !pkg.has(&master) {
                    continue;
                }
                let layouts: Vec<String> =
                    pkg.related(&master, kind::SLIDE_LAYOUT)?.into_iter().map(|(_, l)| l).filter(|l| pkg.has(l)).collect();
                ppt.templates.push(Template::read(pkg, &master)?);
                for l in layouts {
                    ppt.templates.push(Template::read(pkg, &l)?);
                }
            }
            if let Body::Slides { slides, .. } = &doc.body {
                for part in slides.iter().filter_map(|s| s.part.as_deref()) {
                    if let Some(paragraphs) = read_notes(pkg, part)? {
                        ppt.notes.push((part.to_string(), paragraphs));
                    }
                }
            }
        }
        ppt.notes_read = ppt.notes.clone();
        doc.extensions.insert(ppt);
        Ok(())
    }

    fn write_parts(&self, doc: &Document, pkg: &mut Package, _to: Kind) -> Result<()> {
        let Some(ppt) = doc.extensions.get::<Ppt>() else { return Ok(()) };
        // Notes of deleted slides are kept for undo, and a new slide may be written under one's name.
        let live: Vec<&str> = slides(doc).iter().filter_map(|s| s.part.as_deref()).collect();
        for (part, paragraphs) in &ppt.notes {
            if ppt.notes_read.iter().any(|(p, n)| p == part && n == paragraphs) || !pkg.has(part) || !live.contains(&part.as_str()) {
                continue;
            }
            write_notes(pkg, part, paragraphs)?;
        }
        Ok(())
    }
}

/// The layer's data on a presentation, if it was read with the layer.
pub fn ppt(doc: &Document) -> Option<&Ppt> {
    doc.extensions.get::<Ppt>()
}

fn ppt_mut(doc: &mut Document) -> &mut Ppt {
    if doc.extensions.get::<Ppt>().is_none() {
        doc.extensions.insert(Ppt::default());
    }
    doc.extensions.get_mut::<Ppt>().unwrap()
}

fn slides(doc: &Document) -> &[Slide] {
    match &doc.body {
        Body::Slides { slides, .. } => slides,
        _ => &[],
    }
}

fn template<'a>(doc: &'a Document, part: Option<&str>) -> Option<&'a Template> {
    let part = part?;
    ppt(doc)?.templates.iter().find(|t| t.part == part)
}

// ---- layouts ----

/// The presentation's layouts, with their masters' order.
pub fn layouts(doc: &Document) -> Vec<&Template> {
    ppt(doc).map(|p| p.templates.iter().filter(|t| t.layout_type.is_some()).collect()).unwrap_or_default()
}

/// The layout of a type (`title`, `obj`, `twoObj`, `blank`…) or a name ("Title Slide").
pub fn layout<'a>(doc: &'a Document, type_or_name: &str) -> Option<&'a Template> {
    layouts(doc).into_iter().find(|t| t.layout_type.as_deref() == Some(type_or_name) || t.name.eq_ignore_ascii_case(type_or_name))
}

/// Adds a slide on `layout` (a layout's part) after the others, with a frame for each of the
/// layout's placeholders except the date, footer and slide number, and returns its index.
pub fn add_slide(doc: &mut Document, layout: &str) -> Option<usize> {
    let t = template(doc, Some(layout))?.clone();
    let frames = t
        .placeholders
        .iter()
        .filter(|p| !matches!(p.kind.as_str(), "dt" | "ftr" | "sldNum"))
        .enumerate()
        .map(|(i, p)| Frame {
            id: i as u32 + 2,
            name: p.name.clone(),
            description: None,
            hidden: false,
            shape: Default::default(),
            placeholder: Some(Ph { kind: (p.kind != "obj").then(|| p.kind.clone()), index: p.index }),
            link: None,
            content: FrameContent::Shape {
                text: Some(TextBody { paragraphs: vec![Paragraph::default()], origin: Origin::default() }),
                connector: false,
            },
            origin: Origin::default(),
        })
        .collect();
    let part = free_part(doc, &[]);
    let Body::Slides { slides, .. } = &mut doc.body else { return None };
    slides.push(Slide { frames, part: Some(part), layout: Some(layout.to_string()), ..Default::default() });
    Some(slides.len() - 1)
}

/// A slide part name no slide, part in the package, speaker notes or name in `also` has. Notes
/// of a deleted slide stay for undo, so its name isn't reused.
fn free_part(doc: &Document, also: &[String]) -> String {
    let notes = ppt(doc).into_iter().flat_map(|p| p.notes.iter().map(|(s, _)| s.as_str()));
    let taken: Vec<&str> =
        slides(doc).iter().filter_map(|s| s.part.as_deref()).chain(also.iter().map(String::as_str)).chain(notes).collect();
    let in_pkg = |p: &str| doc.package.as_ref().is_some_and(|pkg| pkg.has(p));
    (1..).map(|n| format!("ppt/slides/slide{n}.xml")).find(|p| !taken.contains(&p.as_str()) && !in_pkg(p)).unwrap()
}

// ---- inheritance ----

/// A frame with what it inherits filled in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub xfrm: Option<Xfrm>,
    /// Each paragraph's formatting and its runs' (in [`Paragraph::content`] order, runs only).
    pub paragraphs: Vec<(ParaProps, Vec<RunProps>)>,
}

/// A frame on a slide with its position and text formatting worked out from its layout and
/// master, the way PowerPoint shows it. Theme fonts (`+mj-lt`) become their names.
pub fn resolve(doc: &Document, slide: &Slide, frame: &Frame) -> Resolved {
    let layout = template(doc, slide.layout.as_deref());
    let master =
        template(doc, layout.and_then(|l| l.master.as_deref())).or_else(|| ppt(doc)?.templates.iter().find(|t| t.layout_type.is_none()));
    let ph = frame.placeholder.as_ref().map(|p| (p.kind.clone().unwrap_or_else(|| "obj".into()), p.index));
    let from_layout = ph.as_ref().and_then(|(k, i)| layout?.matching(k, *i));
    let from_master = ph.as_ref().and_then(|(k, _)| master?.matching(&from_layout.map(|p| p.kind.clone()).unwrap_or(k.clone()), None));
    let xfrm =
        frame.shape.xfrm.clone().or_else(|| from_layout.and_then(|p| p.xfrm.clone())).or_else(|| from_master.and_then(|p| p.xfrm.clone()));

    let empty: Levels = vec![Default::default(); 9];
    let base = match (&ph, master) {
        (Some((k, _)), Some(m)) => match templates::family(k) {
            "title" => &m.title,
            "other" => &m.other,
            _ => &m.body,
        },
        (None, _) => ppt(doc).map(|p| &p.default_text).filter(|l| !l.is_empty()).unwrap_or(&empty),
        _ => &empty,
    };
    let theme: Option<&Theme> = master.and_then(|m| m.theme.as_ref()).or(doc.theme.as_ref());
    let own = match &frame.content {
        FrameContent::Shape { text: Some(body), .. } => templates::levels(body.origin.get().and_then(|o| o.child(ns::A, "lstStyle"))),
        _ => empty.clone(),
    };
    let stack: Vec<&Levels> =
        [Some(base), from_master.map(|p| &p.levels), from_layout.map(|p| &p.levels), Some(&own)].into_iter().flatten().collect();
    let paragraphs = frame
        .paragraphs()
        .into_iter()
        .map(|p| {
            let level = p.props.level.unwrap_or(0).min(8) as usize;
            let mut para = ParaProps::default();
            let mut run = RunProps::default();
            for l in &stack {
                if let Some((pp, rp)) = l.get(level) {
                    para.overlay(pp);
                    run.overlay(rp);
                }
            }
            para.overlay(&p.props);
            if let Some(default) = p.origin.get().and_then(|o| o.path(ns::A, &["pPr", "defRPr"])) {
                run.overlay(&office_core::formats::dml::read_run_props(default));
            }
            let runs = p
                .runs()
                .map(|r| {
                    let mut props = run.clone();
                    props.overlay(&r.props);
                    if let Some(theme) = theme {
                        for f in [&mut props.font, &mut props.font_east_asian, &mut props.font_complex] {
                            if let Some(name) = f.as_deref().and_then(|t| theme.font(t)) {
                                *f = Some(name.to_string());
                            }
                        }
                    }
                    props
                })
                .collect();
            (para, runs)
        })
        .collect();
    Resolved { xfrm, paragraphs }
}

/// The color a slide shows `c` as: theme colors through the master's color map (and the
/// layout's and slide's overrides of it), so `tx1` is the theme's dark or light color as the
/// master says.
pub fn color(doc: &Document, slide: &Slide, c: &Color) -> Rgba {
    color_with_override(doc, slide, c, None)
}

fn color_with_override(doc: &Document, slide: &Slide, c: &Color, override_map: Option<&Element>) -> Rgba {
    let layout = template(doc, slide.layout.as_deref());
    let master = template(doc, layout.and_then(|l| l.master.as_deref()));
    let slide_map: Vec<(String, String)> = slide
        .origin
        .get()
        .and_then(|o| o.path(ns::P, &["clrMapOvr"]))
        .and_then(|o| o.child(ns::A, "overrideClrMapping"))
        .map(|m| m.attrs.iter().filter(|a| !a.is_ns_decl()).map(|a| (a.local.clone(), a.value.clone())).collect())
        .unwrap_or_default();
    let use_master =
        slide.origin.get().and_then(|o| o.child(ns::P, "clrMapOvr")).is_some_and(|o| o.child(ns::A, "masterClrMapping").is_some());
    let maps: Vec<&[(String, String)]> = [
        Some(slide_map.as_slice()),
        layout.filter(|_| !use_master).map(|l| l.color_map.as_slice()),
        master.map(|m| m.color_map.as_slice()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let theme = master.and_then(|m| m.theme.as_ref()).or(doc.theme.as_ref());
    c.resolve(&|name| {
        let mapped = override_map
            .and_then(|m| m.attr(name))
            .or_else(|| maps.iter().find_map(|m| m.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())))
            .unwrap_or(name);
        theme?.scheme_color(mapped)
    })
}

// ---- notes ----

fn read_notes(pkg: &Package, slide_part: &str) -> Result<Option<Vec<Paragraph>>> {
    let Some((_, part)) = pkg.related(slide_part, kind::NOTES_SLIDE)?.into_iter().next() else { return Ok(None) };
    if !pkg.has(&part) {
        return Ok(None);
    }
    let xml = pkg.xml(&part)?;
    // Notes whose relationships can't be read still open, without their links.
    let rels = PartRels::new(pkg, &part).ok();
    Ok(Some(
        notes_body(&xml.root).map(|b| b.children_named(ns::A, "p").map(|p| read_paragraph(p, rels.as_ref())).collect()).unwrap_or_default(),
    ))
}

/// The notes placeholder's text body in a notes slide.
fn notes_body(root: &Element) -> Option<&Element> {
    let tree = root.path(ns::P, &["cSld", "spTree"])?;
    tree.children_named(ns::P, "sp")
        .find(|sp| sp.path(ns::P, &["nvSpPr", "nvPr", "ph"]).is_some_and(|ph| ph.attr("type") == Some("body")))?
        .child(ns::P, "txBody")
}

/// A slide's speaker notes.
pub fn notes(doc: &Document, slide_index: usize) -> Vec<Paragraph> {
    let Some(part) = slides(doc).get(slide_index).and_then(|s| s.part.as_deref()) else { return Vec::new() };
    ppt(doc).and_then(|p| p.notes.iter().find(|(s, _)| s == part)).map(|(_, n)| n.clone()).unwrap_or_default()
}

/// Sets a slide's speaker notes. A slide added without a part name gets one, so the notes stay
/// with it however the slides are arranged.
pub fn set_notes(doc: &mut Document, slide_index: usize, paragraphs: Vec<Paragraph>) {
    if slides(doc).get(slide_index).is_none() {
        return;
    }
    if slides(doc)[slide_index].part.is_none() {
        let part = free_part(doc, &[]);
        if let Body::Slides { slides, .. } = &mut doc.body {
            slides[slide_index].part = Some(part);
        }
    }
    let part = slides(doc)[slide_index].part.clone().unwrap();
    put_notes(doc, part, paragraphs);
}

fn put_notes(doc: &mut Document, part: String, paragraphs: Vec<Paragraph>) {
    let ppt = ppt_mut(doc);
    match ppt.notes.iter_mut().find(|(s, _)| *s == part) {
        Some((_, n)) => *n = paragraphs,
        None => ppt.notes.push((part, paragraphs)),
    }
}

/// Part names for copies of slides `indexes`, with the speaker notes of each slide that has
/// them stored under its copy's name. A copy of a slide without notes needs no name, and the
/// copies' notes are their own however either slide's change.
pub fn copy_notes(doc: &mut Document, indexes: &[usize]) -> Vec<Option<String>> {
    let mut named = Vec::new();
    indexes
        .iter()
        .map(|&i| {
            let mut paragraphs = notes(doc, i);
            paragraphs.iter_mut().for_each(drop_part_refs);
            if paragraphs.is_empty() {
                return None;
            }
            let part = free_part(doc, &named);
            named.push(part.clone());
            put_notes(doc, part.clone(), paragraphs);
            Some(part)
        })
        .collect()
}

/// Drops markup in copied notes that names parts by relationship id, which a copy's own notes
/// part doesn't have, keeping the rest. URL links are the model's and are related again when
/// written.
fn drop_part_refs(p: &mut Paragraph) {
    fn strip(o: &mut Origin) {
        let Some(el) = o.get().filter(|el| office_core::formats::common::names_parts(el)) else { return };
        let mut el = el.clone();
        *o = if names_parts_itself(&el) {
            Origin::default()
        } else {
            strip_part_refs(&mut el);
            Origin::new(el)
        };
    }
    strip(&mut p.origin);
    p.content.retain(|i| !matches!(i, Inline::Raw { xml } if names_parts_itself(xml)));
    for i in &mut p.content {
        match i {
            Inline::Text(r) => strip(&mut r.origin),
            Inline::Break { origin, .. } => strip(origin),
            Inline::Raw { xml } => strip_part_refs(xml),
            Inline::Image(_) => {}
        }
    }
}

fn names_parts_itself(el: &Element) -> bool {
    el.attrs.iter().any(|a| !a.is_ns_decl() && a.ns.as_deref().is_some_and(|n| ns::same(n, ns::R)))
}

/// Removes the elements under `el` that name parts by relationship id, and picture bullets,
/// which are invalid without their picture.
fn strip_part_refs(el: &mut Element) {
    el.children.retain(|n| {
        !matches!(n, Node::Element(e) if names_parts_itself(e)
            || (e.is(ns::A, "buBlip") && office_core::formats::common::names_parts(e)))
    });
    el.elements_mut().for_each(strip_part_refs);
}

fn pml(local: &str) -> Element {
    let mut el = Element::new(ns::P, local);
    el.declare(Some("a"), ns::A);
    el.declare(Some("r"), ns::R);
    el.declare(Some("p"), ns::P);
    el
}

fn tree() -> Element {
    Element::new(ns::P, "spTree")
        .with_child(
            Element::new(ns::P, "nvGrpSpPr")
                .with_child(Element::new(ns::P, "cNvPr").with_attr("", "id", "1").with_attr("", "name", ""))
                .with_child(Element::new(ns::P, "cNvGrpSpPr"))
                .with_child(Element::new(ns::P, "nvPr")),
        )
        .with_child(Element::new(ns::P, "grpSpPr"))
}

fn placeholder_sp(id: u32, name: &str, ph: Element) -> Element {
    Element::new(ns::P, "sp")
        .with_child(
            Element::new(ns::P, "nvSpPr")
                .with_child(Element::new(ns::P, "cNvPr").with_attr("", "id", id.to_string()).with_attr("", "name", name))
                .with_child(Element::new(ns::P, "cNvSpPr").with_child(Element::new(ns::A, "spLocks").with_attr("", "noGrp", "1")))
                .with_child(Element::new(ns::P, "nvPr").with_child(ph)),
        )
        .with_child(Element::new(ns::P, "spPr"))
}

/// The presentation's notes master, made (with a theme of its own) if it has none.
fn notes_master(pkg: &mut Package) -> Result<String> {
    let main = pkg.main_part()?;
    if let Some((_, m)) = pkg.related(&main, "notesMaster")?.into_iter().next().filter(|(_, m)| pkg.has(m)) {
        return Ok(m);
    }
    let part = pkg.unused_name("ppt/notesMasters/notesMaster{}.xml");
    let map = [
        "bg1", "lt1", "tx1", "dk1", "bg2", "lt2", "tx2", "dk2", "accent1", "accent1", "accent2", "accent2", "accent3", "accent3",
        "accent4", "accent4", "accent5", "accent5", "accent6", "accent6", "hlink", "hlink", "folHlink", "folHlink",
    ]
    .chunks(2)
    .fold(Element::new(ns::P, "clrMap"), |el, kv| el.with_attr("", kv[0], kv[1]));
    let root = pml("notesMaster").with_child(Element::new(ns::P, "cSld").with_child(tree())).with_child(map);
    pkg.put_xml(&part, &XmlDoc::new(root), NOTES_MASTER);
    let theme = pkg.unused_name("ppt/theme/theme{}.xml");
    pkg.put_xml(&theme, &Theme::office().to_xml(), office_core::opc::content_types::types::THEME);
    pkg.relate(&part, kind::THEME, &theme)?;
    let rid = pkg.relate(&main, "notesMaster", &part)?;
    // The presentation lists its notes master after its slide masters.
    let mut pres = pkg.xml(&main)?;
    pres.root.remove_children(ns::P, "notesMasterIdLst");
    let list = Element::new(ns::P, "notesMasterIdLst").with_child(Element::new(ns::P, "notesMasterId").with_attr(ns::R, "id", rid));
    pres.root.place(list, &[&["sldMasterIdLst"], &["notesMasterIdLst"], &["handoutMasterIdLst"], &["sldIdLst"], &["sldSz"]]);
    pkg.put_xml(&main, &pres, office_core::opc::content_types::types::PPTX_MAIN);
    Ok(part)
}

fn write_notes(pkg: &mut Package, slide_part: &str, paragraphs: &[Paragraph]) -> Result<()> {
    let existing = pkg.related(slide_part, kind::NOTES_SLIDE)?.into_iter().next().map(|(_, p)| p).filter(|p| pkg.has(p));
    let part = match &existing {
        Some(p) => p.clone(),
        None => {
            let master = notes_master(pkg)?;
            let part = pkg.unused_name("ppt/notesSlides/notesSlide{}.xml");
            let image = placeholder_sp(2, "Slide Image Placeholder 1", Element::new(ns::P, "ph").with_attr("", "type", "sldImg"));
            let mut body =
                placeholder_sp(3, "Notes Placeholder 2", Element::new(ns::P, "ph").with_attr("", "type", "body").with_attr("", "idx", "1"));
            body.push(Element::new(ns::P, "txBody").with_child(Element::new(ns::A, "bodyPr")).with_child(Element::new(ns::A, "lstStyle")));
            let root = pml("notes")
                .with_child(Element::new(ns::P, "cSld").with_child(tree().with_child(image).with_child(body)))
                .with_child(Element::new(ns::P, "clrMapOvr").with_child(Element::new(ns::A, "masterClrMapping")));
            pkg.put_xml(&part, &XmlDoc::new(root), NOTES_SLIDE);
            pkg.relate(&part, "notesMaster", &master)?;
            pkg.relate(&part, kind::SLIDE, slide_part)?;
            pkg.relate(slide_part, kind::NOTES_SLIDE, &part)?;
            part
        }
    };
    let mut xml = pkg.xml(&part)?;
    let mut ctx = PartCtx::load(pkg, &part)?;
    let written: Vec<Node> = if paragraphs.is_empty() {
        vec![Node::Element(Element::new(ns::A, "p"))]
    } else {
        paragraphs.iter().map(|p| Node::Element(write_paragraph(p, &mut ctx))).collect()
    };
    let tree = xml.root.child_mut(ns::P, "cSld").and_then(|c| c.child_mut(ns::P, "spTree"));
    if let Some(sp) = tree.and_then(|t| {
        t.elements_mut()
            .find(|sp| sp.is(ns::P, "sp") && sp.path(ns::P, &["nvSpPr", "nvPr", "ph"]).is_some_and(|ph| ph.attr("type") == Some("body")))
    }) {
        let body = sp.child_or_insert(ns::P, "txBody");
        body.remove_children(ns::A, "p");
        body.children.extend(written);
    }
    ctx.finish(pkg);
    pkg.put_xml(&part, &xml, NOTES_SLIDE);
    Ok(())
}

// ---- transitions and animations ----

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    /// `fade`, `push`, `wipe`, `split`, `cover`, `cut`, `dissolve`, `zoom`… (empty: none,
    /// but with the advance settings).
    pub kind: String,
    /// `l`, `r`, `u`, `d`… for the kinds that have a direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// How long it takes, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    /// Whether a click moves to the next slide (PowerPoint's default).
    pub on_click: bool,
    /// Moves on by itself after this many milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_ms: Option<u32>,
}

fn parse_transition(t: &Element) -> Transition {
    let effect = t.elements().find(|e| !e.is(ns::P, "sndAc") && !e.is(ns::P, "extLst"));
    let speed = t.attr("spd").map(|s| match s {
        "slow" => 1000,
        "med" => 750,
        _ => 500,
    });
    Transition {
        kind: effect.map(|e| e.local.clone()).unwrap_or_default(),
        direction: effect.and_then(|e| e.attr("dir")).map(str::to_string),
        duration_ms: t.attr_ns(P14, "dur").and_then(|d| d.parse().ok()).or(speed),
        on_click: t.attr("advClick").and_then(office_core::units::parse_bool).unwrap_or(true),
        after_ms: t.attr("advTm").and_then(|v| v.parse().ok()),
    }
}

/// A slide's transition, if it has one.
pub fn transition(slide: &Slide) -> Option<Transition> {
    let root = slide.origin.get()?;
    if let Some(t) = root.child(ns::P, "transition") {
        return Some(parse_transition(t));
    }
    // Newer transitions come as alternatives; the first PowerPoint can show is the one it uses.
    root.children_named(ns::MC, "AlternateContent")
        .flat_map(|a| a.elements())
        .find_map(|c| c.child(ns::P, "transition"))
        .map(parse_transition)
}

/// Sets (or, for `None`, removes) a slide's transition.
pub fn set_transition(slide: &mut Slide, t: Option<&Transition>) {
    let mut root = slide.origin.get().cloned().filter(|o| o.is(ns::P, "sld")).unwrap_or_else(|| Element::new(ns::P, "sld"));
    let old = root.child(ns::P, "transition").cloned().or_else(|| {
        root.children_named(ns::MC, "AlternateContent").flat_map(|a| a.elements()).find_map(|c| c.child(ns::P, "transition")).cloned()
    });
    let old_fallback = root
        .children_named(ns::MC, "AlternateContent")
        .find_map(|a| a.child(ns::MC, "Fallback").and_then(|f| f.child(ns::P, "transition")))
        .cloned();
    root.remove_children(ns::P, "transition");
    root.children
        .retain(|n| !matches!(n, Node::Element(e) if e.is(ns::MC, "AlternateContent") && e.descendant(ns::P, "transition").is_some()));
    if let Some(t) = t {
        let mut el = Element::new(ns::P, "transition");
        if let Some(d) = t.duration_ms {
            el.set_attr(
                "",
                "spd",
                if d <= 500 {
                    "fast"
                } else if d <= 750 {
                    "med"
                } else {
                    "slow"
                },
            );
        }
        if !t.on_click {
            el.set_attr("", "advClick", "0");
        }
        if let Some(a) = t.after_ms {
            el.set_attr("", "advTm", a.to_string());
        }
        if !t.kind.is_empty() {
            let mut effect = old.as_ref().and_then(|o| o.elements().find(|e| e.local == t.kind)).cloned().unwrap_or_else(|| {
                let namespace = if [
                    "vortex",
                    "ripple",
                    "glitter",
                    "honeycomb",
                    "prism",
                    "shred",
                    "switch",
                    "ferris",
                    "gallery",
                    "doors",
                    "window",
                    "reveal",
                    "flythrough",
                    "warp",
                    "flash",
                ]
                .contains(&t.kind.as_str())
                {
                    P14
                } else {
                    ns::P
                };
                Element::new(namespace, &t.kind)
            });
            effect.remove_attr("", "dir");
            if let Some(d) = &t.direction {
                effect.set_attr("", "dir", d);
            }
            el.push(effect);
        }
        let modern_effect = el.elements().any(|e| !e.in_ns(ns::P));
        let content = if t.duration_ms.is_some() || modern_effect {
            let mut fallback = if modern_effect {
                old_fallback.unwrap_or_else(|| Element::new(ns::P, "transition").with_child(Element::new(ns::P, "fade")))
            } else {
                el.clone()
            };
            fallback.remove_attr(P14, "dur");
            for name in ["spd", "advClick", "advTm"] {
                fallback.remove_attr("", name);
                if let Some(value) = el.attr(name) {
                    fallback.set_attr("", name, value);
                }
            }
            if let Some(duration) = t.duration_ms {
                el.set_attr(P14, "dur", duration.to_string());
            }
            root.declare(Some("p14"), P14);
            root.declare(Some("mc"), ns::MC);
            let mut ignorable: Vec<&str> = root.attr_ns(ns::MC, "Ignorable").unwrap_or("").split_whitespace().collect();
            if !ignorable.contains(&"p14") {
                ignorable.push("p14");
            }
            let ignorable = ignorable.join(" ");
            root.set_attr(ns::MC, "Ignorable", ignorable);
            Element::new(ns::MC, "AlternateContent")
                .with_child(Element::new(ns::MC, "Choice").with_attr("", "Requires", "p14").with_child(el))
                .with_child(Element::new(ns::MC, "Fallback").with_child(fallback))
        } else {
            el
        };
        root.place(content, &[&["cSld"], &["clrMapOvr"], &["transition", "AlternateContent"], &["timing"], &["extLst"]]);
    }
    slide.origin = Origin::new(root);
}

/// The ids of the frames a slide animates, in the order the animations name them.
pub fn animated_frames(slide: &Slide) -> Vec<u32> {
    fn walk(el: &Element, out: &mut Vec<u32>) {
        if el.is(ns::P, "spTgt") {
            if let Some(id) = el.attr("spid").and_then(|i| i.parse().ok()) {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        for e in el.elements() {
            walk(e, out);
        }
    }
    let mut out = Vec::new();
    if let Some(timing) = slide.origin.get().and_then(|o| o.child(ns::P, "timing")) {
        walk(timing, &mut out);
    }
    out
}

/// One click's worth of a slide's animations: the effects a click starts (or, for `auto`, that
/// start by themselves when the previous step, or the slide, has finished).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationStep {
    /// Starts without a click: the step's first effect is "with" or "after previous".
    pub auto: bool,
    pub effects: Vec<Effect>,
}

/// One animation effect on a frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    /// The frame it animates.
    pub frame: u32,
    /// `entr`, `exit`, `emph` or `path` (PowerPoint's `presetClass`).
    pub class: String,
    /// PowerPoint's preset number: 1 Appear, 2 Fly, 10 Fade, 22 Wipe…
    pub preset: u32,
    /// The preset's variant, such as the direction a Fly comes from.
    pub subtype: u32,
    /// When it starts, after the step starts.
    pub delay_ms: u32,
    pub duration_ms: u32,
}

impl AnimationStep {
    /// How long the step takes, from its start to its last effect's end.
    pub fn duration_ms(&self) -> u32 {
        self.effects.iter().map(|e| e.delay_ms + e.duration_ms).max().unwrap_or(0)
    }
}

fn cond_delay(ctn: &Element) -> u32 {
    ctn.child(ns::P, "stCondLst")
        .into_iter()
        .flat_map(|l| l.children_named(ns::P, "cond"))
        .find_map(|c| c.attr("delay").and_then(|d| d.parse::<u32>().ok()))
        .unwrap_or(0)
}

/// The time nodes under a time node: the `cTn` of each `par`, `seq` or behavior below it.
fn child_ctns(ctn: &Element) -> impl Iterator<Item = &Element> {
    ctn.child(ns::P, "childTnLst")
        .into_iter()
        .flat_map(|l| l.elements())
        .filter_map(|e| e.child(ns::P, "cTn").or_else(|| e.child(ns::P, "cBhvr").and_then(|b| b.child(ns::P, "cTn"))))
}

fn effect(ctn: &Element, class: &str, delay_ms: u32) -> Option<Effect> {
    let frame = ctn.descendant(ns::P, "spTgt")?.attr("spid")?.parse().ok()?;
    let duration_ms = child_ctns(ctn).filter_map(|b| Some(cond_delay(b) + b.attr("dur")?.parse::<u32>().ok()?)).max().unwrap_or(0);
    let number = |name: &str| ctn.attr(name).and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(Effect {
        frame,
        class: class.to_string(),
        preset: number("presetID"),
        subtype: number("presetSubtype"),
        delay_ms: delay_ms + cond_delay(ctn),
        duration_ms,
    })
}

/// The effects under a time node, in order, with their start after `offset`.
fn effects(ctn: &Element, offset: u32, out: &mut Vec<(Effect, Option<String>)>) {
    for c in child_ctns(ctn) {
        if let Some(class) = c.attr("presetClass") {
            out.extend(effect(c, class, offset).map(|e| (e, c.attr("nodeType").map(str::to_string))));
        } else {
            effects(c, offset + cond_delay(c), out);
        }
    }
}

/// The animations a slide plays as it's presented, one step per click, from its main sequence.
/// Animations a click on a shape triggers are left out.
pub fn animation_steps(slide: &Slide) -> Vec<AnimationStep> {
    fn main_seq(el: &Element) -> Option<&Element> {
        if el.is(ns::P, "seq") {
            if let Some(ctn) = el.child(ns::P, "cTn").filter(|c| c.attr("nodeType") == Some("mainSeq")) {
                return Some(ctn);
            }
        }
        el.elements().find_map(main_seq)
    }
    let Some(seq) = slide.origin.get().and_then(|o| o.child(ns::P, "timing")).and_then(main_seq) else { return Vec::new() };
    child_ctns(seq)
        .filter_map(|group| {
            let mut found = Vec::new();
            effects(group, 0, &mut found);
            let first = found.first()?.1.clone();
            let auto = match first.as_deref() {
                Some("clickEffect") => false,
                Some(_) => true,
                None => group
                    .child(ns::P, "stCondLst")
                    .is_some_and(|l| l.children_named(ns::P, "cond").all(|c| c.attr("delay") != Some("indefinite"))),
            };
            Some(AnimationStep { auto, effects: found.into_iter().map(|(e, _)| e).collect() })
        })
        .collect()
}

/// Removes a slide's animations.
pub fn clear_animations(slide: &mut Slide) {
    if let Some(mut root) = slide.origin.get().cloned() {
        if root.remove_children(ns::P, "timing") > 0 {
            slide.origin = Origin::new(root);
        }
    }
}

#[cfg(test)]
mod tests;
