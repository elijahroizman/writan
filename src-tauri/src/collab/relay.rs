//! The relay at the middle of a shared document: it puts everyone's edits in one order and hands
//! that order to everyone. It never looks inside an edit. Each client applies the same ordered
//! log the same way, so they all end up with the same document.
//!
//! This is a state machine with no sockets and no Tauri: text comes in through `handle`, and what
//! to send (or which connection to close) comes back as `Outgoing`. The LAN server in `server.rs`
//! drives it today; a cloud server can drive the same code later.
//!
//! The whole log stays in memory for as long as the session lasts, and every newcomer gets all of
//! it. Compacting it into a fresh snapshot is a later step.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// The protocol version in `hello` and `welcome`.
pub const VERSION: u32 = 1;

/// The colors people get, in the order they're handed out. The page has the same list.
pub const COLORS: [&str; 8] = [
    "#e5484d", "#3e63dd", "#30a46c", "#f76b15", "#8e4ec6", "#12a594", "#d6409f", "#ad7f58",
];

/// Every frame, in either direction, is at most this big.
pub const MAX_FRAME: usize = 64 << 20;

/// The transport's name for one connection.
pub type ConnId = u64;

/// What the transport should do after a message: send text to a connection, or close it once
/// everything sent before has gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Send(ConnId, String),
    Close(ConnId),
}

/// Where sessions come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Only sessions made with `Relay::register` exist. The app uses this: sharing a document
    /// registers it, and anyone else's session id is turned away.
    Registered,
    /// The first hello with a snapshot creates its session, and its token and host key become
    /// the session's. For the standalone relay used by tests, and a future cloud prototype.
    Open,
}

struct Snapshot {
    name: String,
    kind: String,
    bytes: String,
}

struct Member {
    conn: ConnId,
    client: String,
    name: String,
    color: String,
    host: bool,
    /// Which AI agent, for one that joined through Writan's MCP server (`claude`, `codex`…).
    agent: Option<String>,
    presence: Option<Box<RawValue>>,
}

struct Session {
    token: String,
    host_key: String,
    snapshot: Option<Snapshot>,
    /// Each op as it appears in `welcome`, `{"seq":…,"client":…,"id":…,"base":…,"steps":…}`,
    /// kept as text so it's serialized once.
    log: Vec<Box<RawValue>>,
    seq: u64,
    /// In the order they joined.
    members: Vec<Member>,
}

impl Session {
    fn new(token: String, host_key: String) -> Self {
        Self {
            token,
            host_key,
            snapshot: None,
            log: Vec::new(),
            seq: 0,
            members: Vec::new(),
        }
    }
}

/// A connection's place: the session its URL named, and once its hello is accepted, who it is.
struct Conn {
    session: String,
    joined: Option<Joined>,
}

struct Joined {
    client: String,
    host: bool,
}

pub struct Relay {
    policy: Policy,
    sessions: HashMap<String, Session>,
    conns: HashMap<ConnId, Conn>,
}

// What clients send. Parsed in two steps (the tag, then the body) because serde's tagged enums
// can't hold a RawValue.

#[derive(Deserialize)]
struct Tag<'a> {
    #[serde(borrow)]
    t: std::borrow::Cow<'a, str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hello {
    v: u32,
    token: String,
    client: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    host_key: Option<String>,
    #[serde(default)]
    snapshot: Option<SnapshotIn>,
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Deserialize)]
struct SnapshotIn {
    name: String,
    kind: String,
    bytes: String,
}

#[derive(Deserialize)]
struct OpIn<'a> {
    id: String,
    base: u64,
    #[serde(borrow)]
    steps: &'a RawValue,
}

#[derive(Deserialize)]
struct PresenceIn<'a> {
    #[serde(borrow)]
    data: &'a RawValue,
}

// What the relay sends.

#[derive(Serialize)]
struct Welcome<'a> {
    t: &'static str,
    v: u32,
    client: &'a str,
    /// The color this copy shows as: the one it asked for, unless someone has it.
    color: &'a str,
    host: bool,
    seq: u64,
    session: SessionInfo<'a>,
    snapshot: &'a str,
    ops: &'a [Box<RawValue>],
    peers: Vec<Peer<'a>>,
}

#[derive(Serialize)]
struct SessionInfo<'a> {
    name: &'a str,
    kind: &'a str,
}

