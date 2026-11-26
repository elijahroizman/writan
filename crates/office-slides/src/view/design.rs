use std::collections::HashMap;

use office_core::drawing::Theme;
use office_core::model::{Body, Frame, FrameContent, Origin, Placeholder, Slide, SlideSize};
use office_core::opc::{names, rels::kind, Package};
use office_core::xml::{ns, Element};
use office_core::{Error, Result};

use super::{art::ArtSet, templates, SStep, SlidesView};
use crate::Template;

const THEMES: &[(&str, &[u8])] = &[
    ("Harbor", include_bytes!("../../assets/themes/Harbor.pptx")),
    ("Summit", include_bytes!("../../assets/themes/Summit.pptx")),
    ("Grove", include_bytes!("../../assets/themes/Grove.pptx")),
    ("Copper", include_bytes!("../../assets/themes/Copper.pptx")),
    ("Graphite", include_bytes!("../../assets/themes/Graphite.pptx")),
    ("Meridian", include_bytes!("../../assets/themes/Meridian.pptx")),
    ("Standard", include_bytes!("../../assets/themes/Standard.pptx")),
    ("Gallery", include_bytes!("../../assets/themes/Gallery.pptx")),
    ("Dune", include_bytes!("../../assets/themes/Dune.pptx")),
];

#[derive(Clone, Debug)]
pub struct Design {
    package: Option<Package>,
    origin: Origin,
    theme: Option<Theme>,
    templates: Vec<Template>,
    slides: Vec<Slide>,
}

