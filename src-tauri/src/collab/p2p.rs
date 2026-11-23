//! Sharing over the internet.
//!
//! While something is shared over the internet, the app runs one `iroh` endpoint: QUIC, direct
//! when hole punching works and through n0's public relays when it doesn't, encrypted end to end
//! either way. The code directory (`directory.rs`) maps each session's six-digit code to the
//! endpoint's address and the session id. A guest dials that address, sends the code and a name,
//! waits for the host to let them in if the host asked for that, and gets the session's token.
//! From then on the stream carries the relay's messages (`relay.rs`), one per length-prefixed
//! frame, into the host's relay as if they came over a WebSocket.
//!
//! The directory knows every code, so it's trusted to give out the right address, and iroh checks
//! that the endpoint dialed is the one at that address. Sending the code over that connection tells
//! nobody anything the directory doesn't already know. A code works for ten minutes, and five
//! wrong ones for a session end it early; the host then makes a new one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use iroh::endpoint::{presets, Connection, Incoming, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

use super::directory;
use super::relay::MAX_FRAME;
use super::server::RelayBridge;
use super::socket::Cmd;

/// Names Writan's stream to iroh.
const ALPN: &[u8] = b"writan/collab/1";

/// What `collab_find` returns for a session found in the directory, for `collab_connect_p2p`.
pub const SCHEME: &str = "writan-p2p://";

/// A guest's first frame, before they're let in, is at most this long.
const MAX_JOIN_FRAME: usize = 4096;
/// Dialing, opening the stream and sending the code take at most this long.
const HANDSHAKE: Duration = Duration::from_secs(15);
/// How long a guest waits for the host to let them in.
const APPROVAL: Duration = Duration::from_secs(60);
/// After being let in, a guest says hello to the relay within this long.
const HELLO: Duration = Duration::from_secs(10);
/// Sharing waits this long for the endpoint to reach n0's relays, so its address has one.
const ONLINE: Duration = Duration::from_secs(5);
/// What's still being sent when a stream ends gets this long to arrive.
const DRAIN: Duration = Duration::from_secs(2);
/// This many wrong codes for a session end its code.
const MAX_WRONG: u32 = 5;
/// Guests that haven't been let in yet, at once, across sessions.
const MAX_HANDSHAKES: usize = 16;
/// Guests waiting for the host to let them in, per session.
const MAX_WAITING: usize = 5;

/// What the directory keeps for a code, as `endpointAddr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub addr: EndpointAddr,
    pub session: String,
}

/// A session found in the directory, and the code it was found with.
#[derive(Debug, Clone)]
pub struct Invite {
    pub entry: DirectoryEntry,
    pub code: String,
}

/// `writan-p2p://<base64url(entry)>?code=<code>`.
pub fn encode_invite(entry: &DirectoryEntry, code: &str) -> Result<String, String> {
    let raw = serde_json::to_vec(entry).map_err(|e| e.to_string())?;
    Ok(format!(
        "{SCHEME}{}?code={code}",
        URL_SAFE_NO_PAD.encode(raw)
    ))
}

pub fn parse_invite(uri: &str) -> Result<Invite, String> {
    let bad = || "That isn't an internet invite.".to_owned();
    let rest = uri.trim().strip_prefix(SCHEME).ok_or_else(bad)?;
    let (payload, code) = rest.split_once("?code=").ok_or_else(bad)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| bad())?;
    let entry = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    Ok(Invite {
        entry,
        code: code.to_owned(),
    })
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<(), String> {
    if payload.len() > MAX_FRAME {
        return Err("Frame too large.".into());
    }
    send.write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .map_err(|e| e.to_string())?;
    send.write_all(payload).await.map_err(|e| e.to_string())
}

/// The next frame, or `None` when the other side finished the stream.
async fn read_frame(recv: &mut RecvStream, max: usize) -> Result<Option<String>, String> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(iroh::endpoint::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > max {
        return Err("Frame too large.".into());
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| "Frame isn't UTF-8.".into())
}

