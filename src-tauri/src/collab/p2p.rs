//! Encrypted P2P transport for internet sharing using `iroh` (QUIC + TLS 1.3 + NAT hole-punching
//! + encrypted relay fallback) and `spake2` (password-authenticated key exchange for 6-digit codes).
//!
//! Each host runs one ephemeral `iroh::Endpoint` while any document is shared over the internet.
//! Guests dial the host's `EndpointAddr` (found via the 6-digit code directory in `directory.rs`),
//! prove knowledge of the 6-digit code with SPAKE2 without ever sending the code on the wire,
//! optionally wait for 1-click host approval, and then speak the exact same JSON protocol
//! (`hello`, `welcome`, `op`, `presence`, `end`) directly into `server.rs`'s `Relay`.
//!
//! Once a guest has joined and received the 192-bit session `token`, subsequent reconnects use
//! `"mode": "token"` so Wi-Fi blips resume seamlessly even after the 10-minute join code expires.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::sync::{mpsc, oneshot};

use super::directory;
use super::relay::MAX_FRAME;
use super::server::RelayBridge;
use super::socket::Cmd;

/// ALPN identifying Writan's P2P collaboration stream.
pub const ALPN: &[u8] = b"writan/collab/1";

/// Prefix for P2P invite URIs returned by `collab_find` and consumed by `collab_connect_p2p`.
pub const P2P_SCHEME: &str = "writan-p2p://";

/// Maximum frame size before a peer is authenticated (prevents unauthenticated OOM).
const MAX_HANDSHAKE_FRAME: usize = 64 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
const ONLINE_WAIT: Duration = Duration::from_secs(5);
const JOIN_CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// After this many failed SPAKE2 handshakes, the join code is invalidated and rotated.
const MAX_FAILED_ATTEMPTS: usize = 5;

const SPAKE_GUEST_ID: &[u8] = b"writan-guest-v1";
const SPAKE_HOST_ID: &[u8] = b"writan-host-v1";

/// Published in the code directory as `endpointAddr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub addr: EndpointAddr,
    pub session: String,
}

/// Decoded `writan-p2p://` invite.
#[derive(Debug, Clone)]
pub struct P2pInvite {
    pub addr: EndpointAddr,
    pub session: String,
    pub code: Option<String>,
    pub token: Option<String>,
}

/// Encodes a `DirectoryEntry` + optional 6-digit `code` or reconnect `token` into a `writan-p2p://` URI.
pub fn encode_invite(
    addr: &EndpointAddr,
    session: &str,
    code: Option<&str>,
    token: Option<&str>,
) -> Result<String, String> {
    let entry = DirectoryEntry {
        addr: addr.clone(),
        session: session.to_owned(),
    };
    let raw = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
    let encoded = URL_SAFE_NO_PAD.encode(raw);
    let mut uri = format!("{P2P_SCHEME}{encoded}");
    let mut sep = '?';
    if let Some(c) = code {
        uri.push(sep);
        uri.push_str("code=");
        uri.push_str(c);
        sep = '&';
    }
    if let Some(t) = token {
        uri.push(sep);
        uri.push_str("token=");
        uri.push_str(t);
    }
    Ok(uri)
}

/// Parses a `writan-p2p://<base64url(DirectoryEntry)>?code=123456` URI.
pub fn parse_invite(uri: &str) -> Result<P2pInvite, String> {
    let rest = uri
        .trim()
        .strip_prefix(P2P_SCHEME)
        .ok_or_else(|| "Not a P2P invite.".to_owned())?;
    let (payload, query) = rest.split_once('?').unwrap_or((rest, ""));
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "Invalid P2P invite address.".to_owned())?;
    let entry: DirectoryEntry =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid P2P invite payload.".to_owned())?;
    let mut code = None;
    let mut token = None;
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "code" if !v.is_empty() => code = Some(v.to_owned()),
                "token" if !v.is_empty() => token = Some(v.to_owned()),
                _ => {}
            }
        }
    }
    Ok(P2pInvite {
        addr: entry.addr,
        session: entry.session,
        code,
        token,
    })
}

/// RFC 2104 HMAC-SHA256 using `sha2::Sha256`.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut o_key_pad = [0x5cu8; BLOCK];
    let mut i_key_pad = [0x36u8; BLOCK];
    for i in 0..BLOCK {
        o_key_pad[i] ^= k_block[i];
        i_key_pad[i] ^= k_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(i_key_pad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_key_pad);
    outer.update(inner_hash);
    outer.finalize().into()
}

