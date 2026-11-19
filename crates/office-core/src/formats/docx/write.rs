use super::props::{patch_para_props, patch_run_props, read_run_props, val, PPR};
use super::read::single_run;
use crate::drawing::Theme;
use crate::formats::common::{names_parts, origin_as, put_if_changed, Out, PartCtx};
use crate::model::{Block, Body, BreakKind, Document, Image, Inline, Kind, Link, Paragraph, Run, Table};
use crate::opc::content_types::types;
use crate::opc::{rels::kind, Package};
use crate::units::Emu;
use crate::xml::{ns, Element, Node, XmlDoc};
use crate::{Error, Result};

const PICTURE_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

/// What writing one part needs.
struct Ctx<'a, 'b> {
    out: &'a mut Out<'b>,
    rels: PartCtx,
    theme: Theme,
    /// The next `wp:docPr` id for a new picture; they must be unique in the document.
    next_pic: u32,
    used_pics: Option<std::collections::HashSet<u32>>,
}

pub fn write(doc: &Document) -> Result<Package> {
    let Body::Flow { blocks } = &doc.body else {
        return Err(Error::Invalid("only a flowing document can be saved as .docx".into()));
    };
    let same = doc.kind == Kind::Word && doc.package.is_some();
    let theme = doc.theme.clone().unwrap_or_else(Theme::office);
    let pkg = if same { doc.package.clone().unwrap() } else { scaffold(&theme)? };
    let mut out = Out::new(doc, pkg, same, "word/media");
    let main = out.pkg.main_part()?;
    // Another package's document can't bring its headers and footers (the parts its r:ids
    // name) along; anything else about it can come, such as a page setup made in code.
    let root = origin_as(&doc.origin, ns::W, "document").filter(|o| same || !names_parts(o));
    let mut root = match root {
        Some(root) => root,
        None => {
            // The scaffold's document, or the source without its blocks.
            let mut r = out.pkg.xml(&main)?.root;
            if let Some(body) = r.child_mut(ns::W, "body") {
                body.children.retain(|n| matches!(n, Node::Element(e) if e.is(ns::W, "sectPr")));
            }
            r
        }
    };
    let mut ctx = Ctx { rels: PartCtx::load(&out.pkg, &main)?, out: &mut out, theme, next_pic: 1, used_pics: None };

    let mut els = Vec::new();
    for b in blocks {
        write_block(b, &mut ctx, &mut els)?;
    }
    let body = root.child_or_insert(ns::W, "body");
    let at = body.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(ns::W, "sectPr"))).unwrap_or(body.children.len());
    body.children.splice(at..at, els.into_iter().map(Node::Element));

    let Ctx { rels, out, .. } = ctx;
    rels.finish(&mut out.pkg);
    put_if_changed(&mut out.pkg, &main, XmlDoc::new(root), types::DOCX_MAIN)?;
    doc.properties.write(&mut out.pkg)?;
    Ok(std::mem::take(&mut out.pkg))
}

/// `blocks` as Word markup for another part of the package than the document (a header, a
/// footnote, a comment), with their pictures and links related from that part. For the Word
/// layer, which puts the elements in the part itself.
pub fn write_blocks(doc: &Document, pkg: &mut Package, part: &str, blocks: &[Block]) -> Result<Vec<Element>> {
    let same = doc.kind == Kind::Word && doc.package.is_some();
    let mut out = Out::new(doc, std::mem::take(pkg), same, "word/media");
    let theme = doc.theme.clone().unwrap_or_else(Theme::office);
    let mut ctx = Ctx { rels: PartCtx::load(&out.pkg, part)?, out: &mut out, theme, next_pic: 1, used_pics: None };
    let mut els = Vec::new();
    for b in blocks {
        write_block(b, &mut ctx, &mut els)?;
    }
    let Ctx { rels, out, .. } = ctx;
    rels.finish(&mut out.pkg);
    *pkg = std::mem::take(&mut out.pkg);
    Ok(els)
}

fn collect_doc_pr(el: &Element, used: &mut std::collections::HashSet<u32>) {
    if el.is(ns::WP, "docPr") {
        if let Some(id) = el.attr("id").and_then(|i| i.parse().ok()) {
            used.insert(id);
        }
    }
    for child in el.elements() {
        collect_doc_pr(child, used);
    }
}

