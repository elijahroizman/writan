//! Optional plugins that don't ship inside the app (LaTeX) are downloaded from the
//! Writan GitHub release when you turn them on. Each archive is
//! signed with the same key as app updates; nothing is unpacked unless the signature checks
//! out. Installed plugins live in `<app data>/plugins/<id>/` and the web view loads them
//! through the `wsplugin:` scheme served here.

use std::borrow::Cow;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Only archives attached to Writan's own releases.
const DOWNLOAD_PREFIX: &str = "https://github.com/elijahroizman/writan/releases/download/";
const MAX_ARCHIVE: u64 = 100 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    id: &'a str,
    received: u64,
    total: u64,
}

fn plugins_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("plugins"))
        .map_err(|e| e.to_string())
}

/// A plugin id is a folder name: lowercase letters, digits and dashes.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn read_manifest(dir: &Path) -> Option<InstalledPlugin> {
    serde_json::from_slice(&fs::read(dir.join("manifest.json")).ok()?).ok()
}

#[tauri::command]
pub fn installed_plugins<R: Runtime>(app: AppHandle<R>) -> Vec<InstalledPlugin> {
    let Ok(dir) = plugins_dir(&app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let manifest = read_manifest(&entry.path())?;
            (valid_id(&name) && manifest.id == name).then_some(manifest)
        })
        .collect()
}

#[tauri::command]
pub async fn install_plugin<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    version: String,
    url: String,
) -> Result<(), String> {
    if !valid_id(&id) {
        return Err("Unknown plugin".into());
    }
    if !url.starts_with(DOWNLOAD_PREFIX) || url.contains("..") {
        return Err("Plugins only download from Writan's releases.".into());
    }
    let pubkey = updater_pubkey(&app)?;
    let root = plugins_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let signature = download(&format!("{url}.sig"), 64 * 1024, |_, _| {})?;
        let archive = download(&url, MAX_ARCHIVE, |received, total| {
            let _ = app.emit(
                "plugin-progress",
                Progress {
                    id: &id,
                    received,
                    total,
                },
            );
        })?;
        let file_name = url.rsplit('/').next().unwrap_or_default();
        verify(&archive, &signature, &pubkey, file_name)?;
        unpack(&archive, &root, &id, &version)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn remove_plugin<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    if !valid_id(&id) {
        return Err("Unknown plugin".into());
    }
    let dir = plugins_dir(&app)?.join(&id);
    match fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("Couldn't remove the plugin: {e}"))
        }
        _ => Ok(()),
    }
}

/// The updater's public key from tauri.conf.json: base64 of a minisign public key file.
pub(crate) fn updater_pubkey<R: Runtime>(app: &AppHandle<R>) -> Result<String, String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|key| key.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "This build can't check plugin signatures.".to_string())
}

fn download(url: &str, limit: u64, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    let failed =
        |_| "Couldn't download the plugin. Check your connection and try again.".to_string();
    let mut response = crate::files::download_agent()
        .get(url)
        .header("User-Agent", "Writan")
        .call()
        .map_err(failed)?;
    let total = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if total > limit {
        return Err("The plugin download is too large.".into());
    }
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let mut bytes = Vec::with_capacity(total as usize);
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|_| "The plugin download was interrupted.".to_string())?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        progress(bytes.len() as u64, total);
    }
    Ok(bytes)
}

