use crate::drawing::Theme;
use crate::formats::common::{origin_as, put_if_changed, rel_attr, Out, PartCtx};
use crate::formats::dml::{write_paragraph, write_text_body};
use crate::model::{Block, Body, Document, Frame, FrameContent, Image, Kind, Placeholder, Slide, SlideSize, Table};
use crate::opc::content_types::types;
use crate::opc::{rels::kind, Package};
use crate::xml::{ns, Element, XmlDoc};
use crate::{Error, Result};

const TABLE_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/table";
const PRES_PROPS: &str = "application/vnd.openxmlformats-officedocument.presentationml.presProps+xml";
const VIEW_PROPS: &str = "application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml";
const TABLE_STYLES: &str = "application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml";
/// PowerPoint's default table style, "Medium Style 2 – Accent 1".
const DEFAULT_TABLE_STYLE: &str = "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}";

pub fn write(doc: &Document) -> Result<Package> {
    let Body::Slides { size, slides } = &doc.body else {
        return Err(Error::Invalid("only a presentation can be saved as .pptx".into()));
    };
    let same = doc.kind == Kind::Slides && doc.package.is_some();
    let pkg = if same { doc.package.clone().unwrap() } else { scaffold(doc.theme.as_ref().unwrap_or(&Theme::office()), *size) };
    let mut out = Out::new(doc, pkg, same, "ppt/media");
    let main = out.pkg.main_part()?;
    let mut pres = out.pkg.xml(&main)?;
    if let Some(origin) = origin_as(&doc.origin, ns::P, "presentation").filter(|_| same) {
        pres.root = origin;
    }
    let mut pres_ctx = PartCtx::load(&out.pkg, &main)?;

    // Slides that are gone, and their notes.
    let kept: Vec<&str> = slides.iter().filter_map(|s| s.part.as_deref()).collect();
    for rel in pres_ctx.rels.of_kind(kind::SLIDE).cloned().collect::<Vec<_>>() {
        let Some(part) = out.pkg.target(&main, &rel) else { continue };
        if same && kept.contains(&part.as_str()) {
            continue;
        }
        for (_, notes) in out.pkg.related(&part, kind::NOTES_SLIDE)? {
            out.pkg.remove(&notes);
        }
        out.pkg.remove(&part);
        pres_ctx.rels.remove(&rel.id);
    }

    let old_ids = pres.root.child(ns::P, "sldIdLst").cloned();
    let mut used_ids: std::collections::HashSet<u32> =
        old_ids.iter().flat_map(|l| l.children_named(ns::P, "sldId")).filter_map(|s| s.attr("id")?.parse().ok()).collect();
    let mut next_id = 256u32;
    let mut list = Element::new(ns::P, "sldIdLst");
    let mut layout = first_layout(&out.pkg, &main)?;
    let mut written: Vec<String> = Vec::new();
    // Names new slides ask for, which a new slide before them mustn't take.
    let requested: Vec<&str> = slides.iter().filter_map(|s| s.part.as_deref()).filter(|p| !out.pkg.has(p)).collect();
    for slide in slides {
        // The slide's own layout, if it names one this package has.
        let wanted = slide.layout.clone().filter(|l| same && out.pkg.has(l));
        // A slide copied in the model has its original's part too; the first one keeps it.
        let copy_of = slide.part.as_deref().filter(|p| same && written.iter().any(|w| w == p));
        let mut removed_jumps = std::collections::HashSet::new();
        let part = match slide.part.as_deref().filter(|p| same && out.pkg.has(p) && copy_of.is_none()) {
            Some(p) => {
                let current = out.pkg.related(p, kind::SLIDE_LAYOUT)?.into_iter().next();
                if let (Some(w), Some((rel, now))) = (&wanted, &current) {
                    // Moved to another layout: point the slide's relationship at it.
                    if w != now {
                        let mut rels = out.pkg.rels(p)?;
                        if let Some(r) = rels.0.iter_mut().find(|r| r.id == rel.id) {
                            r.target = crate::opc::names::relative(p, w);
                        }
                        out.pkg.put_rels(p, &rels);
                    }
                }
                // New slides go on the layout of the slide before them.
                layout = wanted.or(current.map(|(_, l)| l)).or(layout);
                p.to_string()
            }
            None => {
                // A new slide can say what its part is to be called (the PowerPoint layer does,
                // to keep speaker notes with it), if the name is free.
                let named = slide.part.clone().filter(|p| !out.pkg.has(p) && p.starts_with("ppt/slides/") && p.ends_with(".xml"));
                let p = named.unwrap_or_else(|| {
                    (1..).map(|n| format!("ppt/slides/slide{n}.xml")).find(|p| !out.pkg.has(p) && !requested.contains(&p.as_str())).unwrap()
                });
                out.pkg.put_xml(&p, &XmlDoc::new(Element::new(ns::P, "sld")), types::PPTX_SLIDE);
                match slide
                    .relationship_source
                    .as_deref()
                    .or(copy_of)
                    .zip(doc.package.as_ref())
                    .filter(|(part, input)| same && input.has(part))
                {
                    // Copy from the original package even if the source slide was deleted.
                    Some((original, input)) => {
                        let mut rels = input.rels(original)?;
                        rels.0.retain(|r| {
                            if r.is(kind::SLIDE)
                                && !input
                                    .target(original, r)
                                    .is_some_and(|target| input.has(&target) && kept.iter().any(|p| p.eq_ignore_ascii_case(&target)))
                            {
                                removed_jumps.insert(r.id.clone());
                                return false;
                            }
                            !r.is(kind::NOTES_SLIDE) && !r.is("comments")
                        });
                        for r in rels.0.iter_mut() {
                            use crate::opc::names::{folder, relative};
                            if let Some(w) = wanted.as_deref().filter(|_| r.is(kind::SLIDE_LAYOUT)) {
                                r.target = relative(&p, w);
                            } else if folder(original) != folder(&p) {
                                if let Some(target) = input.target(original, r) {
                                    let suffix = &r.target[r.target.find(['#', '?']).unwrap_or(r.target.len())..];
                                    r.target = relative(&p, &target) + suffix;
                                }
                            }
                        }
                        out.pkg.put_rels(&p, &rels);
                    }
                    None => {
                        let layout = wanted.or_else(|| layout.clone()).ok_or_else(|| Error::MissingPart("a slide layout".into()))?;
                        out.pkg.relate(&p, kind::SLIDE_LAYOUT, &layout)?;
                    }
                }
                p
            }
        };
        write_slide(slide, &part, &mut out, &removed_jumps)?;
        let rid = pres_ctx.relate(kind::SLIDE, &part);
        written.push(part);
        let existing =
            old_ids.iter().flat_map(|l| l.children_named(ns::P, "sldId")).find(|s| rel_attr(s, "id") == Some(rid.as_str())).cloned();
        list.push(if let Some(existing) = existing {
            existing
        } else {
            while used_ids.contains(&next_id) {
                next_id = next_id
                    .checked_add(1)
                    .filter(|&id| id <= i32::MAX as u32)
                    .ok_or_else(|| Error::Invalid("no slide ids remain".into()))?;
            }
            let id = next_id;
            used_ids.insert(id);
            Element::new(ns::P, "sldId").with_attr("", "id", id.to_string()).with_attr(ns::R, "id", rid.clone())
        });
    }
    const PRESENTATION: &[&[&str]] = &[
        &["sldMasterIdLst"],
        &["notesMasterIdLst"],
        &["handoutMasterIdLst"],
        &["sldIdLst"],
        &["sldSz"],
        &["notesSz"],
        &["smartTags"],
        &["embeddedFontLst"],
        &["custShowLst"],
        &["photoAlbum"],
        &["custDataLst"],
        &["kinsoku"],
        &["defaultTextStyle"],
        &["modifyVerifier"],
        &["extLst"],
    ];
    if slides.is_empty() {
        pres.root.remove_children(ns::P, "sldIdLst");
    } else {
        pres.root.place(list, PRESENTATION);
    }
    let sz = pres.root.child_or_insert(ns::P, "sldSz");
    let before = (sz.attr("cx").and_then(|v| v.parse().ok()), sz.attr("cy").and_then(|v| v.parse().ok()));
    if before != (Some(size.width.0), Some(size.height.0)) {
        sz.set_attr("", "cx", size.width.0.to_string());
        sz.set_attr("", "cy", size.height.0.to_string());
        // The type names a standard size; a changed size is a custom one.
        sz.remove_attr("", "type");
    }
    // sldSz may have been made at the end; put it where it goes.
    if let Some(i) = pres.root.children.iter().position(|n| matches!(n, crate::xml::Node::Element(e) if e.is(ns::P, "sldSz"))) {
        if let crate::xml::Node::Element(e) = pres.root.children.remove(i) {
            pres.root.place(e, PRESENTATION);
        }
    }
    put_if_changed(&mut out.pkg, &main, pres, types::PPTX_MAIN)?;
    pres_ctx.finish(&mut out.pkg);
    doc.properties.write(&mut out.pkg)?;
    Ok(out.pkg)
}