fn confirm_mac(key: &[u8], label: &[u8], msg_a: &[u8], msg_b: &[u8]) -> [u8; 32] {
    let mut transcript = Vec::with_capacity(label.len() + msg_a.len() + msg_b.len() + 2);
    transcript.extend_from_slice(label);
    transcript.push(0);
    transcript.extend_from_slice(msg_a);
    transcript.push(0);
    transcript.extend_from_slice(msg_b);
    hmac_sha256(key, &transcript)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn write_frame(send: &mut iroh::endpoint::SendStream, payload: &[u8]) -> Result<(), String> {
    if payload.len() > MAX_FRAME {
        return Err("Frame too large.".into());
    }
    let len = (payload.len() as u32).to_be_bytes();
    send.write_all(&len).await.map_err(|e| e.to_string())?;
    send.write_all(payload).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn read_frame(
    recv: &mut iroh::endpoint::RecvStream,
    max_len: usize,
) -> Result<Option<String>, String> {
    let mut len_buf = [0u8; 4];
    match recv.read_exact(&mut len_buf).await {
        Ok(()) => {}
        Err(iroh::endpoint::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > max_len {
        return Err("Frame exceeds maximum size.".into());
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| "Frame is not valid UTF-8.".into())
}

// Handshake wire messages.

#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
enum ClientInit {
    Code {
        session: String,
        name: String,
        spake: String,
    },
    Token {
        session: String,
        token: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerSpake {
    ok: bool,
    #[serde(default)]
    spake: String,
    #[serde(default)]
    confirm: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Serialize, Deserialize)]
struct ClientConfirm {
    confirm: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerOutcome {
    ok: bool,
    #[serde(default)]
    pending: bool,
    #[serde(default)]
    session: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// Events surfaced from the P2P host to the Tauri frontend.
pub trait HostEvents: Send + Sync + 'static {
    fn on_join_request(&self, request_id: String, session: String, name: String);
    fn on_code_changed(&self, session: String, code: String, expires_at: u64, rotated_by_lockout: bool);
}

struct SessionEntry {
    token: String,
    code: String,
    expires_at: Instant,
    expires_at_ms: u64,
    failed_attempts: usize,
    require_approval: bool,
    directory_url: Option<String>,
    host_secret: Option<String>,
}

struct HostInner {
    sessions: HashMap<String, SessionEntry>,
    approvals: HashMap<String, oneshot::Sender<bool>>,
    next_approval: u64,
}

/// Outcome of registering a session on the P2P host.
#[derive(Debug, Clone)]
pub struct P2pShareResult {
    pub code: String,
    pub expires_at_ms: u64,
    pub addr: EndpointAddr,
}

/// Host-side `iroh` P2P listener, shared across all internet-shared sessions in this app.
pub struct P2pHost {
    endpoint: Endpoint,
    inner: Arc<Mutex<HostInner>>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl P2pHost {
    /// Starts an ephemeral `iroh::Endpoint` with n0 relay support for internet NAT traversal.
    pub async fn start(
        bridge: RelayBridge,
        events: Arc<dyn HostEvents>,
    ) -> Result<Self, String> {
        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| format!("Couldn't start P2P endpoint: {e}"))?;
        // Wait briefly for relay / address discovery so EndpointAddr includes our home relay.
        let _ = tokio::time::timeout(ONLINE_WAIT, endpoint.online()).await;
        Ok(Self::from_endpoint(endpoint, bridge, events))
    }

    #[cfg(test)]
    async fn start_local(
        bridge: RelayBridge,
        events: Arc<dyn HostEvents>,
    ) -> Result<Self, String> {
        let endpoint = Endpoint::builder(presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| format!("Couldn't start local P2P endpoint: {e}"))?;
        Ok(Self::from_endpoint(endpoint, bridge, events))
    }

    fn from_endpoint(
        endpoint: Endpoint,
        bridge: RelayBridge,
        events: Arc<dyn HostEvents>,
    ) -> Self {
        let inner = Arc::new(Mutex::new(HostInner {
            sessions: HashMap::new(),
            approvals: HashMap::new(),
            next_approval: 1,
        }));
        let accept_ep = endpoint.clone();
        let accept_inner = inner.clone();
        let accept_bridge = bridge.clone();
        let accept_events = events.clone();
        let accept_task = tokio::spawn(async move {
            while let Some(incoming) = accept_ep.accept().await {
                let inner = accept_inner.clone();
                let bridge = accept_bridge.clone();
                let events = accept_events.clone();
                let ep = accept_ep.clone();
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else { return };
                    let Ok((send, recv)) = conn.accept_bi().await else {
                        return;
                    };
                    handle_peer(send, recv, ep, inner, bridge, events).await;
                });
            }
        });
        Self {
            endpoint,
            inner,
            accept_task,
        }
    }

    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// Registers a session with the directory Worker and begins accepting SPAKE2 joins for it.
    pub async fn register_internet(
        &self,
        session: String,
        token: String,
        require_approval: bool,
        dir_url: String,
    ) -> Result<P2pShareResult, String> {
        let addr = self.endpoint.addr();
        let entry = serde_json::to_value(DirectoryEntry {
            addr: addr.clone(),
            session: session.clone(),
        })
        .map_err(|e| e.to_string())?;
        let dir_clone = dir_url.clone();
        let created = tokio::task::spawn_blocking(move || directory::create_room(&dir_clone, &entry))
            .await
            .map_err(|e| e.to_string())??;
        let ttl = Duration::from_secs(created.ttl_secs.max(60));
        let mut guard = self.inner.lock().unwrap();
        guard.sessions.insert(
            session,
            SessionEntry {
                token,
                code: created.code.clone(),
                expires_at: Instant::now() + ttl,
                expires_at_ms: created.expires_at,
                failed_attempts: 0,
                require_approval,
                directory_url: Some(dir_url),
                host_secret: Some(created.host_secret),
            },
        );
        Ok(P2pShareResult {
            code: created.code,
            expires_at_ms: created.expires_at,
            addr,
        })
    }

    /// Rotates a session's 6-digit code in the directory and resets its 10-minute window and strike count.
    pub async fn rotate_code(&self, session: &str) -> Result<(String, u64), String> {
        let (old_code, dir_url, host_secret) = {
            let guard = self.inner.lock().unwrap();
            let s = guard
                .sessions
                .get(session)
                .ok_or_else(|| "Session is not shared over the internet.".to_owned())?;
            (
                s.code.clone(),
                s.directory_url.clone(),
                s.host_secret.clone(),
            )
        };
        let addr = self.endpoint.addr();
        let entry = serde_json::to_value(DirectoryEntry {
            addr,
            session: session.to_owned(),
        })
        .map_err(|e| e.to_string())?;

        let (new_code, expires_at_ms, ttl_secs, new_secret) = match (dir_url, host_secret) {
            (Some(base), Some(secret)) => {
                let base_c = base.clone();
                let old_c = old_code.clone();
                let sec_c = secret.clone();
                let entry_c = entry.clone();
                let rotated = tokio::task::spawn_blocking(move || {
                    match directory::rotate_room(&base_c, &old_c, &sec_c, Some(&entry_c)) {
                        Ok(r) => Ok::<_, String>((r.code, r.expires_at, r.ttl_secs, None)),
                        Err(_) => {
                            // If the old code aged out of the Worker, allocate a brand-new room.
                            let c = directory::create_room(&base_c, &entry_c)?;
                            Ok((c.code, c.expires_at, c.ttl_secs, Some(c.host_secret)))
                        }
                    }
                })
                .await
                .map_err(|e| e.to_string())??;
                rotated
            }
            _ => return Err("No directory configured for this session.".into()),
        };

        let mut guard = self.inner.lock().unwrap();
        if let Some(s) = guard.sessions.get_mut(session) {
            s.code = new_code.clone();
            s.expires_at = Instant::now() + Duration::from_secs(ttl_secs.max(60));
            s.expires_at_ms = expires_at_ms;
            s.failed_attempts = 0;
            if let Some(sec) = new_secret {
                s.host_secret = Some(sec);
            }
        }
        Ok((new_code, expires_at_ms))
    }

    pub fn set_require_approval(&self, session: &str, require_approval: bool) {
        if let Some(s) = self.inner.lock().unwrap().sessions.get_mut(session) {
            s.require_approval = require_approval;
        }
    }

    /// Resolves a pending host join-approval prompt (`Allow` or `Deny`).
    pub fn resolve_approval(&self, request_id: &str, allow: bool) -> bool {
        let tx = self.inner.lock().unwrap().approvals.remove(request_id);
        match tx {
            Some(tx) => tx.send(allow).is_ok(),
            None => false,
        }
    }

    /// Unregisters a session and removes its code from the directory Worker.
    pub fn unregister(&self, session: &str) {
        let removed = self.inner.lock().unwrap().sessions.remove(session);
        if let Some(entry) = removed {
            if let (Some(base), Some(secret)) = (entry.directory_url, entry.host_secret) {
                let code = entry.code;
                std::thread::spawn(move || directory::delete_room(&base, &code, &secret));
            }
        }
    }

    pub fn has_sessions(&self) -> bool {
        !self.inner.lock().unwrap().sessions.is_empty()
    }

    /// Removes all sessions from the directory and closes the endpoint.
    pub fn shutdown(self) {
        let drained: Vec<SessionEntry> = {
            let mut guard = self.inner.lock().unwrap();
            guard.approvals.clear();
            guard.sessions.drain().map(|(_, v)| v).collect()
        };
        for entry in drained {
            if let (Some(base), Some(secret)) = (entry.directory_url, entry.host_secret) {
                let code = entry.code;
                std::thread::spawn(move || directory::delete_room(&base, &code, &secret));
            }
        }
        self.accept_task.abort();
        let ep = self.endpoint;
        tokio::spawn(async move {
            ep.close().await;
        });
    }

    #[cfg(test)]
    fn register_local_for_test(
        &self,
        session: &str,
        token: &str,
        code: &str,
        require_approval: bool,
        ttl: Duration,
    ) {
        let mut guard = self.inner.lock().unwrap();
        guard.sessions.insert(
            session.to_owned(),
            SessionEntry {
                token: token.to_owned(),
                code: code.to_owned(),
                expires_at: Instant::now() + ttl,
                expires_at_ms: 0,
                failed_attempts: 0,
                require_approval,
                directory_url: None,
                host_secret: None,
            },
        );
    }
}

async fn send_outcome_err(send: &mut iroh::endpoint::SendStream, code: &str, message: &str) {
    let msg = serde_json::to_string(&ServerOutcome {
        ok: false,
        pending: false,
        session: String::new(),
        token: String::new(),
        code: code.to_owned(),
        message: message.to_owned(),
    })
    .unwrap_or_default();
    let _ = write_frame(send, msg.as_bytes()).await;
    let _ = send.finish();
}

async fn handle_peer(
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    endpoint: Endpoint,
    inner: Arc<Mutex<HostInner>>,
    bridge: RelayBridge,
    events: Arc<dyn HostEvents>,
) {
    let Ok(Ok(Some(init_text))) =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut recv, MAX_HANDSHAKE_FRAME)).await
    else {
        return;
    };
    let Ok(init) = serde_json::from_str::<ClientInit>(&init_text) else {
        send_outcome_err(&mut send, "bad-message", "Invalid join handshake.").await;
        return;
    };

    let (session_id, token) = match init {
        ClientInit::Token { session, token } => {
            let valid = {
                let guard = inner.lock().unwrap();
                guard
                    .sessions
                    .get(&session)
                    .is_some_and(|s| constant_time_eq(s.token.as_bytes(), token.as_bytes()))
            };
            if !valid {
                send_outcome_err(
                    &mut send,
                    "bad-token",
                    "This share session is no longer active.",
                )
                .await;
                return;
            }
            let ok_msg = serde_json::to_string(&ServerOutcome {
                ok: true,
                pending: false,
                session: session.clone(),
                token: token.clone(),
                code: String::new(),
                message: String::new(),
            })
            .unwrap_or_default();
            if write_frame(&mut send, ok_msg.as_bytes()).await.is_err() {
                return;
            }
            (session, token)
        }
        ClientInit::Code {
            session,
            name,
            spake,
        } => {
            let Ok(msg_a) = URL_SAFE_NO_PAD.decode(&spake) else {
                send_outcome_err(&mut send, "bad-message", "Invalid handshake data.").await;
                return;
            };
            let lookup = {
                let guard = inner.lock().unwrap();
                match guard.sessions.get(&session) {
                    None => Err(("no-session", "This document isn't being shared any more.")),
                    Some(s) if Instant::now() >= s.expires_at => {
                        Err(("expired", "That code expired. Ask for a new code."))
                    }
                    Some(s) if s.failed_attempts >= MAX_FAILED_ATTEMPTS => Err((
                        "locked",
                        "Too many wrong attempts. Ask for the new code.",
                    )),
                    Some(s) => Ok((s.code.clone(), s.token.clone(), s.require_approval)),
                }
            };
            let (expected_code, session_token, require_approval) = match lookup {
                Ok(v) => v,
                Err((code, message)) => {
                    let reply = serde_json::to_string(&ServerSpake {
                        ok: false,
                        spake: String::new(),
                        confirm: String::new(),
                        code: code.to_owned(),
                        message: message.to_owned(),
                    })
                    .unwrap_or_default();
                    let _ = write_frame(&mut send, reply.as_bytes()).await;
                    let _ = send.finish();
                    return;
                }
            };

            let (state_b, msg_b) = Spake2::<Ed25519Group>::start_b(
                &Password::new(expected_code.as_bytes()),
                &Identity::new(SPAKE_GUEST_ID),
                &Identity::new(SPAKE_HOST_ID),
            );
            let Ok(shared_key) = state_b.finish(&msg_a) else {
                record_failed_attempt(&session, &endpoint, &inner, &events);
                let reply = serde_json::to_string(&ServerSpake {
                    ok: false,
                    spake: String::new(),
                    confirm: String::new(),
                    code: "bad-code".into(),
                    message: "That code didn't match.".into(),
                })
                .unwrap_or_default();
                let _ = write_frame(&mut send, reply.as_bytes()).await;
                let _ = send.finish();
                return;
            };

            let server_confirm =
                confirm_mac(&shared_key, b"writan-server-confirm", &msg_a, &msg_b);
            let reply = serde_json::to_string(&ServerSpake {
                ok: true,
                spake: URL_SAFE_NO_PAD.encode(&msg_b),
                confirm: URL_SAFE_NO_PAD.encode(server_confirm),
                code: String::new(),
                message: String::new(),
            })
            .unwrap_or_default();
            if write_frame(&mut send, reply.as_bytes()).await.is_err() {
                return;
            }

            let Ok(Ok(Some(confirm_text))) = tokio::time::timeout(
                HANDSHAKE_TIMEOUT,
                read_frame(&mut recv, MAX_HANDSHAKE_FRAME),
            )
            .await
            else {
                record_failed_attempt(&session, &endpoint, &inner, &events);
                return;
            };
            let Ok(client_confirm_msg) = serde_json::from_str::<ClientConfirm>(&confirm_text)
            else {
                record_failed_attempt(&session, &endpoint, &inner, &events);
                send_outcome_err(&mut send, "bad-code", "That code didn't match.").await;
                return;
            };
            let Ok(got_confirm) = URL_SAFE_NO_PAD.decode(&client_confirm_msg.confirm) else {
                record_failed_attempt(&session, &endpoint, &inner, &events);
                send_outcome_err(&mut send, "bad-code", "That code didn't match.").await;
                return;
            };
            let expected_client_confirm =
                confirm_mac(&shared_key, b"writan-client-confirm", &msg_a, &msg_b);
            if !constant_time_eq(&got_confirm, &expected_client_confirm) {
                record_failed_attempt(&session, &endpoint, &inner, &events);
                send_outcome_err(&mut send, "bad-code", "That code didn't match.").await;
                return;
            }

            // Reset strike counter on a valid SPAKE2 proof.
            if let Some(s) = inner.lock().unwrap().sessions.get_mut(&session) {
                s.failed_attempts = 0;
            }

            if require_approval {
                let pending_msg = serde_json::to_string(&ServerOutcome {
                    ok: false,
                    pending: true,
                    session: String::new(),
                    token: String::new(),
                    code: String::new(),
                    message: "Waiting for the host to approve…".into(),
                })
                .unwrap_or_default();
                if write_frame(&mut send, pending_msg.as_bytes()).await.is_err() {
                    return;
                }
                let (tx, rx) = oneshot::channel();
                let request_id = {
                    let mut guard = inner.lock().unwrap();
                    let id = format!("req-{}", guard.next_approval);
                    guard.next_approval += 1;
                    guard.approvals.insert(id.clone(), tx);
                    id
                };
                let display = if name.trim().is_empty() {
                    "Someone".to_owned()
                } else {
                    name.trim().chars().take(64).collect()
                };
                events.on_join_request(request_id.clone(), session.clone(), display);
                let approved = tokio::time::timeout(APPROVAL_TIMEOUT, rx)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or(false);
                inner.lock().unwrap().approvals.remove(&request_id);
                if !approved {
                    send_outcome_err(
                        &mut send,
                        "denied",
                        "The person sharing didn't approve the request.",
                    )
                    .await;
                    return;
                }
            }

            let ok_msg = serde_json::to_string(&ServerOutcome {
                ok: true,
                pending: false,
                session: session.clone(),
                token: session_token.clone(),
                code: String::new(),
                message: String::new(),
            })
            .unwrap_or_default();
            if write_frame(&mut send, ok_msg.as_bytes()).await.is_err() {
                return;
            }
            (session, session_token)
        }
    };
    let _ = token;

    // Authenticated! Bridge this QUIC stream straight into the Relay.
    let (conn_id, mut out_rx) = bridge.connect_async(&session_id);

    let write_task = tokio::spawn(async move {
        while let Some(cmd) = out_rx.recv().await {
            match cmd {
                Cmd::Text(text) => {
                    if write_frame(&mut send, text.as_bytes()).await.is_err() {
                        break;
                    }
                }
                Cmd::Close => break,
            }
        }
        let _ = send.finish();
    });

    while let Ok(Some(text)) = read_frame(&mut recv, MAX_FRAME).await {
        bridge.handle(conn_id, &text);
    }

    bridge.disconnect(conn_id);
    write_task.abort();
}

fn record_failed_attempt(
    session: &str,
    endpoint: &Endpoint,
    inner: &Arc<Mutex<HostInner>>,
    events: &Arc<dyn HostEvents>,
) {
    let rotate_info = {
        let mut guard = inner.lock().unwrap();
        let Some(s) = guard.sessions.get_mut(session) else {
            return;
        };
        s.failed_attempts += 1;
        if s.failed_attempts < MAX_FAILED_ATTEMPTS {
            return;
        }
        // Invalidate current code immediately so further guesses fail until rotation finishes.
        s.expires_at = Instant::now();
        (
            s.code.clone(),
            s.directory_url.clone(),
            s.host_secret.clone(),
        )
    };
    let (old_code, Some(base), Some(secret)) = rotate_info else {
        return;
    };
    let session_owned = session.to_owned();
    let addr = endpoint.addr();
    let inner_c = inner.clone();
    let events_c = events.clone();
    tokio::spawn(async move {
        let entry = serde_json::to_value(DirectoryEntry {
            addr,
            session: session_owned.clone(),
        })
        .ok();
        let rotated = tokio::task::spawn_blocking(move || {
            directory::rotate_room(&base, &old_code, &secret, entry.as_ref())
        })
        .await
        .ok()
        .and_then(Result::ok);
        if let Some(r) = rotated {
            let mut guard = inner_c.lock().unwrap();
            if let Some(s) = guard.sessions.get_mut(&session_owned) {
                s.code = r.code.clone();
                s.expires_at = Instant::now() + Duration::from_secs(r.ttl_secs.max(60));
                s.expires_at_ms = r.expires_at;
                s.failed_attempts = 0;
            }
            drop(guard);
            events_c.on_code_changed(session_owned, r.code, r.expires_at, true);
        }
    });
}

/// Guest-side P2P connection handle.
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

/// Result of establishing an authenticated P2P connection as a guest.
pub struct P2pConnected {
    pub client: P2pClient,
    pub session: String,
    pub token: String,
}

/// Dials a host over `iroh`, runs SPAKE2 (or token reconnect), and pumps relay frames.
pub async fn connect_p2p(
    invite: P2pInvite,
    name: String,
    on_text: impl FnMut(&str) + Send + 'static,
    on_closed: impl FnOnce(String) + Send + 'static,
) -> Result<P2pConnected, String> {
    let endpoint = Endpoint::bind(presets::N0)
        .await
        .map_err(|e| format!("Couldn't start P2P client: {e}"))?;
    connect_with_endpoint(endpoint, invite, name, on_text, on_closed).await
}

async fn connect_with_endpoint(
    endpoint: Endpoint,
    invite: P2pInvite,
    name: String,
    mut on_text: impl FnMut(&str) + Send + 'static,
    on_closed: impl FnOnce(String) + Send + 'static,
) -> Result<P2pConnected, String> {
    let conn = tokio::time::timeout(
        Duration::from_secs(20),
        endpoint.connect(invite.addr, ALPN),
    )
    .await
    .map_err(|_| "Timed out reaching the person sharing.".to_owned())?
    .map_err(|e| format!("Couldn't reach the person sharing: {e}"))?;

    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("Couldn't open P2P stream: {e}"))?;

    let (session, token) = if let Some(tok) = invite.token {
        let init = serde_json::to_string(&ClientInit::Token {
            session: invite.session.clone(),
            token: tok,
        })
        .map_err(|e| e.to_string())?;
        write_frame(&mut send, init.as_bytes()).await?;
        let reply_text = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            read_frame(&mut recv, MAX_HANDSHAKE_FRAME),
        )
        .await
        .map_err(|_| "Timed out reconnecting.".to_owned())?
        .map_err(|e| format!("Reconnect failed: {e}"))?
        .ok_or_else(|| "Connection closed while reconnecting.".to_owned())?;
        let outcome: ServerOutcome = serde_json::from_str(&reply_text)
            .map_err(|_| "Invalid reconnect response.".to_owned())?;
        if !outcome.ok {
            return Err(if outcome.message.is_empty() {
                "Couldn't reconnect to the shared document.".into()
            } else {
                outcome.message
            });
        }
        (outcome.session, outcome.token)
    } else {
        let code = invite
            .code
            .ok_or_else(|| "Missing join code for P2P connection.".to_owned())?;
        let (state_a, msg_a) = Spake2::<Ed25519Group>::start_a(
            &Password::new(code.as_bytes()),
            &Identity::new(SPAKE_GUEST_ID),
            &Identity::new(SPAKE_HOST_ID),
        );
        let init = serde_json::to_string(&ClientInit::Code {
            session: invite.session.clone(),
            name,
            spake: URL_SAFE_NO_PAD.encode(&msg_a),
        })
        .map_err(|e| e.to_string())?;
        write_frame(&mut send, init.as_bytes()).await?;

        let server_text = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            read_frame(&mut recv, MAX_HANDSHAKE_FRAME),
        )
        .await
        .map_err(|_| "Timed out during secure handshake.".to_owned())?
        .map_err(|e| format!("Handshake failed: {e}"))?
        .ok_or_else(|| "The host closed the connection.".to_owned())?;
        let server_msg: ServerSpake = serde_json::from_str(&server_text)
            .map_err(|_| "Invalid handshake response.".to_owned())?;
        if !server_msg.ok {
            return Err(if server_msg.message.is_empty() {
                "That code didn't match.".into()
            } else {
                server_msg.message
            });
        }

        let msg_b = URL_SAFE_NO_PAD
            .decode(&server_msg.spake)
            .map_err(|_| "Invalid handshake key.".to_owned())?;
        let got_server_confirm = URL_SAFE_NO_PAD
            .decode(&server_msg.confirm)
            .map_err(|_| "Invalid handshake confirmation.".to_owned())?;
        let shared_key = state_a
            .finish(&msg_b)
            .map_err(|_| "That code didn't match.".to_owned())?;
        let expected_server_confirm =
            confirm_mac(&shared_key, b"writan-server-confirm", &msg_a, &msg_b);
        if !constant_time_eq(&got_server_confirm, &expected_server_confirm) {
            return Err("That code didn't match.".into());
        }

        let client_confirm = confirm_mac(&shared_key, b"writan-client-confirm", &msg_a, &msg_b);
        let confirm_payload = serde_json::to_string(&ClientConfirm {
            confirm: URL_SAFE_NO_PAD.encode(client_confirm),
        })
        .map_err(|e| e.to_string())?;
        write_frame(&mut send, confirm_payload.as_bytes()).await?;

        // Wait for final outcome (may first receive a `pending: true` frame while host approves).
        loop {
            let outcome_text = tokio::time::timeout(
                APPROVAL_TIMEOUT + Duration::from_secs(5),
                read_frame(&mut recv, MAX_HANDSHAKE_FRAME),
            )
            .await
            .map_err(|_| "Timed out waiting for the host to approve.".to_owned())?
            .map_err(|e| format!("Connection failed: {e}"))?
            .ok_or_else(|| "The host closed the connection.".to_owned())?;
            let outcome: ServerOutcome = serde_json::from_str(&outcome_text)
                .map_err(|_| "Invalid join response.".to_owned())?;
            if outcome.pending {
                continue;
            }
            if !outcome.ok {
                return Err(if outcome.message.is_empty() {
                    "Couldn't join the shared document.".into()
                } else {
                    outcome.message
                });
            }
            break (outcome.session, outcome.token);
        }
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<Cmd>();
    tokio::spawn(async move {
        let write_task = tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    Cmd::Text(text) => {
                        if write_frame(&mut send, text.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Cmd::Close => break,
                }
            }
            let _ = send.finish();
        });

        let reason = loop {
            match read_frame(&mut recv, MAX_FRAME).await {
                Ok(Some(text)) => on_text(&text),
                Ok(None) => break "closed".to_owned(),
                Err(e) => break e,
            }
        };
        write_task.abort();
        // Keep conn and endpoint alive until stream finishes, then close cleanly.
        drop(conn);
        endpoint.close().await;
        on_closed(reason);
    });

    Ok(P2pConnected {
        client: P2pClient { tx },
        session,
        token,
    })
}

