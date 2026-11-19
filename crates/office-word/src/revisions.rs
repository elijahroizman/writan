//! Tracked changes: listing them, and accepting or rejecting them all.
//!
//! The core keeps Word's revision marks as raw markup: an inserted run sits in `w:ins`, a
//! deleted one in `w:del` (its text as `w:delText`), moved text in `w:moveFrom` and
//! `w:moveTo`, and formatting changes as `w:rPrChange` and `w:pPrChange` holding the old
//! formatting. Accepting keeps the new and drops the marks; rejecting puts the old back.

use office_core::formats::docx::{read_inlines_in, read_para_props, read_run_props};
use office_core::model::{Block, Document, Inline, Origin, Paragraph, Table};
use office_core::xml::{ns, Element, Node};
use office_core::Result;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RevisionKind {
    Insertion,
    Deletion,
    MovedFrom,
    MovedTo,
    /// Formatting changed (a run's, a paragraph's, a table's).
    Formatting,
    /// A paragraph mark inserted or deleted: paragraphs split or joined.
    Paragraph,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// The text inserted, deleted or moved (empty for formatting).
    pub text: String,
}

fn kind_of(el: &Element) -> Option<RevisionKind> {
    if !el.in_ns(ns::W) {
        return None;
    }
    Some(match el.local.as_str() {
        "ins" => RevisionKind::Insertion,
        "del" => RevisionKind::Deletion,
        "moveFrom" => RevisionKind::MovedFrom,
        "moveTo" => RevisionKind::MovedTo,
        "rPrChange" | "pPrChange" | "tblPrChange" | "trPrChange" | "tcPrChange" | "sectPrChange" | "numberingChange" => {
            RevisionKind::Formatting
        }
        _ => return None,
    })
}

fn revision(el: &Element, kind: RevisionKind) -> Revision {
    let text = if kind == RevisionKind::Formatting { String::new() } else { deleted_text(el) };
    Revision {
        kind,
        author: el.attr_ns(ns::W, "author").unwrap_or("").to_string(),
        date: el.attr_ns(ns::W, "date").map(str::to_string),
        text,
    }
}

/// Text, counting deleted text as text.
fn deleted_text(el: &Element) -> String {
    let mut s = String::new();
    for n in &el.children {
        match n {
            Node::Element(e) if e.in_ns(ns::W) && (e.local == "t" || e.local == "delText") => s.push_str(&e.text()),
            Node::Element(e) if e.is(ns::W, "tab") => s.push('\t'),
            Node::Element(e) => s.push_str(&deleted_text(e)),
            _ => {}
        }
    }
    s
}

/// Every revision in the document's body, in order.
pub fn list(doc: &Document) -> Vec<Revision> {
    let mut out = Vec::new();
    if let office_core::model::Body::Flow { blocks } = &doc.body {
        list_blocks(blocks, &mut out);
    }
    out
}

fn list_blocks(blocks: &[Block], out: &mut Vec<Revision>) {
    for b in blocks {
        match b {
            Block::Paragraph(p) => list_paragraph(p, out),
            Block::Table(t) => {
                if let Some(o) = t.origin.get() {
                    find_marks(o, out, false);
                }
                for row in &t.rows {
                    if let Some(o) = row.origin.get() {
                        find_marks(o, out, false);
                    }
                    for cell in &row.cells {
                        if let Some(o) = cell.origin.get() {
                            find_marks(o, out, false);
                        }
                        list_blocks(&cell.blocks, out);
                    }
                }
            }
            Block::Raw { xml, .. } => find_marks(xml, out, true),
        }
    }
}

