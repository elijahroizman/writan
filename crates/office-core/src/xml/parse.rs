use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{ns, Attr, Declaration, Element, Node, XmlDoc};

/// Parses a part. Accepts UTF-8 (with or without a byte order mark) and UTF-16, which some
/// producers write. Custom entities are never expanded, so a part can't reach outside itself.
/// How deep elements may nest. Office markup stays far below this; a part nested deeper is
/// rejected, because walking (and dropping) the tree is recursive.
pub const MAX_DEPTH: usize = 256;

pub fn parse(bytes: &[u8]) -> Result<XmlDoc, String> {
    let text = decode(bytes)?;
    let mut reader = Reader::from_str(&text);
    let config = reader.config_mut();
    config.trim_text(false);
    config.expand_empty_elements = false;
    config.check_end_names = true;

    let mut parser = Parser::default();
    let mut declaration = None;
    // The element being built at each depth; the document's top level is collected separately.
    let mut stack: Vec<Element> = Vec::new();
    let mut top: Vec<Node> = Vec::new();

    loop {
        let event = reader.read_event().map_err(|e| format!("{e} at byte {}", reader.error_position()))?;
        let node = match event {
            Event::Start(start) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(format!("elements nested more than {MAX_DEPTH} deep"));
                }
                stack.push(parser.open(&start)?);
                continue;
            }
            Event::Empty(start) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(format!("elements nested more than {MAX_DEPTH} deep"));
                }
                let el = parser.open(&start)?;
                parser.close();
                Node::Element(el)
            }
            Event::End(_) => {
                let el = stack.pop().ok_or("closing tag without an opening one")?;
                parser.close();
                Node::Element(el)
            }
            Event::Text(t) => Node::Text(t.xml10_content().into_owned()),
            Event::GeneralRef(r) => {
                let ch = match r.resolve_char_ref().map_err(|e| e.to_string())? {
                    Some(c) => c.to_string(),
                    None => predefined(&r).map(str::to_string).unwrap_or_else(|| format!("&{};", &*r)),
                };
                Node::Text(ch)
            }
            Event::CData(c) => Node::CData(c.to_string()),
            Event::Comment(c) => Node::Comment(c.to_string()),
            Event::PI(pi) => Node::Pi(pi.target().to_string(), pi.content().trim_start().to_string()),
            Event::DocType(d) => Node::DocType(d.to_string()),
            Event::Decl(d) => {
                let version = d.version().map_err(|e| e.to_string())?.into_owned();
                let standalone = d.standalone().transpose().map_err(|e| e.to_string())?.map(|s| s.into_owned());
                declaration = Some(Declaration { version, standalone });
                continue;
            }
            Event::Eof => break,
        };
        let siblings = match stack.last_mut() {
            Some(parent) => &mut parent.children,
            None => &mut top,
        };
        // Text arrives in pieces around entity references; keep it as one node.
        match (siblings.last_mut(), node) {
            (Some(Node::Text(prev)), Node::Text(more)) => prev.push_str(&more),
            (_, node) => siblings.push(node),
        }
    }
    if !stack.is_empty() {
        return Err("the part ends before its root element closes".into());
    }

    let root_at = top.iter().position(|n| matches!(n, Node::Element(_))).ok_or("the part has no root element")?;
    let after = top.split_off(root_at + 1);
    let Some(Node::Element(root)) = top.pop() else { unreachable!() };
    if after.iter().any(|n| matches!(n, Node::Element(_))) {
        return Err("the part has more than one root element".into());
    }
    Ok(XmlDoc { declaration, before: top, root, after })
}

/// The part as text: borrowed when it's UTF-8 already, which nearly every part is, so a big
/// part isn't copied before it's parsed.
pub(super) fn decode(bytes: &[u8]) -> Result<std::borrow::Cow<'_, str>, String> {
    let utf16 = |bytes: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| from(*c)).collect();
        String::from_utf16(&units).map(std::borrow::Cow::Owned).map_err(|e| e.to_string())
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] | rest => std::str::from_utf8(rest).map(std::borrow::Cow::Borrowed).map_err(|e| e.to_string()),
    }
}

fn predefined(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    })
}

/// Resolves prefixes to namespaces as elements open and close.
#[derive(Default)]
struct Parser {
    /// (prefix, uri) bindings, innermost last; `None` is the default namespace.
    scope: Vec<(Option<String>, Option<Arc<str>>)>,
    /// How many bindings each open element added.
    pushed: Vec<usize>,
    /// One shared copy of each namespace URI.
    interned: HashMap<String, Arc<str>>,
}

