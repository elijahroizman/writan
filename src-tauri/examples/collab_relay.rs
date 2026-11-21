//! A relay on its own, without the app, for browser tests and trying out a server elsewhere:
//!
//! ```sh
//! cargo run --example collab_relay -- --port 47812
//! ```
//!
//! Unlike the app's, it takes any session id: the first hello that brings a snapshot creates
//! the session, and its token and host key become the session's.

use writan_lib::collab::relay::Policy;
use writan_lib::collab::server::Server;

fn main() {
    let mut port = 47812;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or_else(|| exit("--port needs a number"));
            }
            _ => exit(&format!(
                "unknown argument {arg}; usage: collab_relay [--port N]"
            )),
        }
    }
    let server = Server::start(Policy::Open, &[port])
        .unwrap_or_else(|e| exit(&format!("couldn't listen on port {port}: {e}")));
    println!(
        "Relay listening on ws://0.0.0.0:{}/v1/<session>",
        server.port()
    );
    loop {
        std::thread::park();
    }
}

fn exit(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}