/// Finishes `send` and gives what's in it a moment to arrive.
async fn finish(send: &mut SendStream) {
    let _ = send.finish();
    let _ = tokio::time::timeout(DRAIN, send.stopped()).await;
}

/// A guest's first frame.
#[derive(Serialize, Deserialize)]
struct Join {
    session: String,
    code: String,
    name: String,
}

/// The host's answers to it: `waiting` while the host decides, then `ok` with the token, or
/// `error` (`no-session`, `expired`, `bad-code`, `busy`, `denied`, `bad-message`) and why.
#[derive(Serialize, Deserialize, Default)]
struct Answer {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    waiting: bool,
    #[serde(default)]
    token: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: String,
}

async fn answer(send: &mut SendStream, a: &Answer) -> Result<(), String> {
    write_frame(
        send,
        serde_json::to_string(a).unwrap_or_default().as_bytes(),
    )
    .await
}

/// What the host's page hears about.
pub trait HostEvents: Send + Sync + 'static {
    /// Someone with the code asks to join `session` (`waiting`), or stopped waiting: let in,
    /// turned away, gave up, or ran out of time.
    fn join_request(&self, request: &str, session: &str, name: &str, waiting: bool);
    /// Too many wrong codes ended `session`'s code.
    fn code_locked(&self, session: &str);
}

/// The directory's room for a session, which only its secret changes.
struct Room {
    url: String,
    secret: String,
}

struct Entry {
    token: String,
    code: String,
    expires: Instant,
    wrong: u32,
    approval: bool,
    waiting: usize,
    room: Option<Room>,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Entry>,
    /// Guests waiting to be let in, by request id: the session and the host's answer.
    approvals: HashMap<String, (String, oneshot::Sender<bool>)>,
    next: u64,
    closed: bool,
}

type Shared = Arc<Mutex<Inner>>;

/// The app's endpoint, while something is shared over the internet.
pub struct P2pHost {
    endpoint: Endpoint,
    inner: Shared,
    accept: tokio::task::JoinHandle<()>,
    /// One code change at a time.
    rotating: tokio::sync::Mutex<()>,
}