impl SlidesView {
    pub fn theme_names() -> Vec<&'static str> {
        std::iter::once("Default").chain(THEMES.iter().map(|(name, _)| *name)).collect()
    }

    pub fn can_apply_theme(&self) -> bool {
        self.replica.is_none() && self.doc.package.is_some()
    }

    fn design(&self) -> Design {
        Design {
            package: self.doc.package.clone(),
            origin: self.doc.origin.clone(),
            theme: self.doc.theme.clone(),
            templates: crate::ppt(&self.doc).map(|p| p.templates.clone()).unwrap_or_default(),
            slides: self.slides().to_vec(),
        }
    }

    pub(super) fn put_design(&mut self, design: &Design) {
        self.doc.package = design.package.clone();
        self.doc.origin = design.origin.clone();
        self.doc.theme = design.theme.clone();
        crate::ppt_mut(&mut self.doc).templates = design.templates.clone();
        *self.slides_mut() = design.slides.clone();
        self.art = ArtSet::read(&self.doc);
        self.cache.clear();
        self.touch_all();
    }

    pub fn apply_theme(&mut self, name: &str) -> Result<bool> {
        if !self.can_apply_theme() {
            return Ok(false);
        }
        let default = name == "Default";
        let donor = if default {
            SlidesView::blank().doc.package.unwrap()
        } else {
            let bytes = THEMES.iter().find(|(n, _)| *n == name).ok_or_else(|| Error::Invalid(format!("unknown theme: {name}")))?.1;
            Package::read(bytes)?
        };
        let before = self.design();
        let mut after = before.clone();
        let package = after.package.as_mut().unwrap();
        let main = package.main_part()?;
        let source_main = donor.main_part()?;
        let source_master =
            donor.related(&source_main, kind::SLIDE_MASTER)?.into_iter().next().ok_or_else(|| Error::MissingPart("theme master".into()))?.1;
        let Body::Slides { size, .. } = &self.doc.body else { return Ok(false) };
        let reference = SlideSize::default();
        let scale = (size.width.0 as f64 / reference.width.0 as f64, size.height.0 as f64 / reference.height.0 as f64);
        let slug = name.to_ascii_lowercase();
        let folder = format!("ppt/writan-design/{slug}");
        let mut copied = HashMap::new();
        let master = copy_part(&donor, &source_master, package, &folder, scale, &mut copied)?;
        if package.related(&main, kind::SLIDE_MASTER)?.first().is_some_and(|(_, current)| current == &master) {
            return Ok(false);
        }
        let mut rels = package.rels(&main)?;
        rels.0.retain(|r| !r.is(kind::SLIDE_MASTER) && !r.is(kind::THEME));
        package.put_rels(&main, &rels);
        let rid = package.relate(&main, kind::SLIDE_MASTER, &master)?;
        if let Some((_, theme)) = package.related(&master, kind::THEME)?.into_iter().next() {
            package.relate(&main, kind::THEME, &theme)?;
        }
        let mut pres = self.doc.origin.get().cloned().unwrap_or(package.xml(&main)?.root);
        *pres.child_or_insert(ns::P, "sldMasterIdLst") = Element::new(ns::P, "sldMasterIdLst")
            .with_child(Element::new(ns::P, "sldMasterId").with_attr("", "id", "2147483648").with_attr(ns::R, "id", rid));
        after.origin = Origin::new(pres);
        let master = Template::read(package, &master)?;
        after.theme = master.theme.clone();
        let layouts: Vec<Template> = package
            .related(&master.part, kind::SLIDE_LAYOUT)?
            .into_iter()
            .map(|(_, part)| Template::read(package, &part))
            .collect::<Result<_>>()?;
        let blank = layouts
            .iter()
            .find(|l| l.layout_type.as_deref() == Some("blank"))
            .or_else(|| layouts.first())
            .ok_or_else(|| Error::MissingPart("theme layout".into()))?;
        for (slide, original) in after.slides.iter_mut().zip(&before.slides) {
            let old = before.templates.iter().find(|t| Some(t.part.as_str()) == original.layout.as_deref());
            let plain = plain_frames(original, old, scale);
            let layout = plain
                .as_ref()
                .and_then(|(name, _)| layouts.iter().find(|l| l.name == *name))
                .or_else(|| old.and_then(|old| layouts.iter().find(|l| l.name.eq_ignore_ascii_case(&old.name))))
                .or_else(|| old.and_then(|old| layouts.iter().find(|l| l.layout_type == old.layout_type)))
                .unwrap_or(blank);
            slide.layout = Some(layout.part.clone());
            if let Some(origin) = slide.origin.get() {
                let mut origin = origin.clone();
                origin.remove_attr("", "showMasterSp");
                origin.remove_children(ns::P, "clrMapOvr");
                if let Some(c) = origin.child_mut(ns::P, "cSld") {
                    if c.child(ns::P, "bg")
                        .and_then(|b| b.path(ns::P, &["bgPr"]))
                        .and_then(|b| b.path(ns::A, &["solidFill", "schemeClr"]))
                        .and_then(|color| color.attr("val"))
                        == Some("bg1")
                    {
                        c.remove_children(ns::P, "bg");
                    }
                }
                slide.origin = Origin::new(origin);
            }
            for (frame, previous) in slide.frames.iter_mut().zip(&original.frames) {
                let plain_frame = plain.as_ref().and_then(|(_, frames)| frames.iter().find(|f| f.name == frame.name));
                let ph = previous.placeholder.as_ref();
                let target = if let Some(ph) = ph {
                    layout.matching(ph.kind.as_deref().unwrap_or("obj"), ph.index)
                } else if plain_frame.is_some() {
                    layout
                        .placeholders
                        .iter()
                        .find(|p| (p.index.unwrap_or(0) == 0 && frame.name == "Title") || (p.index == Some(1) && frame.name != "Title"))
                } else {
                    continue;
                };
                if let Some(p) = target {
                    frame.placeholder = Some(Placeholder { kind: (p.kind != "obj").then(|| p.kind.clone()), index: p.index });
                    if let Some(plain) = plain_frame {
                        if frame.shape.xfrm == plain.shape.xfrm {
                            frame.shape.xfrm = None;
                        }
                        inherit_plain_text(frame, plain);
                    }
                } else if let Some(ph) = ph {
                    frame.shape.xfrm = crate::resolve(&self.doc, original, previous).xfrm;
                    frame.placeholder = None;
                    if default {
                        restore_plain_frame(frame, old, ph, scale, previous.shape.xfrm.is_some());
                    }
                }
            }
        }
        after.templates = std::iter::once(master).chain(layouts).collect();
        let sel = self.sel.clone();
        self.put_design(&after);
        self.record(vec![SStep::Design { before: Box::new(before), after: Box::new(after) }], sel, None);
        Ok(true)
    }
}

