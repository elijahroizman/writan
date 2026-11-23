//! Editing a document together over the internet or local network.
//!
//! Sharing starts a relay (`relay.rs`, run by `server.rs`) in this app. The relay only puts edits
//! in order; every copy of the document, the host's included, applies that order itself. The
//! host's page connects to its own relay over 127.0.0.1 like any guest, through `client.rs`,
//! because the web view isn't allowed to open WebSockets itself.
//!
//! For internet sharing (`p2p.rs` + `directory.rs`), the host also runs an ephemeral `iroh` P2P
//! endpoint and registers a 6-digit code with a 10-minute TTL in the code directory Worker.
//! Guests authenticate with SPAKE2 (the 6-digit code is never sent on the wire) and optional
//! 1-click host approval, then bridge directly into the same `Relay`.
//!
//! Messages reach the page as `collab-message` events (`{ conn, data }`) and the end of a
//! connection as `collab-closed` (`{ conn, reason }`).

pub mod client;
pub mod codes;
pub mod directory;
pub mod mcp;
pub mod p2p;
pub mod relay;
pub mod server;
mod socket;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use p2p::{DirectoryEntry, HostEvents, P2pClient, P2pHost};
use relay::Policy;
use server::Server;

/// The relay's port when it's free. Guests looking for a join code ask this port.
const PORT: u16 = codes::DISCOVERY_PORT;

/// The relay, while anything is shared.
static SERVER: Mutex<Option<Server>> = Mutex::new(None);

/// The ephemeral `iroh` P2P host, while any document is shared over the internet.
static P2P_HOST: LazyLock<tokio::sync::Mutex<Option<Arc<P2pHost>>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

enum Conn {
    Ws(client::Client),
    P2p(P2pClient),
}

impl Conn {
    fn send(&self, data: String) -> bool {
        match self {
            Conn::Ws(c) => c.send(data),
            Conn::P2p(c) => c.send(data),
        }
    }

    fn close(&self) {
        match self {
            Conn::Ws(c) => c.close(),
            Conn::P2p(c) => c.close(),
        }
    }
}

/// The page's open connections, by the id the page knows them by.
static CLIENTS: LazyLock<Mutex<HashMap<u32, Conn>>> = LazyLock::new(Default::default);
static NEXT_CONN: AtomicU32 = AtomicU32::new(1);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    pub session: String,
    pub token: String,
    pub host_key: String,
    pub port: u16,
    /// This machine's addresses on the local network, the likeliest first.
    pub addresses: Vec<String>,
    /// Legacy LAN invite URL (`ws://<first address>:<port>/v1/<session>?token=<token>`).
    pub invite_url: String,
    /// Six digits guests can join with (internet + LAN when `internet` is true, else LAN only).
    pub code: String,
    /// For the host's own connection.
    pub local_url: String,
    /// Whether internet P2P sharing is active for this session.
    pub internet: bool,
    /// Unix epoch milliseconds when the current 6-digit internet code expires (`None` for LAN-only).
    pub expires_at: Option<u64>,
    /// Whether internet joiners must be approved by the host.
    pub require_approval: bool,
    /// If internet sharing was requested but couldn't start (e.g. offline), why it fell back to LAN.
    pub internet_error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodeUpdate {
    pub session: String,
    pub code: String,
    pub expires_at: u64,
    pub rotated_by_lockout: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequestEvent {
    pub request_id: String,
    pub session: String,
    pub name: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct P2pConnInfo {
    pub conn: u32,
    pub session: String,
    pub token: String,
}

#[derive(Serialize, Clone)]
struct MessageEvent<'a> {
    conn: u32,
    data: &'a str,
}

#[derive(Serialize, Clone)]
struct ClosedEvent {
    conn: u32,
    reason: String,
}

struct AppHostEvents<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> HostEvents for AppHostEvents<R> {
    fn on_join_request(&self, request_id: String, session: String, name: String) {
        let _ = self.app.emit(
            "collab-join-request",
            JoinRequestEvent {
                request_id,
                session,
                name,
            },
        );
    }

    fn on_code_changed(
        &self,
        session: String,
        code: String,
        expires_at: u64,
        rotated_by_lockout: bool,
    ) {
        let _ = self.app.emit(
            "collab-code-changed",
            CodeUpdate {
                session,
                code,
                expires_at,
                rotated_by_lockout,
            },
        );
    }
}

/// `bytes` random bytes from the system, as URL-safe text.
fn random_text(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// This machine's IPv4 addresses on the networks it's on. Connecting a UDP socket sends
/// nothing, but makes the system pick the interface it would route through. The public
/// address finds the main network; the private ones find others, like a second adapter.
fn lan_addresses() -> Vec<Ipv4Addr> {
    let targets = [
        "1.1.1.1:80",
        "10.254.254.254:80",
        "172.16.0.1:80",
        "192.168.0.1:80",
    ];
    let mut found = Vec::new();
    for target in targets {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if socket.connect(target).is_err() {
            continue;
        }
        if let Ok(addr) = socket.local_addr() {
            if let IpAddr::V4(ip) = addr.ip() {
                if !ip.is_loopback() && !ip.is_unspecified() && !found.contains(&ip) {
                    found.push(ip);
                }
            }
        }
    }
    found
}

/// Makes macOS ask whether Writan may reach the local network, if nobody has answered yet.
#[tauri::command]
pub fn collab_local_network() {
    #[cfg(target_os = "macos")]
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        let _ = socket.send_to(&[0u8; 12], "224.0.0.251:5353");
    }
}

/// Opens System Settings at Privacy & Security › Local Network, where Writan can be allowed
/// after someone said no. Only on macOS.
#[tauri::command]
pub fn collab_network_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork")
            .spawn();
    }
}