fn first_layout(pkg: &Package, main: &str) -> Result<Option<String>> {
    for (_, master) in pkg.related(main, kind::SLIDE_MASTER)? {
        if let Some((_, layout)) = pkg.related(&master, kind::SLIDE_LAYOUT)?.into_iter().next() {
            return Ok(Some(layout));
        }
    }
    Ok(None)
}

fn write_slide(slide: &Slide, part: &str, out: &mut Out, removed_jumps: &std::collections::HashSet<String>) -> Result<()> {
    let mut ctx = PartCtx::load(&out.pkg, part)?;
    let mut root = origin_as(&slide.origin, ns::P, "sld").unwrap_or_else(new_slide);
    if slide.hidden {
        root.set_attr("", "show", "0");
    } else {
        root.remove_attr("", "show");
    }
    let c_sld = root.child_or_insert(ns::P, "cSld");
    match &slide.name {
        Some(n) => c_sld.set_attr("", "name", n),
        None => {
            c_sld.remove_attr("", "name");
        }
    }
    let tree = c_sld.child_or_insert(ns::P, "spTree");
    if tree.child(ns::P, "nvGrpSpPr").is_none() {
        *tree = new_tree();
    }
    // The tree's own id is not a frame. A new tree uses 1; a file may use another.
    let root_id = tree
        .child(ns::P, "nvGrpSpPr")
        .and_then(|nv| nv.child(ns::P, "cNvPr"))
        .and_then(|c| c.attr("id"))
        .and_then(|id| id.parse().ok())
        .unwrap_or(1);
    let mut ids = FrameIds::new(&slide.frames, root_id);
    let mut shapes = Vec::new();
    for frame in &slide.frames {
        if let Some(el) = write_frame(frame, &mut ctx, out, &mut ids)? {
            shapes.push(el);
        }
    }
    // Shapes go after the tree's own properties and before its extensions.
    let at = tree
        .children
        .iter()
        .position(|n| matches!(n, crate::xml::Node::Element(e) if e.is(ns::P, "extLst")))
        .unwrap_or(tree.children.len());
    tree.children.splice(at..at, shapes.into_iter().map(crate::xml::Node::Element));
    if !removed_jumps.is_empty() {
        remove_jumps(&mut root, removed_jumps);
    }
    let doc = XmlDoc::new(root);
    ctx.finish(&mut out.pkg);
    put_if_changed(&mut out.pkg, part, doc, types::PPTX_SLIDE)
}

