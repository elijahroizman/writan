//! Sharing over the internet.
//!
//! While something is shared over the internet, the app runs one `iroh` endpoint: QUIC, direct
//! when hole punching works and through n0's public relays when it doesn't, encrypted end to end
//! either way. The code directory (`directory.rs`) maps each session's six-digit code to the
//! endpoint's address, the session id and a key that's new with each code. A guest dials that
//! address, sends the key and a name, waits for the host to let them in if the host asked for
//! that, and gets the session's token and a seat: a secret of their own that their AI agents join
//! with later, after the code has stopped working too. An agent asks to be let in like a person,
//! and the host sees it as theirs ("Yoav's Claude").
//! From then on the stream carries the relay's messages (`relay.rs`), one per length-prefixed
//! frame, into the host's relay as if they came over a WebSocket.
//!
//! The directory knows every code, so it's trusted to give out the right address, and iroh checks
//! that the endpoint dialed is the one at that address. The key is 128 random bits, so only someone
//! who looked the current code up has it: guessing is the directory's to limit, and an invite from
//! an earlier code stops working when the host makes a new one. A code works for ten minutes, for
//! joining: who's in stays in.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use iroh::endpoint::{presets, Connection, Incoming, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

use super::client::MAX_INCOMING;
use super::directory;
use super::relay::MAX_FRAME;
use super::server::RelayBridge;
use super::socket::{Cmd, WRITE_TIMEOUT};

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
/// Guests that haven't said hello to the relay yet, at once, across sessions.
const MAX_HANDSHAKES: usize = 16;
/// Guests waiting for the host to let them in, per session.
const MAX_WAITING: usize = 5;

/// What the directory keeps for a code, as `endpointAddr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub addr: EndpointAddr,
    pub session: String,
    /// New with each code, so an invite from an earlier one doesn't work.
    pub key: String,
}

/// `writan-p2p://<base64url(entry)>`.
pub fn encode_invite(entry: &DirectoryEntry) -> Result<String, String> {
    let raw = serde_json::to_vec(entry).map_err(|e| e.to_string())?;
    Ok(format!("{SCHEME}{}", URL_SAFE_NO_PAD.encode(raw)))
}

pub fn parse_invite(uri: &str) -> Result<DirectoryEntry, String> {
    let bad = || "That isn't an internet invite.".to_owned();
    let payload = uri.trim().strip_prefix(SCHEME).ok_or_else(bad)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| bad())?;
    serde_json::from_slice(&bytes).map_err(|_| bad())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Sends one frame. The other side limits what it takes. Like a WebSocket's write timeout, a
/// side that takes nothing for `WRITE_TIMEOUT` is given up on, so what's waiting for it can't pile
/// up for good.
async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<(), String> {
    let len = u32::try_from(payload.len()).map_err(|_| "Frame too large.".to_owned())?;
    for part in std::iter::once(&len.to_be_bytes()[..]).chain(payload.chunks(64 << 10)) {
        tokio::time::timeout(WRITE_TIMEOUT, send.write_all(part))
            .await
            .map_err(|_| "The other side stopped reading.".to_owned())?
            .map_err(|e| e.to_string())?;
    }
    Ok(())
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
    // Grown as it arrives, so a length alone doesn't make this side set aside a gigabyte.
    let mut buf = Vec::with_capacity(len.min(1 << 20));
    let mut chunk = vec![0u8; len.min(64 << 10)];
    while buf.len() < len {
        let want = (len - buf.len()).min(chunk.len());
        match recv
            .read(&mut chunk[..want])
            .await
            .map_err(|e| e.to_string())?
        {
            Some(n) => buf.extend_from_slice(&chunk[..n]),
            None => return Err("The connection ended mid-message.".into()),
        }
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| "Frame isn't UTF-8.".into())
}

/// Finishes `send` and gives what's in it a moment to arrive.
async fn finish(send: &mut SendStream) {
    let _ = send.finish();
    let _ = tokio::time::timeout(DRAIN, send.stopped()).await;
}

