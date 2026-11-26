use super::{ns, Attr, Element, Node, XmlDoc};

pub fn doc_to_bytes(doc: &XmlDoc) -> Vec<u8> {
    let mut out = String::new();
    if let Some(d) = &doc.declaration {
        out.push_str(&format!("<?xml version=\"{}\" encoding=\"UTF-8\"", d.version));
        if let Some(s) = &d.standalone {
            out.push_str(&format!(" standalone=\"{s}\""));
        }
        out.push_str("?>");
    }
    let mut w = Writer { out, scope: Vec::new(), strict: is_strict(&doc.root) };
    for n in &doc.before {
        w.node(n);
    }
    w.element(&doc.root);
    for n in &doc.after {
        w.node(n);
    }
    w.out.into_bytes()
}

pub fn element_to_string(el: &Element) -> String {
    element_to_string_in(el, &[])
}

/// One element as XML where `scope`'s namespace declarations are already in force, so it
/// declares only what they don't.
pub fn element_to_string_in(el: &Element, scope: &[Attr]) -> String {
    let strict = is_strict(el) || scope.iter().any(|a| a.is_ns_decl() && ns::canonical(&a.value) != a.value);
    let scope = scope.iter().filter(|a| a.is_ns_decl()).map(|a| (a.prefix.as_ref().map(|_| a.local.clone()), a.value.clone())).collect();
    let mut w = Writer { out: String::new(), scope, strict };
    w.element(el);
    w.out
}

fn is_strict(el: &Element) -> bool {
    el.ns.as_deref().is_some_and(|uri| ns::canonical(uri) != uri)
}

struct Writer {
    strict: bool,
    out: String,
    /// (prefix, uri) bindings in force, innermost last; `None` is the default namespace and an
    /// empty uri undeclares it.
    scope: Vec<(Option<String>, String)>,
}

impl Writer {
    fn bound(&self, prefix: Option<&str>) -> Option<&str> {
        if prefix == Some("xml") {
            return Some(ns::XML);
        }
        self.scope.iter().rev().find(|(p, _)| p.as_deref() == prefix).map(|(_, u)| u.as_str()).filter(|u| !u.is_empty())
    }

    /// A prefix bound to `uri` right now, if any (not the default namespace when `prefixed`).
    fn prefix_of(&self, uri: &str, prefixed: bool) -> Option<Option<String>> {
        for (i, (p, u)) in self.scope.iter().enumerate().rev() {
            if u != uri || (prefixed && p.is_none()) {
                continue;
            }
            // Only if nothing inside rebinds that prefix.
            if !self.scope[i + 1..].iter().any(|(q, _)| q == p) {
                return Some(p.clone());
            }
        }
        None
    }

    fn node(&mut self, n: &Node) {
        match n {
            Node::Element(e) => self.element(e),
            Node::Text(t) => escape_text(t, &mut self.out),
            Node::CData(t) => {
                // "]]>" can't appear inside CDATA; split it across two sections.
                self.out.push_str("<![CDATA[");
                self.out.push_str(&t.replace("]]>", "]]]]><![CDATA[>"));
                self.out.push_str("]]>");
            }
            Node::Comment(t) => {
                self.out.push_str("<!--");
                self.out.push_str(&t.replace("--", "- -"));
                self.out.push_str("-->");
            }
            Node::Pi(target, content) => {
                self.out.push_str("<?");
                self.out.push_str(target);
                if !content.is_empty() {
                    self.out.push(' ');
                    self.out.push_str(content);
                }
                self.out.push_str("?>");
            }
            Node::DocType(t) => {
                self.out.push_str("<!DOCTYPE ");
                self.out.push_str(t.trim_start());
                self.out.push('>');
            }
        }
    }