fn plain_frames(slide: &Slide, layout: Option<&Template>, (sx, sy): (f64, f64)) -> Option<(&'static str, Vec<Frame>)> {
    if layout.is_some_and(|l| l.layout_type.as_deref() != Some("blank"))
        || !slide.frames.iter().any(|f| f.name == "Title" && f.placeholder.is_none())
    {
        return None;
    }
    for (name, frames) in [("Title Slide", templates::plain_title(sx, sy)), ("Title and Content", templates::plain_content(sx, sy))] {
        if slide.frames.iter().any(|f| f.name == frames[1].name && f.placeholder.is_none()) {
            return Some((name, frames));
        }
    }
    None
}

fn inherit_plain_text(frame: &mut Frame, plain: &Frame) {
    let FrameContent::Shape { text: Some(body), .. } = &mut frame.content else { return };
    let Some(props) = plain.paragraphs()[0].runs().next().map(|r| &r.props) else { return };
    for p in &mut body.paragraphs {
        for r in p.runs_mut() {
            let target = &mut r.props;
            if target.font == props.font {
                target.font = None;
            }
            if target.size == props.size {
                target.size = None;
            }
            if target.color == props.color {
                target.color = None;
            }
            if target.bold == props.bold {
                target.bold = None;
            }
        }
    }
    if let Some(origin) = frame.origin.get() {
        let mut origin = origin.clone();
        if let Some(nv) = origin.child_mut(ns::P, "nvSpPr").and_then(|n| n.child_mut(ns::P, "cNvSpPr")) {
            nv.remove_attr("", "txBox");
        }
        frame.origin = Origin::new(origin);
    }
}

fn restore_plain_frame(frame: &mut Frame, layout: Option<&Template>, ph: &Placeholder, (sx, sy): (f64, f64), moved: bool) {
    let frames = match layout.map(|l| l.name.as_str()) {
        Some("Title Slide") => templates::plain_title(sx, sy),
        Some("Title and Content") => templates::plain_content(sx, sy),
        _ => return,
    };
    let Some(plain) = frames.get(ph.index.unwrap_or(0) as usize) else { return };
    let FrameContent::Shape { text: Some(body), .. } = &mut frame.content else { return };
    if let Some(props) = plain.paragraphs()[0].runs().next().map(|r| &r.props) {
        for p in &mut body.paragraphs {
            for r in p.runs_mut() {
                let mut restored = props.clone();
                restored.overlay(&r.props);
                r.props = restored;
            }
        }
    }
    frame.name = plain.name.clone();
    if !moved {
        frame.shape.xfrm = plain.shape.xfrm.clone();
    }
}

fn copy_part(
    source: &Package,
    part: &str,
    target: &mut Package,
    folder: &str,
    scale: (f64, f64),
    copied: &mut HashMap<String, String>,
) -> Result<String> {
    if let Some(found) = copied.get(part) {
        return Ok(found.clone());
    }
    let dest = format!("{folder}/{part}");
    copied.insert(part.to_string(), dest.clone());
    let content_type = source.content_type(part).ok_or_else(|| Error::MissingPart(part.into()))?;
    if scale != (1.0, 1.0) && (content_type.ends_with(".slideMaster+xml") || content_type.ends_with(".slideLayout+xml")) {
        let mut xml = source.xml(part)?;
        scale_art(&mut xml.root, scale);
        target.put_xml(&dest, &xml, content_type);
    } else {
        target.put(&dest, source.part(part)?.to_vec(), content_type);
    }
    let mut rels = source.rels(part)?;
    for rel in rels.0.iter_mut() {
        if let Some(next) = source.target(part, rel) {
            let new = copy_part(source, &next, target, folder, scale, copied)?;
            rel.target = names::relative(&dest, &new);
        }
    }
    target.put_rels(&dest, &rels);
    Ok(dest)
}