impl Ctx<'_, '_> {
    /// Scan only when a picture needs a new id. Saving existing pictures needs no allocation.
    fn picture_id(&mut self) -> Result<u32> {
        if self.used_pics.is_none() {
            let mut used = std::collections::HashSet::new();
            for part in self.out.pkg.part_names().filter(|p| p.starts_with("word/") && p.ends_with(".xml")) {
                if let Ok(xml) = self.out.pkg.xml(part) {
                    collect_doc_pr(&xml.root, &mut used);
                }
            }
            self.used_pics = Some(used);
        }
        let used = self.used_pics.as_mut().unwrap();
        while used.contains(&self.next_pic) {
            self.next_pic = self.next_pic.checked_add(1).ok_or_else(|| Error::Invalid("no picture ids remain".into()))?;
        }
        let id = self.next_pic;
        used.insert(id);
        Ok(id)
    }
}

/// Raw markup from PowerPoint or Excel means nothing in Word.
fn foreign(xml: &Element) -> bool {
    xml.in_ns(ns::A) || xml.in_ns(ns::P) || xml.in_ns(ns::X)
}

fn write_block(b: &Block, ctx: &mut Ctx, out: &mut Vec<Element>) -> Result<()> {
    match b {
        Block::Paragraph(p) => out.push(write_paragraph(p, ctx)?),
        Block::Table(t) => out.push(write_table(t, ctx)?),
        Block::Raw { xml, .. } if !foreign(xml) => out.push(xml.clone()),
        Block::Raw { .. } => {}
    }
    Ok(())
}

fn write_paragraph(p: &Paragraph, ctx: &mut Ctx) -> Result<Element> {
    let mut el = origin_as(&p.origin, ns::W, "p").unwrap_or_else(|| Element::new(ns::W, "p"));
    let had = el.child(ns::W, "pPr").cloned();
    el.children.clear();
    let mut ppr = had.clone().unwrap_or_else(|| Element::new(ns::W, "pPr"));
    patch_para_props(&p.props, &mut ppr);
    let mark_now = ppr.child(ns::W, "rPr").map(read_run_props);
    if mark_now != p.mark {
        match &p.mark {
            Some(m) => {
                let mut rpr = ppr.child(ns::W, "rPr").cloned().unwrap_or_else(|| Element::new(ns::W, "rPr"));
                patch_run_props(m, &mut rpr, &ctx.theme);
                ppr.place(rpr, PPR);
            }
            None => ppr.clear(ns::W, &["rPr"]),
        }
    }
    if had.is_some() || ppr.elements().next().is_some() || !ppr.attrs.is_empty() {
        el.push(ppr);
    }

    // Runs that share a link go in one w:hyperlink.
    let mut link: Option<(Link, Element)> = None;
    let mut link_origin: Option<&Element> = None;
    let doc = ctx.out.doc;
    for inline in &p.content {
        let run_link = match inline {
            Inline::Text(r) => r.link.clone(),
            _ => None,
        };
        let wrapper = match inline {
            Inline::Text(r) => super::read::hyperlink_origin(doc, r),
            _ => None,
        };
        if link.as_ref().map(|(l, _)| l) != run_link.as_ref() || wrapper.map(std::ptr::from_ref) != link_origin.map(std::ptr::from_ref) {
            if let Some((_, h)) = link.take() {
                el.push(h);
            }
            link_origin = wrapper;
            if let Some(l) = run_link {
                let h = hyperlink(&l, wrapper, &mut ctx.rels);
                link = Some((l, h));
            }
        }
        let target = match &mut link {
            Some((_, h)) => h,
            None => &mut el,
        };
        match inline {
            Inline::Text(run) => write_run(run, ctx, target),
            Inline::Break { kind, origin } => {
                target.push(write_break(*kind, origin.get().filter(|o| o.is(ns::W, "r"))));
            }
            Inline::Image(image) => {
                let r = write_image(image, ctx)?;
                target.push(r);
            }
            Inline::Raw { xml } if !foreign(xml) => {
                target.push(xml.clone());
            }
            Inline::Raw { .. } => {}
        }
    }
    if let Some((_, h)) = link {
        el.push(h);
    }
    Ok(el)
}