    fn element(&mut self, el: &Element) {
        let mark = self.scope.len();
        for a in el.attrs.iter().filter(|a| a.is_ns_decl()) {
            let p = a.prefix.as_ref().map(|_| a.local.clone());
            self.scope.push((p, a.value.clone()));
        }
        // Declarations this element needs and doesn't have, written after its own.
        let mut extra: Vec<(Option<String>, String)> = Vec::new();

        let prefix = self.resolve(el.prefix.as_deref(), el.ns.as_deref(), false, mark, &mut extra);
        let name = join(&prefix, &el.local);
        let mut attrs: Vec<(String, &str)> = Vec::with_capacity(el.attrs.len());
        for a in &el.attrs {
            let qname = if a.is_ns_decl() { a.qname() } else { self.attr_name(a, mark, &prefix, &attrs, &mut extra) };
            attrs.push((qname, &a.value));
        }

        self.out.push('<');
        self.out.push_str(&name);
        for (qname, value) in &attrs {
            self.out.push(' ');
            self.out.push_str(qname);
            self.out.push_str("=\"");
            escape_attr(value, &mut self.out);
            self.out.push('"');
        }
        for (p, uri) in &extra {
            match p {
                Some(p) => {
                    self.out.push_str(" xmlns:");
                    self.out.push_str(p);
                    self.out.push_str("=\"");
                }
                None => self.out.push_str(" xmlns=\""),
            }
            escape_attr(uri, &mut self.out);
            self.out.push('"');
        }
        if el.children.is_empty() {
            self.out.push_str("/>");
        } else {
            self.out.push('>');
            for c in &el.children {
                self.node(c);
            }
            self.out.push_str("</");
            self.out.push_str(&name);
            self.out.push('>');
        }
        self.scope.truncate(mark);
    }

    /// The prefix to write a name in `uri` with ("" for none), binding one first if nothing in
    /// scope does. Attributes need a real prefix: the default namespace doesn't apply to them.
    fn resolve(
        &mut self,
        prefix: Option<&str>,
        uri: Option<&str>,
        attr: bool,
        reserved: usize,
        extra: &mut Vec<(Option<String>, String)>,
    ) -> String {
        let Some(uri) = uri else {
            // A prefix that was never declared where it was read: write it as it was.
            if let Some(p) = prefix {
                return p.to_string();
            }
            // No namespace: an unprefixed element is right unless a default namespace is in force.
            if !attr && self.bound(None).is_some() {
                self.bind(None, "", extra);
            }
            return String::new();
        };
        let uri = if self.strict { ns::strict(uri) } else { uri };
        if self.bound(prefix) == Some(uri) && !(attr && prefix.is_none()) {
            return prefix.unwrap_or_default().to_string();
        }
        if let Some(p) = self.prefix_of(uri, attr) {
            return p.unwrap_or_default();
        }
        // Bind the name's own prefix, or Office's usual one, or a made-up one.
        let mut p = prefix.map(str::to_string);
        if attr && p.is_none() {
            p = Some(ns::prefix_for(uri).map(str::to_string).unwrap_or_else(|| self.fresh_prefix()));
        }
        if self.scope[reserved..].iter().any(|(q, u)| q == &p && u != uri) {
            p = Some(self.fresh_prefix());
        }
        self.bind(p.clone(), uri, extra);
        p.unwrap_or_default()
    }

    fn attr_name(
        &mut self,
        a: &Attr,
        reserved: usize,
        element_prefix: &str,
        attrs: &[(String, &str)],
        extra: &mut Vec<(Option<String>, String)>,
    ) -> String {
        if a.ns.is_none() {
            return a.qname();
        }
        // An inherited binding used by this tag's name cannot be rebound by an attribute.
        let wanted = a.ns.as_deref().unwrap();
        let wanted = if self.strict { ns::strict(wanted) } else { wanted };
        let preferred = a.prefix.as_deref().or_else(|| ns::prefix_for(wanted));
        let fresh;
        let prefix = if preferred
            .is_some_and(|p| p == element_prefix || attrs.iter().any(|(name, _)| name.split_once(':').is_some_and(|(used, _)| used == p)))
            && self.bound(preferred) != Some(wanted)
        {
            fresh = self.fresh_prefix();
            Some(fresh.as_str())
        } else {
            preferred
        };
        let p = self.resolve(prefix, a.ns.as_deref(), true, reserved, extra);
        format!("{p}:{}", a.local)
    }

    fn bind(&mut self, prefix: Option<String>, uri: &str, extra: &mut Vec<(Option<String>, String)>) {
        self.scope.push((prefix.clone(), uri.to_string()));
        extra.push((prefix, uri.to_string()));
    }

    fn fresh_prefix(&self) -> String {
        (0..).map(|i| format!("ns{i}")).find(|p| self.bound(Some(p)).is_none()).unwrap()
    }
}

fn join(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    }
}

/// Characters XML 1.0 can't hold at all, not even escaped: control characters other than tab
/// and line breaks, and U+FFFE and U+FFFF. Writing one would make the part unreadable, so
/// they're left out. (Excel has its own escape for them; see `formats::xlsx::escape`.)
fn allowed(c: char) -> bool {
    !matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')
}

fn escape_text(t: &str, out: &mut String) {
    for c in t.chars().filter(|&c| allowed(c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(t: &str, out: &mut String) {
    for c in t.chars().filter(|&c| allowed(c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}
