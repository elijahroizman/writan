//! Sends anonymous usage events to PostHog's capture API. The page decides what to send and
//! when (src/state/analytics.ts); this only posts the batch, adds the system and processor
//! type, and keeps the app from quitting in the middle of a post. Rust sends it rather than the
//! page so the webview's content security policy stays closed to outside hosts.
//! See docs/internals/analytics.md.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// More than the page ever sends at once (its queue holds 200), so a bad caller can't post
/// an unbounded body.
const MAX_EVENTS: usize = 250;
const MAX_BODY: usize = 512 * 1024;

/// Posts in progress, for `wait_on_exit`.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct InFlight;

impl InFlight {
    fn start() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        InFlight
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// `https://us.i.posthog.com` becomes `https://us.i.posthog.com/batch/`. Only an https origin
/// with no path, query or credentials is accepted, so the page can't post anywhere else
/// through this.
fn batch_url(host: &str) -> Result<String, String> {
    let host = host.trim().trim_end_matches('/');
    let rest = host
        .strip_prefix("https://")
        .ok_or("The analytics host must use https.")?;
    let valid = !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':');
    if !valid {
        return Err("The analytics host must be a plain https origin.".into());
    }
    Ok(format!("{host}/batch/"))
}

fn os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

/// The request body: the batch with this computer's system and processor type added to every
/// event, and PostHog's own `$os` so its dashboards fill in.
fn body(key: &str, mut events: Vec<serde_json::Value>) -> Result<Vec<u8>, String> {
    if events.len() > MAX_EVENTS {
        events.truncate(MAX_EVENTS);
    }
    for event in &mut events {
        let Some(props) = event.get_mut("properties").and_then(|p| p.as_object_mut()) else {
            return Err("An analytics event has no properties.".into());
        };
        props.insert("os".into(), os().into());
        props.insert("$os".into(), os().into());
        props.insert("arch".into(), std::env::consts::ARCH.into());
    }
    let body = serde_json::to_vec(&serde_json::json!({ "api_key": key, "batch": events }))
        .map_err(|e| e.to_string())?;
    if body.len() > MAX_BODY {
        return Err("The analytics batch is too large.".into());
    }
    Ok(body)
}

fn post(url: &str, body: &[u8]) -> Result<(), String> {
    crate::files::http_agent()
        .post(url)
        .header("User-Agent", "Writan")
        .header("Content-Type", "application/json")
        .send(body)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Posts one batch. A failure is retried once after five seconds, then the batch is dropped.
#[tauri::command]
pub async fn analytics_send(
    host: String,
    key: String,
    events: Vec<serde_json::Value>,
) -> Result<(), String> {
    if key.trim().is_empty() || events.is_empty() {
        return Ok(());
    }
    let url = batch_url(&host)?;
    let body = body(&key, events)?;
    let guard = InFlight::start();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        if post(&url, &body).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(5));
        post(&url, &body)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Called as the app exits: gives a post that's already going up to two seconds to finish,
/// so the events sent while quitting aren't cut off. Returns at once when nothing is sending.
pub fn wait_on_exit() {
    let until = Instant::now() + Duration::from_secs(2);
    while IN_FLIGHT.load(Ordering::SeqCst) > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_url_accepts_only_https_origins() {
        assert_eq!(
            batch_url("https://us.i.posthog.com").unwrap(),
            "https://us.i.posthog.com/batch/"
        );
        assert_eq!(
            batch_url("https://eu.i.posthog.com/").unwrap(),
            "https://eu.i.posthog.com/batch/"
        );
        assert_eq!(
            batch_url("https://ph.example.org:8443").unwrap(),
            "https://ph.example.org:8443/batch/"
        );
        assert!(batch_url("http://us.i.posthog.com").is_err());
        assert!(batch_url("https://example.com/upload?x=1").is_err());
        assert!(batch_url("https://user:pw@example.com").is_err());
        assert!(batch_url("https://").is_err());
        assert!(batch_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn body_adds_system_to_every_event() {
        let events = vec![
            serde_json::json!({ "event": "app_launched", "properties": { "distinct_id": "a" } }),
            serde_json::json!({ "event": "app_active", "properties": { "distinct_id": "a" } }),
        ];
        let body: serde_json::Value =
            serde_json::from_slice(&body("phc_test", events).unwrap()).unwrap();
        assert_eq!(body["api_key"], "phc_test");
        for event in body["batch"].as_array().unwrap() {
            assert_eq!(event["properties"]["os"], os());
            assert_eq!(event["properties"]["arch"], std::env::consts::ARCH);
        }
    }

    #[test]
    fn body_refuses_events_without_properties() {
        assert!(body("k", vec![serde_json::json!({ "event": "x" })]).is_err());
    }

    #[test]
    fn body_caps_the_batch() {
        let events = (0..MAX_EVENTS + 10)
            .map(|_| serde_json::json!({ "event": "x", "properties": {} }))
            .collect();
        let body: serde_json::Value = serde_json::from_slice(&body("k", events).unwrap()).unwrap();
        assert_eq!(body["batch"].as_array().unwrap().len(), MAX_EVENTS);
    }
}
