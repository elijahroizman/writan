//! HTTP client for Writan's 6-digit code directory Worker (`worker/writan-join`).
//!
//! Only maps ephemeral 6-digit codes to a host's `iroh` `EndpointAddr` + session id.
//! Document bytes, edits, and encryption keys never touch the directory.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default public directory Worker URL. Can be overridden at build time or runtime
/// with `WRITAN_JOIN_URL` (e.g. for self-hosting or local tests).
pub const DEFAULT_URL: &str = "https://writan-join.floral-bird-5f7a.workers.dev";

const USER_AGENT: &str = concat!("Writan/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(8);

pub fn directory_url() -> String {
    if let Ok(url) = std::env::var("WRITAN_JOIN_URL") {
        let trimmed = url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    option_env!("WRITAN_JOIN_URL")
        .map(|s| s.trim().trim_end_matches('/'))
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_URL)
        .to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedRoom {
    pub code: String,
    pub expires_at: u64,
    pub ttl_secs: u64,
    pub host_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatedRoom {
    pub code: String,
    pub expires_at: u64,
    pub ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupRoom {
    pub endpoint_addr: Value,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<String>,
}

/// Native TLS, as everywhere in the app (see `files::http_agent`): ureq's default, Rustls, isn't
/// built in and panics.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .new_agent()
}

fn read_json<T: serde::de::DeserializeOwned>(
    mut resp: ureq::http::Response<ureq::Body>,
) -> Result<T, String> {
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Couldn't read directory response: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("Bad directory response: {e}"))
}

fn read_error(mut resp: ureq::http::Response<ureq::Body>, fallback: &str) -> String {
    resp.body_mut()
        .read_to_string()
        .ok()
        .and_then(|t| serde_json::from_str::<ErrorBody>(&t).ok())
        .and_then(|b| b.error)
        .unwrap_or_else(|| fallback.to_owned())
}

/// Allocates a globally unique 6-digit code pointing to `endpoint_addr` (10-minute TTL).
pub fn create_room(base: &str, endpoint_addr: &Value) -> Result<CreatedRoom, String> {
    let url = format!("{}/rooms", base.trim_end_matches('/'));
    let body = serde_json::to_vec(&serde_json::json!({ "endpointAddr": endpoint_addr }))
        .map_err(|e| e.to_string())?;
    let resp = agent()
        .post(&url)
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| format!("Couldn't reach the sharing directory: {e}"))?;
    let status = resp.status().as_u16();
    if status == 201 || status == 200 {
        read_json::<CreatedRoom>(resp)
    } else {
        Err(read_error(
            resp,
            &format!("Directory refused the share ({status})."),
        ))
    }
}

/// Looks up an active 6-digit code. Returns `Ok(None)` on 404, `Err` on rate-limit or network error.
pub fn lookup_room(base: &str, code: &str) -> Result<Option<LookupRoom>, String> {
    let url = format!("{}/rooms/{code}", base.trim_end_matches('/'));
    let resp = agent()
        .get(&url)
        .call()
        .map_err(|e| format!("Couldn't reach the sharing directory: {e}"))?;
    let status = resp.status().as_u16();
    match status {
        200 => read_json::<LookupRoom>(resp).map(Some),
        404 => Ok(None),
        429 => Err(read_error(
            resp,
            "Too many wrong codes. Wait a minute and try again.",
        )),
        _ => Err(read_error(
            resp,
            &format!("Directory lookup failed ({status})."),
        )),
    }
}

/// Replaces `old_code` with a fresh code for another ten minutes. `None` when the directory has
/// forgotten the room.
pub fn rotate_room(
    base: &str,
    old_code: &str,
    host_secret: &str,
    endpoint_addr: &Value,
) -> Result<Option<RotatedRoom>, String> {
    let url = format!("{}/rooms/{old_code}/rotate", base.trim_end_matches('/'));
    let body = serde_json::to_vec(&serde_json::json!({ "endpointAddr": endpoint_addr }))
        .map_err(|e| e.to_string())?;
    let resp = agent()
        .post(&url)
        .header("Authorization", &format!("Bearer {host_secret}"))
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| format!("Couldn't reach the sharing directory: {e}"))?;
    match resp.status().as_u16() {
        200 => read_json::<RotatedRoom>(resp).map(Some),
        404 => Ok(None),
        status => Err(read_error(
            resp,
            &format!("Couldn't make a new code ({status})."),
        )),
    }
}

/// Removes `code` from the directory (best-effort on unshare).
pub fn delete_room(base: &str, code: &str, host_secret: &str) {
    let url = format!("{}/rooms/{code}", base.trim_end_matches('/'));
    let _ = agent()
        .delete(&url)
        .header("Authorization", &format!("Bearer {host_secret}"))
        .call();
}
