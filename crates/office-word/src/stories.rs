//! The document's other stories: headers and footers, footnotes and endnotes, and comments.
//! Each is read into the core's blocks, so the same model (and the same editing) covers
//! them, and written back when it changes.

use office_core::formats::docx::{read_blocks_in, write_blocks};
use office_core::model::{blocks_text, Block, Document};
use office_core::opc::{content_types::types, Package};
use office_core::xml::{ns, Element, Node, XmlDoc};
use office_core::Result;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoryKind {
    Header,
    Footer,
}

/// A header or footer part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Story {
    pub kind: StoryKind,
    pub part: String,
    pub blocks: Vec<Block>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteKind {
    Footnote,
    Endnote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub kind: NoteKind,
    pub id: String,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initials: Option<String>,
    /// A W3C date-time, as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    pub blocks: Vec<Block>,
}

impl Comment {
    pub fn text(&self) -> String {
        blocks_text(&self.blocks)
    }
}

pub const HEADER: &str = "header";
pub const FOOTER: &str = "footer";
pub const FOOTNOTES: &str = "footnotes";
pub const ENDNOTES: &str = "endnotes";
pub const COMMENTS: &str = "comments";
const COMMENTS_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";

/// The document's headers and footers, in relationship order.
pub fn read_headers(doc: &mut Document) -> Result<Vec<Story>> {
    let Some(pkg) = &doc.package else { return Ok(Vec::new()) };
    let main = pkg.main_part()?;
    let mut parts = Vec::new();
    for (kind, rel) in [(StoryKind::Header, HEADER), (StoryKind::Footer, FOOTER)] {
        for (_, part) in pkg.related(&main, rel)? {
            if pkg.has(&part) {
                parts.push((kind, part));
            }
        }
    }
    let mut out = Vec::new();
    for (kind, part) in parts {
        let xml = doc.package.as_ref().unwrap().xml(&part)?;
        out.push(Story { kind, blocks: read_blocks_in(doc, &part, &xml.root)?, part });
    }
    Ok(out)
}

/// Footnotes and endnotes, without the separators Word keeps among them.
pub fn read_notes(doc: &mut Document) -> Result<Vec<Note>> {
    let mut out = Vec::new();
    for (kind, rel, local) in [(NoteKind::Footnote, FOOTNOTES, "footnote"), (NoteKind::Endnote, ENDNOTES, "endnote")] {
        let Some(part) = related_part(doc, rel)? else { continue };
        let xml = doc.package.as_ref().unwrap().xml(&part)?;
        for n in xml.root.children_named(ns::W, local) {
            if n.attr_ns(ns::W, "type").is_some_and(|t| t != "normal") {
                continue;
            }
            let Some(id) = n.attr_ns(ns::W, "id") else { continue };
            out.push(Note { kind, id: id.to_string(), blocks: read_blocks_in(doc, &part, n)? });
        }
    }
    Ok(out)
}

pub fn read_comments(doc: &mut Document) -> Result<Vec<Comment>> {
    let Some(part) = related_part(doc, COMMENTS)? else { return Ok(Vec::new()) };
    let xml = doc.package.as_ref().unwrap().xml(&part)?;
    let mut out = Vec::new();
    for c in xml.root.children_named(ns::W, "comment") {
        let attr = |k: &str| c.attr_ns(ns::W, k).map(str::to_string);
        let Some(id) = attr("id") else { continue };
        out.push(Comment {
            id,
            author: attr("author").unwrap_or_default(),
            initials: attr("initials"),
            date: attr("date"),
            blocks: read_blocks_in(doc, &part, c)?,
        });
    }
    Ok(out)
}

fn related_part(doc: &Document, rel: &str) -> Result<Option<String>> {
    let Some(pkg) = &doc.package else { return Ok(None) };
    let main = pkg.main_part()?;
    Ok(pkg.related(&main, rel)?.into_iter().map(|(_, p)| p).find(|p| pkg.has(p)))
}