fn hyperlink(l: &Link, origin: Option<&Element>, rels: &mut PartCtx) -> Element {
    let mut h = origin.cloned().unwrap_or_else(|| Element::new(ns::W, "hyperlink"));
    h.children.clear();
    let current_url = h.attr_ns(ns::R, "id").and_then(|id| rels.url(id));
    if current_url != l.url.as_deref() {
        h.remove_attr(ns::R, "id");
        if let Some(url) = &l.url {
            h.set_attr(ns::R, "id", rels.link(url));
        }
    }
    for (name, value) in [("anchor", &l.anchor), ("tooltip", &l.tooltip)] {
        if h.attr_ns(ns::W, name) != value.as_deref() {
            h.remove_attr(ns::W, name);
            if let Some(value) = value {
                h.set_attr(ns::W, name, value);
            }
        }
    }
    if origin.is_none() {
        h.set_attr(ns::W, "history", "1");
    }
    h
}

/// A page or column break's run. An untouched run of just the break goes back as it was;
/// a break split out of a longer run keeps that run's attributes and formatting.
fn write_break(kind: BreakKind, origin: Option<&Element>) -> Element {
    let t = if kind == BreakKind::Page { "page" } else { "column" };
    let br = Element::new(ns::W, "br").with_attr(ns::W, "type", t);
    let Some(o) = origin else { return Element::new(ns::W, "r").with_child(br) };
    let mut content = o.elements().filter(|e| !e.is(ns::W, "rPr") && !e.is(ns::W, "lastRenderedPageBreak"));
    let only_break = content.next().is_some_and(|e| e.is(ns::W, "br") && e.attr_ns(ns::W, "type") == Some(t)) && content.next().is_none();
    if only_break {
        return o.clone();
    }
    let mut r = Element::new(o.ns.as_deref().unwrap_or(ns::W), "r");
    r.attrs = o.attrs.clone();
    if let Some(rpr) = o.child(ns::W, "rPr") {
        r.push(rpr.clone());
    }
    r.with_child(br)
}

fn write_run(run: &Run, ctx: &mut Ctx, parent: &mut Element) {
    let origin = run.origin.get().filter(|o| o.is(ns::W, "r"));
    // A run nobody touched goes back exactly as it was.
    if let Some(o) = origin {
        if single_run(o).is_some_and(|(text, props)| text == run.text && props == run.props) {
            parent.push(o.clone());
            return;
        }
    }
    let mut r = Element::new(origin.and_then(|o| o.ns.as_deref()).unwrap_or(ns::W), "r");
    if let Some(o) = origin {
        r.attrs = o.attrs.clone();
    }
    let had = origin.and_then(|o| o.child(ns::W, "rPr")).cloned();
    let mut rpr = had.clone().unwrap_or_else(|| Element::new(ns::W, "rPr"));
    patch_run_props(&run.props, &mut rpr, &ctx.theme);
    if had.is_some() || rpr.elements().next().is_some() {
        r.push(rpr);
    }
    let mut text = String::new();
    let flush = |text: &mut String, r: &mut Element| {
        if text.is_empty() {
            return;
        }
        let mut t = Element::new(ns::W, "t");
        if text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace) || text.contains("  ") {
            t.set_attr(ns::XML, "space", "preserve");
        }
        r.push(t.with_text(std::mem::take(text)));
    };
    for c in run.text.chars() {
        let special = match c {
            '\t' => "tab",
            '\n' => "br",
            '\u{2011}' => "noBreakHyphen",
            '\u{AD}' => "softHyphen",
            c => {
                text.push(c);
                continue;
            }
        };
        flush(&mut text, &mut r);
        r.push(Element::new(ns::W, special));
    }
    flush(&mut text, &mut r);
    parent.push(r);
}