/// Shares a document: starts the relay if need be and makes a session for it.
/// When `internet` is true (default), also starts an ephemeral `iroh` P2P endpoint and registers
/// a 6-digit code in the directory Worker (falling back to LAN-only if offline).
#[tauri::command]
pub async fn collab_share<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    kind: String,
    internet: Option<bool>,
    require_approval: Option<bool>,
) -> Result<ShareInfo, String> {
    let _ = (name, kind);
    let want_internet = internet.unwrap_or(true);
    let require_approval = require_approval.unwrap_or(true);

    let (session, token, host_key, port, bridge) = {
        let mut slot = SERVER.lock().unwrap();
        let server = running(&mut slot).map_err(|e| format!("Couldn't start sharing: {e}"))?;
        let session = random_text(12)?;
        let token = random_text(24)?;
        let host_key = random_text(24)?;
        server.register(&session, &token, &host_key);
        (session, token, host_key, server.port(), server.bridge())
    };

    let mut internet_active = false;
    let mut expires_at = None;
    let mut internet_error = None;
    let mut internet_code = None;

    if want_internet {
        let mut host_slot = P2P_HOST.lock().await;
        if host_slot.is_none() {
            let events: Arc<dyn HostEvents> = Arc::new(AppHostEvents { app: app.clone() });
            match P2pHost::start(bridge, events).await {
                Ok(h) => *host_slot = Some(Arc::new(h)),
                Err(e) => internet_error = Some(e),
            }
        }
        if let Some(host) = host_slot.as_ref().cloned() {
            drop(host_slot);
            match host
                .register_internet(
                    session.clone(),
                    token.clone(),
                    require_approval,
                    directory::directory_url(),
                )
                .await
            {
                Ok(res) => {
                    internet_active = true;
                    expires_at = Some(res.expires_at_ms);
                    internet_code = Some(res.code);
                }
                Err(e) => {
                    internet_error = Some(e);
                }
            }
        }
    }

    let code = {
        let slot = SERVER.lock().unwrap();
        let server = slot
            .as_ref()
            .ok_or_else(|| "Sharing stopped before it finished starting.".to_owned())?;
        if let Some(c) = internet_code {
            // Also answer the same 6-digit code on LAN for fast local discovery when
            // host approval isn't required. When approval is required, keep joins going
            // through P2P so the approval gate is enforced consistently.
            if !require_approval {
                server.set_code(&c, &session, &token);
            }
            c
        } else {
            server.add_code(&session, &token)?
        }
    };

    let addresses: Vec<String> = lan_addresses().iter().map(Ipv4Addr::to_string).collect();
    let first = addresses.first().map_or("127.0.0.1", String::as_str);
    Ok(ShareInfo {
        invite_url: format!("ws://{first}:{port}/v1/{session}?token={token}"),
        local_url: format!("ws://127.0.0.1:{port}/v1/{session}"),
        code,
        session,
        token,
        host_key,
        port,
        addresses,
        internet: internet_active,
        expires_at,
        require_approval,
        internet_error,
    })
}

/// Rotates an internet-shared session's 6-digit code and resets its 10-minute window.
#[tauri::command]
pub async fn collab_rotate_code(session: String) -> Result<CodeUpdate, String> {
    let host = P2P_HOST
        .lock()
        .await
        .as_ref()
        .cloned()
        .ok_or_else(|| "Internet sharing isn't active.".to_owned())?;
    let (code, expires_at) = host.rotate_code(&session).await?;
    Ok(CodeUpdate {
        session,
        code,
        expires_at,
        rotated_by_lockout: false,
    })
}

