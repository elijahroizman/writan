use super::is_tree_props;
use crate::drawing::{ShapeProps, Xfrm};
use crate::formats::common::{expect_root, read_theme, rel_attr, shell, PartRels};
use crate::formats::dml::{read_paragraph, read_text_body};
use crate::model::{Block, Body, Cell, Document, Frame, FrameContent, Image, Kind, Placeholder, Row, Slide, SlideSize, Table};
use crate::opc::{Package, Properties};
use crate::units::{parse_bool, Emu};
use crate::xml::{ns, Element};
use crate::Result;

const TABLE_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/table";

pub fn read(pkg: Package) -> Result<Document> {
    let main = pkg.main_part()?;
    let pres = pkg.xml(&main)?;
    expect_root(&pres.root, &main, ns::P, "presentation")?;
    let rels = PartRels::new(&pkg, &main)?;
    let mut doc = Document::new(Kind::Slides);
    doc.properties = Properties::read(&pkg)?;

    // The first master's theme: what new slides, and slides on that master, are drawn with.
    let master = pres.root.path(ns::P, &["sldMasterIdLst", "sldMasterId"]).and_then(|m| rel_attr(m, "id")).and_then(|id| rels.part(id));
    doc.theme = match master {
        Some(m) if pkg.has(&m) => read_theme(&pkg, &m)?,
        _ => None,
    };

    let size = pres.root.child(ns::P, "sldSz").map(|s| {
        let n = |k: &str| s.attr(k).and_then(|v| v.parse().ok()).map(Emu);
        SlideSize { width: n("cx").unwrap_or(SlideSize::default().width), height: n("cy").unwrap_or(SlideSize::default().height) }
    });

    let mut slides = Vec::new();
    for sld in pres.root.path(ns::P, &["sldIdLst"]).into_iter().flat_map(|l| l.children_named(ns::P, "sldId")) {
        let Some(part) = rel_attr(sld, "id").and_then(|id| rels.part(id)).filter(|p| pkg.has(p)) else { continue };
        slides.push(read_slide(&pkg, &part, &mut doc)?);
    }
    doc.body = Body::Slides { size: size.unwrap_or_default(), slides };
    doc.origin = shell(&pres.root, |_| false);
    doc.package = Some(pkg);
    Ok(doc)
}

/// The frames on any slide-like part (a layout's or master's shapes, for drawing them behind
/// slides), with pictures' media registered on `doc`.
pub fn read_part_frames(pkg: &Package, part: &str, doc: &mut Document) -> Result<Vec<Frame>> {
    let xml = pkg.xml(part)?;
    let rels = PartRels::new(pkg, part)?;
    let tree = xml.root.path(ns::P, &["cSld", "spTree"]);
    Ok(tree.map(|t| t.elements().filter(|e| !is_tree_props(e)).map(|e| read_frame(e, &rels, doc)).collect()).unwrap_or_default())
}

/// The shapes of the SmartArt diagram in `graphic_data` (in `part`) as frames, positioned from
/// the diagram's top left. See [`crate::formats::diagram::shapes`].
pub fn diagram_frames(pkg: &Package, part: &str, graphic_data: &Element, size: (Emu, Emu)) -> Vec<Frame> {
    let shapes = crate::formats::diagram::shapes(pkg, part, graphic_data, size);
    let Ok(rels) = PartRels::new(pkg, part) else { return Vec::new() };
    // Diagram shapes have no pictures of their own to register; their media stay unread.
    let mut scratch = Document::new(Kind::Slides);
    shapes.iter().map(|e| read_frame(e, &rels, &mut scratch)).collect()
}

