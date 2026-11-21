//! Editing a document together on the local network.
//!
//! Sharing starts a relay (`relay.rs`, run by `server.rs`) in this app. The relay only puts edits
//! in order; every copy of the document, the host's included, applies that order itself. The
//! host's page connects to its own relay over 127.0.0.1 like any guest, through `client.rs`,
//! because the web view isn't allowed to open WebSockets itself.
//!
//! Messages reach the page as `collab-message` events (`{ conn, data }`) and the end of a
//! connection as `collab-closed` (`{ conn, reason }`). A relay says nothing until it has the
//! client's hello, so no message arrives before `collab_connect` has returned the conn id.

pub mod client;
pub mod relay;
pub mod server;
mod socket;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use relay::Policy;
use server::Server;

/// The relay's port when it's free, so invitations from the same machine tend to look alike.
const PORT: u16 = 47811;

/// The relay, while anything is shared.
static SERVER: Mutex<Option<Server>> = Mutex::new(None);

/// The page's open connections, by the id the page knows them by.
static CLIENTS: LazyLock<Mutex<HashMap<u32, client::Client>>> = LazyLock::new(Default::default);
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
    /// For guests: `ws://<first address>:<port>/v1/<session>?token=<token>`.
    pub invite_url: String,
    /// For the host's own connection.
    pub local_url: String,
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

/// Shares a document: starts the relay if need be and makes a session for it. The page then
/// connects to `localUrl` and says hello with `hostKey` and the document. `name` and `kind` are
/// for the page's own records; guests see the ones in the host's hello.
#[tauri::command]
pub fn collab_share(name: String, kind: String) -> Result<ShareInfo, String> {
    let _ = (name, kind);
    let mut slot = SERVER.lock().unwrap();
    if slot.is_none() {
        let server = Server::start(Policy::Registered, &[PORT, 0])
            .map_err(|e| format!("Couldn't start sharing: {e}"))?;
        *slot = Some(server);
    }
    let server = slot.as_ref().expect("started above");
    let session = random_text(12)?;
    let token = random_text(24)?;
    let host_key = random_text(24)?;
    server.register(&session, &token, &host_key);
    let port = server.port();
    let addresses: Vec<String> = lan_addresses().iter().map(Ipv4Addr::to_string).collect();
    let first = addresses.first().map_or("127.0.0.1", String::as_str);
    Ok(ShareInfo {
        invite_url: format!("ws://{first}:{port}/v1/{session}?token={token}"),
        local_url: format!("ws://127.0.0.1:{port}/v1/{session}"),
        session,
        token,
        host_key,
        port,
        addresses,
    })
}

/// Stops sharing: everyone in the session is told and disconnected. When nothing else is
/// shared, the relay stops listening.
#[tauri::command]
pub fn collab_unshare(session: String) {
    let mut slot = SERVER.lock().unwrap();
    if let Some(server) = slot.as_ref() {
        server.end_session(&session);
        if server.session_count() == 0 {
            *slot = None;
        }
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
        clients.insert(conn, client);
        Ok(conn)
    })
    .await
    .map_err(|e| e.to_string())?
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
