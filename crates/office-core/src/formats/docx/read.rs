use super::props::{read_para_props, read_run_props, val};
use crate::formats::common::{expect_root, read_theme, rel_attr, shell, PartRels};
use crate::model::{Block, Body, BreakKind, Cell, Document, Image, Inline, Kind, Link, Origin, Paragraph, Row, Run, Table};
use crate::opc::{Package, Properties};
use crate::units::Emu;
use crate::xml::{ns, Element, Node};
use crate::Result;

/// A run keeps its own origin for the Word layer; its hyperlink wrapper is shared here.
/// Keeping the run origin alive also prevents its allocation address from being reused.
#[derive(Clone, Debug, Default)]
pub(super) struct Hyperlinks(std::collections::HashMap<usize, (Origin, Origin)>);

pub(super) fn hyperlink_origin<'a>(doc: &'a Document, run: &Run) -> Option<&'a Element> {
    let key = std::sync::Arc::as_ptr(run.origin.0.as_ref()?) as usize;
    doc.extensions.get::<Hyperlinks>()?.0.get(&key)?.1.get()
}

pub fn read(pkg: Package) -> Result<Document> {
    let main = pkg.main_part()?;
    let xml = pkg.xml_shared(&main)?;
    expect_root(&xml.root, &main, ns::W, "document")?;
    let rels = PartRels::new(&pkg, &main)?;
    let mut doc = Document::new(Kind::Word);
    doc.properties = Properties::read(&pkg)?;
    doc.theme = read_theme(&pkg, &main)?;

    let body = xml.root.child(ns::W, "body");
    let blocks = body.map(|b| read_blocks(b, &rels, &mut doc, false, |e| e.is(ns::W, "sectPr"))).unwrap_or_default();
    doc.body = Body::Flow { blocks };
    // The document without the body's blocks; the final section properties stay at its end.
    let mut origin = shell(&xml.root, |e| e.is(ns::W, "body")).0.map(|a| (*a).clone()).unwrap_or_else(|| Element::new(ns::W, "document"));
    if let Some(body_el) = body {
        if let Some(kept) = shell(body_el, |e| !e.is(ns::W, "sectPr")).0 {
            origin.children.push(Node::Element((*kept).clone()));
        }
    }
    doc.origin = Origin::new(origin);
    doc.package = Some(pkg);
    Ok(doc)
}

/// The blocks in `container`, an element of `part` holding paragraphs and tables the way
/// `w:body` does (`w:hdr`, `w:ftr`, a `w:footnote`, a `w:comment`). For the Word layer.
pub fn read_blocks_in(doc: &mut Document, part: &str, container: &Element) -> Result<Vec<Block>> {
    let pkg = doc.package.take().ok_or_else(|| crate::Error::MissingPart(part.to_string()))?;
    let result =
        PartRels::new(&pkg, part).map(|rels| read_blocks(container, &rels, doc, false, |e| e.is(ns::W, "pPr") || e.is(ns::W, "tcPr")));
    doc.package = Some(pkg);
    result
}

/// What's in `container` read as a paragraph's content, for markup that wraps runs the way a
/// paragraph does (`w:ins`, `w:moveTo`, a content control's `w:sdtContent`). For the Word layer.
pub fn read_inlines_in(doc: &mut Document, part: &str, container: &Element) -> Result<Vec<Inline>> {
    let pkg = doc.package.take().ok_or_else(|| crate::Error::MissingPart(part.to_string()))?;
    let mut p = Element::new(ns::W, "p");
    p.children = container.children.clone();
    let result = PartRels::new(&pkg, part).map(|rels| read_paragraph(&p, &rels, doc, false).content);
    doc.package = Some(pkg);
    result
}

/// The block-level children of `w:body`, a table cell, or another container. `skip` says which
/// children belong to the container itself.
fn read_blocks(parent: &Element, rels: &PartRels, doc: &mut Document, preview: bool, skip: impl Fn(&Element) -> bool) -> Vec<Block> {
    let mut blocks = Vec::new();
    for e in parent.elements().filter(|e| !skip(e)) {
        read_block(e, rels, doc, preview, &mut blocks);
    }
    blocks
}

