//! Editing one document from several copies at once.
//!
//! Every copy opens the same bytes, then applies the same changes in the same order. A relay
//! (on the sharing person's computer now, a server later) puts the changes in order and sends
//! each one to every copy, the sender included. It never reads them, so it doesn't need the
//! engine.
//!
//! A change is the layer's undo steps (a paragraph replaced, a slide changed, cells changed),
//! each sent with a fingerprint of what it replaced and the new content. A copy applies a step
//! only if what's there now has that fingerprint. Otherwise someone changed the same thing first
//! and the step is dropped, by every copy alike, since they all have the same document when the
//! step's turn comes. Before checking, a layer may move a step to where earlier changes put its
//! content: a paragraph typed in after someone added one above it is one further down.
//!
//! Local changes show at once. Until the relay sends one back it's pending. When any change
//! arrives, the pending ones are taken back, the arrival is applied, and the pending ones are
//! put on again (or dropped, if they no longer fit). So every copy agrees once the pending
//! changes have come back.

use std::collections::{HashSet, VecDeque};

use office_core::model::{Document, MediaData};
use serde::Serialize;
use serde_json::{json, Value};

use crate::history::Step;

/// How many applied changes are kept for moving steps that were made without them.
const RECENT: usize = 512;

/// What a layer does to take part.
pub trait Shared {
    type Step: Step;
    /// Where the person is in the document (their selection), kept across others' changes.
    type Mark;

    fn replica(&mut self) -> &mut Option<Replica<Self::Step>>;
    /// A step as sent: where it applies, the fingerprint of what it replaced, the new content.
    fn send_step(&self, step: &Self::Step) -> Value;
    /// Applies a sent step if what it replaced is still there, moved past `since` (steps other
    /// copies made that the sender hadn't seen). Returns the step as applied here. Neither
    /// records nor sends it.
    fn take_step(&mut self, step: &Value, since: &[&Self::Step]) -> Option<Self::Step>;
    /// Takes back a step this copy applied, without recording or sending it.
    fn revert_step(&mut self, step: &Self::Step);
    fn doc(&mut self) -> &mut Document;
    fn mark(&self) -> Self::Mark;
    /// Puts the selection back, moved past `through` (steps just applied from another copy).
    fn restore(&mut self, mark: Self::Mark, through: &[Self::Step]);
    /// What else a change needs sent along: a layer's own tables that steps refer into.
    fn send_extra(&mut self) -> Value {
        Value::Null
    }
    fn take_extra(&mut self, _extra: &Value) {}
}

struct Pending<S> {
    id: String,
    base: u64,
    sent: Value,
    /// The steps as they're applied here, or `None` while they don't fit.
    applied: Option<Vec<S>>,
}

struct Applied<S> {
    seq: u64,
    client: String,
    steps: Vec<S>,
}

/// One copy's part in a shared document.
pub struct Replica<S> {
    client: String,
    /// The last change from the relay that's been applied.
    seq: u64,
    count: u64,
    /// Steps made here since the last [`take`].
    outbox: Vec<S>,
    pending: VecDeque<Pending<S>>,
    recent: VecDeque<Applied<S>>,
    /// Pictures every copy has.
    media: HashSet<String>,
}

impl<S: Step> Replica<S> {
    pub fn new(client: &str, doc: &Document) -> Replica<S> {
        Replica {
            client: client.to_string(),
            seq: 0,
            count: 0,
            outbox: Vec::new(),
            pending: VecDeque::new(),
            recent: VecDeque::new(),
            media: doc.media.iter().map(|m| m.id.clone()).collect(),
        }
    }

    /// Notes steps made here, to send.
    pub fn made(&mut self, steps: impl IntoIterator<Item = S>) {
        for s in steps {
            if let Some(last) = self.outbox.last_mut() {
                if last.absorb(&s) {
                    continue;
                }
            }
            self.outbox.push(s);
        }
    }

    pub fn client(&self) -> &str {
        &self.client
    }