fn write_image(image: &Image, ctx: &mut Ctx) -> Result<Element> {
    let rid = ctx.rels.image(ctx.out, &image.media)?;
    let size = |el: &mut Element| {
        el.set_attr("", "cx", image.width.0.to_string());
        el.set_attr("", "cy", image.height.0.to_string());
    };
    if let Some(mut r) = origin_as(&image.origin, ns::W, "r") {
        if let Some(inline) = r.child_mut(ns::W, "drawing").and_then(|d| d.child_mut(ns::WP, "inline")) {
            size(inline.child_or_insert(ns::WP, "extent"));
            if let Some(pr) = inline.child_mut(ns::WP, "docPr") {
                crate::formats::dml::set_opt(pr, "descr", image.description.clone());
            }
            if let Some(ext) = descend(
                inline,
                &[(ns::A, "graphic"), (ns::A, "graphicData"), (ns::PIC, "pic"), (ns::PIC, "spPr"), (ns::A, "xfrm"), (ns::A, "ext")],
            ) {
                size(ext);
            }
            if let Some(blip) =
                descend(inline, &[(ns::A, "graphic"), (ns::A, "graphicData"), (ns::PIC, "pic"), (ns::PIC, "blipFill"), (ns::A, "blip")])
            {
                blip.set_attr(ns::R, "embed", rid);
            }
            return Ok(r);
        }
    }
    let id = ctx.picture_id()?;
    let name = if image.name.is_empty() { format!("Picture {id}") } else { image.name.clone() };
    let mut doc_pr = Element::new(ns::WP, "docPr").with_attr("", "id", id.to_string()).with_attr("", "name", &name);
    if let Some(d) = &image.description {
        doc_pr.set_attr("", "descr", d);
    }
    let mut extent = Element::new(ns::WP, "extent");
    size(&mut extent);
    let mut ext = Element::new(ns::A, "ext");
    size(&mut ext);
    let pic = Element::new(ns::PIC, "pic")
        .with_child(
            Element::new(ns::PIC, "nvPicPr")
                .with_child(Element::new(ns::PIC, "cNvPr").with_attr("", "id", "0").with_attr("", "name", &name))
                .with_child(Element::new(ns::PIC, "cNvPicPr")),
        )
        .with_child(
            Element::new(ns::PIC, "blipFill")
                .with_child(Element::new(ns::A, "blip").with_attr(ns::R, "embed", rid))
                .with_child(Element::new(ns::A, "stretch").with_child(Element::new(ns::A, "fillRect"))),
        )
        .with_child(
            Element::new(ns::PIC, "spPr")
                .with_child(
                    Element::new(ns::A, "xfrm")
                        .with_child(Element::new(ns::A, "off").with_attr("", "x", "0").with_attr("", "y", "0"))
                        .with_child(ext),
                )
                .with_child(Element::new(ns::A, "prstGeom").with_attr("", "prst", "rect").with_child(Element::new(ns::A, "avLst"))),
        );
    let inline = ["distT", "distB", "distL", "distR"]
        .iter()
        .fold(Element::new(ns::WP, "inline"), |el, k| el.with_attr("", k, "0"))
        .with_child(extent)
        .with_child(doc_pr)
        .with_child(Element::new(ns::WP, "cNvGraphicFramePr").with_child(Element::new(ns::A, "graphicFrameLocks").with_attr(
            "",
            "noChangeAspect",
            "1",
        )))
        .with_child(
            Element::new(ns::A, "graphic").with_child(Element::new(ns::A, "graphicData").with_attr("", "uri", PICTURE_URI).with_child(pic)),
        );
    Ok(Element::new(ns::W, "r").with_child(Element::new(ns::W, "drawing").with_child(inline)))
}

fn descend<'e>(el: &'e mut Element, path: &[(&str, &str)]) -> Option<&'e mut Element> {
    path.iter().try_fold(el, |e, (n, l)| e.child_mut(n, l))
}

/// The children of `w:tcPr`, in schema order.
const TC_PR: &[&[&str]] = &[
    &["cnfStyle"],
    &["tcW"],
    &["gridSpan"],
    &["hMerge"],
    &["vMerge"],
    &["tcBorders"],
    &["shd"],
    &["noWrap"],
    &["tcMar"],
    &["textDirection"],
    &["tcFitText"],
    &["vAlign"],
    &["hideMark"],
    &["headers"],
    &["cellIns"],
    &["cellDel"],
    &["cellMerge"],
    &["tcPrChange"],
];

/// Replace the reader's slots in order, keeping unmodeled siblings in place. Removed model
/// children drop their slots; added ones follow the last slot, before any trailing raw XML.
fn fill_table_slots(parent: &mut Element, local: &str, children: Vec<Element>) {
    let last = parent.children.iter().rposition(|n| matches!(n, Node::Element(e) if e.is(ns::W, local)));
    let mut children = children.into_iter();
    let old = std::mem::take(&mut parent.children);
    for (i, node) in old.into_iter().enumerate() {
        if matches!(&node, Node::Element(e) if e.is(ns::W, local)) {
            if let Some(child) = children.next() {
                parent.push(child);
            }
        } else {
            parent.children.push(node);
        }
        if last == Some(i) {
            parent.children.extend(children.by_ref().map(Node::Element));
        }
    }
    parent.children.extend(children.map(Node::Element));
}