/// Writes headers and footers whose blocks differ from `before` back into their parts.
pub fn write_headers(doc: &Document, pkg: &mut Package, stories: &[Story], before: &[Story]) -> Result<()> {
    for s in stories {
        if before.iter().any(|b| b == s) || !pkg.has(&s.part) {
            continue;
        }
        let mut xml = pkg.xml(&s.part)?;
        xml.root.children = write_blocks(doc, pkg, &s.part, &non_empty(&s.blocks))?.into_iter().map(Node::Element).collect();
        let media = pkg.content_type(&s.part).unwrap_or(types::XML).to_string();
        pkg.put_xml(&s.part, &xml, &media);
    }
    Ok(())
}

/// A header, footer, note or comment must have a paragraph.
fn non_empty(blocks: &[Block]) -> Vec<Block> {
    if blocks.is_empty() {
        vec![Block::Paragraph(Default::default())]
    } else {
        blocks.to_vec()
    }
}

/// Writes notes whose blocks changed back into their parts.
pub fn write_notes(doc: &Document, pkg: &mut Package, notes: &[Note], before: &[Note]) -> Result<()> {
    let main = pkg.main_part()?;
    for (kind, rel, local) in [(NoteKind::Footnote, FOOTNOTES, "footnote"), (NoteKind::Endnote, ENDNOTES, "endnote")] {
        let changed: Vec<&Note> = notes.iter().filter(|n| n.kind == kind && !before.contains(n)).collect();
        if changed.is_empty() {
            continue;
        }
        let Some((_, part)) = pkg.related(&main, rel)?.into_iter().next() else { continue };
        let mut xml = pkg.xml(&part)?;
        for note in changed {
            let els = write_blocks(doc, pkg, &part, &non_empty(&note.blocks))?;
            if let Some(el) = xml.root.elements_mut().find(|e| e.is(ns::W, local) && e.attr_ns(ns::W, "id") == Some(note.id.as_str())) {
                el.children = els.into_iter().map(Node::Element).collect();
            }
        }
        let media = pkg.content_type(&part).unwrap_or(types::XML).to_string();
        pkg.put_xml(&part, &xml, &media);
    }
    Ok(())
}

/// Writes the comments if they changed, making the comments part if the document has none.
pub fn write_comments(doc: &Document, pkg: &mut Package, comments: &[Comment], before: &[Comment]) -> Result<()> {
    if comments == before {
        return Ok(());
    }
    let main = pkg.main_part()?;
    let part = pkg
        .related(&main, COMMENTS)?
        .into_iter()
        .next()
        .map(|(_, p)| p)
        .filter(|p| pkg.has(p))
        .unwrap_or_else(|| "word/comments.xml".to_string());
    let mut xml = if pkg.has(&part) {
        pkg.xml(&part)?
    } else {
        let mut root = Element::new(ns::W, "comments");
        root.declare(Some("w"), ns::W);
        XmlDoc::new(root)
    };
    // Comments as they were read keep their own element (and anything Word put on it).
    let mut kept: Vec<Element> = Vec::new();
    for c in comments {
        let mut el = xml
            .root
            .elements()
            .find(|e| e.is(ns::W, "comment") && e.attr_ns(ns::W, "id") == Some(c.id.as_str()))
            .cloned()
            .unwrap_or_else(|| Element::new(ns::W, "comment"));
        el.set_attr(ns::W, "id", &c.id);
        el.set_attr(ns::W, "author", &c.author);
        for (k, v) in [("initials", &c.initials), ("date", &c.date)] {
            match v {
                Some(v) => el.set_attr(ns::W, k, v),
                None => {
                    el.remove_attr(ns::W, k);
                }
            }
        }
        if !before.iter().any(|b| b.id == c.id && b.blocks == c.blocks) {
            el.children = write_blocks(doc, pkg, &part, &non_empty(&c.blocks))?.into_iter().map(Node::Element).collect();
        }
        kept.push(el);
    }
    xml.root.children = kept.into_iter().map(Node::Element).collect();
    pkg.put_xml(&part, &xml, COMMENTS_TYPE);
    pkg.relate(&main, COMMENTS, &part)?;
    Ok(())
}