impl P2pHost {
    pub async fn start(bridge: RelayBridge, events: Arc<dyn HostEvents>) -> Result<Self, String> {
        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| format!("Couldn't start sharing over the internet: {e}"))?;
        let _ = tokio::time::timeout(ONLINE, endpoint.online()).await;
        Ok(Self::with(endpoint, bridge, events))
    }

    fn with(endpoint: Endpoint, bridge: RelayBridge, events: Arc<dyn HostEvents>) -> Self {
        let inner: Shared = Arc::default();
        let (ep, shared) = (endpoint.clone(), inner.clone());
        let accept = tokio::spawn(async move {
            let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
            while let Some(incoming) = ep.accept().await {
                let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                    incoming.refuse();
                    continue;
                };
                let (inner, bridge, events) = (shared.clone(), bridge.clone(), events.clone());
                tokio::spawn(peer(incoming, permit, inner, bridge, events));
            }
        });
        Self {
            endpoint,
            inner,
            accept,
            rotating: tokio::sync::Mutex::new(()),
        }
    }

    fn entry(&self, session: &str) -> Result<serde_json::Value, String> {
        serde_json::to_value(DirectoryEntry {
            addr: self.endpoint.addr(),
            session: session.to_owned(),
        })
        .map_err(|e| e.to_string())
    }

    /// Puts `session` in the directory under a new code. Returns the code and when it expires
    /// (Unix ms).
    pub async fn register(
        &self,
        session: &str,
        token: &str,
        approval: bool,
        url: String,
    ) -> Result<(String, u64), String> {
        let entry = self.entry(session)?;
        let base = url.clone();
        let room = tokio::task::spawn_blocking(move || directory::create_room(&base, &entry))
            .await
            .map_err(|e| e.to_string())??;
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            drop(inner);
            delete_room(
                Room {
                    url,
                    secret: room.host_secret,
                },
                room.code,
            );
            return Err("Sharing stopped.".into());
        }
        inner.sessions.insert(
            session.to_owned(),
            Entry {
                token: token.to_owned(),
                code: room.code.clone(),
                expires: Instant::now() + Duration::from_secs(room.ttl_secs),
                wrong: 0,
                approval,
                waiting: 0,
                room: Some(Room {
                    url,
                    secret: room.host_secret,
                }),
            },
        );
        Ok((room.code, room.expires_at))
    }

    /// Gives `session` a new code for another ten minutes. The old one stops working.
    pub async fn rotate(&self, session: &str) -> Result<(String, u64), String> {
        let _one = self.rotating.lock().await;
        let (old, url, secret) = {
            let inner = self.inner.lock().unwrap();
            let s = inner
                .sessions
                .get(session)
                .ok_or("This document isn't shared over the internet.")?;
            let room = s
                .room
                .as_ref()
                .ok_or("This document isn't in the directory.")?;
            (s.code.clone(), room.url.clone(), room.secret.clone())
        };
        let entry = self.entry(session)?;
        let base = url.clone();
        let (code, expires_at, ttl, new_secret) = tokio::task::spawn_blocking(move || {
            match directory::rotate_room(&base, &old, &secret, &entry)? {
                Some(r) => Ok::<_, String>((r.code, r.expires_at, r.ttl_secs, None)),
                // The directory forgot the room (it keeps one a day after it expires).
                None => {
                    let r = directory::create_room(&base, &entry)?;
                    Ok((r.code, r.expires_at, r.ttl_secs, Some(r.host_secret)))
                }
            }
        })
        .await
        .map_err(|e| e.to_string())??;
        let mut inner = self.inner.lock().unwrap();
        let Some(s) = inner.sessions.get_mut(session) else {
            drop(inner);
            if let Some(secret) = new_secret {
                delete_room(Room { url, secret }, code);
            }
            return Err("This document isn't shared any more.".into());
        };
        s.code = code.clone();
        s.expires = Instant::now() + Duration::from_secs(ttl);
        s.wrong = 0;
        if let (Some(secret), Some(room)) = (new_secret, s.room.as_mut()) {
            room.secret = secret;
        }
        Ok((code, expires_at))
    }

    pub fn set_approval(&self, session: &str, on: bool) {
        if let Some(s) = self.inner.lock().unwrap().sessions.get_mut(session) {
            s.approval = on;
        }
    }

    /// The host's answer to a request to join. False when it's no longer waiting.
    pub fn answer(&self, request: &str, allow: bool) -> bool {
        let waiting = self.inner.lock().unwrap().approvals.remove(request);
        waiting.is_some_and(|(_, tx)| tx.send(allow).is_ok())
    }

    /// Stops letting anyone into `session` and takes its code out of the directory. Whoever is
    /// still waiting is turned away.
    pub fn unregister(&self, session: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.approvals.retain(|_, (s, _)| s != session);
        if let Some(Entry {
            room: Some(room),
            code,
            ..
        }) = inner.sessions.remove(session)
        {
            delete_room(room, code);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().sessions.is_empty()
    }

    /// Unregisters everything and closes the endpoint, after what's being sent (the relay's
    /// `end`) has had a moment to arrive.
    pub fn shutdown(&self) {
        let sessions: Vec<Entry> = {
            let mut inner = self.inner.lock().unwrap();
            inner.closed = true;
            inner.approvals.clear();
            inner.sessions.drain().map(|(_, e)| e).collect()
        };
        for e in sessions {
            if let Some(room) = e.room {
                delete_room(room, e.code);
            }
        }
        self.accept.abort();
        let endpoint = self.endpoint.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DRAIN).await;
            endpoint.close().await;
        });
    }
}

fn delete_room(room: Room, code: String) {
    std::thread::spawn(move || directory::delete_room(&room.url, &code, &room.secret));
}

