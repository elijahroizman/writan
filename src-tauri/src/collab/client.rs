//! The app's own connection to a relay. The web view's content security policy blocks
//! WebSockets to arbitrary hosts, so the page talks to relays through this.
//!
//! The host's app uses it too, over 127.0.0.1, so sharing and joining run the same code.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::WebSocket;

use super::socket::{pump, set_pump_timeouts, Cmd};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// A welcome holds the document and the whole edit log, so it can be much bigger than one of
/// the frames the relay accepts. This only guards against a relay sending endless data.
const MAX_INCOMING: usize = 1 << 30;

/// A connection that's open but not yet reading, so the caller can file it away before the
/// first message or the close can arrive.
pub struct Pending {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

/// A running connection. Dropping it closes the connection.
pub struct Client {
    queue: Sender<Cmd>,
}

/// Connects to `ws://` or `wss://` and does the WebSocket handshake. A query string (an invite
/// link's token) isn't sent: the token goes in the hello.
pub fn connect(url: &str) -> Result<Pending, String> {
    let url = url.split(['?', '#']).next().unwrap_or_default();
    let request = url.into_client_request().map_err(|e| e.to_string())?;
    let uri = request.uri();
    let tls = match uri.scheme_str() {
        Some("ws") => false,
        Some("wss") => true,
        _ => return Err(format!("{url} isn't a ws:// or wss:// address.")),
    };
    let host = uri
        .host()
        .ok_or_else(|| format!("{url} has no host."))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });

    let mut last = format!("Couldn't find {host}.");
    let stream = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .find_map(
            |addr| match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(s) => Some(s),
                Err(e) => {
                    last = e.to_string();
                    None
                }
            },
        )
        .ok_or(last)?;
    let handle = stream.try_clone().map_err(|e| e.to_string())?;
    handle
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .and_then(|()| handle.set_write_timeout(Some(HANDSHAKE_TIMEOUT)))
        .map_err(|e| e.to_string())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_INCOMING))
        .max_frame_size(Some(MAX_INCOMING));
    let (ws, _) = tungstenite::client_tls_with_config(request, stream, Some(config), None)
        .map_err(|e| e.to_string())?;
    set_pump_timeouts(&handle).map_err(|e| e.to_string())?;
    Ok(Pending { ws })
}

impl Pending {
    /// Starts reading on a thread of its own. `on_text` gets each text frame; `on_closed` gets
    /// why the connection ended, once, after the last `on_text`.
    pub fn run(
        self,
        mut on_text: impl FnMut(&str) + Send + 'static,
        on_closed: impl FnOnce(String) + Send + 'static,
    ) -> Result<Client, String> {
        let (queue, rx): (Sender<Cmd>, Receiver<Cmd>) = channel();
        let mut ws = self.ws;
        std::thread::Builder::new()
            .name("collab-client".into())
            .spawn(move || {
                let reason = pump(&mut ws, &rx, None, &mut on_text);
                on_closed(reason);
            })
            .map_err(|e| e.to_string())?;
        Ok(Client { queue })
    }
}

impl Client {
    /// Queues a text frame. False once the connection has ended.
    pub fn send(&self, text: String) -> bool {
        self.queue.send(Cmd::Text(text)).is_ok()
    }

    /// Sends what's queued, then closes.
    pub fn close(&self) {
        let _ = self.queue.send(Cmd::Close);
    }
}
