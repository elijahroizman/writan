//! One WebSocket's life on one thread, for both the relay's connections and the app's own.
//!
//! tungstenite's sockets are blocking and can't be split into a reading half and a writing half,
//! so a thread waits for incoming frames with a short read timeout and, between reads, sends
//! whatever was queued for it. Queuing never blocks, so a slow reader only slows itself down.
//! The cost is that queued text waits at most `POLL` before it goes.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use tungstenite::{Error, Message, WebSocket};

/// How long a read waits before the thread checks its queue.
pub const POLL: Duration = Duration::from_millis(15);

/// A write that makes no progress for this long means the other end is gone.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for the other end to answer a close before hanging up anyway.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// What other threads ask of a connection.
pub enum Cmd {
    Text(String),
    /// Send what's queued so far, then close.
    Close,
}

/// Switches a connected socket from handshake timeouts to the pump's.
pub fn set_pump_timeouts(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(POLL))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_nodelay(true)
}

/// Runs the connection until it ends and says why. `on_text` gets each text frame. When
/// `first_by` is set, a connection that has sent nothing by then is closed.
pub fn pump<S: Read + Write>(
    ws: &mut WebSocket<S>,
    queue: &Receiver<Cmd>,
    first_by: Option<Instant>,
    mut on_text: impl FnMut(&str),
) -> String {
    let mut heard = false;
    let mut closing: Option<Instant> = None;
    let mut reason: Option<String> = None;
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => {
                heard = true;
                if closing.is_none() {
                    on_text(text.as_str());
                }
            }
            // tungstenite answers the close itself; the next read reports the end.
            Ok(Message::Close(frame)) => {
                reason = Some(match frame {
                    Some(f) if !f.reason.is_empty() => f.reason.to_string(),
                    _ => "closed".into(),
                });
            }
            Ok(_) => {}
            Err(Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                return reason.unwrap_or_else(|| "closed".into())
            }
            Err(e) => return e.to_string(),
        }

        if closing.is_none() && !heard && first_by.is_some_and(|by| Instant::now() >= by) {
            reason = Some("said nothing".into());
            let _ = ws.close(None);
            closing = Some(Instant::now());
        }
        while closing.is_none() {
            match queue.try_recv() {
                Ok(Cmd::Text(text)) => {
                    if let Err(e) = ws.write(Message::text(text)) {
                        // Writing after the other end closed: why it closed says more.
                        return reason.unwrap_or_else(|| e.to_string());
                    }
                }
                Ok(Cmd::Close) | Err(TryRecvError::Disconnected) => {
                    let _ = ws.close(None);
                    closing = Some(Instant::now());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        match ws.flush() {
            Ok(()) => {}
            Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                return reason.unwrap_or_else(|| "closed".into())
            }
            // The socket blocks, so a write that would block has already waited WRITE_TIMEOUT.
            Err(e) => return e.to_string(),
        }
        if closing.is_some_and(|at| at.elapsed() > CLOSE_GRACE) {
            return reason.unwrap_or_else(|| "closed".into());
        }
    }
}