fn scale_art(el: &mut Element, (sx, sy): (f64, f64)) {
    if el.is(ns::A, "xfrm") || el.is(ns::P, "xfrm") {
        for (tag, attrs) in [("off", ["x", "y"]), ("ext", ["cx", "cy"]), ("chOff", ["x", "y"]), ("chExt", ["cx", "cy"])] {
            if let Some(child) = el.child_mut(ns::A, tag) {
                for (attr, factor) in attrs.into_iter().zip([sx, sy]) {
                    if let Some(n) = child.attr(attr).and_then(|v| v.parse::<f64>().ok()) {
                        child.set_attr("", attr, ((n * factor).round() as i64).to_string());
                    }
                }
            }
        }
    }
    el.elements_mut().for_each(|child| scale_art(child, (sx, sy)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUTS: [&str; 9] =
        ["Title and Content", "Title Slide", "Section Header", "Two Content", "Comparison", "Title Only", "Quote", "Closing", "Blank"];

    fn content(view: &SlidesView) -> Vec<Vec<String>> {
        view.slides().iter().map(|slide| slide.frames.iter().flat_map(Frame::paragraphs).map(|p| p.plain_text()).collect()).collect()
    }

    #[test]
    fn new_text_boxes_use_light_theme_ink_on_dark_layouts() {
        use office_core::drawing::{Color, Rgb};

        for (theme, dark_layout, light_layout) in
            [("Graphite", "Blank", None), ("Gallery", "Title Slide", Some("Blank")), ("Summit", "Title Slide", Some("Blank"))]
        {
            let mut view = SlidesView::blank();
            view.apply_theme(theme).unwrap();
            for (layout, light_ink) in std::iter::once((dark_layout, true)).chain(light_layout.map(|name| (name, false))) {
                let layout = crate::layout(&view.doc, layout).unwrap().part.clone();
                view.add_slide(1, Some(&layout));
                view.insert_text_box();
                view.insert_text("Sample");
                let slide = &view.slides()[view.sel.active];
                let frame = slide.frames.last().unwrap();
                let inherited = &crate::resolve(&view.doc, slide, frame).paragraphs[0].1[0].color;
                let ink = crate::color(&view.doc, slide, inherited.as_ref().unwrap_or(&Color::scheme("tx1"))).rgb;
                assert_eq!(ink == Rgb(255, 255, 255), light_ink, "{theme}: {layout}");
            }
        }
    }

    #[test]
    fn themes_switch_the_master_and_round_trip_without_importing_sample_slides() {
        let mut view = SlidesView::blank();
        let original = content(&view);
        let plain = view.slides()[0].frames[0].shape.xfrm.clone();
        view.set_notes(1, "Keep these notes");
        view.toggle_hidden(&[1]);
        for (name, _) in THEMES {
            assert!(view.apply_theme(name).unwrap());
            assert_eq!(view.doc.theme.as_ref().unwrap().name, *name);
            assert_eq!(crate::layouts(&view.doc).iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), LAYOUTS);
            assert_eq!(crate::template(&view.doc, view.slides()[0].layout.as_deref()).unwrap().name, "Title Slide");
            assert_eq!(crate::template(&view.doc, view.slides()[1].layout.as_deref()).unwrap().name, "Title and Content");
            assert!(view.slides()[0].frames[0].shape.xfrm.is_none());
            assert_ne!(crate::resolve(&view.doc, &view.slides()[0], &view.slides()[0].frames[0]).xfrm, plain);
            assert_eq!(content(&view), original);
            view = SlidesView::open(&view.save().unwrap()).unwrap();
            assert_eq!(view.doc.theme.as_ref().unwrap().name, *name);
            assert_eq!(content(&view), original);
            assert!(view.slides()[1].hidden);
            assert_eq!(crate::notes(&view.doc, 1)[0].plain_text(), "Keep these notes");
        }
        assert!(view.apply_theme("Default").unwrap());
        assert_eq!(crate::layouts(&view.doc).len(), 1);
        assert_eq!(view.slides()[0].frames[0].shape.xfrm, plain);
        assert_eq!(content(&view), original);
        assert!(view.undo());
        assert_eq!(view.doc.theme.as_ref().unwrap().name, "Dune");
        assert!(view.redo());
        assert_eq!(crate::layouts(&view.doc).len(), 1);
    }

    #[test]
    fn named_custom_layouts_with_the_same_powerpoint_type_map_by_name() {
        let mut view = SlidesView::open(THEMES[0].1).unwrap();
        let original = content(&view);
        let names = |v: &SlidesView| {
            v.slides().iter().map(|s| crate::template(&v.doc, s.layout.as_deref()).unwrap().name.clone()).collect::<Vec<_>>()
        };
        let before = names(&view);
        assert!(before.contains(&"Quote".into()) && before.contains(&"Closing".into()));
        assert!(view.apply_theme("Graphite").unwrap());
        assert_eq!(names(&view), before);
        assert_eq!(content(&view), original);
    }
}