/// Default join-code TTL in seconds (10 minutes).
pub const fn default_ttl_secs() -> u64 {
    JOIN_CODE_TTL.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::relay::Policy;
    use crate::collab::server::Server;

    struct TestEvents {
        requests: Mutex<Vec<(String, String, String)>>,
    }

    impl HostEvents for TestEvents {
        fn on_join_request(&self, request_id: String, session: String, name: String) {
            self.requests
                .lock()
                .unwrap()
                .push((request_id, session, name));
        }

        fn on_code_changed(&self, _session: String, _code: String, _expires_at: u64, _lockout: bool) {}
    }

    #[test]
    fn invite_round_trip() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ep = Endpoint::builder(presets::Minimal).bind().await.unwrap();
            let addr = ep.addr();
            let uri = encode_invite(&addr, "sess123", Some("482913"), None).unwrap();
            assert!(uri.starts_with(P2P_SCHEME));
            let parsed = parse_invite(&uri).unwrap();
            assert_eq!(parsed.session, "sess123");
            assert_eq!(parsed.code.as_deref(), Some("482913"));
            assert_eq!(parsed.token, None);
            assert_eq!(parsed.addr.id, addr.id);
            ep.close().await;
        });
    }

    #[test]
    fn spake2_p2p_roundtrip_and_token_reconnect() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc1", "secret-token-123456", "host-key-123");
            let events = Arc::new(TestEvents {
                requests: Mutex::new(Vec::new()),
            });
            let host = P2pHost::start_local(server.bridge(), events.clone())
                .await
                .unwrap();
            host.register_local_for_test(
                "doc1",
                "secret-token-123456",
                "482913",
                false,
                Duration::from_secs(60),
            );

            // Wrong code fails SPAKE2 cleanly.
            let bad_ep = Endpoint::builder(presets::Minimal).bind().await.unwrap();
            let bad = connect_with_endpoint(
                bad_ep,
                P2pInvite {
                    addr: host.addr(),
                    session: "doc1".into(),
                    code: Some("999999".into()),
                    token: None,
                },
                "Mallory".into(),
                |_| {},
                |_| {},
            )
            .await;
            assert!(bad.is_err());

            // Seed the relay with a host snapshot over local bridge so guest gets welcome.
            let (host_conn, mut _host_rx) = server.bridge().connect_async("doc1");
            server.bridge().handle(
                host_conn,
                r##"{"t":"hello","v":1,"token":"secret-token-123456","client":"h","name":"Host","color":"#f00","hostKey":"host-key-123","snapshot":{"name":"Doc.docx","kind":"docx","bytes":"UEsDBA=="}}"##,
            );

            // Correct code succeeds via SPAKE2 and bridges into Relay.
            let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<String>();
            let guest_ep = Endpoint::builder(presets::Minimal).bind().await.unwrap();
            let connected = connect_with_endpoint(
                guest_ep,
                P2pInvite {
                    addr: host.addr(),
                    session: "doc1".into(),
                    code: Some("482913".into()),
                    token: None,
                },
                "Alex".into(),
                move |t| {
                    let _ = msg_tx.send(t.to_owned());
                },
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(connected.session, "doc1");
            assert_eq!(connected.token, "secret-token-123456");

            assert!(connected.client.send(
                r##"{"t":"hello","v":1,"token":"secret-token-123456","client":"g1","name":"Alex","color":"#0f0"}"##
                    .into()
            ));
            let welcome = tokio::time::timeout(Duration::from_secs(5), msg_rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(welcome.contains(r#""t":"welcome""#), "{welcome}");
            connected.client.close();

            // Even after the join code expires, reconnecting with the session token succeeds.
            host.inner.lock().unwrap().sessions.get_mut("doc1").unwrap().expires_at =
                Instant::now() - Duration::from_secs(1);
            let (re_tx, mut re_rx) = mpsc::unbounded_channel::<String>();
            let guest_ep2 = Endpoint::builder(presets::Minimal).bind().await.unwrap();
            let reconnected = connect_with_endpoint(
                guest_ep2,
                P2pInvite {
                    addr: host.addr(),
                    session: "doc1".into(),
                    code: None,
                    token: Some(connected.token),
                },
                "Alex".into(),
                move |t| {
                    let _ = re_tx.send(t.to_owned());
                },
                |_| {},
            )
            .await
            .unwrap();
            assert!(reconnected.client.send(
                r##"{"t":"hello","v":1,"token":"secret-token-123456","client":"g1","name":"Alex","color":"#0f0"}"##
                    .into()
            ));
            let welcome2 = tokio::time::timeout(Duration::from_secs(5), re_rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(welcome2.contains(r#""t":"welcome""#), "{welcome2}");
            reconnected.client.close();
            host.shutdown();
        });
    }

    #[test]
    fn host_approval_gate_allows_and_denies() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = Server::start(Policy::Registered, &[0]).unwrap();
            server.register("doc2", "tok2", "hk2");
            let events = Arc::new(TestEvents {
                requests: Mutex::new(Vec::new()),
            });
            let host = Arc::new(
                P2pHost::start_local(server.bridge(), events.clone())
                    .await
                    .unwrap(),
            );
            host.register_local_for_test("doc2", "tok2", "112233", true, Duration::from_secs(60));

            // Deny case
            let host_deny = host.clone();
            let events_deny = events.clone();
            let resolver = tokio::spawn(async move {
                loop {
                    let req = events_deny.requests.lock().unwrap().pop();
                    if let Some((id, _, name)) = req {
                        assert_eq!(name, "Bob");
                        assert!(host_deny.resolve_approval(&id, false));
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });
            let ep1 = Endpoint::builder(presets::Minimal).bind().await.unwrap();
            let denied = connect_with_endpoint(
                ep1,
                P2pInvite {
                    addr: host.addr(),
                    session: "doc2".into(),
                    code: Some("112233".into()),
                    token: None,
                },
                "Bob".into(),
                |_| {},
                |_| {},
            )
            .await;
            resolver.await.unwrap();
            assert!(denied.is_err());

            // Allow case
            let host_allow = host.clone();
            let events_allow = events.clone();
            let resolver2 = tokio::spawn(async move {
                loop {
                    let req = events_allow.requests.lock().unwrap().pop();
                    if let Some((id, _, name)) = req {
                        assert_eq!(name, "Carol");
                        assert!(host_allow.resolve_approval(&id, true));
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });
            let ep2 = Endpoint::builder(presets::Minimal).bind().await.unwrap();
            let allowed = connect_with_endpoint(
                ep2,
                P2pInvite {
                    addr: host.addr(),
                    session: "doc2".into(),
                    code: Some("112233".into()),
                    token: None,
                },
                "Carol".into(),
                |_| {},
                |_| {},
            )
            .await
            .unwrap();
            resolver2.await.unwrap();
            assert_eq!(allowed.token, "tok2");
            allowed.client.close();
        });
    }
}