/// Toggles whether new internet joiners need 1-click host approval.
#[tauri::command]
pub async fn collab_set_require_approval(
    session: String,
    require_approval: bool,
) -> Result<(), String> {
    if let Some(host) = P2P_HOST.lock().await.as_ref() {
        host.set_require_approval(&session, require_approval);
    }
    Ok(())
}

/// Approves or denies a pending internet join request.
#[tauri::command]
pub async fn collab_approve_join(request_id: String, allow: bool) -> Result<(), String> {
    let host = P2P_HOST
        .lock()
        .await
        .as_ref()
        .cloned()
        .ok_or_else(|| "Internet sharing isn't active.".to_owned())?;
    if host.resolve_approval(&request_id, allow) {
        Ok(())
    } else {
        Err("That join request already expired.".into())
    }
}

/// Finds who is sharing with a six-digit code, racing the local network against the
/// internet code directory. Returns either a `ws://` (LAN) or `writan-p2p://` (internet) invite.
#[tauri::command]
pub async fn collab_find(code: String) -> Result<String, String> {
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    if !codes::is_code(&code) {
        return Err("A join code is six digits.".into());
    }

    let lan_code = code.clone();
    let mut lan = tauri::async_runtime::spawn_blocking(move || {
        codes::find(&lan_code, &lan_addresses())
    });

    let dir_code = code.clone();
    let dir_url = directory::directory_url();
    let mut internet = tauri::async_runtime::spawn_blocking(move || {
        match directory::lookup_room(&dir_url, &dir_code)? {
            Some(found) => {
                let entry: DirectoryEntry = serde_json::from_value(found.endpoint_addr)
                    .map_err(|_| "Invalid directory address.".to_owned())?;
                p2p::encode_invite(&entry.addr, &entry.session, Some(&dir_code), None).map(Some)
            }
            None => Ok(None),
        }
    });

    let (first_lan, first_net) = tokio::select! {
        r = &mut lan => (Some(r.map_err(|e| e.to_string()).and_then(|x| x)), None),
        r = &mut internet => (None, Some(r.map_err(|e| e.to_string()).and_then(|x| x))),
    };

    if let Some(Ok(url)) = &first_lan {
        return Ok(url.clone());
    }
    if let Some(Ok(Some(uri))) = &first_net {
        return Ok(uri.clone());
    }

    // Wait for the other lookup before giving up.
    let lan_res = match first_lan {
        Some(r) => r,
        None => lan.await.map_err(|e| e.to_string()).and_then(|x| x),
    };
    if let Ok(url) = lan_res {
        return Ok(url);
    }
    let net_res = match first_net {
        Some(r) => r,
        None => internet.await.map_err(|e| e.to_string()).and_then(|x| x),
    };
    match net_res {
        Ok(Some(uri)) => Ok(uri),
        Ok(None) => Err("Nobody is sharing with that code right now.".into()),
        Err(e) => Err(e),
    }
}

/// The relay's server, started if it isn't running.
fn running(slot: &mut Option<Server>) -> std::io::Result<&Server> {
    if slot.is_none() {
        *slot = Some(Server::start(Policy::Registered, &[PORT, 0])?);
    }
    Ok(slot.as_ref().expect("started above"))
}