/// A guest's first frame. An agent joining through a guest's app sends the guest's `seat`
/// instead of the code's key, and which agent it is.
#[derive(Serialize, Deserialize)]
struct Join {
    session: String,
    #[serde(default)]
    key: String,
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    seat: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
}

/// The host's answers to it: `waiting` while the host decides, then `ok` with the token (and for a
/// person, their seat), or `error` (`no-session`, `expired`, `no-seat`, `busy`, `denied`,
/// `bad-message`) and why.
#[derive(Serialize, Deserialize, Default)]
struct Answer {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    waiting: bool,
    #[serde(default)]
    token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    seat: String,
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
    /// Someone with the code, or a guest's agent (`agent`), asks to join `session` (`waiting`), or
    /// stopped waiting: let in, turned away, gave up, or ran out of time.
    fn join_request(
        &self,
        request: &str,
        session: &str,
        name: &str,
        agent: Option<&str>,
        waiting: bool,
    );
}

/// The directory's room for a session, which only its secret changes.
struct Room {
    url: String,
    secret: String,
}

struct Entry {
    token: String,
    code: String,
    key: String,
    expires: Instant,
    approval: bool,
    waiting: usize,
    room: Option<Room>,
    /// The guests let in so far, by their seat: their names, for their agents'.
    seats: HashMap<String, String>,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Entry>,
    /// Sessions being put in the directory, which keep the endpoint open too.
    registering: usize,
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
        // Without a relay, guests behind another router couldn't reach this one.
        if tokio::time::timeout(ONLINE, endpoint.online())
            .await
            .is_err()
        {
            endpoint.close().await;
            return Err("Couldn't reach the internet relays.".into());
        }
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

    /// A new key, and the directory's entry with it.
    fn entry(&self, session: &str) -> Result<(String, serde_json::Value), String> {
        let key = super::random_text(16)?;
        let entry = serde_json::to_value(DirectoryEntry {
            addr: self.endpoint.addr(),
            session: session.to_owned(),
            key: key.clone(),
        })
        .map_err(|e| e.to_string())?;
        Ok((key, entry))
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
        let (key, entry) = self.entry(session)?;
        let base = url.clone();
        self.inner.lock().unwrap().registering += 1;
        let created = tokio::task::spawn_blocking(move || directory::create_room(&base, &entry))
            .await
            .map_err(|e| e.to_string());
        let mut inner = self.inner.lock().unwrap();
        inner.registering -= 1;
        let room = created??;
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
                key,
                expires: Instant::now() + Duration::from_secs(room.ttl_secs),
                approval,
                waiting: 0,
                room: Some(Room {
                    url,
                    secret: room.host_secret,
                }),
                seats: HashMap::new(),
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
        let (key, entry) = self.entry(session)?;
        let (base, old_secret) = (url.clone(), secret.clone());
        let (code, expires_at, ttl, new_secret) = tokio::task::spawn_blocking(move || {
            match directory::rotate_room(&base, &old, &old_secret, &entry)? {
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
            // Stopped sharing meanwhile, which removed the old code, not this one.
            drop(inner);
            let secret = new_secret.unwrap_or(secret);
            delete_room(Room { url, secret }, code);
            return Err("This document isn't shared any more.".into());
        };
        s.code = code.clone();
        s.key = key;
        s.expires = Instant::now() + Duration::from_secs(ttl);
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

    /// Whether nothing is shared or being shared, so the endpoint can close.
    pub fn is_empty(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.sessions.is_empty() && inner.registering == 0
    }

    /// Unregisters everything and closes the endpoint, after what's being sent (the relay's
    /// `end`) has had a moment to arrive.
    pub fn shutdown(&self) {
        self.close();
        let endpoint = self.endpoint.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DRAIN).await;
            endpoint.close().await;
        });
    }