/// Checks a guest's code. On success, the session's token and whether the host lets them in.
/// Second in the error is whether this wrong code was the one that ended the session's code.
fn check(
    inner: &mut Inner,
    join: &Join,
) -> Result<(String, bool), (&'static str, &'static str, bool)> {
    let Some(s) = inner.sessions.get_mut(&join.session) else {
        return Err(("no-session", "This document isn't shared any more.", false));
    };
    if s.wrong >= MAX_WRONG || Instant::now() >= s.expires {
        return Err((
            "expired",
            "That code has expired. Ask for a new one.",
            false,
        ));
    }
    if !constant_time_eq(s.code.as_bytes(), join.code.as_bytes()) {
        s.wrong += 1;
        return Err(("bad-code", "That code didn't match.", s.wrong == MAX_WRONG));
    }
    if s.approval {
        if s.waiting >= MAX_WAITING {
            return Err((
                "busy",
                "Too many people are waiting to join. Try again in a minute.",
                false,
            ));
        }
        s.waiting += 1;
    }
    Ok((s.token.clone(), s.approval))
}

/// One guest: the code, the host's approval, then the relay.
async fn peer(
    incoming: Incoming,
    permit: OwnedSemaphorePermit,
    inner: Shared,
    bridge: RelayBridge,
    events: Arc<dyn HostEvents>,
) {
    let opened = tokio::time::timeout(HANDSHAKE, async {
        let conn = incoming.await.ok()?;
        let (send, mut recv) = conn.accept_bi().await.ok()?;
        let text = read_frame(&mut recv, MAX_JOIN_FRAME).await.ok()??;
        Some((conn, send, recv, text))
    })
    .await;
    let Ok(Some((conn, mut send, recv, text))) = opened else {
        return;
    };
    let admitted = admit(&conn, &mut send, &text, &inner, &*events).await;
    drop(permit);
    match admitted {
        Some(session) => relay(&conn, send, recv, &session, &bridge).await,
        None => finish(&mut send).await,
    }
    conn.close(0u32.into(), b"");
}

/// Answers a guest's `Join`. The session they're let into, if they are.
async fn admit(
    conn: &Connection,
    send: &mut SendStream,
    text: &str,
    inner: &Shared,
    events: &dyn HostEvents,
) -> Option<String> {
    let refuse = |error: &str, message: &str| Answer {
        error: error.into(),
        message: message.into(),
        ..Answer::default()
    };
    let Ok(join) = serde_json::from_str::<Join>(text) else {
        let _ = answer(
            send,
            &refuse("bad-message", "That isn't a request to join."),
        )
        .await;
        return None;
    };
    let checked = check(&mut inner.lock().unwrap(), &join);
    let (token, approval) = match checked {
        Ok(v) => v,
        Err((error, message, locked)) => {
            if locked {
                events.code_locked(&join.session);
            }
            let _ = answer(send, &refuse(error, message)).await;
            return None;
        }
    };
    let token = if approval {
        let name: String = join.name.trim().chars().take(64).collect();
        let name = if name.is_empty() {
            "Someone".to_owned()
        } else {
            name
        };
        let (tx, rx) = oneshot::channel();
        let request = {
            let mut inner = inner.lock().unwrap();
            inner.next += 1;
            let id = format!("join-{}", inner.next);
            inner
                .approvals
                .insert(id.clone(), (join.session.clone(), tx));
            id
        };
        let waiting = Answer {
            waiting: true,
            ..Answer::default()
        };
        let allowed = answer(send, &waiting).await.is_ok() && {
            events.join_request(&request, &join.session, &name, true);
            tokio::select! {
                r = rx => r.unwrap_or(false),
                () = tokio::time::sleep(APPROVAL) => false,
                _ = conn.closed() => false,
            }
        };
        let token = {
            let mut inner = inner.lock().unwrap();
            inner.approvals.remove(&request);
            inner.sessions.get_mut(&join.session).map(|s| {
                s.waiting = s.waiting.saturating_sub(1);
                s.token.clone()
            })
        };
        events.join_request(&request, &join.session, &name, false);
        match token {
            Some(t) if allowed => t,
            Some(_) => {
                let _ = answer(
                    send,
                    &refuse("denied", "The person sharing didn't let you in."),
                )
                .await;
                return None;
            }
            None => {
                let _ = answer(
                    send,
                    &refuse("no-session", "This document isn't shared any more."),
                )
                .await;
                return None;
            }
        }
    } else {
        token
    };
    let ok = Answer {
        ok: true,
        token,
        ..Answer::default()
    };
    answer(send, &ok).await.ok()?;
    Some(join.session)
}

