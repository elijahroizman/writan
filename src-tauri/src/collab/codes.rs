//! Six-digit join codes, so a guest types `482 913` instead of a link.
//!
//! The host's relay keeps a code for each shared session and answers lookups for it: over HTTP
//! at `GET /join/<code>` on the relay's port, and over UDP on `DISCOVERY_PORT`, where a guest
//! broadcasts `writan-join 1 <code>`. Either answer is `{"session", "token", "port"}`, and the
//! guest builds the same invite link it would have been sent before (`find`).
//!
//! A code here starts with 0, and the directory's (`directory.rs`) never do, so a guest knows where
//! to look without asking both. That leaves a hundred thousand values, so wrong guesses are
//! limited: `MAX_MISSES_PER_IP` a minute from
//! one address and `MAX_MISSES` from everyone, after which lookups are refused until the minute
//! is up. A guest's search reaches every Writan on the network several times, so the same code
//! from the same address counts once.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Where relays listen for broadcast lookups, and where a guest looks over TCP first.
pub const DISCOVERY_PORT: u16 = 47811;
/// The start of a UDP lookup.
const ASK: &str = "writan-join 1 ";
const WINDOW: Duration = Duration::from_secs(60);
const MAX_MISSES_PER_IP: usize = 20;
const MAX_MISSES: usize = 100;

/// What a lookup answers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Found {
    pub session: String,
    pub token: String,
    pub port: u16,
}

#[derive(Default)]
pub struct Codes {
    /// Code to (session, token).
    by_code: HashMap<String, (String, String)>,
    /// Recent wrong guesses: when, from where, and the code.
    misses: Vec<(Instant, IpAddr, String)>,
}

pub type SharedCodes = Arc<Mutex<Codes>>;

/// Whether `code` is six digits.
pub fn is_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `code` is one for the local network rather than the directory: it starts with 0.
pub fn is_local(code: &str) -> bool {
    is_code(code) && code.starts_with('0')
}

/// A 0 and five random digits.
fn random_code() -> Result<String, String> {
    let mut buf = [0u8; 4];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    // 2^32 isn't a multiple of 100,000, which leans toward low codes by 0.002%.
    Ok(format!("0{:05}", u32::from_le_bytes(buf) % 100_000))
}

impl Codes {
    /// A new code for a session, unlike any other here.
    pub fn add(&mut self, session: &str, token: &str) -> Result<String, String> {
        loop {
            let code = random_code()?;
            if !self.by_code.contains_key(&code) {
                self.by_code
                    .insert(code.clone(), (session.to_owned(), token.to_owned()));
                return Ok(code);
            }
        }
    }

    pub fn remove_session(&mut self, session: &str) {
        self.by_code.retain(|_, (s, _)| s != session);
    }

    pub fn clear(&mut self) {
        self.by_code.clear();
    }

    /// The session behind `code`, asked from `from`. `None` when there's none, or when too many
    /// wrong guesses came lately.
    pub fn lookup(&mut self, code: &str, from: IpAddr, now: Instant) -> Option<(String, String)> {
        self.misses
            .retain(|(at, _, _)| now.duration_since(*at) < WINDOW);
        let from_here = self.misses.iter().filter(|(_, ip, _)| *ip == from);
        // Asking again for a code already missed isn't another guess.
        if from_here.clone().any(|(_, _, c)| c == code) {
            return None;
        }
        if from_here.count() >= MAX_MISSES_PER_IP || self.misses.len() >= MAX_MISSES {
            return None;
        }
        let found = self.by_code.get(code).cloned();
        if found.is_none() {
            self.misses.push((now, from, code.to_owned()));
        }
        found
    }
}

/// Answers `GET /join/<code>`. A request from a web page is refused, so a site can't look codes
/// up: it has an `Origin`, or, for a page that rebinds its own name to this address, a `Host`
/// that's a name rather than an address.
pub fn serve_http(mut stream: TcpStream, codes: &SharedCodes, port: u16) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let request = loop {
        let Ok(n) = stream.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&buf) {
            Ok(httparse::Status::Complete(_)) => {
                let path = req.path.unwrap_or("").to_owned();
                let web = req.headers.iter().any(|h| {
                    h.name.eq_ignore_ascii_case("origin")
                        || (h.name.eq_ignore_ascii_case("host") && !is_address(h.value))
                });
                break (req.method == Some("GET"), path, web);
            }
            Ok(httparse::Status::Partial) if buf.len() < 16 * 1024 => continue,
            _ => return,
        }
    };
    let (get, path, web) = request;
    let code = path.strip_prefix("/join/").unwrap_or("");
    let from = stream
        .peer_addr()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |a| a.ip());
    let found = if get && !web && is_code(code) {
        codes.lock().unwrap().lookup(code, from, Instant::now())
    } else {
        None
    };
    let response = match found {
        Some((session, token)) => {
            let body = serde_json::to_string(&Found {
                session,
                token,
                port,
            })
            .unwrap_or_default();
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        }
        None if web => {
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
        }
        None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
    };
    let _ = stream.write_all(response.as_bytes());
}

