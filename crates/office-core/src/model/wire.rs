//! The model as one copy of a shared document sends a change to the others.
//!
//! The model's ordinary JSON leaves out what it doesn't need to show a node: each node's
//! [`Origin`] (the markup the model didn't read) and a few things kept for saving. A change sent
//! to another copy needs them, or that copy would save a paragraph without its unknown
//! properties. So serializing inside [`encode`] includes them, and deserializing inside
//! [`decode`] reads them back.
//!
//! Origins are large and mostly unchanged by an edit: typing in a paragraph keeps its origin. So
//! a change is sent as what it replaced and what replaced it, and an origin in the new content
//! that was already in the old is sent as its place in the old content (`{"r": 3}`). The copy
//! receiving it collects the origins of its own old content, which is the same as the sender's,
//! in the same order ([`collect`]), and takes the origin from there, so it's shared just as the
//! sender's is. An origin that's new is sent whole (`{"x": "<w:p …>"}`).
//!
//! Outside these three calls, nothing changes: the model's JSON is as before.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::Origin;
use crate::xml::{parse_element, Element};

enum Mode {
    Off,
    /// Serializing old content: note each origin, in order.
    Collect(Vec<Arc<Element>>),
    /// Serializing new content: origins found in the old by address are sent by place.
    Encode(HashMap<usize, usize>),
    /// Deserializing new content against the receiver's old content's origins.
    Decode(Vec<Arc<Element>>),
}

thread_local! {
    static MODE: RefCell<Mode> = const { RefCell::new(Mode::Off) };
}

/// Puts the mode back when the call ends, even by an error or a panic.
struct Scope;

impl Scope {
    fn enter(mode: Mode) -> Scope {
        MODE.with(|m| *m.borrow_mut() = mode);
        Scope
    }

    fn leave() -> Mode {
        MODE.with(|m| std::mem::replace(&mut *m.borrow_mut(), Mode::Off))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        Scope::leave();
    }
}

/// Runs `f` (which serializes old content) and returns the origins it passed, in order.
pub fn collect<R>(f: impl FnOnce() -> R) -> (R, Vec<Arc<Element>>) {
    let scope = Scope::enter(Mode::Collect(Vec::new()));
    let r = f();
    let found = match Scope::leave() {
        Mode::Collect(v) => v,
        _ => Vec::new(),
    };
    drop(scope);
    (r, found)
}

/// Runs `f` (which serializes new content) so that origins among `old` are sent by place.
pub fn encode<R>(old: &[Arc<Element>], f: impl FnOnce() -> R) -> R {
    let mut places = HashMap::new();
    for (i, a) in old.iter().enumerate() {
        places.entry(Arc::as_ptr(a) as *const () as usize).or_insert(i);
    }
    let _scope = Scope::enter(Mode::Encode(places));
    f()
}

/// Runs `f` (which deserializes new content) so that origins sent by place come from `old`.
pub fn decode<R>(old: Vec<Arc<Element>>, f: impl FnOnce() -> R) -> R {
    let _scope = Scope::enter(Mode::Decode(old));
    f()
}

/// Whether a field kept out of the model's JSON stays out: everywhere but inside [`collect`] and
/// [`encode`]. For `skip_serializing_if`.
pub fn off<T>(_: &T) -> bool {
    MODE.with(|m| matches!(*m.borrow(), Mode::Off | Mode::Decode(_)))
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Sent {
    Place { r: usize },
    Whole { x: String },
}

/// `with` for an [`Origin`] field.
pub mod origin {
    use super::*;

    pub fn serialize<S: Serializer>(o: &Origin, s: S) -> Result<S::Ok, S::Error> {
        let Some(el) = &o.0 else { return s.serialize_none() };
        let sent = MODE.with(|m| match &mut *m.borrow_mut() {
            Mode::Collect(found) => {
                found.push(el.clone());
                None
            }
            Mode::Encode(places) => Some(match places.get(&(Arc::as_ptr(el) as *const () as usize)) {
                Some(&r) => Sent::Place { r },
                None => Sent::Whole { x: el.to_xml_string() },
            }),
            _ => None,
        });
        match sent {
            Some(sent) => s.serialize_some(&sent),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Origin, D::Error> {
        let Some(sent) = Option::<Sent>::deserialize(d)? else { return Ok(Origin::default()) };
        Ok(match sent {
            Sent::Place { r } => Origin(MODE.with(|m| match &*m.borrow() {
                Mode::Decode(old) => old.get(r).cloned(),
                _ => None,
            })),
            Sent::Whole { x } => parse_element(&x).map(Origin::new).map_err(serde::de::Error::custom)?,
        })
    }
}

/// `with` for a block's preview (`Arc<[T]>`), which serde doesn't do without its `rc` feature.
pub mod shared_slice {
    use super::*;

    pub fn serialize<S: Serializer, T: Serialize>(v: &Arc<[T]>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Arc<[T]>, D::Error> {
        Ok(Arc::from(Vec::<T>::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Inline, Paragraph, Run};
    use crate::xml::ns;

    fn para(text: &str, origin: &Origin) -> Block {
        let run = Run { text: text.into(), origin: origin.clone(), ..Default::default() };
        Block::Paragraph(Paragraph { content: vec![Inline::Text(run)], origin: origin.clone(), ..Default::default() })
    }

    fn origin_of(b: &Block) -> &Origin {
        match b {
            Block::Paragraph(p) => &p.origin,
            _ => unreachable!(),
        }
    }

    #[test]
    fn plain_json_is_unchanged() {
        let o = Origin::new(Element::new(ns::W, "p"));
        let json = serde_json::to_string(&para("Hi", &o)).unwrap();
        assert!(!json.contains("origin"), "{json}");
    }

    #[test]
    fn origins_go_by_place_and_come_back_shared() {
        let kept = Origin::new(Element::new(ns::W, "p").with_attr("", "id", "1"));
        let before = vec![para("Hello", &kept)];
        let mut after = before.clone();
        after.push(para("new", &Origin::new(Element::new(ns::W, "p").with_attr("", "id", "2"))));

        let (_, old) = collect(|| serde_json::to_value(&before).unwrap());
        let sent = encode(&old, || serde_json::to_value(&after).unwrap());
        let text = sent.to_string();
        assert!(text.contains(r#""r":0"#), "{text}");
        assert!(text.contains(r#""x":"#), "{text}");

        // The receiver has its own copy of the old content, with its own origins.
        let theirs = vec![para("Hello", &Origin::new(Element::new(ns::W, "p").with_attr("", "id", "1")))];
        let (_, their_old) = collect(|| serde_json::to_value(&theirs).unwrap());
        let got: Vec<Block> = decode(their_old, || serde_json::from_value(sent).unwrap());
        assert_eq!(got, after);
        assert_eq!(origin_of(&got[0]).id(), origin_of(&theirs[0]).id());
        assert_eq!(origin_of(&got[1]).get().unwrap().attr("id"), Some("2"));
    }
}