impl Parser {
    fn intern(&mut self, uri: &str) -> Arc<str> {
        if let Some(a) = self.interned.get(uri) {
            return a.clone();
        }
        let a: Arc<str> = Arc::from(uri);
        self.interned.insert(uri.to_string(), a.clone());
        a
    }

    fn lookup(&self, prefix: Option<&str>) -> Option<Arc<str>> {
        if prefix == Some("xml") {
            return Some(Arc::from(ns::XML));
        }
        self.scope.iter().rev().find(|(p, _)| p.as_deref() == prefix).and_then(|(_, uri)| uri.clone())
    }

    fn open(&mut self, start: &BytesStart) -> Result<Element, String> {
        let mut attrs = Vec::new();
        let mut pushed = 0;
        for a in start.attributes().with_checks(false) {
            let a = a.map_err(|e| e.to_string())?;
            let (prefix, local) = split(a.key.0);
            let value = unescape_attr(&a.value);
            let attr = Attr { prefix, local, ns: None, value };
            if attr.is_ns_decl() {
                let bound = if attr.prefix.is_some() { Some(attr.local.clone()) } else { None };
                let uri = (!attr.value.is_empty()).then(|| self.intern(&attr.value));
                self.scope.push((bound, uri));
                pushed += 1;
            }
            attrs.push(attr);
        }
        self.pushed.push(pushed);

        let xmlns = self.intern(ns::XMLNS);
        for a in &mut attrs {
            a.ns = if a.is_ns_decl() {
                Some(xmlns.clone())
            } else {
                // Unprefixed attributes are in no namespace, whatever the default is.
                a.prefix.as_deref().and_then(|p| self.lookup(Some(p)))
            };
        }
        let (prefix, local) = split(start.name().0);
        let ns = self.lookup(prefix.as_deref());
        // QName values and markup-compatibility prefix lists need their bindings even when
        // this element is later written apart from its ancestors.
        let mut needed = Vec::new();
        for a in attrs.iter().filter(|a| !a.is_ns_decl()) {
            let prefix_list = (ns.as_deref().is_some_and(|n| n == super::ns::MC) && a.local == "Requires")
                || (a.ns.as_deref().is_some_and(|n| n == super::ns::MC) && matches!(a.local.as_str(), "Ignorable" | "MustUnderstand"));
            if !prefix_list && !a.value.as_bytes().contains(&b':') {
                continue;
            }
            for token in a.value.split_whitespace() {
                let p = match token.split_once(':') {
                    Some((p, _)) => p,
                    None if prefix_list => token,
                    None => continue,
                };
                if p == "xml"
                    || attrs.iter().any(|a| a.is_ns_decl() && a.prefix.is_some() && a.local == p)
                    || needed.iter().any(|a: &Attr| a.local == p)
                {
                    continue;
                }
                if let Some(uri) = self.lookup(Some(p)) {
                    needed.push(Attr { prefix: Some("xmlns".into()), local: p.into(), ns: Some(xmlns.clone()), value: uri.to_string() });
                }
            }
        }
        attrs.extend(needed);
        Ok(Element { prefix, local, ns, attrs, children: Vec::new() })
    }

    fn close(&mut self) {
        let n = self.pushed.pop().unwrap_or(0);
        self.scope.truncate(self.scope.len() - n);
    }
}

fn split(qname: &str) -> (Option<String>, String) {
    match qname.split_once(':') {
        Some((p, l)) => (Some(p.to_string()), l.to_string()),
        None => (None, qname.to_string()),
    }
}

/// Undoes escaping in an attribute value and normalizes literal whitespace to spaces, as an XML
/// processor must (escaped `&#10;` stays a newline).
fn unescape_attr(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find(['&', '\t', '\n', '\r']) {
        out.push_str(&rest[..i]);
        let c = rest.as_bytes()[i];
        rest = &rest[i + 1..];
        if c != b'&' {
            // A CR LF pair is one line break, so one space.
            if c == b'\r' && rest.starts_with('\n') {
                rest = &rest[1..];
            }
            out.push(' ');
            continue;
        }
        let Some(end) = rest.find(';') else {
            out.push('&');
            continue;
        };
        let name = &rest[..end];
        let resolved = if let Some(num) = name.strip_prefix('#') {
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => num.parse().ok(),
            };
            code.and_then(char::from_u32).map(String::from)
        } else {
            predefined(name).map(str::to_string)
        };
        match resolved {
            Some(s) => {
                out.push_str(&s);
                rest = &rest[end + 1..];
            }
            None => out.push('&'),
        }
    }
    out.push_str(rest);
    out
}