fn write_table(table: &Table, ctx: &mut Ctx) -> Result<Element> {
    let mut tbl = origin_as(&table.origin, ns::W, "tbl").unwrap_or_else(|| {
        let border = |side: &str| {
            Element::new(ns::W, side)
                .with_attr(ns::W, "val", "single")
                .with_attr(ns::W, "sz", "4")
                .with_attr(ns::W, "space", "0")
                .with_attr(ns::W, "color", "auto")
        };
        let borders = ["top", "left", "bottom", "right", "insideH", "insideV"]
            .iter()
            .fold(Element::new(ns::W, "tblBorders"), |el, s| el.with_child(border(s)));
        Element::new(ns::W, "tbl").with_child(
            Element::new(ns::W, "tblPr")
                .with_child(Element::new(ns::W, "tblW").with_attr(ns::W, "w", "0").with_attr(ns::W, "type", "auto"))
                .with_child(borders),
        )
    });
    let pr = tbl.child_or_insert(ns::W, "tblPr");
    // PowerPoint's style GUIDs aren't Word's style ids.
    let style = table.style.clone().filter(|s| !s.starts_with('{'));
    if pr.child(ns::W, "tblStyle").and_then(val).map(str::to_string) != style {
        pr.remove_children(ns::W, "tblStyle");
        if let Some(s) = style {
            pr.children.insert(0, Node::Element(Element::new(ns::W, "tblStyle").with_attr(ns::W, "val", s)));
        }
    }
    let grid = crate::formats::common::grid(
        tbl.child(ns::W, "tblGrid"),
        &table.columns,
        || Element::new(ns::W, "tblGrid"),
        |col, w| {
            let mut col = col.unwrap_or_else(|| Element::new(ns::W, "gridCol"));
            if col.attr_ns(ns::W, "w").and_then(|v| Emu::parse(v, Emu::PER_TWIP)) != Some(w) {
                col.set_attr(ns::W, "w", w.twips().to_string());
            }
            col
        },
    );
    tbl.place(grid, &[&["tblPr"], &["tblGrid"], &["tr"]]);

    let mut rows = Vec::new();
    for (r, row) in table.rows.iter().enumerate() {
        let mut tr = origin_as(&row.origin, ns::W, "tr").unwrap_or_else(|| Element::new(ns::W, "tr"));
        let trpr_now = tr.child(ns::W, "trPr");
        let height_now = trpr_now
            .and_then(|p| p.child(ns::W, "trHeight"))
            .and_then(|h| h.attr_ns(ns::W, "val"))
            .and_then(|v| Emu::parse(v, Emu::PER_TWIP));
        let header_now = trpr_now.and_then(|p| super::props::toggle(p, "tblHeader")).unwrap_or(false);
        if height_now != row.height || header_now != row.header {
            let trpr = tr.child_or_insert(ns::W, "trPr");
            if height_now != row.height {
                trpr.remove_children(ns::W, "trHeight");
                if let Some(h) = row.height {
                    trpr.push(Element::new(ns::W, "trHeight").with_attr(ns::W, "val", h.twips().to_string()));
                }
            }
            if header_now != row.header {
                trpr.remove_children(ns::W, "tblHeader");
                if row.header {
                    trpr.push(Element::new(ns::W, "tblHeader"));
                }
            }
            // trPr comes first in a row, after tblPrEx.
            if let Some(i) = tr.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(ns::W, "trPr"))) {
                let trpr = tr.children.remove(i);
                let at =
                    tr.children.iter().position(|n| matches!(n, Node::Element(e) if !e.is(ns::W, "tblPrEx"))).unwrap_or(tr.children.len());
                tr.children.insert(at, trpr);
            }
        }
        let omitted = |local: &str| {
            tr.child(ns::W, "trPr")
                .and_then(|pr| pr.child(ns::W, local))
                .and_then(val)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
                .min(row.cells.len())
        };
        let old_before = omitted("gridBefore");
        let old_after = omitted("gridAfter");
        // Materialize omitted slots if the caller put content or formatting in them.
        let empty = crate::model::Cell::default();
        let before = row.cells.iter().take(old_before).take_while(|cell| **cell == empty).count();
        let after = row.cells.iter().rev().take(old_after).take_while(|cell| **cell == empty).count();
        for (name, old, new) in [("gridBefore", old_before, before), ("gridAfter", old_after, after)] {
            if old != new {
                let pr = tr.child_or_insert(ns::W, "trPr");
                pr.remove_children(ns::W, name);
                if new > 0 {
                    pr.push(Element::new(ns::W, name).with_attr(ns::W, "val", new.to_string()));
                }
            }
        }
        let mut cells = Vec::new();
        let mut c = before;
        let end = row.cells.len().saturating_sub(after);
        while c < end {
            let cell = &row.cells[c];
            // Word stores no cell under a column span: skip what a cell to the left covers.
            let (or, oc) = table.owner(r, c).unwrap_or((r, c));
            if cell.covered && or == r {
                c += 1;
                continue;
            }
            let owner = &table.rows[or].cells[oc];
            let span = if cell.covered { owner.col_span } else { cell.col_span };
            let mut tc =
                origin_as(&cell.origin, ns::W, "tc").unwrap_or_else(|| {
                    let width: i64 = table.columns.iter().skip(c).take(span as usize).map(|w| w.twips()).sum();
                    Element::new(ns::W, "tc").with_child(
                        Element::new(ns::W, "tcPr").with_child(
                            Element::new(ns::W, "tcW").with_attr(ns::W, "w", width.to_string()).with_attr(ns::W, "type", "dxa"),
                        ),
                    )
                });
            let tcpr = tc.child_or_insert(ns::W, "tcPr");
            let span_now: u32 = tcpr.child(ns::W, "gridSpan").and_then(val).and_then(|v| v.parse().ok()).unwrap_or(1);
            if span_now != span {
                tcpr.clear(ns::W, &["gridSpan"]);
                if span > 1 {
                    tcpr.place(Element::new(ns::W, "gridSpan").with_attr(ns::W, "val", span.to_string()), TC_PR);
                }
            }
            let merge = if cell.covered {
                Some(None)
            } else if cell.row_span > 1 {
                Some(Some("restart"))
            } else {
                None
            };
            let merge_now = tcpr.child(ns::W, "vMerge").map(|m| val(m).filter(|v| *v == "restart"));
            if merge_now != merge {
                tcpr.clear(ns::W, &["vMerge"]);
                if let Some(v) = merge {
                    let mut m = Element::new(ns::W, "vMerge");
                    if let Some(v) = v {
                        m.set_attr(ns::W, "val", v);
                    }
                    tcpr.place(m, TC_PR);
                }
            }
            let fill_now = tcpr
                .child(ns::W, "shd")
                .and_then(|s| s.attr_ns(ns::W, "fill"))
                .and_then(crate::drawing::Rgb::parse)
                .map(crate::drawing::Color::rgb);
            if fill_now != cell.fill {
                tcpr.clear(ns::W, &["shd"]);
                if let Some(color) = &cell.fill {
                    let hex = color.resolve(&|n| ctx.theme.scheme_color(n)).rgb.hex();
                    tcpr.place(
                        Element::new(ns::W, "shd").with_attr(ns::W, "val", "clear").with_attr(ns::W, "color", "auto").with_attr(
                            ns::W,
                            "fill",
                            hex,
                        ),
                        TC_PR,
                    );
                }
            }
            let mut blocks = Vec::new();
            for b in &cell.blocks {
                write_block(b, ctx, &mut blocks)?;
            }
            // A cell must end with a paragraph.
            if !blocks.last().is_some_and(|b| b.is(ns::W, "p")) {
                blocks.push(Element::new(ns::W, "p"));
            }
            tc.children.extend(blocks.into_iter().map(Node::Element));
            cells.push(tc);
            c += span.max(1) as usize;
        }
        fill_table_slots(&mut tr, "tc", cells);
        rows.push(tr);
    }
    fill_table_slots(&mut tbl, "tr", rows);
    Ok(tbl)
}