fn read_block(e: &Element, rels: &PartRels, doc: &mut Document, preview: bool, out: &mut Vec<Block>) {
    if e.is(ns::W, "p") {
        out.push(Block::Paragraph(read_paragraph(e, rels, doc, preview)));
    } else if e.is(ns::W, "tbl") {
        out.push(Block::Table(read_table(e, rels, doc, preview)));
    } else {
        let content = e.is(ns::W, "sdt").then(|| e.child(ns::W, "sdtContent")).flatten();
        if !preview {
            let preview = content.map(|c| read_blocks(c, rels, doc, true, |_| false).into()).unwrap_or_default();
            out.push(Block::Raw { xml: e.clone(), preview });
        } else if let Some(content) = content.or_else(|| e.is(ns::W, "customXml").then_some(e)) {
            // Flatten supported wrappers instead of copying their raw subtrees.
            for child in content.elements() {
                read_block(child, rels, doc, true, out);
            }
        }
    }
}

fn read_paragraph(p: &Element, rels: &PartRels, doc: &mut Document, preview: bool) -> Paragraph {
    let ppr = p.child(ns::W, "pPr");
    let mut content = Vec::new();
    for child in p.elements() {
        if child.is(ns::W, "pPr") {
            continue;
        }
        if child.is(ns::W, "r") {
            read_run(child, None, rels, doc, &mut content);
        } else if child.is(ns::W, "hyperlink") && child.elements().all(|r| r.is(ns::W, "r") && is_plain_run(r)) {
            let link = Link {
                url: rel_attr(child, "id").and_then(|id| rels.url(id)),
                anchor: child.attr_ns(ns::W, "anchor").map(str::to_string),
                tooltip: child.attr_ns(ns::W, "tooltip").map(str::to_string),
            };
            let wrapper = shell(child, |_| true);
            if doc.extensions.get::<Hyperlinks>().is_none() {
                doc.extensions.insert(Hyperlinks::default());
            }
            for r in child.elements() {
                let start = content.len();
                read_run(r, Some(&link), rels, doc, &mut content);
                for inline in &content[start..] {
                    if let Inline::Text(run) = inline {
                        let key = std::sync::Arc::as_ptr(run.origin.0.as_ref().unwrap()) as usize;
                        doc.extensions.get_mut::<Hyperlinks>().unwrap().0.insert(key, (run.origin.clone(), wrapper.clone()));
                    }
                }
            }
        } else if preview && child.is(ns::W, "hyperlink") {
            // TOC hyperlinks can wrap both plain runs and PAGEREF fields. Keep the runs
            // separate so their tabs, formatting and cached field results can be laid out.
            for r in child.elements() {
                if r.is(ns::W, "r") {
                    read_run(r, None, rels, doc, &mut content);
                } else {
                    content.push(Inline::Raw { xml: r.clone() });
                }
            }
        } else {
            // Bookmarks, fields, tracked changes, comment marks, equations, content controls…
            content.push(Inline::Raw { xml: child.clone() });
        }
    }
    Paragraph {
        props: ppr.map(read_para_props).unwrap_or_default(),
        content,
        mark: ppr.and_then(|p| p.child(ns::W, "rPr")).map(read_run_props),
        origin: shell(p, |c| !c.is(ns::W, "pPr")),
    }
}

/// Run children the model turns into text.
fn is_text_child(e: &Element) -> bool {
    e.in_ns(ns::W)
        && matches!(e.local.as_str(), "rPr" | "t" | "tab" | "br" | "cr" | "noBreakHyphen" | "softHyphen" | "lastRenderedPageBreak")
}

/// A run of only text (and the breaks and tabs the model writes as characters).
fn is_plain_run(r: &Element) -> bool {
    r.elements().all(is_text_child)
}

/// A run with an inline picture and nothing else but formatting.
fn inline_picture(r: &Element) -> Option<&Element> {
    let mut drawings = r.elements().filter(|e| !e.is(ns::W, "rPr"));
    let drawing = drawings.next().filter(|d| d.is(ns::W, "drawing"))?;
    if drawings.next().is_some() {
        return None;
    }
    let inline = drawing.child(ns::WP, "inline")?;
    inline.descendant(ns::PIC, "pic").and(Some(inline))
}

