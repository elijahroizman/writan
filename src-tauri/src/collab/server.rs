//! The relay on the network: a WebSocket server at `ws://<this machine>:<port>/v1/<session>`.
//!
//! Each connection gets a thread (see `socket.rs`). The relay itself sits behind one lock, and
//! messages are queued to their connections while that lock is held, so every client sees ops
//! in the order the relay numbered them. Nothing here needs Tauri, so the standalone relay in
//! `examples/collab_relay.rs` runs the same code.
//!
//! The app starts a server when the first document is shared and drops it when the last one
//! stops, which closes the port. Connections already open finish on their own.

use std::collections::HashMap;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::protocol::WebSocketConfig;

use super::relay::{ConnId, Outgoing, Policy, Relay, MAX_FRAME};
use super::socket::{pump, set_pump_timeouts, Cmd};

/// A connection must finish its handshake and send its hello within this long.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

struct State {
    relay: Relay,
    queues: HashMap<ConnId, Sender<Cmd>>,
    next: ConnId,
}

impl State {
    /// Queues the relay's answers. Called with the lock held, which keeps ops in order.
    fn dispatch(&mut self, out: Vec<Outgoing>) {
        for o in out {
            match o {
                Outgoing::Send(conn, text) => {
                    if let Some(q) = self.queues.get(&conn) {
                        let _ = q.send(Cmd::Text(text));
                    }
                }
                Outgoing::Close(conn) => {
                    if let Some(q) = self.queues.remove(&conn) {
                        let _ = q.send(Cmd::Close);
                    }
                }
            }
        }
    }
}

type Shared = Arc<Mutex<State>>;

/// A running relay server. Dropping it stops accepting connections.
pub struct Server {
    state: Shared,
    port: u16,
    stop: Arc<AtomicBool>,
    accepting: Option<JoinHandle<()>>,
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
        let accepting = {
            let (state, stop) = (state.clone(), stop.clone());
            std::thread::Builder::new()
                .name("collab-accept".into())
                .spawn(move || accept(listener, state, stop))?
        };
        Ok(Server {
            state,
            port,
            stop,
            accepting: Some(accepting),
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

    /// Ends a session for everyone in it, as the host's `end` would.
    pub fn end_session(&self, session: &str) {
        let mut state = self.state.lock().unwrap();
        let out = state.relay.end_session(session);
        state.dispatch(out);
    }

    /// Ends every session (see `Relay::end_all`).
    pub fn end_all(&self) {
        let mut state = self.state.lock().unwrap();
        let out = state.relay.end_all();
        state.dispatch(out);
    }

    pub fn session_count(&self) -> usize {
        self.state.lock().unwrap().relay.session_count()
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
    }
}

fn accept(listener: TcpListener, state: Shared, stop: Arc<AtomicBool>) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        let _ = std::thread::Builder::new()
            .name("collab-conn".into())
            .spawn(move || serve(stream, state));
    }
}

/// The session id in a `/v1/<session>` path.
fn session_from_path(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/v1/")?;
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

fn serve(stream: TcpStream, state: Shared) {
    let started = Instant::now();
    let Ok(handle) = stream.try_clone() else {
        return;
    };
    if handle.set_read_timeout(Some(HELLO_TIMEOUT)).is_err()
        || handle.set_write_timeout(Some(HELLO_TIMEOUT)).is_err()
    {
        return;
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
    if set_pump_timeouts(&handle).is_err() {
        return;
    }

    let (tx, rx) = channel();
    let conn = {
        let mut s = state.lock().unwrap();
        let conn = s.next;
        s.next += 1;
        s.queues.insert(conn, tx);
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
}
