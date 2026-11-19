//! Relationships: how parts point at each other (`_rels/*.rels`).

use crate::xml::{ns, Element, XmlDoc};

/// Relationship types, by their last segment. Transitional and strict files spell the start of
/// the URI differently, so types compare by this suffix ([`Relationship::is`]).
pub mod kind {
    pub const OFFICE_DOCUMENT: &str = "officeDocument";
    pub const CORE_PROPERTIES: &str = "core-properties";
    pub const EXTENDED_PROPERTIES: &str = "extended-properties";
    pub const THUMBNAIL: &str = "thumbnail";
    pub const THEME: &str = "theme";
    pub const IMAGE: &str = "image";
    pub const HYPERLINK: &str = "hyperlink";
    pub const STYLES: &str = "styles";
    pub const NUMBERING: &str = "numbering";
    pub const SETTINGS: &str = "settings";
    pub const FONT_TABLE: &str = "fontTable";
    pub const SLIDE: &str = "slide";
    pub const SLIDE_LAYOUT: &str = "slideLayout";
    pub const SLIDE_MASTER: &str = "slideMaster";
    pub const NOTES_SLIDE: &str = "notesSlide";
    pub const CHART: &str = "chart";
    pub const WORKSHEET: &str = "worksheet";
    pub const SHARED_STRINGS: &str = "sharedStrings";
    pub const DRAWING: &str = "drawing";

    /// The full transitional URI for a type.
    pub fn uri(kind: &str) -> String {
        match kind {
            CORE_PROPERTIES => "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties".into(),
            THUMBNAIL => "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail".into(),
            _ => format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    /// The full type URI.
    pub rel_type: String,
    /// As written: relative to the owning part, or a URL when `external`.
    pub target: String,
    pub external: bool,
}

impl Relationship {
    /// Whether this relationship has type `kind` (one of [`kind`]'s constants).
    pub fn is(&self, kind: &str) -> bool {
        self.rel_type.rsplit('/').next() == Some(kind)
    }
}

/// One part's relationships, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relationships(pub RelationshipList);

/// Ordered relationships. Mutating the list invalidates its lookup index.
#[derive(Clone, Debug, Default)]
pub struct RelationshipList {
    entries: Vec<Relationship>,
    index: std::sync::OnceLock<RelIndex>,
}

#[derive(Clone, Debug, Default)]
struct RelIndex {
    ids: std::collections::HashMap<String, usize>,
    links: std::collections::HashMap<String, usize>,
    next: usize,
}

impl RelationshipList {
    fn index(&self) -> &RelIndex {
        self.index.get_or_init(|| {
            let mut index = RelIndex { next: 1, ..Default::default() };
            for (i, r) in self.entries.iter().enumerate() {
                index.ids.entry(r.id.clone()).or_insert(i);
                if r.external && r.is(kind::HYPERLINK) {
                    index.links.entry(r.target.clone()).or_insert(i);
                }
            }
            index
        })
    }
}

impl From<Vec<Relationship>> for RelationshipList {
    fn from(entries: Vec<Relationship>) -> Self {
        Self { entries, index: std::sync::OnceLock::new() }
    }
}

impl std::ops::Deref for RelationshipList {
    type Target = Vec<Relationship>;
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl std::ops::DerefMut for RelationshipList {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.index.take();
        &mut self.entries
    }
}

impl PartialEq for RelationshipList {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}
impl Eq for RelationshipList {}

impl<'a> IntoIterator for &'a RelationshipList {
    type Item = &'a Relationship;
    type IntoIter = std::slice::Iter<'a, Relationship>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl Relationships {
    pub fn parse(doc: &XmlDoc) -> Relationships {
        let rels: Vec<Relationship> = doc
            .root
            .children_named(ns::PKG_REL, "Relationship")
            .filter_map(|r| {
                Some(Relationship {
                    id: r.attr("Id")?.to_string(),
                    rel_type: r.attr("Type")?.to_string(),
                    target: r.attr("Target")?.to_string(),
                    external: r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External")),
                })
            })
            .collect();
        Relationships(rels.into())
    }

    pub fn to_xml(&self) -> XmlDoc {
        // Office's prefix table has no prefix for this namespace, so it's the default one.
        let mut root = Element::new(ns::PKG_REL, "Relationships");
        root.declare(None, ns::PKG_REL);
        for r in &self.0 {
            let mut el = Element::new(ns::PKG_REL, "Relationship")
                .with_attr("", "Id", &r.id)
                .with_attr("", "Type", &r.rel_type)
                .with_attr("", "Target", &r.target);
            if r.external {
                el.set_attr("", "TargetMode", "External");
            }
            root.push(el);
        }
        XmlDoc::new(root)
    }

    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.0.index().ids.get(id).map(|&i| &self.0.entries[i])
    }

    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Relationship> + 'a {
        self.0.iter().filter(move |r| r.is(kind))
    }

    pub fn first(&self, kind: &str) -> Option<&Relationship> {
        self.0.iter().find(|r| r.is(kind))
    }

    /// Adds a relationship and returns its new id (`rId1`, `rId2`… whichever is free).
    pub fn add(&mut self, kind: &str, target: impl Into<String>, external: bool) -> String {
        self.0.index();
        let index = self.0.index.get_mut().unwrap();
        let id = loop {
            let id = format!("rId{}", index.next);
            index.next += 1;
            if !index.ids.contains_key(&id) {
                break id;
            }
        };
        let target = target.into();
        let at = self.0.entries.len();
        index.ids.insert(id.clone(), at);
        if external && kind == kind::HYPERLINK {
            index.links.entry(target.clone()).or_insert(at);
        }
        self.0.entries.push(Relationship { id: id.clone(), rel_type: kind::uri(kind), target, external });
        id
    }

    pub fn remove(&mut self, id: &str) -> Option<Relationship> {
        let i = *self.0.index().ids.get(id)?;
        Some(self.0.remove(i))
    }

    /// Reuses an external hyperlink, or adds one.
    pub fn link(&mut self, url: &str) -> String {
        if let Some(&i) = self.0.index().links.get(url) {
            return self.0.entries[i].id.clone();
        }
        self.add(kind::HYPERLINK, url, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml;

    #[test]
    fn parses_and_writes() {
        let src = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rId1" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/hyperlink" Target="https://example.com/?a=1&amp;b=2" TargetMode="External"/></Relationships>"#;
        let mut rels = Relationships::parse(&xml::parse(src.as_bytes()).unwrap());
        assert_eq!(rels.0.len(), 2);
        assert!(rels.get("rId2").unwrap().is(kind::IMAGE));
        let link = rels.first(kind::HYPERLINK).unwrap();
        assert!(link.external);
        assert_eq!(link.target, "https://example.com/?a=1&b=2");
        assert_eq!(rels.add(kind::THEME, "theme/theme1.xml", false), "rId3");

        let out = String::from_utf8(rels.to_xml().to_bytes()).unwrap();
        assert!(
            out.contains(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2""#),
            "{out}"
        );
        assert_eq!(Relationships::parse(&xml::parse(out.as_bytes()).unwrap()), rels);
    }
}