/// A new package with what Word needs: the document with an A4 page, a theme, and styles that
/// give text Word's usual look (Calibri 11 from the theme, 8 pt after each paragraph).
fn scaffold(theme: &Theme) -> Result<Package> {
    let mut pkg = Package::new();
    let main = "word/document.xml";
    let mut root = Element::new(ns::W, "document");
    for (p, uri) in [("w", ns::W), ("r", ns::R), ("wp", ns::WP), ("a", ns::A), ("pic", ns::PIC)] {
        root.declare(Some(p), uri);
    }
    let w = |local: &str, attrs: &[(&str, &str)]| attrs.iter().fold(Element::new(ns::W, local), |el, (k, v)| el.with_attr(ns::W, k, *v));
    let sect = Element::new(ns::W, "sectPr")
        .with_child(w("pgSz", &[("w", "11906"), ("h", "16838")]))
        .with_child(w(
            "pgMar",
            &[
                ("top", "1440"),
                ("right", "1440"),
                ("bottom", "1440"),
                ("left", "1440"),
                ("header", "708"),
                ("footer", "708"),
                ("gutter", "0"),
            ],
        ))
        .with_child(w("cols", &[("space", "708")]));
    root.push(Element::new(ns::W, "body").with_child(sect));
    pkg.put_xml(main, &XmlDoc::new(root), types::DOCX_MAIN);
    pkg.relate("", kind::OFFICE_DOCUMENT, main)?;

    pkg.put_xml("word/styles.xml", &default_styles(), types::DOCX_STYLES);
    pkg.relate(main, kind::STYLES, "word/styles.xml")?;
    pkg.put_xml("word/theme/theme1.xml", &theme.to_xml(), types::THEME);
    pkg.relate(main, kind::THEME, "word/theme/theme1.xml")?;
    Ok(pkg)
}