    /// A short form of this copy's id, for naming what it adds (pictures, parts) so another
    /// copy can't pick the same name.
    pub fn tag(&self) -> String {
        self.client.chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.outbox.is_empty()
    }
}

/// The steps applied since `base` that `author` made without: the other copies' ones.
fn since<'a, S>(recent: &'a VecDeque<Applied<S>>, base: u64, author: &str) -> Vec<&'a S> {
    recent.iter().filter(|a| a.seq > base && a.client != author).flat_map(|a| a.steps.iter()).collect()
}

/// Adds a picture to `doc` and returns its id. While shared, the id names this copy, so two
/// copies adding pictures at once don't pick the same one.
pub fn add_media<S: Step>(doc: &mut Document, replica: Option<&Replica<S>>, content_type: &str, bytes: Vec<u8>) -> String {
    let Some(r) = replica else { return doc.add_media(content_type, bytes) };
    let tag = r.tag();
    let id = (1..).map(|n| format!("new-{tag}:{n}")).find(|id| doc.media(id).is_none()).unwrap();
    doc.media.push(office_core::model::Media {
        id: id.clone(),
        content_type: content_type.to_string(),
        data: MediaData::Bytes(bytes.into()),
    });
    id
}

/// The changes made here since the last call, for the relay: `{ id, base, steps }`. `None` when
/// there are none.
pub fn take<V: Shared>(v: &mut V) -> Option<Value> {
    let mut r = v.replica().take()?;
    let out = (|| {
        if r.outbox.is_empty() {
            return None;
        }
        let steps = std::mem::take(&mut r.outbox);
        let sent: Vec<Value> = steps.iter().map(|s| v.send_step(s)).collect();
        let media = send_media(v.doc(), &mut r.media);
        let extra = v.send_extra();
        r.count += 1;
        let id = format!("{}:{}", r.client, r.count);
        let body = json!({ "steps": sent, "media": media, "extra": extra });
        r.pending.push_back(Pending { id: id.clone(), base: r.seq, sent: body.clone(), applied: Some(steps) });
        Some(json!({ "id": id, "base": r.seq, "steps": body }))
    })();
    *v.replica() = Some(r);
    out
}

/// Applies a change from the relay: `{ seq, client, id, base, steps }`. Returns whether the
/// document changed.
pub fn receive<V: Shared>(v: &mut V, op: &Value) -> bool {
    let Some(mut r) = v.replica().take() else { return false };
    let seq = op["seq"].as_u64().unwrap_or(0);
    let changed = seq > r.seq && {
        let client = op["client"].as_str().unwrap_or("").to_string();
        let id = op["id"].as_str().unwrap_or("");
        let base = op["base"].as_u64().unwrap_or(0);
        let mark = v.mark();
        for p in r.pending.iter_mut().rev() {
            for s in p.applied.take().unwrap_or_default().iter().rev() {
                v.revert_step(s);
            }
        }
        let mine = client == r.client;
        if mine && r.pending.front().is_some_and(|p| p.id == id) {
            r.pending.pop_front();
        }
        let applied = apply(v, &op["steps"], &mut r.media, &since(&r.recent, base, &client));
        r.seq = seq;
        let moved = if mine { Vec::new() } else { applied.clone().unwrap_or_default() };
        if let Some(steps) = applied {
            r.recent.push_back(Applied { seq, client, steps });
            if r.recent.len() > RECENT {
                r.recent.pop_front();
            }
        }
        for p in r.pending.iter_mut() {
            p.applied = apply(v, &p.sent, &mut r.media, &since(&r.recent, p.base, &r.client));
        }
        v.restore(mark, &moved);
        true
    };
    *v.replica() = Some(r);
    changed
}

/// Applies a change's steps, all or none.
fn apply<V: Shared>(v: &mut V, body: &Value, media: &mut HashSet<String>, since: &[&V::Step]) -> Option<Vec<V::Step>> {
    take_media(v.doc(), &body["media"], media);
    v.take_extra(&body["extra"]);
    let mut done = Vec::new();
    for s in body["steps"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        match v.take_step(s, since) {
            Some(step) => done.push(step),
            None => {
                for s in done.iter().rev() {
                    v.revert_step(s);
                }
                return None;
            }
        }
    }
    Some(done)
}