/// A plain run's text and formatting, if it reads as one model run (no page or column break
/// splits it): what the writer compares a run with to see if it can reuse the run's origin.
pub(super) fn single_run(r: &Element) -> Option<(String, crate::model::RunProps)> {
    if !r.is(ns::W, "r")
        || !is_plain_run(r)
        || r.children_named(ns::W, "br").any(|b| matches!(b.attr_ns(ns::W, "type"), Some("page" | "column")))
    {
        return None;
    }
    let mut out = Vec::new();
    read_text_run(r, None, &mut out);
    match out.pop() {
        Some(Inline::Text(run)) if out.is_empty() => Some((run.text, run.props)),
        _ => None,
    }
}

fn read_run(r: &Element, link: Option<&Link>, rels: &PartRels, doc: &mut Document, out: &mut Vec<Inline>) {
    if let Some(inline) = inline_picture(r) {
        if let Some(image) = read_inline_picture(r, inline, rels, doc) {
            out.push(Inline::Image(image));
            return;
        }
    }
    if !is_plain_run(r) {
        out.push(Inline::Raw { xml: r.clone() });
        return;
    }
    read_text_run(r, link, out);
}

fn read_text_run(r: &Element, link: Option<&Link>, out: &mut Vec<Inline>) {
    let props = r.child(ns::W, "rPr").map(read_run_props).unwrap_or_default();
    // A run's text, split where a page or column break sits inside it. The whole run is the
    // origin, so the writer can put an untouched run back exactly as it was.
    let origin = Origin::new(r.clone());
    let push = |text: &mut String, out: &mut Vec<Inline>| {
        if !text.is_empty() {
            out.push(Inline::Text(Run { text: std::mem::take(text), props: props.clone(), link: link.cloned(), origin: origin.clone() }));
        }
    };
    let mut text = String::new();
    let mut any = false;
    for c in r.elements() {
        match c.local.as_str() {
            "t" => text.push_str(&c.text()),
            "tab" => text.push('\t'),
            "cr" => text.push('\n'),
            "noBreakHyphen" => text.push('\u{2011}'),
            "softHyphen" => text.push('\u{AD}'),
            "br" => match c.attr_ns(ns::W, "type") {
                Some(kind @ ("page" | "column")) => {
                    push(&mut text, out);
                    out.push(Inline::Break {
                        kind: if kind == "page" { BreakKind::Page } else { BreakKind::Column },
                        origin: origin.clone(),
                    });
                    any = true;
                    continue;
                }
                _ => text.push('\n'),
            },
            // Word's note of where the last layout broke the page; Word writes it again.
            _ => continue,
        }
        any = true;
    }
    if !text.is_empty() || !any {
        // An empty run still carries formatting someone put there; keep it.
        out.push(Inline::Text(Run { text, props, link: link.cloned(), origin }));
    }
}

fn read_inline_picture(r: &Element, inline: &Element, rels: &PartRels, doc: &mut Document) -> Option<Image> {
    let blip = inline.descendant(ns::A, "blip")?;
    let media = rels.media(doc, rel_attr(blip, "embed")?)?;
    let extent = inline.child(ns::WP, "extent");
    let size = |k: &str| extent.and_then(|e| e.attr(k)).and_then(|v| v.parse().ok()).map(Emu).unwrap_or_default();
    let doc_pr = inline.child(ns::WP, "docPr");
    Some(Image {
        media,
        width: size("cx"),
        height: size("cy"),
        name: doc_pr.and_then(|d| d.attr("name")).unwrap_or("").to_string(),
        description: doc_pr.and_then(|d| d.attr("descr")).map(str::to_string),
        origin: Origin::new(r.clone()),
    })
}

/// The most grid slots spans fill in a table row: Word's limit on columns.
const MAX_ROW_SLOTS: usize = 63;

/// Keep empty slots for modeled children so raw content controls, revision markers and other
/// siblings stay on the same side of each cell or row when the table is written back.
fn table_shell(el: &Element, slot: &str) -> Origin {
    let mut copy =
        Element { prefix: el.prefix.clone(), local: el.local.clone(), ns: el.ns.clone(), attrs: el.attrs.clone(), children: Vec::new() };
    for n in &el.children {
        match n {
            Node::Element(e) if e.is(ns::W, slot) => {
                copy.push(Element::new(ns::W, slot));
            }
            Node::Text(t) if t.trim().is_empty() => {}
            n => copy.children.push(n.clone()),
        }
    }
    Origin::new(copy)
}