    /// `shutdown`, waiting until it's done, for quitting.
    pub async fn shutdown_and_wait(&self) {
        let deletes = self.close();
        tokio::time::sleep(DRAIN).await;
        let _ = tokio::time::timeout(DRAIN, self.endpoint.close()).await;
        let joined =
            tokio::task::spawn_blocking(move || deletes.into_iter().for_each(|d| drop(d.join())));
        let _ = tokio::time::timeout(DRAIN, joined).await;
    }

    /// Lets nobody else in and takes every code out of the directory.
    fn close(&self) -> Vec<std::thread::JoinHandle<()>> {
        let sessions: Vec<Entry> = {
            let mut inner = self.inner.lock().unwrap();
            inner.closed = true;
            inner.approvals.clear();
            inner.sessions.drain().map(|(_, e)| e).collect()
        };
        self.accept.abort();
        sessions
            .into_iter()
            .filter_map(|e| Some(delete_room(e.room?, e.code)))
            .collect()
    }
}

fn delete_room(room: Room, code: String) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || directory::delete_room(&room.url, &code, &room.secret))
}

/// Who a checked `Join` is.
struct Checked {
    token: String,
    /// Whether the host lets them in.
    approval: bool,
    /// For an agent joining with a guest's seat: that guest's name.
    owner: Option<String>,
}

/// Checks a guest's key, or an agent's seat. A code stops working after ten minutes; a seat lasts
/// as long as the session.
fn check(inner: &mut Inner, join: &Join) -> Result<Checked, (&'static str, &'static str)> {
    let Some(s) = inner.sessions.get_mut(&join.session) else {
        return Err(("no-session", "This document isn't shared any more."));
    };
    let owner = if join.seat.is_empty() {
        if !constant_time_eq(s.key.as_bytes(), join.key.as_bytes()) || Instant::now() >= s.expires {
            return Err(("expired", "That code has expired. Ask for a new one."));
        }
        None
    } else {
        // Every seat is compared, so how long it takes doesn't say which came close.
        let found = s.seats.iter().fold(None, |found, (seat, name)| {
            if constant_time_eq(seat.as_bytes(), join.seat.as_bytes()) {
                Some(name.clone())
            } else {
                found
            }
        });
        if found.is_none() || join.agent.is_none() {
            return Err((
                "no-seat",
                "The person sharing doesn't know this copy any more. Join with a new code.",
            ));
        }
        found
    };
    if s.approval {
        if s.waiting >= MAX_WAITING {
            return Err((
                "busy",
                "Too many people are waiting to join. Try again in a minute.",
            ));
        }
        s.waiting += 1;
    }
    Ok(Checked {
        token: s.token.clone(),
        approval: s.approval,
        owner,
    })
}