fn remove_jumps(el: &mut Element, ids: &std::collections::HashSet<String>) {
    el.children.retain(|child| {
        !matches!(child, crate::xml::Node::Element(e)
        if (e.is(ns::A, "hlinkClick") || e.is(ns::A, "hlinkMouseOver"))
            && e.attr("action").is_some_and(|action| action.eq_ignore_ascii_case("ppaction://hlinksldjump"))
            && rel_attr(e, "id").is_some_and(|id| ids.contains(id)))
    });
    el.elements_mut().for_each(|e| remove_jumps(e, ids));
}

/// Frame ids, unique within a slide: a frame's own, or the next free one for frames without.
struct FrameIds {
    next: u32,
    used: std::collections::HashSet<u32>,
}

impl FrameIds {
    /// `root_id` is the spTree group. 0 means "no id", and 1 is reserved because a new tree uses it.
    fn new(frames: &[Frame], root_id: u32) -> FrameIds {
        fn collect(frames: &[Frame], used: &mut std::collections::HashSet<u32>) {
            for frame in frames {
                used.insert(frame.id);
                if let FrameContent::Group { frames } = &frame.content {
                    collect(frames, used);
                }
            }
        }
        let mut used = std::collections::HashSet::from([0, 1]);
        if root_id != 0 {
            used.insert(root_id);
        }
        collect(frames, &mut used);
        FrameIds { next: 2, used }
    }

    fn id(&mut self, frame: &Frame) -> Result<u32> {
        if frame.id != 0 {
            return Ok(frame.id);
        }
        while self.used.contains(&self.next) {
            self.next = self.next.checked_add(1).ok_or_else(|| Error::Invalid("no shape ids remain".into()))?;
        }
        let id = self.next;
        self.used.insert(id);
        Ok(id)
    }
}

