//! A small XML tree that keeps everything a part had: element and attribute order, namespace
//! declarations, comments, processing instructions and whitespace. Office files are full of
//! markup the compiler doesn't model, and writing a part back must not lose any of it.
//!
//! Elements remember the prefix they were written with and the namespace it resolved to.
//! Lookups go by namespace, so `w:p` and `ns0:p` bound to the same URI are the same element.
//! When writing, prefixes are re-bound where needed, so an element moved into another part, or
//! made with [`Element::new`], always comes out declared.

pub mod ns;
mod parse;
pub mod stream;
mod write;

use std::sync::Arc;

pub use parse::parse;

/// A parsed XML part.
#[derive(Clone, Debug, PartialEq)]
pub struct XmlDoc {
    pub declaration: Option<Declaration>,
    /// Comments, processing instructions and whitespace before the root element.
    pub before: Vec<Node>,
    pub root: Element,
    /// Anything after the root element.
    pub after: Vec<Node>,
}

/// `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>`. Written back as UTF-8 whatever
/// encoding the part was read in.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub version: String,
    pub standalone: Option<String>,
}

impl Default for Declaration {
    fn default() -> Self {
        Declaration { version: "1.0".into(), standalone: Some("yes".into()) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
    CData(String),
    Comment(String),
    /// A processing instruction: target, then the rest.
    Pi(String, String),
    DocType(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub prefix: Option<String>,
    pub local: String,
    /// The namespace URI the prefix resolved to where the element was read.
    pub ns: Option<Arc<str>>,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub prefix: Option<String>,
    pub local: String,
    pub ns: Option<Arc<str>>,
    pub value: String,
}

impl Attr {
    /// `xmlns` or `xmlns:x`.
    pub fn is_ns_decl(&self) -> bool {
        self.prefix.as_deref() == Some("xmlns") || (self.prefix.is_none() && self.local == "xmlns")
    }

    pub fn qname(&self) -> String {
        qualified(self.prefix.as_deref(), &self.local)
    }
}

fn qualified(prefix: Option<&str>, local: &str) -> String {
    match prefix {
        Some(p) => format!("{p}:{local}"),
        None => local.to_string(),
    }
}

fn ns_matches(have: Option<&Arc<str>>, want: &str) -> bool {
    match have {
        Some(uri) => ns::same(uri, want),
        None => want.is_empty(),
    }
}

impl Element {
    /// An element in `ns` (or no namespace, for ""), with Office's usual prefix for it.
    pub fn new(ns: &str, local: &str) -> Element {
        Element {
            prefix: ns::prefix_for(ns).map(str::to_string),
            local: local.to_string(),
            ns: (!ns.is_empty()).then(|| Arc::from(ns)),
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The name as written, `w:p`.
    pub fn qname(&self) -> String {
        qualified(self.prefix.as_deref(), &self.local)
    }

    /// Whether this is `local` in namespace `ns` ("" for no namespace).
    pub fn is(&self, ns: &str, local: &str) -> bool {
        self.local == local && ns_matches(self.ns.as_ref(), ns)
    }

    pub fn in_ns(&self, ns: &str) -> bool {
        ns_matches(self.ns.as_ref(), ns)
    }

    // ---- attributes ----

    /// An attribute without a namespace, such as `x` in `<a:off x="0"/>`. Word's `w:val` is
    /// namespaced: use [`Element::attr_ns`] for it.
    pub fn attr(&self, local: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.ns.is_none() && !a.is_ns_decl() && a.local == local).map(|a| a.value.as_str())
    }

    /// A namespaced attribute, `r:id` or `w:val`.
    pub fn attr_ns(&self, ns: &str, local: &str) -> Option<&str> {
        self.attrs.iter().find(|a| !a.is_ns_decl() && a.local == local && ns_matches(a.ns.as_ref(), ns)).map(|a| a.value.as_str())
    }

    /// Sets an attribute (no namespace for ""), keeping its place if it's already there.
    pub fn set_attr(&mut self, ns: &str, local: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some(a) = self.attrs.iter_mut().find(|a| !a.is_ns_decl() && a.local == local && ns_matches(a.ns.as_ref(), ns)) {
            a.value = value;
            return;
        }
        self.attrs.push(Attr {
            prefix: if ns.is_empty() { None } else { ns::prefix_for(ns).map(str::to_string) },
            local: local.to_string(),
            ns: (!ns.is_empty()).then(|| Arc::from(ns)),
            value,
        });
    }

    pub fn remove_attr(&mut self, ns: &str, local: &str) -> Option<String> {
        let i = self.attrs.iter().position(|a| !a.is_ns_decl() && a.local == local && ns_matches(a.ns.as_ref(), ns))?;
        Some(self.attrs.remove(i).value)
    }

    /// Declares `prefix` for `uri` on this element (the default namespace for `None`), as Office
    /// does on a part's root.
    pub fn declare(&mut self, prefix: Option<&str>, uri: &str) {
        let (p, local) = match prefix {
            Some(p) => (Some("xmlns".to_string()), p.to_string()),
            None => (None, "xmlns".to_string()),
        };
        if let Some(a) = self.attrs.iter_mut().find(|a| a.is_ns_decl() && a.prefix == p && a.local == local) {
            a.value = uri.to_string();
            return;
        }
        self.attrs.push(Attr { prefix: p, local, ns: Some(Arc::from(ns::XMLNS)), value: uri.to_string() });
    }

    // ---- children ----

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn child(&self, ns: &str, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.is(ns, local))
    }

    pub fn child_mut(&mut self, ns: &str, local: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.is(ns, local))
    }

    pub fn children_named<'a>(&'a self, ns: &'a str, local: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.is(ns, local))
    }

    /// Follows a path of children in one namespace: `el.path(ns::A, &["bodyPr", "normAutofit"])`.
    pub fn path(&self, ns: &str, names: &[&str]) -> Option<&Element> {
        names.iter().try_fold(self, |el, name| el.child(ns, name))
    }

    /// The first descendant (depth first) named `local` in `ns`.
    pub fn descendant(&self, ns: &str, local: &str) -> Option<&Element> {
        for el in self.elements() {
            if el.is(ns, local) {
                return Some(el);
            }
            if let Some(found) = el.descendant(ns, local) {
                return Some(found);
            }
        }
        None
    }

    pub fn push(&mut self, el: Element) -> &mut Element {
        self.children.push(Node::Element(el));
        match self.children.last_mut() {
            Some(Node::Element(e)) => e,
            _ => unreachable!(),
        }
    }

    /// The child named `local` in `ns`, made (and appended) if there isn't one.
    pub fn child_or_insert(&mut self, ns: &str, local: &str) -> &mut Element {
        match self.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(ns, local))) {
            Some(i) => match &mut self.children[i] {
                Node::Element(e) => e,
                _ => unreachable!(),
            },
            None => self.push(Element::new(ns, local)),
        }
    }

    /// Removes every child named `local` in `ns`, returning how many went.
    pub fn remove_children(&mut self, ns: &str, local: &str) -> usize {
        let before = self.children.len();
        self.children.retain(|n| !matches!(n, Node::Element(e) if e.is(ns, local)));
        before - self.children.len()
    }

    /// Puts `child` where the schema wants it. `order` lists the element's child slots in schema
    /// order, each slot the names that can fill it (a fill is one of `noFill`, `solidFill`…).
    /// Whatever fills `child`'s slot now is replaced; otherwise `child` goes before the first child
    /// in a later slot. Children in no slot (extensions, unknown markup) stay where they are.
    pub fn place(&mut self, child: Element, order: &[&[&str]]) {
        let slot_of = |e: &Element| order.iter().position(|names| names.contains(&e.local.as_str()));
        let Some(slot) = slot_of(&child) else {
            self.children.push(Node::Element(child));
            return;
        };
        let child_ns = child.ns.clone();
        let same = |n: &Node| matches!(n, Node::Element(e) if slot_of(e) == Some(slot) && same_ns(&e.ns, &child_ns));
        if let Some(i) = self.children.iter().position(same) {
            self.children[i] = Node::Element(child);
            let mut seen = false;
            self.children.retain(|n| {
                if !same(n) {
                    return true;
                }
                let keep = !seen;
                seen = true;
                keep
            });
            return;
        }
        let at = self.children.iter().position(|n| matches!(n, Node::Element(e) if slot_of(e).is_some_and(|s| s > slot)));
        match at {
            Some(i) => self.children.insert(i, Node::Element(child)),
            None => self.children.push(Node::Element(child)),
        }
    }

    /// Removes the children in `ns` named any of `names` (one slot of a [`Element::place`] order).
    pub fn clear(&mut self, ns: &str, names: &[&str]) {
        self.children.retain(|n| !matches!(n, Node::Element(e) if e.in_ns(ns) && names.contains(&e.local.as_str())));
    }

    /// The element's text and its descendants', concatenated.
    pub fn text(&self) -> String {
        let mut out = String::new();
        collect_text(self, &mut out);
        out
    }

    // ---- building ----

    pub fn with_attr(mut self, ns: &str, local: &str, value: impl Into<String>) -> Element {
        self.set_attr(ns, local, value);
        self
    }

    pub fn with_child(mut self, child: Element) -> Element {
        self.children.push(Node::Element(child));
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Element {
        self.children.push(Node::Text(text.into()));
        self
    }

    /// This element alone as XML, with every namespace it uses declared on it.
    pub fn to_xml_string(&self) -> String {
        write::element_to_string(self)
    }

    /// This element as XML where `scope`'s namespace declarations are in force (as
    /// [`stream::Hollow::scope`] gives them), declaring only what they don't.
    pub fn to_xml_string_in(&self, scope: &[Attr]) -> String {
        write::element_to_string_in(self, scope)
    }
}

