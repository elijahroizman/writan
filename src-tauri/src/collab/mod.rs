//! Editing a document together, on the local network or over the internet.
//!
//! Sharing starts a relay (`relay.rs`, run by `server.rs`) in this app. The relay only puts edits
//! in order; every copy of the document, the host's included, applies that order itself. The
//! host's page connects to its own relay over 127.0.0.1 like any guest, through `client.rs`,
//! because the web view isn't allowed to open WebSockets itself.
//!
//! In the app, sharing also puts the session's code in the code directory (`directory.rs`), and
//! guests anywhere reach the relay through an `iroh` connection to this app (`p2p.rs`). When that
//! can't start, the code works on the local network only (`codes.rs`).
//!
//! Messages reach the page as `collab-message` events (`{ conn, data }`) and the end of a
//! connection as `collab-closed` (`{ conn, reason }`). A relay says nothing until it has the
//! client's hello, so no message arrives before `collab_connect` has returned the conn id.

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
use server::RelayBridge;
use server::Server;

/// The relay's port when it's free. Guests looking for a join code ask this port.
const PORT: u16 = codes::DISCOVERY_PORT;

/// The relay, while anything is shared.
static SERVER: Mutex<Option<Server>> = Mutex::new(None);

/// This app's `iroh` endpoint, while something is shared over the internet.
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
    /// For guests on the network: `ws://<first address>:<port>/v1/<session>?token=<token>`.
    pub invite_url: String,
    /// Six digits a guest joins with: anywhere when `internet`, else on the network (`codes.rs`).
    pub code: String,
    /// For the host's own connection.
    pub local_url: String,
    /// Whether the code is in the directory, for guests anywhere.
    pub internet: bool,
    /// When the code stops working (Unix ms), when `internet`.
    pub expires_at: Option<u64>,
    /// Whether guests from the internet wait for the host to let them in.
    pub require_approval: bool,
    /// Why sharing over the internet didn't start, when it didn't.
    pub internet_error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodeUpdate {
    pub session: String,
    pub code: String,
    pub expires_at: u64,
}

/// `collab-join-request`: someone waits to be let in (`waiting`), or stopped waiting.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct JoinRequestEvent<'a> {
    request: &'a str,
    session: &'a str,
    name: &'a str,
    waiting: bool,
}

/// `collab-code-locked`: too many wrong codes ended a session's code.
#[derive(Serialize, Clone, Debug)]
struct CodeLockedEvent<'a> {
    session: &'a str,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct P2pConnInfo {
    pub conn: u32,
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
    fn join_request(&self, request: &str, session: &str, name: &str, waiting: bool) {
        let event = JoinRequestEvent {
            request,
            session,
            name,
            waiting,
        };
        let _ = self.app.emit("collab-join-request", event);
    }

    fn code_locked(&self, session: &str) {
        let _ = self
            .app
            .emit("collab-code-locked", CodeLockedEvent { session });
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
///
/// macOS 15 and later ask the first time an app does something on the local network that
/// macOS checks, and until the person allows it, connections to other machines there fail with
/// "no route to host". Connecting to the router isn't checked, but sending to the mDNS group is,
/// so sharing and joining start with an empty mDNS query (a header asking nothing, which
/// responders ignore). That way the question comes when the person has just pressed Share or
/// Join, with the reason from the app's Info.plist. macOS says nothing about the answer; a
/// guest's connection failing with "no route to host" is how the page finds out. Other
/// systems have no such permission.
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

/// Shares a document: starts the relay if need be and makes a session for it. The page then
/// connects to `localUrl` and says hello with `hostKey` and the document. `name` and `kind` are
/// for the page's own records; guests see the ones in the host's hello.
///
/// Unless `internet` is false, the session's code goes in the directory too, for guests anywhere,
/// who wait for the host to let them in unless `requireApproval` is false. When that fails
/// (offline, say), the code is for the local network, as `internetError` says.
#[tauri::command]
pub async fn collab_share<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    kind: String,
    internet: Option<bool>,
    require_approval: Option<bool>,
) -> Result<ShareInfo, String> {
    let _ = (name, kind);
    let approval = require_approval.unwrap_or(true);
    let (session, token, host_key, port, bridge) = {
        let mut slot = SERVER.lock().unwrap();
        let server = running(&mut slot).map_err(|e| format!("Couldn't start sharing: {e}"))?;
        let session = random_text(12)?;
        let token = random_text(24)?;
        let host_key = random_text(24)?;
        server.register(&session, &token, &host_key);
        (session, token, host_key, server.port(), server.bridge())
    };
    let online = if internet.unwrap_or(true) {
        Some(share_online(&app, bridge, &session, &token, approval).await)
    } else {
        None
    };
    let (code, expires_at, internet_error) = match online {
        Some(Ok((code, expires))) => (code, Some(expires), None),
        // An internet code is never answered on the local network, where it would let anyone
        // nearby in without the host's approval and after it's been replaced.
        other => {
            let slot = SERVER.lock().unwrap();
            let server = slot.as_ref().ok_or("Sharing stopped before it started.")?;
            (
                server.add_code(&session, &token)?,
                None,
                other.and_then(Result::err),
            )
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
        internet: expires_at.is_some(),
        expires_at,
        require_approval: approval,
        internet_error,
    })
}

