//! The office compiler's Word layer: what only Word documents have, on top of `office-core`'s
//! paragraphs and tables.
//!
//! - **Styles**: what text really looks like once its style, the document defaults and its
//!   own formatting are put together ([`resolved_para`], [`resolved_run`]), and Word's
//!   built-in styles added when a document uses one it doesn't define.
//! - **Lists**: which list a paragraph is in, its label ([`list_labels`]), and new lists.
//! - **Sections**: page size, orientation, margins and columns ([`sections`]).
//! - **Headers, footers, footnotes, endnotes and comments**, as the core's blocks.
//! - **Tracked changes**: listing, accepting and rejecting them ([`revisions`]).
//!
//! Add [`Words`] to a compiler to have it read and write all this.

pub mod numbering;
pub mod revisions;
pub mod sections;
pub mod stories;
pub mod styles;
pub mod view;

use office_core::formats::docx::{default_styles, val};
use office_core::model::{Block, Body, Document, Inline, Kind, Origin, ParaProps, Paragraph, Run, RunProps};
use office_core::opc::{content_types::types, rels::kind, Package};
use office_core::xml::{ns, Element, XmlDoc};
use office_core::{Layer, Result};

pub use numbering::{ListKind, Numbering};
pub use revisions::{Revision, RevisionKind};
pub use sections::Section;
pub use stories::{Comment, Note, NoteKind, Story, StoryKind};
pub use styles::{Style, StyleKind, Styles};

/// The layer: add it to a [`office_core::Compiler`].
pub struct Words;

/// What the layer keeps on a document (in its extensions).
#[derive(Clone, Debug, Default)]
pub struct Word {
    pub styles: Styles,
    /// The style sheet with built-in styles added during editing.
    styles_xml: Option<XmlDoc>,
    pub numbering: Numbering,
    /// The numbering part as read, with any lists added since.
    numbering_xml: Option<XmlDoc>,
    numbering_changed: bool,
    pub headers: Vec<Story>,
    pub notes: Vec<Note>,
    pub comments: Vec<Comment>,
    /// The stories as read, to tell which changed.
    read: (Vec<Story>, Vec<Note>, Vec<Comment>),
}

const NUMBERING_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";

impl Layer for Words {
    fn name(&self) -> &'static str {
        "Word"
    }

    fn kinds(&self) -> &[Kind] {
        &[Kind::Word]
    }

    // before_write is the default no-op, so saving needn't copy the document for it.
    fn edits_before_write(&self, _: Kind) -> bool {
        false
    }

    fn after_read(&self, doc: &mut Document) -> Result<()> {
        let mut word = Word::default();
        if let Some(pkg) = &doc.package {
            let main = pkg.main_part()?;
            let part = |rel: &str| -> Result<Option<XmlDoc>> {
                match pkg.related(&main, rel)?.into_iter().next() {
                    Some((_, p)) if pkg.has(&p) => Ok(Some(pkg.xml(&p)?)),
                    _ => Ok(None),
                }
            };
            word.styles = Styles::parse(&part(kind::STYLES)?.unwrap_or_else(default_styles));
            word.numbering_xml = part(kind::NUMBERING)?;
            word.numbering = word.numbering_xml.as_ref().map(Numbering::parse).unwrap_or_default();
        } else {
            word.styles = Styles::parse(&default_styles());
        }
        word.headers = stories::read_headers(doc)?;
        word.notes = stories::read_notes(doc)?;
        word.comments = stories::read_comments(doc)?;
        word.read = (word.headers.clone(), word.notes.clone(), word.comments.clone());
        doc.extensions.insert(word);
        Ok(())
    }

    fn write_parts(&self, doc: &Document, pkg: &mut Package, _to: Kind) -> Result<()> {
        let main = pkg.main_part()?;
        if let Some(sheet) = word(doc).and_then(|w| w.styles_xml.as_ref()) {
            let part = pkg.related(&main, kind::STYLES)?.into_iter().next().map(|(_, p)| p).unwrap_or_else(|| "word/styles.xml".into());
            pkg.put_xml(&part, sheet, types::DOCX_STYLES);
            pkg.relate(&main, kind::STYLES, &part)?;
        }
        // Built-in styles the document uses and its style sheet doesn't have.
        let used = used_styles(doc);
        // A file's style sheet was parsed on reading it; going back to the part (Word's runs to
        // hundreds of kilobytes) is only worth it when one of those is missing from it.
        let sheet_has = |id: &String| doc.package.is_some() && word(doc).is_some_and(|w| w.styles.get(id).is_some());
        let missing = used.iter().any(|id| !sheet_has(id) && styles::builtin(id).is_some());
        if let Some((_, part)) = pkg.related(&main, kind::STYLES)?.into_iter().next().filter(|_| missing) {
            let mut sheet = pkg.xml(&part)?;
            if styles::add_missing(&mut sheet, &used) {
                pkg.put_xml(&part, &sheet, types::DOCX_STYLES);
            }
        }
        let Some(word) = doc.extensions.get::<Word>() else { return Ok(()) };
        if word.numbering_changed {
            if let Some(xml) = &word.numbering_xml {
                let part = pkg
                    .related(&main, kind::NUMBERING)?
                    .into_iter()
                    .next()
                    .map(|(_, p)| p)
                    .unwrap_or_else(|| "word/numbering.xml".to_string());
                pkg.put_xml(&part, xml, NUMBERING_TYPE);
                pkg.relate(&main, kind::NUMBERING, &part)?;
            }
        }
        stories::write_headers(doc, pkg, &word.headers, &word.read.0)?;
        stories::write_notes(doc, pkg, &word.notes, &word.read.1)?;
        stories::write_comments(doc, pkg, &word.comments, &word.read.2)?;
        Ok(())
    }
}