/// Whether a `Host` header is an IP address (with or without a port) or localhost.
fn is_address(host: &[u8]) -> bool {
    let Ok(host) = std::str::from_utf8(host) else {
        return false;
    };
    let name = host.rsplit_once(':').map_or(host, |(name, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            name
        } else {
            host
        }
    });
    name == "localhost" || name.parse::<IpAddr>().is_ok()
}

/// Answers broadcast lookups on `socket` until `stop` is set. Only a code that's here gets an
/// answer, so a guest hears from the one computer sharing it.
pub fn serve_udp(socket: UdpSocket, codes: SharedCodes, port: u16, stop: Arc<AtomicBool>) {
    let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
    let mut buf = [0u8; 64];
    while !stop.load(Ordering::SeqCst) {
        let Ok((n, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        let Some(code) = std::str::from_utf8(&buf[..n])
            .ok()
            .and_then(|t| t.strip_prefix(ASK))
            .map(str::trim)
        else {
            continue;
        };
        if !is_code(code) {
            continue;
        }
        let found = codes
            .lock()
            .unwrap()
            .lookup(code, from.ip(), Instant::now());
        if let Some((session, token)) = found {
            let reply = serde_json::to_string(&Found {
                session,
                token,
                port,
            })
            .unwrap_or_default();
            let _ = socket.send_to(reply.as_bytes(), from);
        }
    }
}

/// How long a guest looks before saying nobody has the code.
const SEARCH: Duration = Duration::from_secs(4);
/// How long one TCP connection may take on the local network.
const CONNECT: Duration = Duration::from_millis(600);
/// TCP lookups at once.
const WORKERS: usize = 48;

/// Finds the computer sharing `code` on the networks this one is on, and returns the invite
/// link (`ws://<ip>:<port>/v1/<session>?token=<token>`).
///
/// It broadcasts the code over UDP, and at the same time asks `GET /join/<code>` of every
/// address in each of this computer's /24 networks on `DISCOVERY_PORT`, since some networks
/// drop broadcasts. This computer is asked too. The first answer wins. When nothing answers
/// and the system refused to reach the network (macOS before the person allows Writan on the
/// local network: the broadcast, or every address, fails with "no route to host"), the error
/// says so, so the page can wait for them. A single address that can't be reached says
/// nothing: most of a network's addresses have nobody on them.
pub fn find(code: &str, lan: &[Ipv4Addr]) -> Result<String, String> {
    if !is_code(code) {
        return Err("A code is six digits.".into());
    }
    let (tx, rx) = channel::<(Ipv4Addr, Found)>();
    let done = Arc::new(AtomicBool::new(false));
    let refused = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + SEARCH;

    // UDP: the limited broadcast, each network's own (assuming /24), and this computer.
    {
        let (tx, done, refused) = (tx.clone(), done.clone(), refused.clone());
        let mut targets: Vec<Ipv4Addr> = vec![Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST];
        for ip in lan {
            let [a, b, c, _] = ip.octets();
            targets.push(Ipv4Addr::new(a, b, c, 255));
        }
        let ask = format!("{ASK}{code}");
        std::thread::spawn(move || {
            let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
                return;
            };
            let _ = socket.set_broadcast(true);
            let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
            let mut buf = [0u8; 1024];
            let mut next_send = Instant::now();
            while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
                if Instant::now() >= next_send {
                    for t in &targets {
                        if let Err(e) = socket.send_to(ask.as_bytes(), (*t, DISCOVERY_PORT)) {
                            // 255.255.255.255 can fail the same way for lack of a route,
                            // so only a network's own broadcast address says it's refused.
                            if !t.is_loopback() && !t.is_broadcast() && is_refusal(&e) {
                                refused.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                    next_send = Instant::now() + Duration::from_secs(1);
                }
                if let Ok((n, SocketAddr::V4(from))) = socket.recv_from(&mut buf) {
                    if let Ok(found) = serde_json::from_slice::<Found>(&buf[..n]) {
                        let _ = tx.send((*from.ip(), found));
                        return;
                    }
                }
            }
        });
    }

    // TCP: every address on each /24, this computer first.
    let mut hosts: Vec<Ipv4Addr> = vec![Ipv4Addr::LOCALHOST];
    for ip in lan {
        let [a, b, c, _] = ip.octets();
        for d in 1..=254u8 {
            let h = Ipv4Addr::new(a, b, c, d);
            if h != *ip && !hosts.contains(&h) {
                hosts.push(h);
            }
        }
    }
    let queue = Arc::new(Mutex::new(hosts.into_iter()));
    // Addresses on the network asked, and how many of those the system refused to reach.
    let asked = Arc::new(AtomicUsize::new(0));
    let refusals = Arc::new(AtomicUsize::new(0));
    for _ in 0..WORKERS {
        let (tx, done, queue) = (tx.clone(), done.clone(), queue.clone());
        let (asked, refusals) = (asked.clone(), refusals.clone());
        let code = code.to_owned();
        std::thread::spawn(move || loop {
            if done.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return;
            }
            let Some(host) = queue.lock().unwrap().next() else {
                return;
            };
            match ask_tcp(host, &code) {
                Ok(Some(found)) => {
                    let _ = tx.send((host, found));
                    return;
                }
                Err(e) if !host.is_loopback() && is_refusal(&e) => {
                    refusals.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
            if !host.is_loopback() {
                asked.fetch_add(1, Ordering::SeqCst);
            }
        });
    }
    drop(tx);

    let answer = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    done.store(true, Ordering::SeqCst);
    match answer {
        Ok((ip, found)) => Ok(format!(
            "ws://{ip}:{}/v1/{}?token={}",
            found.port, found.session, found.token
        )),
        Err(_)
            if refused.load(Ordering::SeqCst) || {
                let asked = asked.load(Ordering::SeqCst);
                asked > 0 && refusals.load(Ordering::SeqCst) == asked
            } =>
        {
            Err("Couldn't reach the network: no route to host".into())
        }
        Err(_) => Err(
            "Nobody on your network is sharing with that code. Check it, and that you're both on the same network."
                .into(),
        ),
    }
}

/// macOS's "no route to host", which it gives while Writan isn't allowed on the local network.
fn is_refusal(e: &std::io::Error) -> bool {
    cfg!(target_os = "macos") && e.raw_os_error() == Some(65)
}

/// Asks one address for the code. `Ok(None)` when it answered without it.
fn ask_tcp(host: Ipv4Addr, code: &str) -> std::io::Result<Option<Found>> {
    let mut s = TcpStream::connect_timeout(&(host, DISCOVERY_PORT).into(), CONNECT)?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    s.write_all(
        format!("GET /join/{code} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )?;
    let mut out = Vec::new();
    let mut chunk = [0u8; 1024];
    while out.len() < 8192 {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&out);
    if !text.starts_with("HTTP/1.1 200") {
        return Ok(None);
    }
    Ok(text
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collab_codes_are_six_digits_and_unique() {
        let mut codes = Codes::default();
        let a = codes.add("s1", "t1").unwrap();
        let b = codes.add("s2", "t2").unwrap();
        assert!(is_local(&a) && is_local(&b));
        assert!(!is_local("123456"));
        assert_ne!(a, b);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let now = Instant::now();
        assert_eq!(codes.lookup(&a, ip, now), Some(("s1".into(), "t1".into())));
        codes.remove_session("s1");
        assert_eq!(codes.lookup(&a, ip, now), None);
        assert!(codes.lookup(&b, ip, now).is_some());
        assert!(!is_code("12345") && !is_code("12345a") && !is_code("1234567"));
    }

    #[test]
    fn collab_codes_stop_answering_after_many_wrong_guesses() {
        let mut codes = Codes::default();
        let real = codes.add("s", "t").unwrap();
        let wrong = if real == "000000" { "000001" } else { "000000" };
        let guesser = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 66));
        let guest = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));
        let now = Instant::now();
        // Asking for the same wrong code again, as a guest's search does, is one guess.
        for _ in 0..5 {
            assert_eq!(codes.lookup(wrong, guesser, now), None);
        }
        assert_eq!(codes.misses.len(), 1);
        let others = (500_000..)
            .map(|n| n.to_string())
            .filter(|c| *c != real)
            .take(MAX_MISSES_PER_IP - 1);
        for other in others {
            assert_eq!(codes.lookup(&other, guesser, now), None);
        }
        // The guesser is shut out, even with the right code; others aren't.
        assert_eq!(codes.lookup(&real, guesser, now), None);
        assert!(codes.lookup(&real, guest, now).is_some());
        // A minute later it may try again.
        assert!(codes.lookup(&real, guesser, now + WINDOW).is_some());
        // Many addresses together hit the overall limit.
        let mut codes = Codes::default();
        let real = codes.add("s", "t").unwrap();
        let wrong = if real == "000000" { "000001" } else { "000000" };
        for i in 0..MAX_MISSES {
            let ip = IpAddr::V4(Ipv4Addr::new(10, 1, (i / 250) as u8, (i % 250) as u8));
            codes.lookup(wrong, ip, now);
        }
        assert_eq!(codes.lookup(&real, guest, now), None);
    }
}