fn write_frame(frame: &Frame, ctx: &mut PartCtx, out: &mut Out, ids: &mut FrameIds) -> Result<Option<Element>> {
    let id = ids.id(frame)?;
    let mut el = match &frame.content {
        FrameContent::Shape { text, connector } => {
            let (local, nv) = if *connector { ("cxnSp", "nvCxnSpPr") } else { ("sp", "nvSpPr") };
            let mut el = origin_as(&frame.origin, ns::P, local).unwrap_or_else(|| {
                let c_nv = if *connector { "cNvCxnSpPr" } else { "cNvSpPr" };
                let mut c = Element::new(ns::P, c_nv);
                // A text box is a plain rectangle of text; a filled shape or another outline isn't one.
                let plain = frame.shape.preset.as_deref().is_none_or(|p| p == "rect") && frame.shape.fill.is_none();
                if !*connector && frame.placeholder.is_none() && text.is_some() && plain {
                    c.set_attr("", "txBox", "1");
                }
                Element::new(ns::P, local).with_child(nv_props(nv, c)).with_child(Element::new(ns::P, "spPr"))
            });
            patch_nv(&mut el, nv, frame, id);
            frame.shape.patch(el.child_or_insert(ns::P, "spPr"));
            el.remove_children(ns::P, "txBody");
            if let Some(body) = text.as_ref().filter(|_| !*connector) {
                // txBody goes after spPr and style, before extensions.
                el.place(write_text_body(body, ns::P, ctx), &[&["nvSpPr"], &["spPr"], &["style"], &["txBody"], &["extLst"]]);
            }
            el
        }
        FrameContent::Picture { image } => write_picture(frame, image, id, ctx, out)?,
        FrameContent::Table { table } => {
            let mut el = origin_as(&frame.origin, ns::P, "graphicFrame").unwrap_or_else(|| {
                let mut locks = Element::new(ns::A, "graphicFrameLocks");
                locks.set_attr("", "noGrp", "1");
                Element::new(ns::P, "graphicFrame")
                    .with_child(nv_props("nvGraphicFramePr", Element::new(ns::P, "cNvGraphicFramePr").with_child(locks)))
                    .with_child(Element::new(ns::P, "xfrm"))
                    .with_child(
                        Element::new(ns::A, "graphic").with_child(Element::new(ns::A, "graphicData").with_attr("", "uri", TABLE_URI)),
                    )
            });
            patch_nv(&mut el, "nvGraphicFramePr", frame, id);
            patch_graphic_xfrm(&mut el, frame);
            let data = el.child_or_insert(ns::A, "graphic").child_or_insert(ns::A, "graphicData");
            data.set_attr("", "uri", TABLE_URI);
            data.remove_children(ns::A, "tbl");
            data.push(write_table(table, ctx));
            el
        }
        FrameContent::Group { frames } => {
            let mut el = origin_as(&frame.origin, ns::P, "grpSp").unwrap_or_else(|| {
                Element::new(ns::P, "grpSp")
                    .with_child(nv_props("nvGrpSpPr", Element::new(ns::P, "cNvGrpSpPr")))
                    .with_child(Element::new(ns::P, "grpSpPr"))
            });
            patch_nv(&mut el, "nvGrpSpPr", frame, id);
            // A group's children are placed in its child space; a new group's is its own box.
            let mut shape = frame.shape.clone();
            if let Some(x) = shape.xfrm.as_mut().filter(|x| x.child.is_none()) {
                x.child = Some((x.x, x.y, x.width, x.height));
            }
            shape.patch(el.child_or_insert(ns::P, "grpSpPr"));
            let at = el
                .children
                .iter()
                .position(|n| matches!(n, crate::xml::Node::Element(e) if e.is(ns::P, "extLst")))
                .unwrap_or(el.children.len());
            let mut children = Vec::new();
            for f in frames {
                if let Some(c) = write_frame(f, ctx, out, ids)? {
                    children.push(crate::xml::Node::Element(c));
                }
            }
            el.children.splice(at..at, children);
            el
        }
        // Charts, diagrams and objects from this format come back as they were; other formats'
        // raw markup can't sit on a slide.
        FrameContent::Raw { xml, chart_part } if xml.in_ns(ns::P) || xml.in_ns(ns::MC) => {
            let mut el = xml.clone();
            if let Some(part) = chart_part {
                if !out.pkg.has(part) {
                    return Err(Error::MissingPart(part.clone()));
                }
                let chart =
                    super::chart_element_mut(&mut el).ok_or_else(|| Error::Invalid("a copied chart has no chart reference".into()))?;
                chart.set_attr(ns::R, "id", ctx.relate(kind::CHART, part));
                patch_nv(&mut el, "nvGraphicFramePr", frame, id);
            }
            if super::chart_element(&el).is_some() {
                patch_graphic_xfrm(&mut el, frame);
            }
            el
        }
        FrameContent::Raw { .. } => return Ok(None),
    };
    // Keep unknown action links and all attributes of unchanged hyperlinks intact.
    if let Some(nv) = el
        .children
        .iter_mut()
        .filter_map(|n| match n {
            crate::xml::Node::Element(e) => Some(e),
            _ => None,
        })
        .find(|e| e.in_ns(ns::P) && e.local.starts_with("nv"))
    {
        let c = nv.child_or_insert(ns::P, "cNvPr");
        crate::formats::dml::patch_link(frame.link.as_ref(), c, ctx);
    }
    Ok(Some(el))
}