fn read_slide(pkg: &Package, part: &str, doc: &mut Document) -> Result<Slide> {
    let xml = pkg.xml(part)?;
    let rels = PartRels::new(pkg, part)?;
    let root = &xml.root;
    let c_sld = root.child(ns::P, "cSld");
    let tree = c_sld.and_then(|c| c.child(ns::P, "spTree"));
    let frames = tree.map(|t| t.elements().filter(|e| !is_tree_props(e)).map(|e| read_frame(e, &rels, doc)).collect()).unwrap_or_default();

    // The slide without its shapes: background, timing, transition and the tree's own properties stay.
    let mut origin = root.clone();
    if let Some(tree) = origin.child_mut(ns::P, "cSld").and_then(|c| c.child_mut(ns::P, "spTree")) {
        let kept = shell(tree, |e| !is_tree_props(e));
        *tree = kept.get().unwrap().clone();
    }
    Ok(Slide {
        frames,
        name: c_sld.and_then(|c| c.attr("name")).filter(|n| !n.is_empty()).map(str::to_string),
        hidden: root.attr("show").and_then(parse_bool) == Some(false),
        part: Some(part.to_string()),
        relationship_source: None,
        layout: pkg.related(part, crate::opc::rels::kind::SLIDE_LAYOUT)?.into_iter().next().map(|(_, l)| l),
        origin: crate::model::Origin::new(origin),
    })
}

fn read_frame(el: &Element, rels: &PartRels, doc: &mut Document) -> Frame {
    // The non-visual properties: p:nvSpPr, p:nvPicPr, p:nvGraphicFramePr, p:nvGrpSpPr, p:nvCxnSpPr.
    let nv = el.elements().find(|e| e.in_ns(ns::P) && e.local.starts_with("nv"));
    let c_nv = nv.and_then(|n| n.child(ns::P, "cNvPr")).or_else(|| el.descendant(ns::P, "cNvPr"));
    let placeholder = nv
        .and_then(|n| n.path(ns::P, &["nvPr", "ph"]))
        .map(|ph| Placeholder { kind: ph.attr("type").map(str::to_string), index: ph.attr("idx").and_then(|i| i.parse().ok()) });
    let mut frame = Frame {
        id: c_nv.and_then(|c| c.attr("id")).and_then(|i| i.parse().ok()).unwrap_or(0),
        name: c_nv.and_then(|c| c.attr("name")).unwrap_or("").to_string(),
        description: c_nv.and_then(|c| c.attr("descr")).map(str::to_string),
        hidden: c_nv.and_then(|c| c.attr("hidden")).and_then(parse_bool).unwrap_or(false),
        shape: ShapeProps::default(),
        placeholder,
        link: c_nv.and_then(|c| crate::formats::dml::read_link(c, Some(rels))),
        // A group's frames are read below. Copying its whole tree here as well, at every level,
        // made nested groups quadratic.
        content: FrameContent::Raw { xml: if el.is(ns::P, "grpSp") { Element::new("", "grpSp") } else { el.clone() }, chart_part: None },
        origin: Default::default(),
    };
    if !el.in_ns(ns::P) {
        // mc:AlternateContent and other wrappers stay whole.
        return frame;
    }
    let sp_pr = el.child(ns::P, "spPr").or_else(|| el.child(ns::P, "grpSpPr"));
    if let Some(sp_pr) = sp_pr {
        frame.shape = ShapeProps::parse(sp_pr);
    }
    match el.local.as_str() {
        "sp" | "cxnSp" => {
            frame.content = FrameContent::Shape {
                text: el.child(ns::P, "txBody").map(|t| read_text_body(t, Some(rels))),
                connector: el.local == "cxnSp",
            };
            frame.origin = shell(el, |c| c.is(ns::P, "txBody"));
        }
        "pic" => {
            let blip = el.path(ns::P, &["blipFill"]).and_then(|b| b.child(ns::A, "blip"));
            let media = blip.and_then(|b| rel_attr(b, "embed")).and_then(|id| rels.media(doc, id));
            let Some(media) = media else {
                // A linked (not embedded) picture, or a broken one: keep it as it is.
                frame.content = FrameContent::Raw { xml: el.clone(), chart_part: None };
                return frame;
            };
            let xfrm = frame.shape.xfrm.clone().unwrap_or_default();
            frame.content = FrameContent::Picture {
                image: Image {
                    media,
                    width: xfrm.width,
                    height: xfrm.height,
                    name: frame.name.clone(),
                    description: frame.description.clone(),
                    origin: Default::default(),
                },
            };
            frame.origin = shell(el, |_| false);
        }
        "graphicFrame" => {
            frame.shape.xfrm = el.child(ns::P, "xfrm").map(Xfrm::parse);
            let data = el
                .path(ns::A, &["graphic", "graphicData"])
                .or_else(|| el.child(ns::A, "graphic").and_then(|g| g.child(ns::A, "graphicData")));
            if let Some(tbl) = data.filter(|d| d.attr("uri") == Some(TABLE_URI)).and_then(|d| d.child(ns::A, "tbl")) {
                frame.content = FrameContent::Table { table: read_table(tbl, rels) };
                // The frame without its table; the writer puts the table back in a:graphicData.
                let mut origin = el.clone();
                if let Some(data) = origin.child_mut(ns::A, "graphic").and_then(|g| g.child_mut(ns::A, "graphicData")) {
                    data.remove_children(ns::A, "tbl");
                }
                frame.origin = crate::model::Origin::new(origin);
            }
            // Charts, diagrams and embedded objects stay raw.
        }
        "grpSp" => {
            let frames = el
                .elements()
                .filter(|e| !(e.is(ns::P, "nvGrpSpPr") || e.is(ns::P, "grpSpPr") || e.is(ns::P, "extLst")))
                .map(|e| read_frame(e, rels, doc))
                .collect();
            frame.content = FrameContent::Group { frames };
            frame.origin = shell(el, |e| !(e.is(ns::P, "nvGrpSpPr") || e.is(ns::P, "grpSpPr") || e.is(ns::P, "extLst")));
        }
        _ => {}
    }
    frame
}