/// A name as people see it: trimmed, at most 64 characters, `Someone` when empty.
fn clean_name(name: &str) -> String {
    let name: String = name.trim().chars().take(64).collect();
    if name.is_empty() {
        "Someone".to_owned()
    } else {
        name
    }
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
    match admit(&conn, &mut send, &text, &inner, &*events).await {
        Some(session) => relay(&conn, send, recv, &session, &bridge, permit).await,
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
    let Checked {
        token,
        approval,
        owner,
    } = match checked {
        Ok(v) => v,
        Err((error, message)) => {
            let _ = answer(send, &refuse(error, message)).await;
            return None;
        }
    };
    let name = clean_name(&join.name);
    let token = if approval {
        // A guest's agent shows as theirs.
        let shown = match &owner {
            Some(owner) => format!("{owner}'s {name}"),
            None => name.clone(),
        };
        let agent = owner.as_ref().and(join.agent.as_deref());
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
            events.join_request(&request, &join.session, &shown, agent, true);
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
        events.join_request(&request, &join.session, &shown, agent, false);
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
    // A person gets a seat for their agents; an agent doesn't pass one on.
    let seat = if owner.is_none() {
        let seat = super::random_text(16).ok()?;
        let mut inner = inner.lock().unwrap();
        let s = inner.sessions.get_mut(&join.session)?;
        s.seats.insert(seat.clone(), name);
        seat
    } else {
        String::new()
    };
    let ok = Answer {
        ok: true,
        token,
        seat,
        ..Answer::default()
    };
    answer(send, &ok).await.ok()?;
    Some(join.session)
}

/// Carries a guest's relay messages until either side stops. The guest counts against
/// `MAX_HANDSHAKES` (`permit`) until their hello.
async fn relay(
    conn: &Connection,
    mut send: SendStream,
    mut recv: RecvStream,
    session: &str,
    bridge: &RelayBridge,
    permit: OwnedSemaphorePermit,
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
        drop(permit);
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

/// Who joins: a person with the code's key, or an agent with its person's seat.
pub struct Guest {
    pub name: String,
    /// Empty for a person.
    pub seat: String,
    pub agent: Option<String>,
}

/// What joining gives: the connection, the session's token for the hello, and for a person the
/// seat their agents join with (empty from a host that has none).
pub type Joined = (P2pClient, String, String);

/// Joins a session found in the directory. `on_text` gets each relay message, and `on_closed` why
/// the connection ended. When `cancel` finishes first, the join is given up, and the host hears
/// that.
pub async fn connect(
    invite: DirectoryEntry,
    guest: Guest,
    on_text: impl FnMut(&str) + Send + 'static,
    on_closed: impl FnOnce(String) + Send + 'static,
    cancel: impl Future<Output = ()>,
) -> Result<Joined, String> {
    let endpoint = Endpoint::bind(presets::N0)
        .await
        .map_err(|e| format!("Couldn't connect over the internet: {e}"))?;
    connect_with(endpoint, invite, guest, on_text, on_closed, cancel).await
}

async fn connect_with(
    endpoint: Endpoint,
    invite: DirectoryEntry,
    guest: Guest,
    mut on_text: impl FnMut(&str) + Send + 'static,
    on_closed: impl FnOnce(String) + Send + 'static,
    cancel: impl Future<Output = ()>,
) -> Result<Joined, String> {
    let joined = tokio::select! {
        r = join(&endpoint, invite, guest) => r,
        () = cancel => Err("Cancelled.".to_owned()),
    };
    let (conn, mut send, mut recv, token, seat) = match joined {
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
                // A welcome holds the document and every change, so it can be big.
                match read_frame(&mut recv, MAX_INCOMING).await {
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
    Ok((P2pClient { tx }, token, seat))
}

async fn join(
    endpoint: &Endpoint,
    invite: DirectoryEntry,
    guest: Guest,
) -> Result<(Connection, SendStream, RecvStream, String, String), String> {
    let unreachable = |e: String| format!("Couldn't reach the person sharing: {e}");
    let (conn, send, mut recv) = tokio::time::timeout(HANDSHAKE, async {
        let conn = endpoint
            .connect(invite.addr, ALPN)
            .await
            .map_err(|e| unreachable(e.to_string()))?;
        let (mut send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| unreachable(e.to_string()))?;
        let join = Join {
            session: invite.session,
            key: invite.key,
            name: guest.name,
            seat: guest.seat,
            agent: guest.agent,
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
            return Ok((conn, send, recv, a.token, a.seat));
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
        agents: Mutex<Vec<Option<String>>>,
    }

    impl HostEvents for Events {
        fn join_request(
            &self,
            request: &str,
            _session: &str,
            name: &str,
            agent: Option<&str>,
            waiting: bool,
        ) {
            self.requests
                .lock()
                .unwrap()
                .push((request.into(), name.into(), waiting));
            self.agents.lock().unwrap().push(agent.map(str::to_owned));
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

    fn add(host: &P2pHost, session: &str, token: &str, key: &str, approval: bool) {
        host.inner.lock().unwrap().sessions.insert(
            session.into(),
            Entry {
                token: token.into(),
                code: "123456".into(),
                key: key.into(),
                expires: Instant::now() + Duration::from_secs(60),
                approval,
                waiting: 0,
                room: None,
                seats: HashMap::new(),
            },
        );
    }

    fn invite(host: &P2pHost, session: &str, key: &str) -> DirectoryEntry {
        DirectoryEntry {
            addr: host.endpoint.addr(),
            session: session.into(),
            key: key.into(),
        }
    }

    fn person(name: &str) -> Guest {
        Guest {
            name: name.into(),
            seat: String::new(),
            agent: None,
        }
    }

    async fn join_as(
        invite: DirectoryEntry,
        who: Guest,
    ) -> Result<(P2pClient, String, String, mpsc::UnboundedReceiver<String>), String> {
        let endpoint = Endpoint::bind(presets::Minimal).await.unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let tx2 = tx.clone();
        let (client, token, seat) = connect_with(
            endpoint,
            invite,
            who,
            move |t| drop(tx.send(t.to_owned())),
            move |_| drop(tx2.send("<closed>".into())),
            std::future::pending(),
        )
        .await?;
        Ok((client, token, seat, rx))
    }

    async fn guest(
        invite: DirectoryEntry,
        name: &str,
    ) -> Result<(P2pClient, String, mpsc::UnboundedReceiver<String>), String> {
        join_as(invite, person(name))
            .await
            .map(|(client, token, _, rx)| (client, token, rx))
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
    fn joins_with_the_key_and_hears_the_end() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let (h, _rx) = server.bridge().connect_async("doc");
            server.bridge().handle(h, HOST_HELLO);
            let host = host(&server, Arc::default()).await;
            add(&host, "doc", "tok", "key-1", false);

            let wrong = guest(invite(&host, "doc", "key-0"), "Mallory").await;
            assert_eq!(
                wrong.err().as_deref(),
                Some("That code has expired. Ask for a new one.")
            );

            let (client, token, mut rx) =
                guest(invite(&host, "doc", "key-1"), "Alex").await.unwrap();
            assert_eq!(token, "tok");
            assert!(client.send(GUEST_HELLO.into()));
            assert!(next(&mut rx).await.contains(r#""t":"welcome""#));

            // A new code's key turns away an invite from the old one.
            host.inner
                .lock()
                .unwrap()
                .sessions
                .get_mut("doc")
                .unwrap()
                .key = "key-2".into();
            assert!(guest(invite(&host, "doc", "key-1"), "Alex").await.is_err());

            // Stopping sharing reaches the guest as the relay's `end`, before the endpoint closes.
            server.end_session("doc");
            host.unregister("doc");
            host.shutdown();
            assert!(next(&mut rx).await.contains(r#""t":"end""#));
            assert_eq!(next(&mut rx).await, "<closed>");
        });
    }

    #[test]
    fn a_welcome_bigger_than_a_guest_frame_arrives() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let (h, _rx) = server.bridge().connect_async("doc");
            server.bridge().handle(h, HOST_HELLO);
            // Changes adding up to more than one frame from a guest may hold.
            let steps = "x".repeat(MAX_FRAME / 4);
            for base in 0..5 {
                let op = format!(r#"{{"t":"op","id":"h:{base}","base":{base},"steps":"{steps}"}}"#);
                server.bridge().handle(h, &op);
            }
            let host = host(&server, Arc::default()).await;
            add(&host, "doc", "tok", "key", false);
            let (client, _, mut rx) = guest(invite(&host, "doc", "key"), "Alex").await.unwrap();
            assert!(client.send(GUEST_HELLO.into()));
            let welcome = tokio::time::timeout(Duration::from_secs(30), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(welcome.len() > MAX_FRAME, "{}", welcome.len());
            assert!(welcome.starts_with(r#"{"t":"welcome""#));
        });
    }

    #[test]
    fn the_host_lets_guests_in_or_not() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let events = Arc::new(Events::default());
            let host = Arc::new(host(&server, events.clone()).await);
            add(&host, "doc", "tok", "key", true);

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
                let joined = guest(invite(&host, "doc", "key"), name).await;
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

    #[test]
    fn a_guest_giving_up_settles_the_request() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let events = Arc::new(Events::default());
            let host = host(&server, events.clone()).await;
            add(&host, "doc", "tok", "key", true);
            let (cancel, cancelled) = oneshot::channel::<()>();
            let endpoint = Endpoint::bind(presets::Minimal).await.unwrap();
            let joining = tokio::spawn(connect_with(
                endpoint,
                invite(&host, "doc", "key"),
                person("Dee"),
                |_| {},
                |_| {},
                async move { drop(cancelled.await) },
            ));
            let asked = |waiting: bool| {
                let events = events.clone();
                async move {
                    while !events
                        .requests
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|r| r.1 == "Dee" && r.2 == waiting)
                    {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(5), asked(true))
                .await
                .unwrap();
            // What `collab_cancel_joins` does.
            cancel.send(()).unwrap();
            assert_eq!(joining.await.unwrap().err().as_deref(), Some("Cancelled."));
            tokio::time::timeout(Duration::from_secs(5), asked(false))
                .await
                .unwrap();
            assert_eq!(host.inner.lock().unwrap().sessions["doc"].waiting, 0);
        });
    }

    #[test]
    fn a_guests_agent_joins_with_their_seat_after_the_code_expires() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc", "tok", "hk");
            let events = Arc::new(Events::default());
            let host = Arc::new(host(&server, events.clone()).await);
            add(&host, "doc", "tok", "key", false);
            let (_alex, _, seat, _rx) = join_as(invite(&host, "doc", "key"), person("Alex"))
                .await
                .unwrap();
            assert!(!seat.is_empty());

            // The code runs out and the host starts asking: Alex's agent still gets to ask.
            {
                let mut inner = host.inner.lock().unwrap();
                let s = inner.sessions.get_mut("doc").unwrap();
                s.expires = Instant::now();
                s.approval = true;
            }
            assert!(guest(invite(&host, "doc", "key"), "Late").await.is_err());
            let agent = |seat: &str| Guest {
                name: "Claude".into(),
                seat: seat.into(),
                agent: Some("claude".into()),
            };
            let host2 = host.clone();
            let events2 = events.clone();
            let decide = tokio::spawn(async move {
                loop {
                    let asked = events2
                        .requests
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|r| r.2)
                        .cloned();
                    if let Some((id, name, _)) = asked {
                        assert!(host2.answer(&id, true));
                        return name;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });
            let (_, token, agent_seat, _) = join_as(invite(&host, "doc", "stale"), agent(&seat))
                .await
                .unwrap();
            assert_eq!(decide.await.unwrap(), "Alex's Claude");
            assert_eq!(events.agents.lock().unwrap()[0].as_deref(), Some("claude"));
            assert_eq!(token, "tok");
            // An agent doesn't pass a seat on.
            assert!(agent_seat.is_empty());

            // A seat nobody was given, or one used by something that isn't an agent, doesn't work.
            let wrong = join_as(invite(&host, "doc", "key"), agent("nope")).await;
            assert!(wrong.err().unwrap().contains("doesn't know this copy"));
            let mut not_agent = agent(&seat);
            not_agent.agent = None;
            assert!(join_as(invite(&host, "doc", "key"), not_agent)
                .await
                .is_err());
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

            assert!(!crate::collab::codes::is_local(&code), "{code}");
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
            let invite = entry;
            let stale = invite.clone();
            let (client, token, _) = connect_with(
                endpoint,
                invite,
                person("Alex"),
                move |t| drop(tx.send(t.to_owned())),
                |_| {},
                std::future::pending(),
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
            let endpoint = Endpoint::bind(presets::N0).await.unwrap();
            let old = connect_with(
                endpoint,
                stale,
                person("Alex"),
                |_| {},
                |_| {},
                std::future::pending(),
            )
            .await;
            assert!(old.is_err(), "an invite from the old code still works");
            host.unregister("doc");
            host.shutdown();
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert!(directory::lookup_room(&url, &new).unwrap().is_none());
        });
    }
}