fn nv_props(local: &str, c_nv_x: Element) -> Element {
    Element::new(ns::P, local).with_child(Element::new(ns::P, "cNvPr")).with_child(c_nv_x).with_child(Element::new(ns::P, "nvPr"))
}

fn patch_graphic_xfrm(el: &mut Element, frame: &Frame) {
    if let Some(x) = &frame.shape.xfrm {
        if el.child(ns::P, "xfrm").map(crate::drawing::Xfrm::parse).as_ref() != Some(x) {
            el.place(x.to_xml(ns::P), &[&["nvGraphicFramePr"], &["xfrm"], &["graphic"], &["extLst"]]);
        }
    }
}

/// Puts the frame's id, name, description, visibility and placeholder into its non-visual properties.
fn patch_nv(el: &mut Element, nv: &str, frame: &Frame, id: u32) {
    let nv = el.child_or_insert(ns::P, nv);
    let c = nv.child_or_insert(ns::P, "cNvPr");
    let id = id.to_string();
    if c.attr("id") != Some(id.as_str()) {
        c.set_attr("", "id", id);
    }
    if c.attr("name") != Some(frame.name.as_str()) {
        c.set_attr("", "name", &frame.name);
    }
    match &frame.description {
        Some(d) => c.set_attr("", "descr", d),
        None => {
            c.remove_attr("", "descr");
        }
    }
    let hidden = c.attr("hidden").and_then(crate::units::parse_bool).unwrap_or(false);
    if hidden != frame.hidden {
        if frame.hidden {
            c.set_attr("", "hidden", "1");
        } else {
            c.remove_attr("", "hidden");
        }
    }
    let nv_pr = nv.child_or_insert(ns::P, "nvPr");
    let current = nv_pr
        .child(ns::P, "ph")
        .map(|ph| Placeholder { kind: ph.attr("type").map(str::to_string), index: ph.attr("idx").and_then(|i| i.parse().ok()) });
    if current != frame.placeholder {
        match &frame.placeholder {
            Some(p) => {
                let ph = nv_pr.child_or_insert(ns::P, "ph");
                crate::formats::dml::set_opt(ph, "type", p.kind.clone());
                crate::formats::dml::set_opt(ph, "idx", p.index.map(|i| i.to_string()));
                // ph comes first in nvPr.
                if let Some(i) = nv_pr.children.iter().position(|n| matches!(n, crate::xml::Node::Element(e) if e.is(ns::P, "ph"))) {
                    let ph = nv_pr.children.remove(i);
                    nv_pr.children.insert(0, ph);
                }
            }
            None => {
                nv_pr.remove_children(ns::P, "ph");
            }
        }
    }
}