fn read_table(tbl: &Element, rels: &PartRels, doc: &mut Document, preview: bool) -> Table {
    let columns: Vec<Emu> = tbl
        .child(ns::W, "tblGrid")
        .map(|g| {
            g.children_named(ns::W, "gridCol")
                .map(|c| c.attr_ns(ns::W, "w").and_then(|w| Emu::parse(w, Emu::PER_TWIP)).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();
    let style = tbl.path(ns::W, &["tblPr", "tblStyle"]).and_then(val).map(str::to_string);

    let mut rows: Vec<Row> = Vec::new();
    // Whether each grid slot continues a row span from above (Word's bare `w:vMerge`).
    let mut continues: Vec<Vec<bool>> = Vec::new();
    for tr in tbl.children_named(ns::W, "tr") {
        let trpr = tr.child(ns::W, "trPr");
        let omitted = |local: &str| {
            trpr.and_then(|pr| pr.child(ns::W, local)).and_then(val).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(MAX_ROW_SLOTS)
        };
        let before = omitted("gridBefore");
        let after = omitted("gridAfter");
        let mut cells = vec![Cell::default(); before];
        let mut cont = vec![false; before];
        for tc in tr.children_named(ns::W, "tc") {
            let tcpr = tc.child(ns::W, "tcPr");
            // Each grid slot under a span gets a cell. Word makes at most 63 columns, so spans
            // fill a row to 63 slots and no further: a damaged (or hostile) file can't claim
            // billions of slots, or many rows' worth of a huge grid.
            let room = MAX_ROW_SLOTS.saturating_sub(cells.len()).max(1) as u32;
            let span: u32 =
                tcpr.and_then(|p| p.child(ns::W, "gridSpan")).and_then(val).and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, room);
            // vMerge="restart" starts a row span; a bare vMerge continues one.
            let continued = tcpr.and_then(|p| p.child(ns::W, "vMerge")).is_some_and(|m| val(m) != Some("restart"));
            let fill = tcpr
                .and_then(|p| p.child(ns::W, "shd"))
                .and_then(|s| s.attr_ns(ns::W, "fill"))
                .and_then(crate::drawing::Rgb::parse)
                .map(crate::drawing::Color::rgb);
            cells.push(Cell {
                blocks: read_blocks(tc, rels, doc, preview, |e| e.is(ns::W, "tcPr")),
                // A covered cell's span is its owner's business.
                col_span: if continued { 1 } else { span },
                row_span: 1,
                covered: continued,
                fill,
                origin: shell(tc, |c| !c.is(ns::W, "tcPr")),
            });
            cont.push(continued);
            // The grid slots under the span, which Word doesn't store.
            for _ in 1..span {
                cells.push(Cell { covered: true, ..Default::default() });
                cont.push(continued);
            }
        }
        let after = after.min(MAX_ROW_SLOTS.saturating_sub(cells.len()));
        cells.resize(cells.len() + after, Cell::default());
        cont.resize(cells.len(), false);
        rows.push(Row {
            cells,
            height: trpr
                .and_then(|p| p.child(ns::W, "trHeight"))
                .and_then(|h| h.attr_ns(ns::W, "val"))
                .and_then(|v| Emu::parse(v, Emu::PER_TWIP)),
            header: trpr.and_then(|p| super::props::toggle(p, "tblHeader")).unwrap_or(false),
            origin: table_shell(tr, "tc"),
        });
        continues.push(cont);
    }
    // A cell that isn't covered spans down over the rows that continue it at its column.
    for r in 0..rows.len() {
        for c in 0..rows[r].cells.len() {
            if rows[r].cells[c].covered {
                continue;
            }
            let below = (r + 1..rows.len()).take_while(|&b| continues[b].get(c) == Some(&true)).count();
            rows[r].cells[c].row_span = 1 + below as u32;
        }
    }
    // Keep the grid metadata and the positions of modeled rows among raw table controls.
    Table { columns, rows, style, origin: table_shell(tbl, "tr") }
}