/// The layer's data on a document, if it was read with the layer.
pub fn word(doc: &Document) -> Option<&Word> {
    doc.extensions.get::<Word>()
}

/// The layer's data on a document, made (with a new document's styles) if it has none.
pub fn word_mut(doc: &mut Document) -> &mut Word {
    if doc.extensions.get::<Word>().is_none() {
        doc.extensions.insert(Word { styles: Styles::parse(&default_styles()), ..Default::default() });
    }
    doc.extensions.get_mut::<Word>().unwrap()
}

pub(crate) fn ensure_style(doc: &mut Document, id: &str) -> bool {
    if word(doc).is_some_and(|w| w.styles.get(id).is_some()) || styles::builtin(id).is_none() {
        return false;
    }
    let mut sheet = word(doc)
        .and_then(|w| w.styles_xml.clone())
        .or_else(|| {
            let pkg = doc.package.as_ref()?;
            let main = pkg.main_part().ok()?;
            let (_, part) = pkg.related(&main, kind::STYLES).ok()?.into_iter().next()?;
            pkg.xml(&part).ok()
        })
        .unwrap_or_else(default_styles);
    styles::add_missing(&mut sheet, &[id.to_string()]);
    let w = word_mut(doc);
    w.styles = Styles::parse(&sheet);
    w.styles_xml = Some(sheet);
    true
}

fn blocks(doc: &Document) -> &[Block] {
    match &doc.body {
        Body::Flow { blocks } => blocks,
        _ => &[],
    }
}

/// Every paragraph in the body, in reading order, tables included.
pub fn paragraphs(doc: &Document) -> Vec<&Paragraph> {
    fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a Paragraph>) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => out.push(p),
                Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).filter(|c| !c.covered).for_each(|c| walk(&c.blocks, out)),
                Block::Raw { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks(doc), &mut out);
    out
}

fn used_styles(doc: &Document) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for p in paragraphs(doc) {
        ids.extend(p.props.style.clone());
        ids.extend(p.runs().filter_map(|r| r.props.style.clone()));
    }
    ids.sort();
    ids.dedup();
    ids
}

// ---- styles ----

/// A paragraph's formatting with its style and the document defaults filled in.
pub fn resolved_para(doc: &Document, p: &Paragraph) -> ParaProps {
    match word(doc) {
        Some(w) => w.styles.resolve_para(&p.props),
        None => p.props.clone(),
    }
}

/// A run's formatting as it shows (see [`Styles::resolve_run`]).
pub fn resolved_run(doc: &Document, p: &Paragraph, r: &Run) -> RunProps {
    match word(doc) {
        Some(w) => w.styles.resolve_run(&p.props, &r.props, doc.theme.as_ref()),
        None => r.props.clone(),
    }
}

/// Full Word run formatting, including outline, shadow, emboss, imprint and vanish.
pub fn resolved_word_run(doc: &Document, p: &Paragraph, r: &Run) -> styles::ResolvedRun {
    match word(doc) {
        Some(w) => {
            w.styles.resolve_run_with_toggles(&p.props, &r.props, r.origin.get().and_then(|o| o.child(ns::W, "rPr")), doc.theme.as_ref())
        }
        None => styles::ResolvedRun { props: r.props.clone(), ..Default::default() },
    }
}

// ---- lists ----