fn list_paragraph(p: &Paragraph, out: &mut Vec<Revision>) {
    if let Some(ppr) = p.origin.get().and_then(|o| o.child(ns::W, "pPr")) {
        if let Some(rpr) = ppr.child(ns::W, "rPr") {
            for m in rpr.elements().filter(|e| e.is(ns::W, "ins") || e.is(ns::W, "del")) {
                out.push(Revision { kind: RevisionKind::Paragraph, ..revision(m, RevisionKind::Paragraph) });
            }
        }
        find_marks(ppr, out, false);
    }
    for i in &p.content {
        match i {
            Inline::Raw { xml } => find_marks(xml, out, true),
            Inline::Text(r) => {
                if let Some(o) = r.origin.get() {
                    find_marks(o, out, false);
                }
            }
            _ => {}
        }
    }
}

/// Revision marks in `el`: `el` itself if it's one, and formatting changes inside.
fn find_marks(el: &Element, out: &mut Vec<Revision>, top: bool) {
    if let Some(k) = kind_of(el).filter(|k| top || *k == RevisionKind::Formatting) {
        out.push(revision(el, k));
        if k != RevisionKind::Formatting {
            // Formatting changes inside an insertion count too.
            for e in el.elements() {
                find_marks(e, out, false);
            }
        }
        return;
    }
    for e in el.elements() {
        if e.is(ns::W, "rPr")
            || e.is(ns::W, "pPr")
            || e.is(ns::W, "r")
            || e.is(ns::W, "trPr")
            || e.is(ns::W, "tcPr")
            || e.is(ns::W, "tblPr")
            || e.is(ns::W, "sectPr")
            || e.is(ns::W, "hyperlink")
        {
            find_marks(e, out, false);
        } else if kind_of(e).is_some_and(|k| k == RevisionKind::Formatting) {
            out.push(revision(e, RevisionKind::Formatting));
        }
    }
}

/// Accepts (`true`) or rejects (`false`) every tracked change in the body.
pub fn resolve_all(doc: &mut Document, accept: bool) -> Result<usize> {
    let part = match &doc.package {
        Some(p) => p.main_part()?,
        None => return Ok(0),
    };
    let office_core::model::Body::Flow { blocks } = &mut doc.body else { return Ok(0) };
    let mut taken = std::mem::take(blocks);
    let mut count = 0;
    resolve_blocks(doc, &part, &mut taken, accept, &mut count)?;
    if let office_core::model::Body::Flow { blocks } = &mut doc.body {
        *blocks = taken;
    }
    Ok(count)
}

fn resolve_blocks(doc: &mut Document, part: &str, blocks: &mut Vec<Block>, accept: bool, count: &mut usize) -> Result<()> {
    let mut i = 0;
    while i < blocks.len() {
        match &mut blocks[i] {
            Block::Paragraph(p) => {
                resolve_paragraph(doc, part, p, accept, count)?;
                // A paragraph mark that goes (deleted and accepted, inserted and rejected) joins
                // the paragraph to the next, which gives the joined one its formatting.
                let goes = mark_goes(p, accept);
                if goes {
                    *count += 1;
                }
                if goes && matches!(blocks.get(i + 1), Some(Block::Paragraph(_))) {
                    let Block::Paragraph(p) = blocks.remove(i) else { unreachable!() };
                    if let Block::Paragraph(n) = &mut blocks[i] {
                        let mut content = p.content;
                        content.append(&mut n.content);
                        n.content = content;
                    }
                    continue;
                }
                if let Block::Paragraph(p) = &mut blocks[i] {
                    strip_mark(p);
                }
            }
            Block::Table(t) => resolve_table(doc, part, t, accept, count)?,
            Block::Raw { .. } => {}
        }
        i += 1;
    }
    Ok(())
}

fn mark_change(p: &Paragraph) -> Option<&str> {
    let rpr = p.origin.get()?.child(ns::W, "pPr")?.child(ns::W, "rPr")?;
    rpr.elements().find(|e| e.is(ns::W, "ins") || e.is(ns::W, "del")).map(|e| e.local.as_str())
}

fn mark_goes(p: &Paragraph, accept: bool) -> bool {
    matches!((mark_change(p), accept), (Some("del"), true) | (Some("ins"), false))
}