fn read_table(tbl: &Element, rels: &PartRels) -> Table {
    let columns = tbl
        .child(ns::A, "tblGrid")
        .map(|g| g.children_named(ns::A, "gridCol").map(|c| Emu(c.attr("w").and_then(|w| w.parse().ok()).unwrap_or(0))).collect())
        .unwrap_or_default();
    let style = tbl.path(ns::A, &["tblPr", "tableStyleId"]).map(Element::text);
    let rows = tbl
        .children_named(ns::A, "tr")
        .enumerate()
        .map(|(i, tr)| Row {
            cells: tr.children_named(ns::A, "tc").map(|tc| read_cell(tc, rels)).collect(),
            height: tr.attr("h").and_then(|h| h.parse().ok()).map(Emu),
            // PowerPoint styles the first row as a header when the table says so.
            header: i == 0 && tbl.child(ns::A, "tblPr").and_then(|p| p.attr("firstRow")).and_then(parse_bool).unwrap_or(false),
            origin: shell(tr, |c| c.is(ns::A, "tc")),
        })
        .collect();
    // The origin keeps the grid, for what its columns carry besides their widths (ids in an extLst).
    Table { columns, rows, style, origin: shell(tbl, |c| c.is(ns::A, "tr")) }
}

/// More rows or columns than any table PowerPoint makes.
const MAX_SPAN: u32 = 1 << 16;

fn read_cell(tc: &Element, rels: &PartRels) -> Cell {
    // A span is at least the cell itself; a span past any real table is a damaged file.
    let n = |k: &str| tc.attr(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(1).clamp(1, MAX_SPAN);
    let flag = |k: &str| tc.attr(k).and_then(parse_bool).unwrap_or(false);
    let body = tc.child(ns::A, "txBody");
    let blocks =
        body.map(|b| b.children_named(ns::A, "p").map(|p| Block::Paragraph(read_paragraph(p, Some(rels)))).collect()).unwrap_or_default();
    let fill = tc.child(ns::A, "tcPr").and_then(|p| p.child(ns::A, "solidFill")).and_then(crate::drawing::Color::in_drawingml);
    // The cell without its paragraphs; its body's settings and tcPr stay.
    let mut origin = tc.clone();
    if let Some(body) = origin.child_mut(ns::A, "txBody") {
        body.remove_children(ns::A, "p");
    }
    let covered = flag("hMerge") || flag("vMerge");
    Cell {
        blocks,
        // Covered cells repeat span attributes in XML; only the owner carries them in the model.
        col_span: if covered { 1 } else { n("gridSpan") },
        row_span: if covered { 1 } else { n("rowSpan") },
        covered,
        fill,
        origin: crate::model::Origin::new(origin),
    }
}