/// Undoes or redoes `steps` (already reversed for undo) where the document is now, which other
/// copies may have changed: each goes through the same check as a step from another copy, and
/// none applies unless all do. Returns the steps as applied, to send.
pub fn replay<V: Shared>(v: &mut V, steps: &[V::Step]) -> Option<Vec<V::Step>> {
    let sent: Vec<Value> = steps.iter().map(|s| v.send_step(s)).collect();
    let mut done = Vec::new();
    for s in &sent {
        match v.take_step(s, &[]) {
            Some(step) => done.push(step),
            None => {
                for s in done.iter().rev() {
                    v.revert_step(s);
                }
                return None;
            }
        }
    }
    Some(done)
}

/// A short fingerprint of a value's JSON (FNV-1a), to check two copies hold the same content.
pub fn fingerprint<T: Serialize + ?Sized>(v: &T) -> String {
    struct Fnv(u64);
    impl std::io::Write for Fnv {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            for b in buf {
                self.0 ^= *b as u64;
                self.0 = self.0.wrapping_mul(0x100000001b3);
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut h = Fnv(0xcbf29ce484222325);
    let _ = serde_json::to_writer(&mut h, v);
    format!("{:016x}", h.0)
}

/// Serializes new content for sending, with its origins sent by place in `old` where they
/// came from there (see [`office_core::model::wire`]).
pub fn send_content<O: Serialize + ?Sized, N: Serialize + ?Sized>(old: &O, new: &N) -> Value {
    let (_, origins) = office_core::model::wire::collect(|| serde_json::to_writer(std::io::sink(), old));
    office_core::model::wire::encode(&origins, || serde_json::to_value(new).unwrap_or(Value::Null))
}

/// Reads content sent by [`send_content`], taking origins from `old`, this copy's own old content.
pub fn take_content<O: Serialize + ?Sized, N: serde::de::DeserializeOwned>(old: &O, new: &Value) -> Option<N> {
    let (_, origins) = office_core::model::wire::collect(|| serde_json::to_writer(std::io::sink(), old));
    office_core::model::wire::decode(origins, || serde_json::from_value(new.clone()).ok())
}

fn send_media(doc: &Document, known: &mut HashSet<String>) -> Value {
    let mut out = Vec::new();
    for m in &doc.media {
        if known.contains(&m.id) {
            continue;
        }
        if let Ok(bytes) = doc.media_bytes(&m.id) {
            out.push(json!({ "id": m.id, "type": m.content_type, "bytes": base64(&bytes) }));
        }
        known.insert(m.id.clone());
    }
    Value::Array(out)
}

fn take_media(doc: &mut Document, sent: &Value, known: &mut HashSet<String>) {
    for m in sent.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let (Some(id), Some(kind), Some(bytes)) = (m["id"].as_str(), m["type"].as_str(), m["bytes"].as_str().and_then(unbase64)) else {
            continue;
        };
        known.insert(id.to_string());
        if doc.media(id).is_none() {
            doc.media.push(office_core::model::Media {
                id: id.to_string(),
                content_type: kind.to_string(),
                data: MediaData::Bytes(bytes.into()),
            });
        }
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let mut table = [255u8; 256];
    for (i, &c) in B64.iter().enumerate() {
        table[c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut n, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=') {
        let v = table[c as usize];
        if v == 255 {
            return None;
        }
        let v = v as u32;
        n = n << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
            n &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for len in 0..10 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(unbase64(&base64(&bytes)).unwrap(), bytes);
        }
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
    }

    #[test]
    fn fingerprints_follow_content() {
        assert_eq!(fingerprint(&vec![1, 2]), fingerprint(&vec![1, 2]));
        assert_ne!(fingerprint(&vec![1, 2]), fingerprint(&vec![2, 1]));
    }
}