/// The list a paragraph is in and its level: its own `w:numPr`, or its style's.
pub fn list_of(doc: &Document, p: &Paragraph) -> Option<(String, u8)> {
    let own = p.origin.get().and_then(|o| o.path(ns::W, &["pPr", "numPr", "numId"])).and_then(val).map(str::to_string);
    let style_num = || {
        let w = word(doc)?;
        let id = p.props.style.clone().or_else(|| w.styles.default_of(StyleKind::Paragraph).map(|s| s.id.clone()))?;
        w.styles.chain(&id).iter().rev().find_map(|s| s.num_id.clone())
    };
    let num = own.or_else(style_num)?;
    let level = p
        .props
        .level
        .or_else(|| {
            let w = word(doc)?;
            let style = p.props.style.as_deref().or_else(|| w.styles.default_of(StyleKind::Paragraph).map(|s| s.id.as_str()))?;
            w.numbering.levels(&num)?.iter().position(|l| l.paragraph_style.as_deref() == Some(style)).map(|i| i as u8)
        })
        .unwrap_or(0);
    Some((num, level))
}

/// Each body paragraph's list label ("1.", "a)", "•"), in [`paragraphs`] order; `None` for
/// paragraphs not in a list.
pub fn list_labels(doc: &Document) -> Vec<Option<String>> {
    let Some(w) = word(doc) else { return paragraphs(doc).iter().map(|_| None).collect() };
    w.numbering.labels(paragraphs(doc).into_iter().map(|p| list_of(doc, p)).collect::<Vec<_>>())
}

/// Makes a new list and returns its id, for [`set_list`].
pub fn add_list(doc: &mut Document, kind: ListKind) -> String {
    let w = word_mut(doc);
    let xml = w.numbering_xml.get_or_insert_with(numbering::empty);
    let id = numbering::add_list(xml, kind);
    w.numbering = Numbering::parse(xml);
    w.numbering_changed = true;
    id
}

/// Puts body block `index` (a paragraph) in list `num_id` at `level`, or takes it out of any
/// list for `None`.
pub fn set_list(doc: &mut Document, index: usize, list: Option<(&str, u8)>) {
    let base_style = word(doc).and_then(|w| w.styles.get("ListParagraph")).and_then(|s| s.based_on.clone());
    let Body::Flow { blocks } = &mut doc.body else { return };
    let Some(Block::Paragraph(p)) = blocks.get_mut(index) else { return };
    let mut o = p.origin.get().cloned().unwrap_or_else(|| Element::new(ns::W, "p"));
    let ppr = o.child_or_insert(ns::W, "pPr");
    ppr.remove_children(ns::W, "numPr");
    if let Some((num, level)) = list {
        let num_pr = Element::new(ns::W, "numPr")
            .with_child(Element::new(ns::W, "ilvl").with_attr(ns::W, "val", level.to_string()))
            .with_child(Element::new(ns::W, "numId").with_attr(ns::W, "val", num));
        ppr.place(num_pr, office_core::formats::docx::PPR);
        p.props.level = Some(level);
        if p.props.style.is_none() {
            p.props.style = Some("ListParagraph".into());
        }
    } else {
        ppr.place(
            Element::new(ns::W, "numPr").with_child(Element::new(ns::W, "numId").with_attr(ns::W, "val", "0")),
            office_core::formats::docx::PPR,
        );
        p.props.level = None;
        p.props.indent_start = None;
        p.props.indent_first_line = None;
        if p.props.style.as_deref() == Some("ListParagraph") {
            p.props.style = base_style;
        }
        if let Some(ind) = ppr.child_mut(ns::W, "ind") {
            for name in ["left", "start", "hanging", "firstLine", "leftChars", "startChars", "hangingChars", "firstLineChars"] {
                ind.remove_attr(ns::W, name);
            }
        }
    }
    // The core writes pPr from the origin, patching only what the model changes.
    p.origin = Origin::new(o);
}

// ---- sections ----

/// Where each section's properties are: a paragraph (by its index among the body's blocks)
/// or the end of the body.
fn section_places(doc: &Document) -> Vec<Option<usize>> {
    let mut out: Vec<Option<usize>> = blocks(doc)
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b, Block::Paragraph(p) if p.origin.get().and_then(|o| o.path(ns::W, &["pPr", "sectPr"])).is_some()))
        .map(|(i, _)| Some(i))
        .collect();
    out.push(None);
    out
}

/// The document's sections, first to last.
pub fn sections(doc: &Document) -> Vec<Section> {
    section_places(doc)
        .into_iter()
        .map(|place| {
            let sect = match place {
                Some(i) => match &blocks(doc)[i] {
                    Block::Paragraph(p) => p.origin.get().and_then(|o| o.path(ns::W, &["pPr", "sectPr"])).cloned(),
                    _ => None,
                },
                None => doc.origin.get().and_then(|o| o.path(ns::W, &["body", "sectPr"])).cloned(),
            };
            sect.map(|s| Section::parse(&s)).unwrap_or_default()
        })
        .collect()
}