fn strip_mark(p: &mut Paragraph) {
    if mark_change(p).is_none() {
        return;
    }
    let mut o = p.origin.get().unwrap().clone();
    if let Some(rpr) = o.child_mut(ns::W, "pPr").and_then(|ppr| ppr.child_mut(ns::W, "rPr")) {
        rpr.remove_children(ns::W, "ins");
        rpr.remove_children(ns::W, "del");
    }
    p.origin = Origin::new(o);
}

fn resolve_paragraph(doc: &mut Document, part: &str, p: &mut Paragraph, accept: bool, count: &mut usize) -> Result<()> {
    // The paragraph's own formatting change.
    if let Some(change) = p.origin.get().and_then(|o| o.child(ns::W, "pPr")).and_then(|ppr| ppr.child(ns::W, "pPrChange")).cloned() {
        *count += 1;
        let mut o = p.origin.get().unwrap().clone();
        let ppr = o.child_mut(ns::W, "pPr").unwrap();
        if accept {
            ppr.remove_children(ns::W, "pPrChange");
        } else if let Some(old) = change.child(ns::W, "pPr") {
            // The old properties, keeping the paragraph mark's formatting and section break.
            let keep: Vec<Node> = ppr
                .children
                .iter()
                .filter(|n| matches!(n, Node::Element(e) if e.is(ns::W, "rPr") || e.is(ns::W, "sectPr")))
                .cloned()
                .collect();
            ppr.children = old.children.clone();
            ppr.children.extend(keep);
            p.props = read_para_props(ppr);
        }
        p.origin = Origin::new(o);
    }
    let mut out = Vec::with_capacity(p.content.len());
    for inline in std::mem::take(&mut p.content) {
        match inline {
            Inline::Raw { xml } if kind_of(&xml).is_some_and(|k| k != RevisionKind::Formatting) => {
                *count += 1;
                let keep = matches!((xml.local.as_str(), accept), ("ins" | "moveTo", true) | ("del" | "moveFrom", false));
                if keep {
                    let content = if accept { xml } else { undelete(&xml) };
                    for mut i in read_inlines_in(doc, part, &content)? {
                        resolve_run_change(&mut i, accept, count);
                        out.push(i);
                    }
                }
            }
            // Range markers for moves mean nothing once the move is settled.
            Inline::Raw { xml }
                if xml.in_ns(ns::W)
                    && matches!(xml.local.as_str(), "moveFromRangeStart" | "moveFromRangeEnd" | "moveToRangeStart" | "moveToRangeEnd") => {}
            mut i => {
                resolve_run_change(&mut i, accept, count);
                out.push(i);
            }
        }
    }
    p.content = out;
    Ok(())
}

/// Deleted text as text again: `w:delText` back to `w:t`, `w:delInstrText` to `w:instrText`.
fn undelete(el: &Element) -> Element {
    let mut el = el.clone();
    fn walk(e: &mut Element) {
        if e.in_ns(ns::W) {
            match e.local.as_str() {
                "delText" => e.local = "t".into(),
                "delInstrText" => e.local = "instrText".into(),
                _ => {}
            }
        }
        for c in e.elements_mut() {
            walk(c);
        }
    }
    walk(&mut el);
    el
}

/// A run's formatting change: accepted, the record of the old goes; rejected, the old comes back.
fn resolve_run_change(i: &mut Inline, accept: bool, count: &mut usize) {
    let Inline::Text(run) = i else { return };
    let Some(change) = run.origin.get().and_then(|o| o.child(ns::W, "rPr")).and_then(|r| r.child(ns::W, "rPrChange")).cloned() else {
        return;
    };
    *count += 1;
    let mut o = run.origin.get().unwrap().clone();
    let rpr = o.child_mut(ns::W, "rPr").unwrap();
    if accept {
        rpr.remove_children(ns::W, "rPrChange");
    } else {
        rpr.children = change.child(ns::W, "rPr").map(|r| r.children.clone()).unwrap_or_default();
        run.props = read_run_props(rpr);
    }
    run.origin = Origin::new(o);
}