#[derive(Serialize)]
struct Peer<'a> {
    client: &'a str,
    name: &'a str,
    color: &'a str,
    host: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<&'a str>,
    presence: Option<&'a RawValue>,
}

#[derive(Serialize)]
struct OpEntry<'a> {
    seq: u64,
    client: &'a str,
    id: &'a str,
    base: u64,
    steps: &'a RawValue,
}

#[derive(Serialize)]
struct Join<'a> {
    t: &'static str,
    client: &'a str,
    name: &'a str,
    color: &'a str,
    host: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<&'a str>,
}

#[derive(Serialize)]
struct Leave<'a> {
    t: &'static str,
    client: &'a str,
}

#[derive(Serialize)]
struct PresenceOut<'a> {
    t: &'static str,
    client: &'a str,
    data: &'a RawValue,
}

#[derive(Serialize)]
struct ErrorOut<'a> {
    t: &'static str,
    code: &'a str,
    message: &'a str,
}

/// Compares secrets in time that doesn't depend on where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// `#rgb` or `#rrggbb`.
fn is_color(c: &str) -> bool {
    c.strip_prefix('#')
        .is_some_and(|h| (h.len() == 3 || h.len() == 6) && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A color nobody in `members` has: `wanted` if it's free, else the first free one in
/// `COLORS`. With every color taken, they repeat in order.
fn pick_color(members: &[Member], wanted: &str) -> String {
    let used = |c: &str| members.iter().any(|m| m.color.eq_ignore_ascii_case(c));
    if is_color(wanted) && !used(wanted) {
        return wanted.to_owned();
    }
    COLORS
        .iter()
        .find(|c| !used(c))
        .unwrap_or(&COLORS[members.len() % COLORS.len()])
        .to_string()
}

fn json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("relay messages always serialize")
}

fn error(conn: ConnId, code: &str, message: &str) -> Outgoing {
    Outgoing::Send(
        conn,
        json(&ErrorOut {
            t: "error",
            code,
            message,
        }),
    )
}

/// An error, then the connection closed.
fn refuse(conn: ConnId, code: &str, message: &str) -> Vec<Outgoing> {
    vec![error(conn, code, message), Outgoing::Close(conn)]
}

fn leave(client: &str) -> String {
    json(&Leave { t: "leave", client })
}

/// Sends `text` to every member of `session` except `skip`.
fn broadcast(session: &Session, skip: Option<ConnId>, text: &str, out: &mut Vec<Outgoing>) {
    for m in &session.members {
        if Some(m.conn) != skip {
            out.push(Outgoing::Send(m.conn, text.to_owned()));
        }
    }
}

impl Relay {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            sessions: HashMap::new(),
            conns: HashMap::new(),
        }
    }

    /// Makes a session that guests can join with `token`, and that only a hello with `host_key`
    /// can start (by bringing the snapshot) and end. Registering an id again replaces its keys
    /// but keeps its document.
    pub fn register(&mut self, session: &str, token: &str, host_key: &str) {
        match self.sessions.get_mut(session) {
            Some(s) => {
                s.token = token.to_owned();
                s.host_key = host_key.to_owned();
            }
            None => {
                self.sessions.insert(
                    session.to_owned(),
                    Session::new(token.to_owned(), host_key.to_owned()),
                );
            }
        }
    }

    pub fn has_session(&self, session: &str) -> bool {
        self.sessions.contains_key(session)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Ends every session, as `end_session` does each.
    pub fn end_all(&mut self) -> Vec<Outgoing> {
        let mut ids: Vec<String> = self.sessions.keys().cloned().collect();
        ids.sort_unstable();
        ids.iter().flat_map(|id| self.end_session(id)).collect()
    }

    /// A new connection, for the session named in its URL. Nothing is sent until its hello.
    pub fn connect(&mut self, conn: ConnId, session: &str) {
        self.conns.insert(
            conn,
            Conn {
                session: session.to_owned(),
                joined: None,
            },
        );
    }

    /// One text frame from `conn`.
    pub fn handle(&mut self, conn: ConnId, text: &str) -> Vec<Outgoing> {
        let Some(c) = self.conns.get(&conn) else {
            // Not connected (or already closed): there's nobody to answer.
            return Vec::new();
        };
        let joined = c.joined.is_some();
        let Ok(tag) = serde_json::from_str::<Tag>(text) else {
            return if joined {
                vec![error(conn, "bad-message", "Not a message the relay knows.")]
            } else {
                refuse(conn, "bad-message", "The first message must be a hello.")
            };
        };
        if !joined {
            return match &*tag.t {
                "hello" => match serde_json::from_str::<Hello>(text) {
                    Ok(hello) => self.hello(conn, hello),
                    Err(e) => refuse(conn, "bad-message", &format!("Bad hello: {e}")),
                },
                _ => refuse(conn, "bad-message", "The first message must be a hello."),
            };
        }
        match &*tag.t {
            "op" => match serde_json::from_str::<OpIn>(text) {
                Ok(op) => self.op(conn, op),
                Err(e) => vec![error(conn, "bad-message", &format!("Bad op: {e}"))],
            },
            "presence" => match serde_json::from_str::<PresenceIn>(text) {
                Ok(p) => self.presence(conn, p.data),
                Err(e) => vec![error(conn, "bad-message", &format!("Bad presence: {e}"))],
            },
            "end" => {
                let (session, host) = self.place(conn);
                if host {
                    self.end_session(&session)
                } else {
                    vec![error(
                        conn,
                        "not-host",
                        "Only the host can end the session.",
                    )]
                }
            }
            "bye" => {
                let mut out = self.disconnect(conn);
                out.push(Outgoing::Close(conn));
                out
            }
            "hello" => vec![error(conn, "bad-message", "Already said hello.")],
            _ => vec![error(conn, "bad-message", "Not a message the relay knows.")],
        }
    }

    /// The connection went away. Tells the others it left.
    pub fn disconnect(&mut self, conn: ConnId) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let Some(c) = self.conns.remove(&conn) else {
            return out;
        };
        let (Some(joined), Some(session)) = (c.joined, self.sessions.get_mut(&c.session)) else {
            return out;
        };
        session.members.retain(|m| m.conn != conn);
        // On the LAN the host's copy is the file: without it, edits would go nowhere.
        if joined.host && self.policy == Policy::Registered {
            return self.end_session(&c.session);
        }
        broadcast(session, None, &leave(&joined.client), &mut out);
        out
    }

    /// Ends the session for everyone, as the host's `end` does: they're told why, then closed.
    pub fn end_session(&mut self, session: &str) -> Vec<Outgoing> {
        let mut out = Vec::new();
        if self.sessions.remove(session).is_none() {
            return out;
        }
        let end = json(&serde_json::json!({ "t": "end", "reason": "host-stopped" }));
        let mut ids: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|(_, c)| c.session == session)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        for id in ids {
            let c = self.conns.remove(&id).expect("listed above");
            if c.joined.is_some() {
                out.push(Outgoing::Send(id, end.clone()));
            }
            out.push(Outgoing::Close(id));
        }
        out
    }

    /// The session id and host flag of a joined connection.
    fn place(&self, conn: ConnId) -> (String, bool) {
        let c = &self.conns[&conn];
        let host = c.joined.as_ref().is_some_and(|j| j.host);
        (c.session.clone(), host)
    }

    fn hello(&mut self, conn: ConnId, hello: Hello) -> Vec<Outgoing> {
        if hello.v != VERSION {
            return refuse(
                conn,
                "version",
                &format!(
                    "This relay speaks version {VERSION}, not {}. Update Writan.",
                    hello.v
                ),
            );
        }
        let session_id = self.conns[&conn].session.clone();
        if !self.sessions.contains_key(&session_id) {
            match (self.policy, &hello.snapshot) {
                (Policy::Registered, _) => {
                    return refuse(conn, "no-session", "Nothing is shared at this address.")
                }
                (Policy::Open, None) => {
                    return refuse(
                        conn,
                        "not-ready",
                        "The host hasn't shared the document yet.",
                    )
                }
                (Policy::Open, Some(_)) => {
                    let Some(host_key) = hello.host_key.clone() else {
                        return refuse(conn, "bad-message", "A new session needs a hostKey.");
                    };
                    self.sessions.insert(
                        session_id.clone(),
                        Session::new(hello.token.clone(), host_key),
                    );
                }
            }
        }
        let session = self.sessions.get_mut(&session_id).expect("made above");
        if !same(&hello.token, &session.token) {
            return refuse(conn, "bad-token", "The invitation's token is wrong.");
        }
        let host = hello
            .host_key
            .as_ref()
            .is_some_and(|k| same(k, &session.host_key));
        if session.snapshot.is_none() {
            match hello.snapshot {
                // The host's first hello brings the document. A host coming back later can
                // send it again; the relay keeps the one it has.
                Some(s) if host => {
                    session.snapshot = Some(Snapshot {
                        name: s.name,
                        kind: s.kind,
                        bytes: s.bytes,
                    })
                }
                _ => {
                    return refuse(
                        conn,
                        "not-ready",
                        "The host hasn't shared the document yet.",
                    )
                }
            }
        }

        let mut out = Vec::new();
        // The same client id again (a reconnect, a second window): the old connection goes,
        // and the others see it leave and come back.
        if let Some(i) = session
            .members
            .iter()
            .position(|m| m.client == hello.client)
        {
            let old = session.members.remove(i);
            self.conns.remove(&old.conn);
            out.push(Outgoing::Close(old.conn));
            broadcast(session, None, &leave(&old.client), &mut out);
        }

        // An agent keeps its own color (Claude's clay…) even if someone has it: its logo and
        // name tell it apart.
        let color = if hello.agent.is_some() && is_color(&hello.color) {
            hello.color.clone()
        } else {
            pick_color(&session.members, &hello.color)
        };
        let snapshot = session.snapshot.as_ref().expect("set above");
        let welcome = json(&Welcome {
            t: "welcome",
            v: VERSION,
            client: &hello.client,
            color: &color,
            host,
            seq: session.seq,
            session: SessionInfo {
                name: &snapshot.name,
                kind: &snapshot.kind,
            },
            snapshot: &snapshot.bytes,
            ops: &session.log,
            peers: session
                .members
                .iter()
                .map(|m| Peer {
                    client: &m.client,
                    name: &m.name,
                    color: &m.color,
                    host: m.host,
                    agent: m.agent.as_deref(),
                    presence: m.presence.as_deref(),
                })
                .collect(),
        });
        out.push(Outgoing::Send(conn, welcome));
        let join = json(&Join {
            t: "join",
            client: &hello.client,
            name: &hello.name,
            color: &color,
            host,
            agent: hello.agent.as_deref(),
        });
        broadcast(session, None, &join, &mut out);
        session.members.push(Member {
            conn,
            client: hello.client.clone(),
            name: hello.name,
            color,
            host,
            agent: hello.agent,
            presence: None,
        });
        self.conns.get_mut(&conn).expect("connected").joined = Some(Joined {
            client: hello.client,
            host,
        });
        out
    }

    fn op(&mut self, conn: ConnId, op: OpIn) -> Vec<Outgoing> {
        let c = &self.conns[&conn];
        let client = &c.joined.as_ref().expect("joined").client;
        let Some(session) = self.sessions.get_mut(&c.session) else {
            return vec![error(conn, "no-session", "The session has ended.")];
        };
        session.seq += 1;
        let entry = json(&OpEntry {
            seq: session.seq,
            client,
            id: &op.id,
            base: op.base,
            steps: op.steps,
        });
        // The broadcast is the log entry with its tag in front: `{"t":"op",` + `"seq":…}`.
        let frame = format!(r#"{{"t":"op",{}"#, &entry[1..]);
        let mut out = Vec::new();
        broadcast(session, None, &frame, &mut out);
        session
            .log
            .push(RawValue::from_string(entry).expect("serialized above"));
        out
    }

    fn presence(&mut self, conn: ConnId, data: &RawValue) -> Vec<Outgoing> {
        let c = &self.conns[&conn];
        let client = &c.joined.as_ref().expect("joined").client;
        let Some(session) = self.sessions.get_mut(&c.session) else {
            return vec![error(conn, "no-session", "The session has ended.")];
        };
        let mut out = Vec::new();
        let frame = json(&PresenceOut {
            t: "presence",
            client,
            data,
        });
        broadcast(session, Some(conn), &frame, &mut out);
        if let Some(m) = session.members.iter_mut().find(|m| m.conn == conn) {
            m.presence = Some(data.to_owned());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const S: &str = "sess";

    fn host_hello(client: &str) -> String {
        json!({
            "t": "hello", "v": 1, "token": "tok", "client": client, "name": "Host", "color": "#f00",
            "hostKey": "hk", "snapshot": { "name": "Report.docx", "kind": "docx", "bytes": "AAAA" },
        })
        .to_string()
    }

    fn guest_hello(client: &str) -> String {
        json!({ "t": "hello", "v": 1, "token": "tok", "client": client, "name": "Guest", "color": "#0f0" })
            .to_string()
    }

    fn registered() -> Relay {
        let mut r = Relay::new(Policy::Registered);
        r.register(S, "tok", "hk");
        r
    }

    /// Connects `conn` and sends its hello.
    fn join(r: &mut Relay, conn: ConnId, hello: &str) -> Vec<Outgoing> {
        r.connect(conn, S);
        r.handle(conn, hello)
    }

    /// The messages sent to `conn`, parsed.
    fn to(out: &[Outgoing], conn: ConnId) -> Vec<Value> {
        out.iter()
            .filter_map(|o| match o {
                Outgoing::Send(c, text) if *c == conn => Some(serde_json::from_str(text).unwrap()),
                _ => None,
            })
            .collect()
    }

    fn closed(out: &[Outgoing], conn: ConnId) -> bool {
        out.contains(&Outgoing::Close(conn))
    }

    fn error_code(out: &[Outgoing], conn: ConnId) -> Option<String> {
        to(out, conn)
            .into_iter()
            .find(|m| m["t"] == "error")
            .map(|m| m["code"].as_str().unwrap().to_owned())
    }

    #[test]
    fn welcome_has_the_document_log_and_peers() {
        let mut r = registered();
        let out = join(&mut r, 1, &host_hello("h"));
        let w = &to(&out, 1)[0];
        assert_eq!(w["t"], "welcome");
        assert_eq!(w["v"], 1);
        assert_eq!(w["client"], "h");
        assert_eq!(w["color"], "#f00");
        assert_eq!(w["host"], true);
        assert_eq!(w["seq"], 0);
        assert_eq!(
            w["session"],
            json!({ "name": "Report.docx", "kind": "docx" })
        );
        assert_eq!(w["snapshot"], "AAAA");
        assert_eq!(w["ops"], json!([]));
        assert_eq!(w["peers"], json!([]));

        r.handle(1, r#"{"t":"op","id":"a","base":0,"steps":[1]}"#);
        r.handle(1, r#"{"t":"presence","data":{"at":3}}"#);
        let out = join(&mut r, 2, &guest_hello("g"));
        let w = &to(&out, 2)[0];
        assert_eq!(w["seq"], 1);
        assert_eq!(
            w["ops"],
            json!([{ "seq": 1, "client": "h", "id": "a", "base": 0, "steps": [1] }])
        );
        assert_eq!(
            w["peers"],
            json!([{ "client": "h", "name": "Host", "color": "#f00", "host": true, "presence": { "at": 3 } }])
        );
        assert_eq!(w["host"], false);
        assert_eq!(
            to(&out, 1),
            vec![
                json!({ "t": "join", "client": "g", "name": "Guest", "color": "#0f0", "host": false })
            ]
        );
    }

    #[test]
    fn everyone_gets_a_color_of_their_own() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h").replace("#f00", COLORS[0]));
        // Asking for a color someone has gets the first free one.
        let out = join(&mut r, 2, &guest_hello("a").replace("#0f0", COLORS[0]));
        assert_eq!(to(&out, 2)[0]["color"], COLORS[1]);
        assert_eq!(to(&out, 1)[0]["color"], COLORS[1]);
        // Not a color at all: the same.
        let out = join(&mut r, 3, &guest_hello("b").replace("#0f0", "red"));
        assert_eq!(to(&out, 3)[0]["color"], COLORS[2]);
        // A free one is kept.
        let out = join(&mut r, 4, &guest_hello("c").replace("#0f0", COLORS[5]));
        assert_eq!(to(&out, 4)[0]["color"], COLORS[5]);
        let colors: Vec<Value> = to(&out, 4)[0]["peers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["color"].clone())
            .collect();
        assert_eq!(
            colors,
            vec![json!(COLORS[0]), json!(COLORS[1]), json!(COLORS[2])]
        );
        // Someone leaving frees theirs.
        r.disconnect(2);
        let out = join(&mut r, 5, &guest_hello("d").replace("#0f0", COLORS[0]));
        assert_eq!(to(&out, 5)[0]["color"], COLORS[1]);
    }

    #[test]
    fn agents_are_named_and_keep_their_color() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h").replace("#f00", COLORS[0]));
        let claude = json!({ "t": "hello", "v": 1, "token": "tok", "client": "a", "name": "Claude", "color": "#D97757", "agent": "claude" });
        let out = join(&mut r, 2, &claude.to_string());
        assert_eq!(to(&out, 1)[0]["agent"], "claude");
        assert_eq!(to(&out, 2)[0]["color"], "#D97757");
        // A second one keeps the color too.
        let out = join(&mut r, 3, &claude.to_string().replace("\"a\"", "\"b\""));
        assert_eq!(to(&out, 3)[0]["color"], "#D97757");
        let peers = &to(&out, 3)[0]["peers"];
        assert_eq!(peers[1]["agent"], "claude");
        // People don't get an agent field at all.
        assert!(peers[0].get("agent").is_none());
    }

    #[test]
    fn wrong_token_and_version_are_refused() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        let bad = guest_hello("g").replace("\"tok\"", "\"nope\"");
        let out = join(&mut r, 2, &bad);
        assert_eq!(error_code(&out, 2).as_deref(), Some("bad-token"));
        assert!(closed(&out, 2));
        assert!(to(&out, 1).is_empty());

        let out = join(&mut r, 3, &guest_hello("g").replace("\"v\":1", "\"v\":2"));
        assert_eq!(error_code(&out, 3).as_deref(), Some("version"));
        assert!(closed(&out, 3));
    }

    #[test]
    fn guests_wait_for_the_host_snapshot() {
        let mut r = registered();
        let out = join(&mut r, 1, &guest_hello("g"));
        assert_eq!(error_code(&out, 1).as_deref(), Some("not-ready"));
        assert!(closed(&out, 1));
        // A guest can't start the session by bringing a snapshot without the host key.
        let fake = host_hello("g").replace("\"hk\"", "\"guess\"");
        let out = join(&mut r, 2, &fake);
        assert_eq!(error_code(&out, 2).as_deref(), Some("not-ready"));
    }

    #[test]
    fn first_message_must_be_hello_and_session_must_exist() {
        let mut r = registered();
        r.connect(1, S);
        let out = r.handle(1, r#"{"t":"op","id":"a","base":0,"steps":[]}"#);
        assert_eq!(error_code(&out, 1).as_deref(), Some("bad-message"));
        assert!(closed(&out, 1));

        r.connect(2, "other");
        let out = r.handle(2, &host_hello("h"));
        assert_eq!(error_code(&out, 2).as_deref(), Some("no-session"));
        assert!(closed(&out, 2));
    }

    #[test]
    fn ops_are_numbered_and_sent_to_everyone() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        join(&mut r, 2, &guest_hello("g"));
        let out = r.handle(2, r#"{"t":"op","id":"x1","base":0,"steps":{"k":[1,2]}}"#);
        let want = json!({ "t": "op", "seq": 1, "client": "g", "id": "x1", "base": 0, "steps": { "k": [1, 2] } });
        assert_eq!(to(&out, 1), vec![want.clone()]);
        assert_eq!(to(&out, 2), vec![want]);
        let out = r.handle(1, r#"{"t":"op","id":"y1","base":1,"steps":[]}"#);
        assert_eq!(to(&out, 2)[0]["seq"], 2);
        assert_eq!(to(&out, 1)[0]["client"], "h");

        let out = r.handle(2, r#"{"t":"op","id":"z","steps":[]}"#);
        assert_eq!(error_code(&out, 2).as_deref(), Some("bad-message"));
        assert!(!closed(&out, 2));
    }

    #[test]
    fn presence_goes_to_the_others() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        join(&mut r, 2, &guest_hello("g"));
        join(&mut r, 3, &guest_hello("g2"));
        let out = r.handle(2, r#"{"t":"presence","data":{"sel":[1,4]}}"#);
        assert!(to(&out, 2).is_empty());
        let want = json!({ "t": "presence", "client": "g", "data": { "sel": [1, 4] } });
        assert_eq!(to(&out, 1), vec![want.clone()]);
        assert_eq!(to(&out, 3), vec![want]);
    }

    #[test]
    fn leaving_is_announced() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        join(&mut r, 2, &guest_hello("g"));
        join(&mut r, 3, &guest_hello("g2"));
        let out = r.disconnect(2);
        let want = json!({ "t": "leave", "client": "g" });
        assert_eq!(to(&out, 1), vec![want.clone()]);
        assert_eq!(to(&out, 3), vec![want.clone()]);
        assert!(r.disconnect(2).is_empty());

        let out = r.handle(3, r#"{"t":"bye"}"#);
        assert_eq!(to(&out, 1), vec![json!({ "t": "leave", "client": "g2" })]);
        assert!(closed(&out, 3));
        // A connection that never joined leaves silently.
        r.connect(9, S);
        assert!(r.disconnect(9).is_empty());
    }

    #[test]
    fn the_host_leaving_ends_a_lan_session() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        join(&mut r, 2, &guest_hello("g"));
        let out = r.disconnect(1);
        assert_eq!(
            to(&out, 2),
            vec![json!({ "t": "end", "reason": "host-stopped" })]
        );
        assert!(closed(&out, 2));
        assert!(!r.has_session(S));
    }

    #[test]
    fn ending_all_ends_every_session() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        join(&mut r, 2, &guest_hello("g"));
        let out = r.end_all();
        assert!(closed(&out, 1) && closed(&out, 2));
        assert_eq!(r.session_count(), 0);
    }

    #[test]
    fn only_the_host_ends_the_session() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        join(&mut r, 2, &guest_hello("g"));
        let out = r.handle(2, r#"{"t":"end"}"#);
        assert_eq!(error_code(&out, 2).as_deref(), Some("not-host"));
        assert!(r.has_session(S));

        let out = r.handle(1, r#"{"t":"end"}"#);
        let end = json!({ "t": "end", "reason": "host-stopped" });
        assert_eq!(to(&out, 1), vec![end.clone()]);
        assert_eq!(to(&out, 2), vec![end]);
        assert!(closed(&out, 1) && closed(&out, 2));
        assert!(!r.has_session(S));
        assert!(r.disconnect(1).is_empty());

        // Unsharing from the app does the same.
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        let out = r.end_session(S);
        assert_eq!(to(&out, 1)[0]["t"], "end");
        assert_eq!(r.session_count(), 0);
    }

    #[test]
    fn a_returning_client_replaces_its_old_connection() {
        let mut r = registered();
        join(&mut r, 1, &host_hello("h"));
        join(&mut r, 2, &guest_hello("g"));
        let out = join(&mut r, 3, &guest_hello("g"));
        assert!(closed(&out, 2));
        assert!(to(&out, 2).is_empty());
        assert_eq!(
            to(&out, 1),
            vec![
                json!({ "t": "leave", "client": "g" }),
                json!({ "t": "join", "client": "g", "name": "Guest", "color": "#0f0", "host": false }),
            ]
        );
        // The welcome doesn't list the client's own old connection.
        assert_eq!(to(&out, 3)[0]["peers"].as_array().unwrap().len(), 1);
        // The old connection closing later says nothing.
        assert!(r.disconnect(2).is_empty());
        let out = r.handle(1, r#"{"t":"op","id":"a","base":0,"steps":[]}"#);
        assert_eq!(to(&out, 3).len(), 1);
        assert!(to(&out, 2).is_empty());
    }

    #[test]
    fn open_relays_make_sessions_on_demand() {
        let mut r = Relay::new(Policy::Open);
        r.connect(1, "new");
        let out = r.handle(1, &guest_hello("g"));
        assert_eq!(error_code(&out, 1).as_deref(), Some("not-ready"));
        assert!(!r.has_session("new"));

        r.connect(2, "new");
        let out = r.handle(2, &host_hello("h"));
        assert_eq!(to(&out, 2)[0]["t"], "welcome");
        assert!(r.has_session("new"));

        r.connect(3, "new");
        let out = r.handle(3, &guest_hello("g").replace("\"tok\"", "\"x\""));
        assert_eq!(error_code(&out, 3).as_deref(), Some("bad-token"));
        r.connect(4, "new");
        let out = r.handle(4, &guest_hello("g"));
        assert_eq!(to(&out, 4)[0]["t"], "welcome");
        assert_eq!(to(&out, 4)[0]["session"]["name"], "Report.docx");
    }
}