fn base64_text(value: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Why a signed download didn't pass [`check_signature`].
pub(crate) enum SignatureError {
    /// The signature file or the key can't be read.
    Damaged,
    /// The signature doesn't match the file.
    Mismatch,
    /// The signature is good, but its trusted comment names another file.
    OtherFile,
}

/// Check a download against the release signing key, as the updater does, and that the
/// signature was made for this file (its trusted comment names it), so an older signed
/// file can't stand in for the one asked for. Plugin archives and the Linux nightly packages
/// are signed this way (`tauri signer sign`).
pub(crate) fn check_signature(
    bytes: &[u8],
    signature: &[u8],
    pubkey: &str,
    file_name: &str,
) -> Result<(), SignatureError> {
    let key = base64_text(pubkey).ok_or(SignatureError::Damaged)?;
    let key = minisign_verify::PublicKey::decode(&key).map_err(|_| SignatureError::Damaged)?;
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(base64_text)
        .ok_or(SignatureError::Damaged)?;
    let signature =
        minisign_verify::Signature::decode(&signature).map_err(|_| SignatureError::Mismatch)?;
    key.verify(bytes, &signature, true)
        .map_err(|_| SignatureError::Mismatch)?;
    let signed_for = signature
        .trusted_comment()
        .split('\t')
        .find_map(|part| part.strip_prefix("file:"))
        .unwrap_or_default();
    if signed_for != file_name {
        return Err(SignatureError::OtherFile);
    }
    Ok(())
}

fn verify(archive: &[u8], signature: &[u8], pubkey: &str, file_name: &str) -> Result<(), String> {
    check_signature(archive, signature, pubkey, file_name).map_err(|e| {
        match e {
            SignatureError::Damaged => "The plugin signature is damaged.",
            SignatureError::Mismatch => {
                "The plugin's signature doesn't match. It wasn't installed."
            }
            SignatureError::OtherFile => {
                "The plugin's signature is for a different file. It wasn't installed."
            }
        }
        .to_string()
    })
}

/// Unpack into a fresh folder beside the old one, then swap it in, so a failed install
/// leaves the previous version working.
fn unpack(archive: &[u8], root: &Path, id: &str, version: &str) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("Couldn't install the plugin: {e}");
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).map_err(|e| fail(&e))?;
    let staging = root.join(format!(".{id}-installing"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| fail(&e))?;
    let result = (|| {
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| fail(&e))?;
            let Some(rel) = entry.enclosed_name() else {
                return Err(fail(&"unsafe path in archive"));
            };
            let out = staging.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&out).map_err(|e| fail(&e))?;
                continue;
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| fail(&e))?;
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes).map_err(|e| fail(&e))?;
            fs::write(&out, bytes).map_err(|e| fail(&e))?;
        }
        match read_manifest(&staging) {
            Some(m) if m.id == id && m.version == version => Ok(()),
            _ => Err(fail(&"the archive is for a different plugin or version")),
        }
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    // Two renames, so the plugin's folder is missing only for an instant (a tab loading it then
    // waits for the install; see loadModule in PluginEditor.tsx).
    let dest = root.join(id);
    let old = root.join(format!(".{id}-old"));
    let _ = fs::remove_dir_all(&old);
    let had_old = fs::rename(&dest, &old).is_ok();
    if let Err(e) = fs::rename(&staging, &dest) {
        if had_old {
            let _ = fs::rename(&old, &dest);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(fail(&e));
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

/// `wsplugin://localhost/<id>/<file>` (`http://wsplugin.localhost/…` on Windows): files of an
/// installed plugin. The page loads them as modules, fonts and styles from another origin, so
/// every response allows any origin.
pub fn serve<R: Runtime>(
    app: &AppHandle<R>,
    request: &Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    let respond = |status: StatusCode, mime: &str, body: Vec<u8>| {
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, mime)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(Cow::Owned(body))
            .unwrap()
    };
    let not_found = || respond(StatusCode::NOT_FOUND, "text/plain", b"Not found".to_vec());
    let Ok(root) = plugins_dir(app) else {
        return not_found();
    };
    let Some(rel) = safe_relative(request.uri().path()) else {
        return not_found();
    };
    match fs::read(root.join(&rel)) {
        Ok(bytes) => respond(StatusCode::OK, mime_type(&rel), bytes),
        Err(_) => not_found(),
    }
}

/// The request path as a relative path made only of plain names: no `..`, no root, no drive.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let decoded = crate::files::percent_decode(path.trim_start_matches('/')).ok()?;
    let rel = PathBuf::from(decoded);
    let plain = rel.components().all(|c| matches!(c, Component::Normal(_)));
    let first = rel.components().next()?.as_os_str().to_str()?.to_owned();
    (plain && valid_id(&first)).then_some(rel)
}

fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "html" => "text/html",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_only_plain_paths_inside_a_plugin() {
        assert_eq!(
            safe_relative("/latex/index.js"),
            Some(PathBuf::from("latex/index.js"))
        );
        assert_eq!(
            safe_relative("/latex/fonts/A%20B.woff2"),
            Some(PathBuf::from("latex/fonts/A B.woff2"))
        );
        assert_eq!(safe_relative("/latex/../secret"), None);
        assert_eq!(safe_relative("/%2E%2E/secret"), None);
        assert_eq!(safe_relative("/.latex-installing/index.js"), None);
        assert_eq!(safe_relative("/"), None);
    }

    // Made with `tauri signer generate` / `tauri signer sign` on a throwaway key, like the release workflow does.
    const TEST_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDA3MDlCRUYwQ0MwQkY1QTYKUldTbTlRdk04TDRKQjFMVWZjMExpSllvU0xQVnpKWHVFNi9WNE1HVm1iSnlNN1VhWURvRGNnQUEK";
    const TEST_SIGNATURE: &[u8] = b"dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTbTlRdk04TDRKQnhEaStoYnF1YW5IQmc2Q2xEdnA0Y1BNdUFpVnp6SkpWVVBWaG1FRUt6LzJXam8wT0Rha2E3VitpM2JzckV0UldMamhpbGVoZHBiSDJkN1lvWUwwQVFFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMjkyMzk4CWZpbGU6ZGVtby0xLjAuMC56aXAKOFRjL1VDZVF1V0lrd1VCaTE2ck43aFRnd1hRUjZzS1o3VmN1L1Z6dUpGNHlVZjN5QVFjUzh4c0R3T0R4ZkNuVHRFM2dMOUw1VU1YbnpNTVVhUVA4Q0E9PQo=";

    #[test]
    fn accepts_only_a_signature_for_this_file() {
        verify(
            b"plugin bytes",
            TEST_SIGNATURE,
            TEST_PUBKEY,
            "demo-1.0.0.zip",
        )
        .unwrap();
        assert!(verify(
            b"plugin bytes!",
            TEST_SIGNATURE,
            TEST_PUBKEY,
            "demo-1.0.0.zip"
        )
        .is_err());
        // A genuine signature for another archive can't stand in for this one.
        assert!(verify(
            b"plugin bytes",
            TEST_SIGNATURE,
            TEST_PUBKEY,
            "demo-2.0.0.zip"
        )
        .is_err());
    }

    #[test]
    fn offline_plugin_download_has_a_useful_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let err = download(&format!("http://{addr}/plugin.zip"), 1024, |_, _| {}).unwrap_err();
        assert!(err.contains("Check your connection"), "{err}");
    }

    #[test]
    fn plugin_ids_are_folder_names() {
        assert!(valid_id("latex"));
        assert!(!valid_id("../x"));
        assert!(!valid_id("Latex"));
        assert!(!valid_id(""));
    }

    #[test]
    fn unpacks_only_a_matching_manifest() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("workspace-plugins-{}", std::process::id()));
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("manifest.json", opts).unwrap();
            zip.write_all(br#"{"id":"demo","version":"1.0.0","entry":"index.js"}"#)
                .unwrap();
            zip.start_file("fonts/a.woff2", opts).unwrap();
            zip.write_all(b"font").unwrap();
            zip.finish().unwrap();
        }
        let archive = buf.into_inner();
        assert!(unpack(&archive, &root, "demo", "2.0.0").is_err());
        assert!(!root.join("demo").exists());
        unpack(&archive, &root, "demo", "1.0.0").unwrap();
        assert_eq!(fs::read(root.join("demo/fonts/a.woff2")).unwrap(), b"font");
        // A bad upgrade must leave the installed version available, with no staging scraps.
        assert!(unpack(&archive, &root, "demo", "2.0.0").is_err());
        assert!(unpack(b"broken zip", &root, "demo", "2.0.0").is_err());
        assert_eq!(read_manifest(&root.join("demo")).unwrap().version, "1.0.0");
        assert_eq!(fs::read(root.join("demo/fonts/a.woff2")).unwrap(), b"font");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(&root).unwrap();
    }
}