/// Carries a guest's relay messages until either side stops.
async fn relay(
    conn: &Connection,
    mut send: SendStream,
    mut recv: RecvStream,
    session: &str,
    bridge: &RelayBridge,
) {
    let (id, mut out) = bridge.connect_async(session);
    let writer = async {
        while let Some(Cmd::Text(text)) = out.recv().await {
            if write_frame(&mut send, text.as_bytes()).await.is_err() {
                break;
            }
        }
        finish(&mut send).await;
    };
    let reader = async {
        // The hello comes first, and soon, as over a WebSocket.
        let Ok(Ok(Some(hello))) =
            tokio::time::timeout(HELLO, read_frame(&mut recv, MAX_FRAME)).await
        else {
            return;
        };
        bridge.handle(id, &hello);
        while let Ok(Some(text)) = read_frame(&mut recv, MAX_FRAME).await {
            bridge.handle(id, &text);
        }
    };
    tokio::select! {
        () = writer => {}
        () = reader => {}
        _ = conn.closed() => {}
    }
    bridge.disconnect(id);
}

/// A guest's connection to a host.
pub struct P2pClient {
    tx: mpsc::UnboundedSender<Cmd>,
}

impl P2pClient {
    pub fn send(&self, text: String) -> bool {
        self.tx.send(Cmd::Text(text)).is_ok()
    }

    pub fn close(&self) {
        let _ = self.tx.send(Cmd::Close);
    }
}

/// Joins a session found in the directory. Returns the connection and the session's token, for
/// the hello. `on_text` gets each relay message, and `on_closed` why the connection ended.
pub async fn connect(
    invite: Invite,
    name: String,
    on_text: impl FnMut(&str) + Send + 'static,
    on_closed: impl FnOnce(String) + Send + 'static,
) -> Result<(P2pClient, String), String> {
    let endpoint = Endpoint::bind(presets::N0)
        .await
        .map_err(|e| format!("Couldn't connect over the internet: {e}"))?;
    connect_with(endpoint, invite, name, on_text, on_closed).await
}