/// Starts this app's endpoint if need be and puts `session` in the directory.
async fn share_online<R: Runtime>(
    app: &AppHandle<R>,
    bridge: RelayBridge,
    session: &str,
    token: &str,
    approval: bool,
) -> Result<(String, u64), String> {
    let host = {
        let mut slot = P2P_HOST.lock().await;
        if slot.is_none() {
            let events = Arc::new(AppHostEvents { app: app.clone() });
            *slot = Some(Arc::new(P2pHost::start(bridge, events).await?));
        }
        slot.clone().expect("set above")
    };
    let registered = host
        .register(session, token, approval, directory::directory_url())
        .await;
    if registered.is_err() {
        let mut slot = P2P_HOST.lock().await;
        if slot.as_ref().is_some_and(|h| h.is_empty()) {
            slot.take().expect("checked").shutdown();
        }
    }
    registered
}

async fn p2p_host() -> Result<Arc<P2pHost>, String> {
    P2P_HOST
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Nothing is shared over the internet.".to_owned())
}

/// Gives an internet-shared session a new code for another ten minutes; the old one stops working.
#[tauri::command]
pub async fn collab_rotate_code(session: String) -> Result<CodeUpdate, String> {
    let (code, expires_at) = p2p_host().await?.rotate(&session).await?;
    Ok(CodeUpdate {
        session,
        code,
        expires_at,
    })
}

/// Whether guests from the internet wait for the host to let them in, from now on.
#[tauri::command]
pub async fn collab_set_require_approval(session: String, require_approval: bool) {
    if let Ok(host) = p2p_host().await {
        host.set_approval(&session, require_approval);
    }
}

/// Lets someone in, or doesn't, after `collab-join-request`.
#[tauri::command]
pub async fn collab_approve_join(request: String, allow: bool) -> Result<(), String> {
    if p2p_host().await?.answer(&request, allow) {
        Ok(())
    } else {
        Err("They stopped waiting.".into())
    }
}

/// Finds who is sharing with a six-digit code and returns their invite: a `writan-p2p://` one
/// from the directory, or else a `ws://` link from the local network (`codes.rs`).
///
/// The directory goes first, so a code shared over the internet is never asked for on the local
/// network, where anyone could hear it and answer.
#[tauri::command]
pub async fn collab_find(code: String) -> Result<String, String> {
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    if !codes::is_code(&code) {
        return Err("A code is six digits.".into());
    }
    let asked = code.clone();
    let online = tauri::async_runtime::spawn_blocking(move || find_online(&asked))
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(Some(invite)) = online {
        return Ok(invite);
    }
    let lan = tauri::async_runtime::spawn_blocking(move || codes::find(&code, &lan_addresses()))
        .await
        .map_err(|e| e.to_string())?;
    match (lan, online) {
        (Ok(url), _) => Ok(url),
        // The page waits for macOS's permission on this one (network.ts).
        (Err(e), _) if e.contains("no route to host") => Err(e),
        (Err(_), Err(online)) => Err(format!(
            "Nobody on your network is sharing with that code, and Writan couldn't look it up online: {online}"
        )),
        (Err(e), Ok(None)) => Err(e),
        (Err(_), Ok(Some(_))) => unreachable!("returned above"),
    }
}

fn find_online(code: &str) -> Result<Option<String>, String> {
    let Some(found) = directory::lookup_room(&directory::directory_url(), code)? else {
        return Ok(None);
    };
    let entry: DirectoryEntry = serde_json::from_value(found.endpoint_addr)
        .map_err(|_| "The directory sent an address Writan can't read.".to_owned())?;
    p2p::encode_invite(&entry, code).map(Some)
}

/// The relay's server, started if it isn't running.
fn running(slot: &mut Option<Server>) -> std::io::Result<&Server> {
    if slot.is_none() {
        *slot = Some(Server::start(Policy::Registered, &[PORT, 0])?);
    }
    Ok(slot.as_ref().expect("started above"))
}

/// Stops sharing: everyone in the session is told and disconnected, and its code leaves the
/// directory. When nothing else is shared and agents aren't let in, the relay stops listening,
/// and when nothing is shared over the internet, so does this app's endpoint.
#[tauri::command]
pub async fn collab_unshare(session: String) {
    // The relay first, so guests over the internet get its `end` before the endpoint closes.
    {
        let mut slot = SERVER.lock().unwrap();
        if let Some(server) = slot.as_ref() {
            server.end_session(&session);
            if server.session_count() == 0 && !server.has_mcp() {
                *slot = None;
            }
        }
    }
    let mut slot = P2P_HOST.lock().await;
    if let Some(host) = slot.as_ref() {
        host.unregister(&session);
        if host.is_empty() {
            slot.take().expect("checked").shutdown();
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
    if let Some(server) = SERVER.lock().unwrap().take() {
        server.end_all();
    }
    if let Some(host) = P2P_HOST.lock().await.take() {
        host.shutdown();
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

/// Joins a session found in the directory (`writan-p2p://…` from `collab_find`), as `name`, and
/// returns the connection's id and the session's token for the hello. If the host lets guests in
/// themselves, this waits for them.
#[tauri::command]
pub async fn collab_connect_p2p<R: Runtime>(
    app: AppHandle<R>,
    url: String,
    name: String,
) -> Result<P2pConnInfo, String> {
    let invite = p2p::parse_invite(&url)?;
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let (on_message, on_closed) = (app.clone(), app);
    let (client, token) = p2p::connect(
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
    CLIENTS.lock().unwrap().insert(conn, Conn::P2p(client));
    Ok(P2pConnInfo { conn, token })
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