fn write_picture(frame: &Frame, image: &Image, id: u32, ctx: &mut PartCtx, out: &mut Out) -> Result<Element> {
    let mut el = origin_as(&frame.origin, ns::P, "pic").unwrap_or_else(|| {
        let mut locks = Element::new(ns::A, "picLocks");
        locks.set_attr("", "noChangeAspect", "1");
        let fill = Element::new(ns::P, "blipFill")
            .with_child(Element::new(ns::A, "blip"))
            .with_child(Element::new(ns::A, "stretch").with_child(Element::new(ns::A, "fillRect")));
        Element::new(ns::P, "pic")
            .with_child(nv_props("nvPicPr", Element::new(ns::P, "cNvPicPr").with_child(locks)))
            .with_child(fill)
            .with_child(Element::new(ns::P, "spPr"))
    });
    patch_nv(&mut el, "nvPicPr", frame, id);
    let rid = ctx.image(out, &image.media)?;
    let blip = el.child_or_insert(ns::P, "blipFill").child_or_insert(ns::A, "blip");
    if rel_attr(blip, "embed") != Some(rid.as_str()) {
        blip.set_attr(ns::R, "embed", rid);
    }
    let mut shape = frame.shape.clone();
    if shape.preset.is_none() && frame.origin.get().is_none() {
        shape.preset = Some("rect".into());
    }
    if shape.xfrm.is_none() {
        shape.xfrm = Some(crate::drawing::Xfrm { width: image.width, height: image.height, ..Default::default() });
    }
    shape.patch(el.child_or_insert(ns::P, "spPr"));
    Ok(el)
}

fn write_table(table: &Table, ctx: &mut PartCtx) -> Element {
    let mut tbl = origin_as(&table.origin, ns::A, "tbl").unwrap_or_else(|| {
        let mut pr = Element::new(ns::A, "tblPr");
        pr.set_attr("", "firstRow", "1");
        pr.set_attr("", "bandRow", "1");
        Element::new(ns::A, "tbl").with_child(pr)
    });
    let pr = tbl.child_or_insert(ns::A, "tblPr");
    let style = table.style.clone().or_else(|| (table.origin.get().is_none()).then(|| DEFAULT_TABLE_STYLE.to_string()));
    let current = pr.child(ns::A, "tableStyleId").map(Element::text);
    // Word's style ids aren't PowerPoint's GUIDs.
    let style = style.filter(|s| s.starts_with('{'));
    if current != style {
        pr.remove_children(ns::A, "tableStyleId");
        if let Some(s) = style {
            pr.push(Element::new(ns::A, "tableStyleId").with_text(s));
        }
    }
    let grid = crate::formats::common::grid(
        tbl.child(ns::A, "tblGrid"),
        &table.columns,
        || Element::new(ns::A, "tblGrid"),
        |col, w| {
            let mut col = col.unwrap_or_else(|| Element::new(ns::A, "gridCol"));
            if col.attr("w").and_then(|v| v.parse().ok()) != Some(w.0) {
                col.set_attr("", "w", w.0.to_string());
            }
            col
        },
    );
    tbl.place(grid, &[&["tblPr"], &["tblGrid"], &["tr"]]);
    for (r, row) in table.rows.iter().enumerate() {
        let mut tr = origin_as(&row.origin, ns::A, "tr").unwrap_or_else(|| Element::new(ns::A, "tr"));
        // PowerPoint requires a row height.
        let h = row.height.map(|h| h.0).unwrap_or(370_840);
        if tr.attr("h") != Some(h.to_string().as_str()) {
            tr.set_attr("", "h", h.to_string());
        }
        let ext = tr
            .children
            .iter()
            .position(|n| matches!(n, crate::xml::Node::Element(e) if e.is(ns::A, "extLst")))
            .unwrap_or(tr.children.len());
        let mut cells = Vec::new();
        for (c, cell) in row.cells.iter().enumerate() {
            let mut tc = origin_as(&cell.origin, ns::A, "tc").unwrap_or_else(|| Element::new(ns::A, "tc"));
            let (or, oc) = if cell.covered { table.owner(r, c).unwrap_or((r, c)) } else { (r, c) };
            let owner = &table.rows[or].cells[oc];
            let n = |v: u32| (v > 1).then(|| v.to_string());
            // Each row's left cell carries gridSpan, each column's top cell rowSpan.
            // Interior cells participate in both merges.
            crate::formats::dml::set_opt(&mut tc, "gridSpan", n(owner.col_span).filter(|_| c == oc));
            crate::formats::dml::set_opt(&mut tc, "rowSpan", n(owner.row_span).filter(|_| r == or));
            crate::formats::dml::set_opt(&mut tc, "hMerge", (c > oc).then(|| "1".to_string()));
            crate::formats::dml::set_opt(&mut tc, "vMerge", (r > or).then(|| "1".to_string()));
            let mut body = tc.child(ns::A, "txBody").cloned().unwrap_or_else(|| {
                Element::new(ns::A, "txBody").with_child(Element::new(ns::A, "bodyPr")).with_child(Element::new(ns::A, "lstStyle"))
            });
            let paragraphs: Vec<_> = cell.blocks.iter().filter_map(|b| if let Block::Paragraph(p) = b { Some(p) } else { None }).collect();
            for p in &paragraphs {
                body.push(write_paragraph(p, ctx));
            }
            if paragraphs.is_empty() {
                body.push(Element::new(ns::A, "p"));
            }
            tc.place(body, &[&["txBody"], &["tcPr"], &["extLst"]]);
            let fill_now = tc.child(ns::A, "tcPr").and_then(|p| p.child(ns::A, "solidFill")).and_then(crate::drawing::Color::in_drawingml);
            if fill_now != cell.fill {
                let pr = tc.child_or_insert(ns::A, "tcPr");
                const TC_PR: &[&[&str]] = &[
                    &["lnL"],
                    &["lnR"],
                    &["lnT"],
                    &["lnB"],
                    &["lnTlToBr"],
                    &["lnBlToTr"],
                    &["cell3D"],
                    &["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"],
                    &["headers"],
                    &["extLst"],
                ];
                match &cell.fill {
                    Some(color) => pr.place(Element::new(ns::A, "solidFill").with_child(color.to_drawingml()), TC_PR),
                    None => pr.clear(ns::A, TC_PR[7]),
                }
            }
            cells.push(crate::xml::Node::Element(tc));
        }
        tr.children.splice(ext..ext, cells);
        tbl.push(tr);
    }
    tbl
}