/// Changes section `index`'s page setup.
pub fn set_section(doc: &mut Document, index: usize, section: &Section) {
    let Some(place) = section_places(doc).get(index).copied() else { return };
    let patch = |root: &mut Element, path: &[&str]| {
        let mut el = root;
        for (i, name) in path.iter().enumerate() {
            if el.child(ns::W, name).is_none() && i + 1 == path.len() {
                el.push(Element::new(ns::W, name));
            }
            el = el.child_or_insert(ns::W, name);
        }
        section.patch(el);
    };
    match place {
        Some(i) => {
            let Body::Flow { blocks } = &mut doc.body else { return };
            if let Block::Paragraph(p) = &mut blocks[i] {
                let mut o = p.origin.get().cloned().unwrap();
                patch(&mut o, &["pPr", "sectPr"]);
                p.origin = Origin::new(o);
            }
        }
        None => {
            let mut o = doc.origin.get().cloned().filter(|o| o.is(ns::W, "document")).unwrap_or_else(|| Element::new(ns::W, "document"));
            patch(&mut o, &["body", "sectPr"]);
            doc.origin = Origin::new(o);
        }
    }
}

// ---- comments ----

fn raw(el: Element) -> Inline {
    Inline::Raw { xml: el }
}

/// Adds a comment on body block `index` (a paragraph) and returns its id. `date` is a W3C
/// date-time, or `None`.
pub fn add_comment(
    doc: &mut Document,
    index: usize,
    author: &str,
    initials: Option<&str>,
    date: Option<&str>,
    text: &str,
) -> Option<String> {
    let w = word_mut(doc);
    let id = (0..).map(|n: u32| n.to_string()).find(|id| !w.comments.iter().any(|c| &c.id == id)).unwrap();
    let Body::Flow { blocks } = &mut doc.body else { return None };
    let Some(Block::Paragraph(p)) = blocks.get_mut(index) else { return None };
    let marker = |local: &str| Element::new(ns::W, local).with_attr(ns::W, "id", &id);
    p.content.insert(0, raw(marker("commentRangeStart")));
    p.content.push(raw(marker("commentRangeEnd")));
    p.content.push(raw(Element::new(ns::W, "r")
        .with_child(Element::new(ns::W, "rPr").with_child(Element::new(ns::W, "rStyle").with_attr(ns::W, "val", "CommentReference")))
        .with_child(marker("commentReference"))));
    word_mut(doc).comments.push(Comment {
        id: id.clone(),
        author: author.to_string(),
        initials: initials.map(str::to_string),
        date: date.map(str::to_string),
        blocks: vec![Block::Paragraph(Paragraph::text(text))],
    });
    Some(id)
}

/// Removes a comment and its marks in the text.
pub fn remove_comment(doc: &mut Document, id: &str) {
    word_mut(doc).comments.retain(|c| c.id != id);
    let mine = |el: &Element| {
        let is_mark =
            el.in_ns(ns::W) && matches!(el.local.as_str(), "commentRangeStart" | "commentRangeEnd") && el.attr_ns(ns::W, "id") == Some(id);
        let is_ref = el.is(ns::W, "r") && el.child(ns::W, "commentReference").is_some_and(|r| r.attr_ns(ns::W, "id") == Some(id));
        is_mark || is_ref
    };
    fn walk(blocks: &mut [Block], mine: &dyn Fn(&Element) -> bool) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => p.content.retain(|i| !matches!(i, Inline::Raw { xml } if mine(xml))),
                Block::Table(t) => t.rows.iter_mut().flat_map(|r| &mut r.cells).for_each(|c| walk(&mut c.blocks, mine)),
                Block::Raw { .. } => {}
            }
        }
    }
    if let Body::Flow { blocks } = &mut doc.body {
        walk(blocks, &mine);
    }
}

/// The text a comment is on.
pub fn commented_text(doc: &Document, id: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for p in paragraphs(doc) {
        for i in &p.content {
            match i {
                Inline::Raw { xml } if xml.attr_ns(ns::W, "id") == Some(id) && xml.is(ns::W, "commentRangeStart") => inside = true,
                Inline::Raw { xml } if xml.attr_ns(ns::W, "id") == Some(id) && xml.is(ns::W, "commentRangeEnd") => inside = false,
                Inline::Text(r) if inside => out.push_str(&r.text),
                _ => {}
            }
        }
        if inside {
            out.push('\n');
        }
    }
    out
}

// ---- tracked changes ----

/// The tracked changes in the body.
pub fn revisions(doc: &Document) -> Vec<Revision> {
    revisions::list(doc)
}

/// Accepts every tracked change in the body; returns how many there were.
pub fn accept_all(doc: &mut Document) -> Result<usize> {
    revisions::resolve_all(doc, true)
}

/// Rejects every tracked change in the body; returns how many there were.
pub fn reject_all(doc: &mut Document) -> Result<usize> {
    revisions::resolve_all(doc, false)
}

#[cfg(test)]
mod tests;