/// Stops sharing: everyone in the session is told and disconnected. When nothing else is
/// shared and agents aren't let in, the relay and P2P endpoint stop listening.
#[tauri::command]
pub async fn collab_unshare(session: String) {
    {
        let mut p2p_slot = P2P_HOST.lock().await;
        if let Some(host) = p2p_slot.as_ref() {
            host.unregister(&session);
            if !host.has_sessions() {
                if let Some(h) = p2p_slot.take() {
                    if let Ok(owned) = Arc::try_unwrap(h) {
                        owned.shutdown();
                    }
                }
            }
        }
    }
    let mut slot = SERVER.lock().unwrap();
    if let Some(server) = slot.as_ref() {
        server.end_session(&session);
        if server.session_count() == 0 && !server.has_mcp() {
            *slot = None;
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsInfo {
    pub port: u16,
    /// The page's key to `/bridge/<key>`.
    pub bridge_key: String,
    /// This machine's addresses on the network, for agents on other computers.
    pub addresses: Vec<String>,
}

/// Lets AI agents in through Writan's MCP server, at `/mcp/<token>` on the relay's port (see
/// `mcp.rs`). The page then connects to `/bridge/<bridgeKey>` to answer their tool calls.
#[tauri::command]
pub fn agents_enable(token: String) -> Result<AgentsInfo, String> {
    if token.len() < 16
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("That isn't a usable token.".into());
    }
    let mut slot = SERVER.lock().unwrap();
    let server = running(&mut slot).map_err(|e| format!("Couldn't let agents in: {e}"))?;
    let bridge_key = random_text(24)?;
    server.set_mcp(Some((&token, &bridge_key)));
    Ok(AgentsInfo {
        port: server.port(),
        bridge_key,
        addresses: lan_addresses().iter().map(Ipv4Addr::to_string).collect(),
    })
}

/// Stops letting agents in. When nothing is shared either, the relay stops listening.
#[tauri::command]
pub fn agents_disable() {
    let mut slot = SERVER.lock().unwrap();
    if let Some(server) = slot.as_ref() {
        server.set_mcp(None);
        if server.session_count() == 0 {
            *slot = None;
        }
    }
}

/// Forgets what an earlier page started: its connections and the sessions it shared. The page
/// calls it when it loads, so reloading it doesn't leave others editing a session no page
/// follows.
#[tauri::command]
pub async fn collab_reset() {
    for (_, c) in CLIENTS.lock().unwrap().drain() {
        c.close();
    }
    if let Some(h) = P2P_HOST.lock().await.take() {
        if let Ok(owned) = Arc::try_unwrap(h) {
            owned.shutdown();
        }
    }
    if let Some(server) = SERVER.lock().unwrap().take() {
        server.end_all();
    }
}

/// Opens a connection to a relay (`ws://` or `wss://`) and returns its id.
#[tauri::command]
pub async fn collab_connect<R: Runtime>(app: AppHandle<R>, url: String) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let pending = client::connect(&url)?;
        let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
        // Held while the connection starts, so a quick close can't remove it before it's added.
        let mut clients = CLIENTS.lock().unwrap();
        let (on_message, on_closed) = (app.clone(), app);
        let client = pending.run(
            move |data| {
                let _ = on_message.emit("collab-message", MessageEvent { conn, data });
            },
            move |reason| {
                CLIENTS.lock().unwrap().remove(&conn);
                let _ = on_closed.emit("collab-closed", ClosedEvent { conn, reason });
            },
        )?;
        clients.insert(conn, Conn::Ws(client));
        Ok(conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens an encrypted `iroh` P2P connection (`writan-p2p://...`), runs SPAKE2 or token
/// reconnect, and returns the connection id along with the authenticated session & token.
#[tauri::command]
pub async fn collab_connect_p2p<R: Runtime>(
    app: AppHandle<R>,
    url: String,
    name: String,
) -> Result<P2pConnInfo, String> {
    let invite = p2p::parse_invite(&url)?;
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let (on_message, on_closed) = (app.clone(), app);
    let connected = p2p::connect_p2p(
        invite,
        name,
        move |data| {
            let _ = on_message.emit("collab-message", MessageEvent { conn, data });
        },
        move |reason| {
            CLIENTS.lock().unwrap().remove(&conn);
            let _ = on_closed.emit("collab-closed", ClosedEvent { conn, reason });
        },
    )
    .await?;
    CLIENTS
        .lock()
        .unwrap()
        .insert(conn, Conn::P2p(connected.client));
    Ok(P2pConnInfo {
        conn,
        session: connected.session,
        token: connected.token,
    })
}

/// Sends one text frame on a connection.
#[tauri::command]
pub fn collab_send(conn: u32, data: String) -> Result<(), String> {
    let clients = CLIENTS.lock().unwrap();
    match clients.get(&conn) {
        Some(c) if c.send(data) => Ok(()),
        _ => Err("Not connected.".into()),
    }
}

/// Closes a connection. `collab-closed` follows.
#[tauri::command]
pub fn collab_close(conn: u32) {
    if let Some(c) = CLIENTS.lock().unwrap().get(&conn) {
        c.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collab_ids_are_url_safe_and_long_enough() {
        let session = random_text(12).unwrap();
        assert_eq!(session.len(), 16);
        let token = random_text(24).unwrap();
        assert_eq!(token.len(), 32);
        assert!(token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(token, random_text(24).unwrap());
    }

    #[test]
    fn collab_client_reaches_a_relay() {
        let server = Server::start(Policy::Open, &[0]).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let closed_tx = tx.clone();
        let client = client::connect(&format!("ws://127.0.0.1:{}/v1/s?token=t", server.port()))
            .unwrap()
            .run(
                move |text| tx.send(text.to_owned()).unwrap(),
                move |reason| closed_tx.send(format!("closed: {reason}")).unwrap(),
            )
            .unwrap();
        assert!(client.send(
            r#"{"t":"hello","v":1,"token":"t","client":"c","hostKey":"k","snapshot":{"name":"a","kind":"docx","bytes":""}}"#
                .into()
        ));
        let welcome = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(welcome.starts_with(r#"{"t":"welcome""#));
        client.close();
        let closed = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(closed.starts_with("closed: "), "{closed}");
    }
}