fn same_ns(a: &Option<Arc<str>>, b: &Option<Arc<str>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => ns::same(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn collect_text(el: &Element, out: &mut String) {
    for n in &el.children {
        match n {
            Node::Text(t) | Node::CData(t) => out.push_str(t),
            Node::Element(e) => collect_text(e, out),
            _ => {}
        }
    }
}

/// Whitespace that only lays the markup out, as pretty-printed files have: whitespace between
/// tags, or a line break. A space as an element's only content (`<a:t> </a:t>`) is text; Office
/// XML never mixes text and elements, so nothing else is.
fn is_layout(n: &Node, between_tags: bool) -> bool {
    matches!(n, Node::Text(t) if t.trim().is_empty() && (between_tags || t.contains('\n')))
}

impl Element {
    /// Whether two elements say the same thing: the same names by namespace (whatever their
    /// prefixes), the same attributes in any order, and the same content, ignoring layout
    /// whitespace between tags.
    pub fn same_content(&self, other: &Element) -> bool {
        self.same_content_in(other, false, false)
    }

    fn same_content_in(&self, other: &Element, preserve_a: bool, preserve_b: bool) -> bool {
        fn key(a: &Attr) -> (String, &str, &str) {
            // Declarations by what they declare; other attributes by namespace.
            let space = if a.is_ns_decl() {
                format!("xmlns:{}", a.qname())
            } else {
                a.ns.as_deref().map(|n| ns::canonical(n).to_string()).unwrap_or_default()
            };
            (space, a.local.as_str(), a.value.as_str())
        }
        fn sorted(e: &Element) -> Vec<(String, &str, &str)> {
            let mut a: Vec<_> = e.attrs.iter().map(key).collect();
            a.sort();
            a
        }
        if self.local != other.local || !same_ns(&self.ns, &other.ns) || sorted(self) != sorted(other) {
            return false;
        }
        let preserve = |e: &Element, inherited| match e.attr_ns(ns::XML, "space") {
            Some("preserve") => true,
            Some("default") => false,
            _ => inherited,
        };
        let (pa, pb) = (preserve(self, preserve_a), preserve(other, preserve_b));
        let tags = |e: &Element| e.children.iter().any(|n| matches!(n, Node::Element(_)));
        let (ta, tb) = (tags(self), tags(other));
        let mut a = self.children.iter().filter(|n| pa || !is_layout(n, ta));
        let mut b = other.children.iter().filter(|n| pb || !is_layout(n, tb));
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some(Node::Element(x)), Some(Node::Element(y))) if x.same_content_in(y, pa, pb) => {}
                (Some(x), Some(y)) if !matches!(x, Node::Element(_)) && x == y => {}
                _ => return false,
            }
        }
    }
}