/// The style sheet a new document gets: Word's defaults (Calibri 11 from the theme, 8 pt after
/// each paragraph, 1.08 line spacing) and the styles every document has.
pub fn default_styles() -> XmlDoc {
    let w = |local: &str, attrs: &[(&str, &str)]| attrs.iter().fold(Element::new(ns::W, local), |el, (k, v)| el.with_attr(ns::W, k, *v));
    let fonts = w(
        "rFonts",
        &[("asciiTheme", "minorHAnsi"), ("hAnsiTheme", "minorHAnsi"), ("eastAsiaTheme", "minorEastAsia"), ("cstheme", "minorBidi")],
    );
    let defaults =
        Element::new(ns::W, "docDefaults")
            .with_child(
                Element::new(ns::W, "rPrDefault").with_child(
                    Element::new(ns::W, "rPr")
                        .with_child(fonts)
                        .with_child(w("sz", &[("val", "22")]))
                        .with_child(w("szCs", &[("val", "22")]))
                        .with_child(w("lang", &[("val", "en-US"), ("eastAsia", "en-US"), ("bidi", "ar-SA")])),
                ),
            )
            .with_child(Element::new(ns::W, "pPrDefault").with_child(
                Element::new(ns::W, "pPr").with_child(w("spacing", &[("after", "160"), ("line", "259"), ("lineRule", "auto")])),
            ));
    let style = |kind: &str, id: &str, name: &str, default: bool| {
        let mut s = w("style", &[("type", kind), ("styleId", id)]);
        if default {
            s.set_attr(ns::W, "default", "1");
        }
        s.with_child(w("name", &[("val", name)]))
    };
    let mut styles = Element::new(ns::W, "styles");
    styles.declare(Some("w"), ns::W);
    let table_normal = style("table", "TableNormal", "Normal Table", true).with_child(
        Element::new(ns::W, "tblPr").with_child(w("tblInd", &[("w", "0"), ("type", "dxa")])).with_child(
            Element::new(ns::W, "tblCellMar")
                .with_child(w("top", &[("w", "0"), ("type", "dxa")]))
                .with_child(w("left", &[("w", "108"), ("type", "dxa")]))
                .with_child(w("bottom", &[("w", "0"), ("type", "dxa")]))
                .with_child(w("right", &[("w", "108"), ("type", "dxa")])),
        ),
    );
    let hyperlink =
        style("character", "Hyperlink", "Hyperlink", false).with_child(w("basedOn", &[("val", "DefaultParagraphFont")])).with_child(
            Element::new(ns::W, "rPr")
                .with_child(w("color", &[("val", "0563C1"), ("themeColor", "hyperlink")]))
                .with_child(w("u", &[("val", "single")])),
        );
    styles = styles
        .with_child(defaults)
        .with_child(style("paragraph", "Normal", "Normal", true).with_child(Element::new(ns::W, "qFormat")))
        .with_child(style("character", "DefaultParagraphFont", "Default Paragraph Font", true))
        .with_child(table_normal)
        .with_child(hyperlink);
    XmlDoc::new(styles)
}