/// Resolve a property snapshot in the object's shell, returning restored properties so
/// the modeled values can be refreshed along with the preserved XML.
fn resolve_properties(origin: &mut Origin, property: &str, accept: bool, count: &mut usize) -> Option<Element> {
    let mut shell = origin.get()?.clone();
    let props = shell.child_mut(ns::W, property)?;
    let change_name = format!("{property}Change");
    let change = props.child(ns::W, &change_name)?.clone();
    *count += 1;
    let restored = if accept {
        props.remove_children(ns::W, &change_name);
        None
    } else {
        let old = change.child(ns::W, property).cloned().unwrap_or_else(|| Element::new(ns::W, property));
        *props = old.clone();
        Some(old)
    };
    *origin = Origin::new(shell);
    restored
}

fn resolve_table(doc: &mut Document, part: &str, t: &mut Table, accept: bool, count: &mut usize) -> Result<()> {
    if let Some(old) = resolve_properties(&mut t.origin, "tblPr", accept, count) {
        t.style = old.child(ns::W, "tblStyle").and_then(office_core::formats::docx::val).map(str::to_string);
    }
    let row_change = |r: &office_core::model::Row| -> Option<String> {
        let trpr = r.origin.get()?.child(ns::W, "trPr")?;
        trpr.elements().find(|e| e.is(ns::W, "ins") || e.is(ns::W, "del")).map(|e| e.local.clone())
    };
    let mut kept = Vec::new();
    let mut original_indices = Vec::new();
    for (index, mut row) in std::mem::take(&mut t.rows).into_iter().enumerate() {
        match (row_change(&row).as_deref(), accept) {
            (Some("del"), true) | (Some("ins"), false) => {
                *count += 1;
                continue;
            }
            (Some(_), _) => {
                *count += 1;
                let mut o = row.origin.get().unwrap().clone();
                if let Some(trpr) = o.child_mut(ns::W, "trPr") {
                    trpr.remove_children(ns::W, "ins");
                    trpr.remove_children(ns::W, "del");
                }
                row.origin = Origin::new(o);
            }
            _ => {}
        }
        if let Some(old) = resolve_properties(&mut row.origin, "trPr", accept, count) {
            row.height = old
                .child(ns::W, "trHeight")
                .and_then(|h| h.attr_ns(ns::W, "val"))
                .and_then(|v| office_core::units::Emu::parse(v, office_core::units::Emu::PER_TWIP));
            row.header = old
                .child(ns::W, "tblHeader")
                .is_some_and(|e| office_core::formats::docx::val(e).and_then(office_core::units::parse_bool).unwrap_or(true));
        }
        for cell in &mut row.cells {
            if let Some(old) = resolve_properties(&mut cell.origin, "tcPr", accept, count) {
                cell.fill = old
                    .child(ns::W, "shd")
                    .and_then(|s| s.attr_ns(ns::W, "fill"))
                    .and_then(office_core::drawing::Rgb::parse)
                    .map(office_core::drawing::Color::rgb);
            }
            resolve_blocks(doc, part, &mut cell.blocks, accept, count)?;
        }
        original_indices.push(index);
        kept.push(row);
    }
    // Count surviving rows in each original span, including deletions in the middle.
    // A row span that reached into a removed row stops where the rows end.
    for r in 0..kept.len() {
        for c in 0..kept[r].cells.len() {
            let end = original_indices[r].saturating_add(kept[r].cells[c].row_span as usize);
            let span = original_indices[r..].iter().take_while(|&&i| i < end).count().max(1) as u32;
            kept[r].cells[c].row_span = span;
        }
    }
    t.rows = kept;
    // And a cell its removed owner covered is a cell of its own again.
    for r in 0..t.rows.len() {
        for c in 0..t.rows[r].cells.len() {
            if t.rows[r].cells[c].covered && t.owner(r, c).is_none_or(|o| o == (r, c)) {
                t.rows[r].cells[c].covered = false;
            }
        }
    }
    Ok(())
}