fn new_tree() -> Element {
    let xfrm = Element::new(ns::A, "xfrm")
        .with_child(Element::new(ns::A, "off").with_attr("", "x", "0").with_attr("", "y", "0"))
        .with_child(Element::new(ns::A, "ext").with_attr("", "cx", "0").with_attr("", "cy", "0"))
        .with_child(Element::new(ns::A, "chOff").with_attr("", "x", "0").with_attr("", "y", "0"))
        .with_child(Element::new(ns::A, "chExt").with_attr("", "cx", "0").with_attr("", "cy", "0"));
    Element::new(ns::P, "spTree")
        .with_child(
            Element::new(ns::P, "nvGrpSpPr")
                .with_child(Element::new(ns::P, "cNvPr").with_attr("", "id", "1").with_attr("", "name", ""))
                .with_child(Element::new(ns::P, "cNvGrpSpPr"))
                .with_child(Element::new(ns::P, "nvPr")),
        )
        .with_child(Element::new(ns::P, "grpSpPr").with_child(xfrm))
}

fn root(local: &str) -> Element {
    let mut el = Element::new(ns::P, local);
    el.declare(Some("a"), ns::A);
    el.declare(Some("r"), ns::R);
    el.declare(Some("p"), ns::P);
    el
}

fn new_slide() -> Element {
    root("sld").with_child(Element::new(ns::P, "cSld").with_child(new_tree()))
}

/// Text defaults: `level` sizes for each of the nine outline levels.
fn levels(container: &str, ns_: &str, size: u32, font: &str) -> Element {
    let mut el = Element::new(ns_, container);
    for lvl in 1..=9 {
        let indent = (lvl - 1) * 457_200;
        let def = Element::new(ns::A, "defRPr")
            .with_attr("", "sz", size.to_string())
            .with_attr("", "kern", "1200")
            .with_child(Element::new(ns::A, "solidFill").with_child(Element::new(ns::A, "schemeClr").with_attr("", "val", "tx1")))
            .with_child(Element::new(ns::A, "latin").with_attr("", "typeface", font))
            .with_child(Element::new(ns::A, "ea").with_attr("", "typeface", font.replace("-lt", "-ea")))
            .with_child(Element::new(ns::A, "cs").with_attr("", "typeface", font.replace("-lt", "-cs")));
        el.push(
            Element::new(ns::A, &format!("lvl{lvl}pPr"))
                .with_attr("", "marL", indent.to_string())
                .with_attr("", "algn", "l")
                .with_child(def),
        );
    }
    el
}

