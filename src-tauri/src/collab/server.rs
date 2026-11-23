//! The relay on the network: a WebSocket server at `ws://<this machine>:<port>/v1/<session>`.
//!
//! Each connection gets a thread (see `socket.rs`). The relay itself sits behind one lock, and
//! messages are queued to their connections while that lock is held, so every client sees ops
//! in the order the relay numbered them. Nothing here needs Tauri, so the standalone relay in
//! `examples/collab_relay.rs` runs the same code.
//!
//! The same port serves Writan's MCP server for AI agents (`mcp.rs`): `/mcp/<token>` is plain
//! HTTP, and `/bridge/<key>` is the page's WebSocket that answers agents' tool calls.
//!
//! Join codes are looked up on it too: `GET /join/<code>`, and UDP broadcasts on
//! `codes::DISCOVERY_PORT` (see `codes.rs`).
//!
//! The app starts a server when the first document is shared, or agents are let in, and drops
//! it when neither is true any more, which closes the port. Connections already open finish on
//! their own.

use std::collections::HashMap;
use std::io;
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::protocol::WebSocketConfig;

use super::codes::{self, SharedCodes};
use super::mcp::{self, Mcp, SharedMcp};
use super::relay::{ConnId, Outgoing, Policy, Relay, MAX_FRAME};
use super::socket::{pump, set_pump_timeouts, Cmd};

/// A connection must finish its handshake and send its hello within this long.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

enum Outbox {
    Sync(Sender<Cmd>),
    Async(tokio::sync::mpsc::UnboundedSender<Cmd>),
}

impl Outbox {
    fn send(&self, cmd: Cmd) {
        match self {
            Outbox::Sync(tx) => {
                let _ = tx.send(cmd);
            }
            Outbox::Async(tx) => {
                let _ = tx.send(cmd);
            }
        }
    }
}

struct State {
    relay: Relay,
    queues: HashMap<ConnId, Outbox>,
    next: ConnId,
}

impl State {
    /// Queues the relay's answers. Called with the lock held, which keeps ops in order.
    fn dispatch(&mut self, out: Vec<Outgoing>) {
        for o in out {
            match o {
                Outgoing::Send(conn, text) => {
                    if let Some(q) = self.queues.get(&conn) {
                        q.send(Cmd::Text(text));
                    }
                }
                Outgoing::Close(conn) => {
                    if let Some(q) = self.queues.remove(&conn) {
                        q.send(Cmd::Close);
                    }
                }
            }
        }
    }
}

type Shared = Arc<Mutex<State>>;

/// Lets connections that aren't WebSockets (`p2p.rs`) into the relay.
#[derive(Clone)]
pub struct RelayBridge {
    state: Shared,
}

impl RelayBridge {
    pub fn connect_async(
        &self,
        session: &str,
    ) -> (ConnId, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut s = self.state.lock().unwrap();
        let conn = s.next;
        s.next += 1;
        s.queues.insert(conn, Outbox::Async(tx));
        s.relay.connect(conn, session);
        (conn, rx)
    }

    pub fn handle(&self, conn: ConnId, text: &str) {
        let mut s = self.state.lock().unwrap();
        let out = s.relay.handle(conn, text);
        s.dispatch(out);
    }

    pub fn disconnect(&self, conn: ConnId) {
        let mut s = self.state.lock().unwrap();
        s.queues.remove(&conn);
        let out = s.relay.disconnect(conn);
        s.dispatch(out);
    }
}

/// A running relay server. Dropping it stops accepting connections.
pub struct Server {
    state: Shared,
    mcp: SharedMcp,
    codes: SharedCodes,
    port: u16,
    stop: Arc<AtomicBool>,
    accepting: Option<JoinHandle<()>>,
    discovering: Option<JoinHandle<()>>,
}