async fn connect_with(
    endpoint: Endpoint,
    invite: Invite,
    name: String,
    mut on_text: impl FnMut(&str) + Send + 'static,
    on_closed: impl FnOnce(String) + Send + 'static,
) -> Result<(P2pClient, String), String> {
    let joined = join(&endpoint, invite, name).await;
    let (conn, mut send, mut recv, token) = match joined {
        Ok(v) => v,
        Err(e) => {
            endpoint.close().await;
            return Err(e);
        }
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<Cmd>();
    tokio::spawn(async move {
        let writer = async {
            while let Some(Cmd::Text(text)) = rx.recv().await {
                if write_frame(&mut send, text.as_bytes()).await.is_err() {
                    break;
                }
            }
            finish(&mut send).await;
            "closed".to_owned()
        };
        let reader = async {
            loop {
                match read_frame(&mut recv, MAX_FRAME).await {
                    Ok(Some(text)) => on_text(&text),
                    Ok(None) => break "closed".to_owned(),
                    Err(e) => break e,
                }
            }
        };
        let reason = tokio::select! {
            r = writer => r,
            r = reader => r,
        };
        conn.close(0u32.into(), b"");
        endpoint.close().await;
        on_closed(reason);
    });
    Ok((P2pClient { tx }, token))
}

async fn join(
    endpoint: &Endpoint,
    invite: Invite,
    name: String,
) -> Result<(Connection, SendStream, RecvStream, String), String> {
    let unreachable = |e: String| format!("Couldn't reach the person sharing: {e}");
    let (conn, send, mut recv) = tokio::time::timeout(HANDSHAKE, async {
        let conn = endpoint
            .connect(invite.entry.addr, ALPN)
            .await
            .map_err(|e| unreachable(e.to_string()))?;
        let (mut send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| unreachable(e.to_string()))?;
        let join = Join {
            session: invite.entry.session,
            code: invite.code,
            name,
        };
        write_frame(
            &mut send,
            serde_json::to_string(&join).unwrap_or_default().as_bytes(),
        )
        .await?;
        Ok::<_, String>((conn, send, recv))
    })
    .await
    .map_err(|_| unreachable("it took too long".into()))??;
    let mut wait = HANDSHAKE;
    loop {
        let text = tokio::time::timeout(wait, read_frame(&mut recv, MAX_JOIN_FRAME))
            .await
            .map_err(|_| "The person sharing didn't answer.".to_owned())??
            .ok_or("The person sharing closed the connection.")?;
        let a: Answer = serde_json::from_str(&text)
            .map_err(|_| "The person sharing sent something unexpected.".to_owned())?;
        if a.ok {
            return Ok((conn, send, recv, a.token));
        }
        if !a.waiting {
            conn.close(0u32.into(), b"");
            return Err(if a.message.is_empty() {
                "Couldn't join.".into()
            } else {
                a.message
            });
        }
        wait = APPROVAL + HANDSHAKE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::relay::Policy;
    use crate::collab::server::Server;

    #[derive(Default)]
    struct Events {
        requests: Mutex<Vec<(String, String, bool)>>,
        locked: Mutex<Vec<String>>,
    }

    impl HostEvents for Events {
        fn join_request(&self, request: &str, _session: &str, name: &str, waiting: bool) {
            self.requests
                .lock()
                .unwrap()
                .push((request.into(), name.into(), waiting));
        }
        fn code_locked(&self, session: &str) {
            self.locked.lock().unwrap().push(session.into());
        }
    }

    async fn host(server: &Server, events: Arc<Events>) -> P2pHost {
        let endpoint = Endpoint::builder(presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        P2pHost::with(endpoint, server.bridge(), events)
    }

    fn add(host: &P2pHost, session: &str, token: &str, code: &str, approval: bool) {
        host.inner.lock().unwrap().sessions.insert(
            session.into(),
            Entry {
                token: token.into(),
                code: code.into(),
                expires: Instant::now() + Duration::from_secs(60),
                wrong: 0,
                approval,
                waiting: 0,
                room: None,
            },
        );
    }

    fn invite(host: &P2pHost, session: &str, code: &str) -> Invite {
        Invite {
            entry: DirectoryEntry {
                addr: host.endpoint.addr(),
                session: session.into(),
            },
            code: code.into(),
        }
    }

    async fn guest(
        invite: Invite,
        name: &str,
    ) -> Result<(P2pClient, String, mpsc::UnboundedReceiver<String>), String> {
        let endpoint = Endpoint::bind(presets::Minimal).await.unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let tx2 = tx.clone();
        let (client, token) = connect_with(
            endpoint,
            invite,
            name.into(),
            move |t| drop(tx.send(t.to_owned())),
            move |_| drop(tx2.send("<closed>".into())),
        )
        .await?;
        Ok((client, token, rx))
    }

    async fn next(rx: &mut mpsc::UnboundedReceiver<String>) -> String {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap()
    }

    const HOST_HELLO: &str = r##"{"t":"hello","v":1,"token":"tok","client":"h","name":"Host","color":"#f00","hostKey":"hk","snapshot":{"name":"Doc.docx","kind":"docx","bytes":"UEsDBA=="}}"##;
    const GUEST_HELLO: &str =
        r##"{"t":"hello","v":1,"token":"tok","client":"g","name":"Alex","color":"#0f0"}"##;

    #[test]
    fn joins_with_the_code_and_hears_the_end() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let (h, _rx) = server.bridge().connect_async("doc");
            server.bridge().handle(h, HOST_HELLO);
            let host = host(&server, Arc::default()).await;
            add(&host, "doc", "tok", "482913", false);

            let wrong = guest(invite(&host, "doc", "999999"), "Mallory").await;
            assert_eq!(wrong.err().as_deref(), Some("That code didn't match."));

            let (client, token, mut rx) =
                guest(invite(&host, "doc", "482913"), "Alex").await.unwrap();
            assert_eq!(token, "tok");
            assert!(client.send(GUEST_HELLO.into()));
            assert!(next(&mut rx).await.contains(r#""t":"welcome""#));

            // Stopping sharing reaches the guest as the relay's `end`, before the endpoint closes.
            server.end_session("doc");
            host.unregister("doc");
            host.shutdown();
            assert!(next(&mut rx).await.contains(r#""t":"end""#));
            assert_eq!(next(&mut rx).await, "<closed>");
        });
    }

    #[test]
    fn wrong_codes_end_the_code() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let events = Arc::new(Events::default());
            let host = host(&server, events.clone()).await;
            add(&host, "doc", "tok", "123456", false);
            for _ in 0..MAX_WRONG {
                assert!(guest(invite(&host, "doc", "000000"), "M").await.is_err());
            }
            assert_eq!(*events.locked.lock().unwrap(), vec!["doc".to_owned()]);
            let right = guest(invite(&host, "doc", "123456"), "A").await;
            assert_eq!(
                right.err().as_deref(),
                Some("That code has expired. Ask for a new one.")
            );
        });
    }

    #[test]
    fn the_host_lets_guests_in_or_not() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let events = Arc::new(Events::default());
            let host = Arc::new(host(&server, events.clone()).await);
            add(&host, "doc", "tok", "112233", true);

            for (name, allow) in [("Bob", false), ("Carol", true)] {
                let (host2, events2) = (host.clone(), events.clone());
                let decide = tokio::spawn(async move {
                    loop {
                        let asked = events2
                            .requests
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|r| r.1 == name && r.2)
                            .cloned();
                        if let Some((id, ..)) = asked {
                            assert!(host2.answer(&id, allow));
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                });
                let joined = guest(invite(&host, "doc", "112233"), name).await;
                decide.await.unwrap();
                match joined {
                    Ok((_, token, _)) => assert!(allow && token == "tok"),
                    Err(e) => assert!(
                        !allow && e == "The person sharing didn't let you in.",
                        "{e}"
                    ),
                }
            }
            // Each request was settled for the page too.
            let settled = events
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| !r.2)
                .count();
            assert_eq!(settled, 2);
            assert_eq!(host.inner.lock().unwrap().sessions["doc"].waiting, 0);
        });
    }

    /// Through the real directory and n0's relays: `cargo test over_the_internet -- --ignored`.
    #[test]
    #[ignore = "needs the internet"]
    fn over_the_internet() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let (h, _rx) = server.bridge().connect_async("doc");
            server.bridge().handle(h, HOST_HELLO);
            let host = P2pHost::start(server.bridge(), Arc::new(Events::default()))
                .await
                .unwrap();
            let url = directory::directory_url();
            let (code, _) = host
                .register("doc", "tok", false, url.clone())
                .await
                .unwrap();

            let found = directory::lookup_room(&url, &code).unwrap().unwrap();
            let mut entry: DirectoryEntry = serde_json::from_value(found.endpoint_addr).unwrap();
            // Only the relay, as for a guest on another network.
            entry
                .addr
                .addrs
                .retain(|a| matches!(a, iroh::TransportAddr::Relay(_)));
            assert!(!entry.addr.addrs.is_empty(), "the address has no relay");
            let (tx, mut rx) = mpsc::unbounded_channel();
            let endpoint = Endpoint::bind(presets::N0).await.unwrap();
            let invite = Invite {
                entry,
                code: code.clone(),
            };
            let (client, token) = connect_with(
                endpoint,
                invite,
                "Alex".into(),
                move |t| drop(tx.send(t.to_owned())),
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(token, "tok");
            assert!(client.send(GUEST_HELLO.into()));
            assert!(next(&mut rx).await.contains(r#""t":"welcome""#));

            let (new, _) = host.rotate("doc").await.unwrap();
            assert_ne!(new, code);
            assert!(directory::lookup_room(&url, &code).unwrap().is_none());
            assert!(directory::lookup_room(&url, &new).unwrap().is_some());
            host.unregister("doc");
            host.shutdown();
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert!(directory::lookup_room(&url, &new).unwrap().is_none());
        });
    }
}