/// A new package with what PowerPoint needs around slides: a master with one blank layout, a
/// theme, and the presentation's own properties parts.
fn scaffold(theme: &Theme, size: SlideSize) -> Package {
    let mut pkg = Package::new();
    let main = "ppt/presentation.xml";
    let master = "ppt/slideMasters/slideMaster1.xml";
    let layout = "ppt/slideLayouts/slideLayout1.xml";
    let theme_part = "ppt/theme/theme1.xml";

    let clr_map = [
        "bg1", "lt1", "tx1", "dk1", "bg2", "lt2", "tx2", "dk2", "accent1", "accent1", "accent2", "accent2", "accent3", "accent3",
        "accent4", "accent4", "accent5", "accent5", "accent6", "accent6", "hlink", "hlink", "folHlink", "folHlink",
    ]
    .chunks(2)
    .fold(Element::new(ns::P, "clrMap"), |el, kv| el.with_attr("", kv[0], kv[1]));
    let bg = Element::new(ns::P, "bg").with_child(
        Element::new(ns::P, "bgRef").with_attr("", "idx", "1001").with_child(Element::new(ns::A, "schemeClr").with_attr("", "val", "bg1")),
    );
    let styles = Element::new(ns::P, "txStyles")
        .with_child(levels("titleStyle", ns::P, 4400, "+mj-lt"))
        .with_child(levels("bodyStyle", ns::P, 2800, "+mn-lt"))
        .with_child(levels("otherStyle", ns::P, 1800, "+mn-lt"));
    let master_xml = root("sldMaster")
        .with_child(Element::new(ns::P, "cSld").with_child(bg).with_child(new_tree()))
        .with_child(clr_map)
        .with_child(
            Element::new(ns::P, "sldLayoutIdLst")
                .with_child(Element::new(ns::P, "sldLayoutId").with_attr("", "id", "2147483649").with_attr(ns::R, "id", "rId1")),
        )
        .with_child(styles);
    let layout_xml = root("sldLayout")
        .with_attr("", "type", "blank")
        .with_attr("", "preserve", "1")
        .with_child(Element::new(ns::P, "cSld").with_attr("", "name", "Blank").with_child(new_tree()))
        .with_child(Element::new(ns::P, "clrMapOvr").with_child(Element::new(ns::A, "masterClrMapping")));
    let pres = root("presentation")
        .with_attr("", "saveSubsetFonts", "1")
        .with_child(
            Element::new(ns::P, "sldMasterIdLst")
                .with_child(Element::new(ns::P, "sldMasterId").with_attr("", "id", "2147483648").with_attr(ns::R, "id", "rId1")),
        )
        .with_child(Element::new(ns::P, "sldSz").with_attr("", "cx", size.width.0.to_string()).with_attr(
            "",
            "cy",
            size.height.0.to_string(),
        ))
        .with_child(Element::new(ns::P, "notesSz").with_attr("", "cx", "6858000").with_attr("", "cy", "9144000"))
        .with_child(levels("defaultTextStyle", ns::P, 1800, "+mn-lt"));

    pkg.put_xml(main, &XmlDoc::new(pres), types::PPTX_MAIN);
    pkg.put_xml(master, &XmlDoc::new(master_xml), types::PPTX_MASTER);
    pkg.put_xml(layout, &XmlDoc::new(layout_xml), types::PPTX_LAYOUT);
    pkg.put_xml(theme_part, &theme.to_xml(), types::THEME);
    let mut props = Element::new(ns::P, "presentationPr");
    props.declare(Some("p"), ns::P);
    pkg.put_xml("ppt/presProps.xml", &XmlDoc::new(props), PRES_PROPS);
    let mut view = Element::new(ns::P, "viewPr");
    view.declare(Some("p"), ns::P);
    pkg.put_xml("ppt/viewProps.xml", &XmlDoc::new(view), VIEW_PROPS);
    let mut table_styles = Element::new(ns::A, "tblStyleLst").with_attr("", "def", DEFAULT_TABLE_STYLE);
    table_styles.declare(Some("a"), ns::A);
    pkg.put_xml("ppt/tableStyles.xml", &XmlDoc::new(table_styles), TABLE_STYLES);

    // rId1 in each part is what the XML above refers to.
    let relate = |pkg: &mut Package, from: &str, k: &str, to: &str| pkg.relate(from, k, to).expect("relating parts of a new package");
    relate(&mut pkg, "", kind::OFFICE_DOCUMENT, main);
    relate(&mut pkg, main, kind::SLIDE_MASTER, master);
    relate(&mut pkg, main, kind::THEME, theme_part);
    relate(&mut pkg, main, "presProps", "ppt/presProps.xml");
    relate(&mut pkg, main, "viewProps", "ppt/viewProps.xml");
    relate(&mut pkg, main, "tableStyles", "ppt/tableStyles.xml");
    relate(&mut pkg, master, kind::SLIDE_LAYOUT, layout);
    relate(&mut pkg, master, kind::THEME, theme_part);
    relate(&mut pkg, layout, kind::SLIDE_MASTER, master);
    pkg
}