impl Server {
    /// Listens on every interface, on the first of `ports` that's free (0 lets the system pick).
    pub fn start(policy: Policy, ports: &[u16]) -> io::Result<Server> {
        let mut last = io::Error::new(io::ErrorKind::InvalidInput, "no port to try");
        let listener = ports
            .iter()
            .find_map(|&port| match TcpListener::bind(("0.0.0.0", port)) {
                Ok(l) => Some(l),
                Err(e) => {
                    last = e;
                    None
                }
            })
            .ok_or(last)?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(Mutex::new(State {
            relay: Relay::new(policy),
            queues: HashMap::new(),
            next: 1,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let mcp: SharedMcp = Arc::new(Mutex::new(None));
        let codes: SharedCodes = Arc::default();
        let accepting = {
            let (state, mcp, codes, stop) =
                (state.clone(), mcp.clone(), codes.clone(), stop.clone());
            std::thread::Builder::new()
                .name("collab-accept".into())
                .spawn(move || accept(listener, state, mcp, codes, port, stop))?
        };
        // Without the UDP port (something else has it), codes are still found over TCP.
        let discovering = UdpSocket::bind(("0.0.0.0", codes::DISCOVERY_PORT))
            .ok()
            .and_then(|socket| {
                let (codes, stop) = (codes.clone(), stop.clone());
                std::thread::Builder::new()
                    .name("collab-discovery".into())
                    .spawn(move || codes::serve_udp(socket, codes, port, stop))
                    .ok()
            });
        Ok(Server {
            state,
            mcp,
            codes,
            port,
            stop,
            accepting: Some(accepting),
            discovering,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// See `Relay::register`.
    pub fn register(&self, session: &str, token: &str, host_key: &str) {
        self.state
            .lock()
            .unwrap()
            .relay
            .register(session, token, host_key);
    }

    /// A six-digit code guests can join `session` with, until it ends.
    pub fn add_code(&self, session: &str, token: &str) -> Result<String, String> {
        self.codes.lock().unwrap().add(session, token)
    }

    /// For connections that aren't WebSockets (`p2p.rs`).
    pub fn bridge(&self) -> RelayBridge {
        RelayBridge {
            state: self.state.clone(),
        }
    }

    /// Ends a session for everyone in it, as the host's `end` would.
    pub fn end_session(&self, session: &str) {
        self.codes.lock().unwrap().remove_session(session);
        let mut state = self.state.lock().unwrap();
        let out = state.relay.end_session(session);
        state.dispatch(out);
    }

    /// Ends every session (see `Relay::end_all`).
    pub fn end_all(&self) {
        self.codes.lock().unwrap().clear();
        let mut state = self.state.lock().unwrap();
        let out = state.relay.end_all();
        state.dispatch(out);
    }

    pub fn session_count(&self) -> usize {
        self.state.lock().unwrap().relay.session_count()
    }

    /// Lets agents in at `/mcp/<token>`, with the page answering at `/bridge/<bridge_key>`, or
    /// (`None`) stops. Agents' sessions start over either way.
    pub fn set_mcp(&self, token_and_key: Option<(&str, &str)>) {
        *self.mcp.lock().unwrap() = token_and_key.map(|(token, key)| Mcp::new(token, key));
    }

    pub fn has_mcp(&self) -> bool {
        self.mcp.lock().unwrap().is_some()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // `accept` blocks until someone connects, so connect to wake it.
        let _ =
            TcpStream::connect_timeout(&([127, 0, 0, 1], self.port).into(), Duration::from_secs(1));
        if let Some(t) = self.accepting.take() {
            let _ = t.join();
        }
        if let Some(t) = self.discovering.take() {
            let _ = t.join();
        }
    }
}

fn accept(
    listener: TcpListener,
    state: Shared,
    mcp: SharedMcp,
    codes: SharedCodes,
    port: u16,
    stop: Arc<AtomicBool>,
) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let (state, mcp, codes) = (state.clone(), mcp.clone(), codes.clone());
        let _ = std::thread::Builder::new()
            .name("collab-conn".into())
            .spawn(move || serve(stream, state, mcp, codes, port));
    }
}

/// The path a request is for, read without taking it off the socket, so the WebSocket
/// handshake can still read the whole request.
fn peek_path(stream: &TcpStream, by: Instant) -> Option<String> {
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.peek(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        if let Some(end) = buf[..n].iter().position(|&b| b == b'\n') {
            let line = std::str::from_utf8(&buf[..end]).ok()?;
            let target = line.split(' ').nth(1)?;
            return Some(target.split('?').next().unwrap_or(target).to_owned());
        }
        if n == buf.len() || Instant::now() >= by {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The page's end of the MCP server: agents' tool calls go out on it, and answers come back.
/// Only this computer can open it.
fn bridge(stream: TcpStream, mcp: SharedMcp, path: &str) {
    let local = stream.peer_addr().is_ok_and(|a| a.ip().is_loopback());
    if !local || !mcp::bridge_allowed(&mcp, path) {
        let mut s = &stream;
        let _ = std::io::Write::write_all(
            &mut s,
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let Ok(mut ws) = tungstenite::accept_with_config(stream, Some(config)) else {
        return;
    };
    if set_pump_timeouts(ws.get_ref()).is_err() {
        return;
    }
    let (tx, rx) = channel();
    let id = mcp::bridge_opened(&mcp, tx);
    pump(&mut ws, &rx, None, |text| mcp::bridge_text(&mcp, text));
    mcp::bridge_closed(&mcp, id);
}

/// The session id in a `/v1/<session>` path.
fn session_from_path(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/v1/")?;
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

fn serve(stream: TcpStream, state: Shared, mcp: SharedMcp, codes: SharedCodes, port: u16) {
    let started = Instant::now();
    // Timeouts go on the socket that reads, never a clone: on Windows a clone's don't reach it,
    // and a read that never times out keeps the pump from sending anything.
    if stream.set_read_timeout(Some(HELLO_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(HELLO_TIMEOUT)).is_err()
    {
        return;
    }
    match peek_path(&stream, started + HELLO_TIMEOUT) {
        Some(p) if p.starts_with("/mcp/") => return mcp::serve_http(stream, &mcp),
        Some(p) if p.starts_with("/bridge/") => return bridge(stream, mcp, &p),
        Some(p) if p.starts_with("/join/") => return codes::serve_http(stream, &codes, port),
        _ => {}
    }
    let mut session = String::new();
    // tungstenite's callback type; its error is an HTTP response, which is big.
    #[allow(clippy::result_large_err)]
    let check_path = |req: &Request, res: Response| -> Result<Response, ErrorResponse> {
        // The token comes in the hello, so a query string (from an invite link) is ignored.
        match session_from_path(req.uri().path()) {
            Some(id) => {
                session = id.to_owned();
                Ok(res)
            }
            None => {
                let mut err = ErrorResponse::new(Some("Not a Writan session.".into()));
                *err.status_mut() = StatusCode::NOT_FOUND;
                Err(err)
            }
        }
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let Ok(mut ws) = tungstenite::accept_hdr_with_config(stream, check_path, Some(config)) else {
        return;
    };
    if set_pump_timeouts(ws.get_ref()).is_err() {
        return;
    }

    let (tx, rx) = channel();
    let conn = {
        let mut s = state.lock().unwrap();
        let conn = s.next;
        s.next += 1;
        s.queues.insert(conn, Outbox::Sync(tx));
        s.relay.connect(conn, &session);
        conn
    };
    pump(&mut ws, &rx, Some(started + HELLO_TIMEOUT), |text| {
        let mut s = state.lock().unwrap();
        let out = s.relay.handle(conn, text);
        s.dispatch(out);
    });
    let mut s = state.lock().unwrap();
    s.queues.remove(&conn);
    let out = s.relay.disconnect(conn);
    s.dispatch(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tungstenite::{Message, WebSocket};

    type Ws = WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

    fn open(port: u16, session: &str) -> Ws {
        let url = format!("ws://127.0.0.1:{port}/v1/{session}?token=ignored");
        let (ws, _) = tungstenite::connect(url).unwrap();
        if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        }
        ws
    }

    fn send(ws: &mut Ws, v: Value) {
        ws.send(Message::text(v.to_string())).unwrap();
    }

    fn recv(ws: &mut Ws) -> Value {
        loop {
            match ws.read().unwrap() {
                Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
                _ => continue,
            }
        }
    }

    #[test]
    fn collab_two_clients_share_a_document_over_sockets() {
        let server = Server::start(Policy::Registered, &[0]).unwrap();
        server.register("doc", "tok", "hk");
        let port = server.port();

        let mut host = open(port, "doc");
        send(
            &mut host,
            json!({ "t": "hello", "v": 1, "token": "tok", "client": "h", "name": "Host",
                    "color": "#f00", "hostKey": "hk",
                    "snapshot": { "name": "Deck.pptx", "kind": "pptx", "bytes": "UEsDBA==" } }),
        );
        assert_eq!(recv(&mut host)["t"], "welcome");
        send(
            &mut host,
            json!({ "t": "op", "id": "h1", "base": 0, "steps": ["a"] }),
        );
        assert_eq!(recv(&mut host)["seq"], 1);

        let mut guest = open(port, "doc");
        send(
            &mut guest,
            json!({ "t": "hello", "v": 1, "token": "tok", "client": "g", "name": "Guest", "color": "#0f0" }),
        );
        let welcome = recv(&mut guest);
        assert_eq!(welcome["snapshot"], "UEsDBA==");
        assert_eq!(welcome["session"]["kind"], "pptx");
        assert_eq!(welcome["ops"][0]["steps"], json!(["a"]));
        assert_eq!(recv(&mut host)["t"], "join");

        // Both send at once; both see the same order.
        for i in 0..20 {
            send(
                &mut host,
                json!({ "t": "op", "id": format!("h{i}"), "base": 1, "steps": [i] }),
            );
            send(
                &mut guest,
                json!({ "t": "op", "id": format!("g{i}"), "base": 1, "steps": [i] }),
            );
        }
        let order = |ws: &mut Ws| -> Vec<(u64, String)> {
            (0..40)
                .map(|_| {
                    let m = recv(ws);
                    assert_eq!(m["t"], "op");
                    (
                        m["seq"].as_u64().unwrap(),
                        m["id"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };
        let seen_by_host = order(&mut host);
        let seen_by_guest = order(&mut guest);
        assert_eq!(seen_by_host, seen_by_guest);
        let seqs: Vec<u64> = seen_by_host.iter().map(|(s, _)| *s).collect();
        assert_eq!(seqs, (2..42).collect::<Vec<_>>());

        // Unsharing tells everyone and closes them.
        server.end_session("doc");
        assert_eq!(
            recv(&mut guest),
            json!({ "t": "end", "reason": "host-stopped" })
        );
        assert_eq!(recv(&mut host)["t"], "end");
        assert_eq!(server.session_count(), 0);
    }

    #[test]
    fn collab_server_turns_away_other_paths() {
        let server = Server::start(Policy::Registered, &[0]).unwrap();
        let url = format!("ws://127.0.0.1:{}/elsewhere", server.port());
        assert!(tungstenite::connect(url).is_err());
        assert_eq!(session_from_path("/v1/abc"), Some("abc"));
        assert_eq!(session_from_path("/v1/"), None);
        assert_eq!(session_from_path("/v1/a/b"), None);
    }

    /// One HTTP request; returns the status line, headers and body.
    fn http(
        port: u16,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> (String, String, String) {
        use std::io::{Read, Write};
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nContent-Length: {}\r\n",
            body.len()
        );
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
        }
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(body);
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let (head, body) = out.split_once("\r\n\r\n").unwrap();
        let (status, headers) = head.split_once("\r\n").unwrap_or((head, ""));
        (status.to_owned(), headers.to_owned(), body.to_owned())
    }

    #[test]
    fn collab_join_codes_are_looked_up_over_http() {
        let server = Server::start(Policy::Registered, &[0]).unwrap();
        let port = server.port();
        server.register("doc", "tok", "hk");
        let code = server.add_code("doc", "tok").unwrap();
        let (status, _, body) = http(port, "GET", &format!("/join/{code}"), &[], "");
        assert!(status.contains("200"), "{status}");
        let found: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            found,
            json!({ "session": "doc", "token": "tok", "port": port })
        );
        // A web page can't look codes up, and a wrong code finds nothing.
        let (status, _, _) = http(
            port,
            "GET",
            &format!("/join/{code}"),
            &[("Origin", "http://evil.example")],
            "",
        );
        assert!(status.contains("403"), "{status}");
        // Nor can one that rebinds its own name to this computer.
        let (status, _, _) = http(
            port,
            "GET",
            &format!("/join/{code}"),
            &[("Host", "evil.example")],
            "",
        );
        assert!(status.contains("403"), "{status}");
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert!(http(port, "GET", &format!("/join/{wrong}"), &[], "")
            .0
            .contains("404"));
        // Once the session ends, its code does too.
        server.end_session("doc");
        assert!(http(port, "GET", &format!("/join/{code}"), &[], "")
            .0
            .contains("404"));
    }

    /// The guest's search finds a relay on this computer. Needs the usual port free.
    #[test]
    fn collab_a_guest_finds_the_relay_by_its_code() {
        let Ok(server) = Server::start(Policy::Registered, &[codes::DISCOVERY_PORT]) else {
            eprintln!("port {} is taken; skipped", codes::DISCOVERY_PORT);
            return;
        };
        server.register("doc", "tok", "hk");
        let code = server.add_code("doc", "tok").unwrap();
        let invite = codes::find(&code, &[]).unwrap();
        // Whichever answers first: localhost over TCP, or the broadcast from this computer's
        // own network address.
        let address = invite
            .strip_prefix("ws://")
            .and_then(|rest| {
                rest.strip_suffix(&format!(":{}/v1/doc?token=tok", codes::DISCOVERY_PORT))
            })
            .unwrap_or_else(|| panic!("{invite}"));
        assert!(address.parse::<std::net::Ipv4Addr>().is_ok(), "{invite}");
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert!(codes::find(wrong, &[]).unwrap_err().contains("Nobody"));
    }

    #[test]
    fn collab_mcp_calls_go_to_the_page_and_back() {
        let server = Server::start(Policy::Registered, &[0]).unwrap();
        let port = server.port();
        // Off until it's turned on.
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"codex-mcp-client"}}}"#;
        assert!(http(port, "POST", "/mcp/tok", &[], init).0.contains("404"));
        server.set_mcp(Some(("tok", "key")));
        assert!(http(port, "POST", "/mcp/nope", &[], init).0.contains("404"));
        // A web page can't drive it.
        assert!(http(
            port,
            "POST",
            "/mcp/tok",
            &[("Origin", "http://evil.example")],
            init
        )
        .0
        .contains("403"));

        let (status, headers, body) = http(
            port,
            "POST",
            "/mcp/tok?agent=codex",
            &[("Content-Type", "application/json")],
            init,
        );
        assert!(status.contains("200"), "{status}");
        let reply: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(reply["result"]["serverInfo"]["name"], "writan");
        let session = headers
            .lines()
            .find_map(|l| l.strip_prefix("Mcp-Session-Id: "))
            .unwrap()
            .to_owned();

        // Without the page, a tool call says so.
        let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let (_, _, body) = http(
            port,
            "POST",
            "/mcp/tok",
            &[("Mcp-Session-Id", &session)],
            list,
        );
        assert!(body.contains("window isn't open"), "{body}");

        // The page's bridge: the wrong key is refused, the right one gets the call.
        assert!(tungstenite::connect(format!("ws://127.0.0.1:{port}/bridge/nope")).is_err());
        let (mut page, _) =
            tungstenite::connect(format!("ws://127.0.0.1:{port}/bridge/key")).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let caller = std::thread::spawn(move || {
            http(
                port,
                "POST",
                "/mcp/tok",
                &[("Mcp-Session-Id", &session)],
                list,
            )
        });
        let call: Value = loop {
            if let Message::Text(t) = page.read().unwrap() {
                break serde_json::from_str(t.as_str()).unwrap();
            }
        };
        assert_eq!(call["t"], "call");
        assert_eq!(call["method"], "tools/list");
        assert_eq!(call["agent"], "codex");
        assert_eq!(call["client"], "codex-mcp-client");
        page.send(Message::text(
            json!({ "t": "result", "id": call["id"], "result": { "tools": [] } }).to_string(),
        ))
        .unwrap();
        let (status, _, body) = caller.join().unwrap();
        assert!(status.contains("200"));
        let reply: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            reply,
            json!({ "jsonrpc": "2.0", "id": 2, "result": { "tools": [] } })
        );

        // A notification gets no answer; an unknown session starts over.
        let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(http(
            port,
            "POST",
            "/mcp/tok",
            &[("Mcp-Session-Id", call["session"].as_str().unwrap())],
            note
        )
        .0
        .contains("202"));
        assert!(http(
            port,
            "POST",
            "/mcp/tok",
            &[("Mcp-Session-Id", "gone")],
            list
        )
        .0
        .contains("404"));
        // The newer handshake isn't spoken; clients fall back to initialize.
        let discover = r#"{"jsonrpc":"2.0","id":3,"method":"server/discover"}"#;
        let (_, _, body) = http(port, "POST", "/mcp/tok", &[], discover);
        assert!(body.contains("-32601"), "{body}");
        // The relay still works on the same port.
        let mut ws = open(port, "nothing");
        send(
            &mut ws,
            json!({ "t": "hello", "v": 1, "token": "x", "client": "c" }),
        );
    }
}