impl XmlDoc {
    /// A part with the usual declaration and `root`.
    pub fn new(root: Element) -> XmlDoc {
        XmlDoc { declaration: Some(Declaration::default()), before: Vec::new(), root, after: Vec::new() }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        write::doc_to_bytes(self)
    }
}

/// Parses one element from a string, as [`Element::to_xml_string`] writes it.
pub fn parse_element(xml: &str) -> Result<Element, String> {
    parse(xml.as_bytes()).map(|d| d.root)
}

/// Concatenates all `<m:t>` text runs inside an Office Math subtree so Writan can detect whether
/// Microsoft Word or PowerPoint modified a saved equation natively.
pub fn omml_text_fingerprint(el: &Element) -> String {
    fn walk(el: &Element, out: &mut String) {
        if el.is(ns::M, "t") {
            out.push_str(&el.text());
            return;
        }
        for c in el.elements() {
            walk(c, out);
        }
    }
    let mut out = String::new();
    walk(el, &mut out);
    out
}

// Raw XML shows up in the model's JSON as its source text, which is readable and self-contained.
impl serde::Serialize for Element {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_xml_string())
    }
}

impl<'de> serde::Deserialize<'de> for Element {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Element, D::Error> {
        let s = String::deserialize(d)?;
        parse_element(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests;
