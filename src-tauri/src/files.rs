use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use tauri::ipc::{InvokeBody, Request, Response};

/// Returns the file's bytes as a raw IPC response (no JSON/base64 round trip),
/// so opening a 50 MB PDF costs one copy.
///
/// Commands that touch the disk are `async`: Tauri runs plain commands on the main thread, so
/// reading a big file there would freeze the window, and several reads would queue behind each other.
#[tauri::command(async)]
pub fn read_file(path: String) -> Result<Response, String> {
    fs::read(&path)
        .map(Response::new)
        .map_err(|e| format!("Couldn't open “{}”: {e}", display_name(&path)))
}

struct HistoryJob {
    root: PathBuf,
    path: PathBuf,
    bytes: Vec<u8>,
    hash: String,
    label: Option<String>,
    disk: Option<crate::history::Stamp>,
}

fn history_worker() -> &'static std::sync::mpsc::SyncSender<HistoryJob> {
    static TX: LazyLock<std::sync::mpsc::SyncSender<HistoryJob>> = LazyLock::new(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<HistoryJob>(8);
        let _ = std::thread::Builder::new()
            .name("writan-history".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    if let Err(e) = crate::history::record_with_hash(
                        &job.root, &job.path, &job.bytes, job.hash, job.label, job.disk,
                    ) {
                        eprintln!("version history: {e}");
                    }
                }
            });
        tx
    });
    &TX
}

/// Receives raw bytes as the request body and the destination path in the
/// percent-encoded `x-path` header. Writes atomically: a crash or full disk
/// mid-save leaves the original file untouched. Returns the saved file's
/// hex SHA-256 digest.
///
/// With `x-expected-sha256`, verifies the existing on-disk file first and
/// fails with `__WRITAN_CONFLICT__` if it changed externally.
/// With an `x-history` header the save is also kept in version history (see history.rs),
/// named by the percent-encoded `x-history-label` if there is one.
#[tauri::command(async)]
pub fn write_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    request: Request<'_>,
) -> Result<String, String> {
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
    let encoded = header("x-path").ok_or("Missing destination path")?;
    let path = PathBuf::from(percent_decode(encoded)?);
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected raw file bytes".into());
    };
    let no_clobber = header("x-no-clobber").is_some();
    if no_clobber {
        atomic_write_noclobber(&path, bytes).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "Choose a new file name for the OCR copy.".into()
            } else {
                format!(
                    "Couldn't save “{}”: {}.",
                    display_name(&path.to_string_lossy()),
                    save_error(&e)
                )
            }
        })?;
        return Ok(crate::history::sha256_hex(bytes));
    }
    if let Some(expected) = header("x-expected-sha256") {
        // A file that changed or went away is a conflict the app asks about. One that can't be
        // read stops the save: Writan can't tell what it would replace.
        match crate::history::sha256_file(&path) {
            Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
            Ok(_) => return Err("__WRITAN_CONFLICT__".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err("__WRITAN_CONFLICT__".into())
            }
            Err(e) => {
                return Err(format!(
                    "Couldn't check “{}” before saving: {e}",
                    display_name(&path.to_string_lossy())
                ))
            }
        }
    }
    let digest = crate::history::sha256_hex(bytes);
    let history = header("x-history").and_then(|_| crate::history::history_root(&app));
    let label = header("x-history-label").map(percent_decode).transpose()?;
    if let Some(root) = &history {
        crate::history::remember_original(root, &path);
    }
    atomic_write(&path, bytes).map_err(|e| {
        format!(
            "Couldn't save “{}”: {}.",
            display_name(&path.to_string_lossy()),
            save_error(&e)
        )
    })?;
    if let Some(root) = history {
        let disk = crate::history::stamp(&path);
        // Bounded FIFO worker keeps saves responsive while capping queued buffers.
        let _ = history_worker().send(HistoryJob {
            root,
            path,
            bytes: bytes.clone(),
            hash: digest.clone(),
            label,
            disk,
        });
    }
    Ok(digest)
}

/// Lets the frontend verify at runtime that `write_file` enforces `x-expected-sha256`
/// (browser stress tests mock `__TAURI_INTERNALS__` without implementing CAS headers).
#[tauri::command]
pub fn write_file_supports_cas() -> bool {
    true
}

/// Downloads a small public resource (a video's thumbnail image) for embedding in a
/// presentation. Only https URLs, capped at 10 MB.
#[tauri::command]
pub async fn fetch_bytes(url: String) -> Result<Response, String> {
    if !url.starts_with("https://") {
        return Err("Only https links are supported".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut response = http_agent().get(&url).call().map_err(|e| e.to_string())?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(10 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok(Response::new(bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `ureq` is built with `default-features = false, features = ["native-tls"]`,
/// but `ureq::get` still defaults to `TlsProvider::Rustls` and panics on any
/// `https://` connection unless the agent explicitly selects `NativeTls`.
/// The timeout keeps a stalled download from blocking the video insert, which
/// waits for the poster.
pub(crate) fn http_agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> =
        LazyLock::new(|| agent(Some(Duration::from_secs(15)), None));
    &AGENT
}

/// For large downloads (plugins): 15 s to connect, 30 s for the reply to start, and 15 minutes
/// for the body, enough for 100 MB on a slow line, so a stalled transfer still ends.
pub(crate) fn download_agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> =
        LazyLock::new(|| agent(None, Some(Duration::from_secs(15 * 60))));
    &AGENT
}

fn agent(global: Option<Duration>, body: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(global)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(body)
        .build()
        .new_agent()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    exists: bool,
    is_dir: bool,
    size: u64,
    modified_ms: u64,
}

#[tauri::command(async)]
pub fn path_info(path: String) -> PathInfo {
    match fs::metadata(&path) {
        Ok(meta) => PathInfo {
            exists: true,
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified_ms: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        },
        Err(_) => PathInfo {
            exists: false,
            is_dir: false,
            size: 0,
            modified_ms: 0,
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
}

/// Folders a document app has no business listing: hidden ones, version control, dependencies.
fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "__pycache__"
                | "target"
                | "$RECYCLE.BIN"
                | "System Volume Information"
        )
}

/// A folder's contents for the folder sidebar: subfolders first, then files, each by name
/// (natural order, ignoring case). Hidden files are left out.
#[tauri::command(async)]
pub fn list_dir(path: String) -> Result<Vec<DirEntry>, String> {
    let mut entries: Vec<DirEntry> = fs::read_dir(&path)
        .map_err(|e| format!("Couldn't open the folder “{}”: {e}", display_name(&path)))?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if skipped(&name) {
                return None;
            }
            // Follows symlinks, so a linked folder opens like a folder.
            let is_dir = fs::metadata(entry.path())
                .map(|m| m.is_dir())
                .unwrap_or(false);
            Some(DirEntry {
                path: entry.path().to_string_lossy().into_owned(),
                name,
                is_dir,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(entries)
}

/// Every file under `root` with one of `extensions`, for searching a folder by name. Stops after
/// `limit` files or 20 folders deep, so pointing it at a home folder can't run away.
#[tauri::command]
pub async fn walk_dir(
    root: String,
    extensions: Vec<String>,
    limit: usize,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let extensions: Vec<String> = extensions.iter().map(|e| e.to_lowercase()).collect();
        let mut out = Vec::new();
        let mut stack = vec![(PathBuf::from(&root), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(read) = fs::read_dir(&dir) else {
                continue;
            };
            let mut subdirs = Vec::new();
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if skipped(&name) {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if depth < 20 {
                        subdirs.push(entry.path());
                    }
                } else if let Some(ext) = Path::new(&name).extension() {
                    if extensions.contains(&ext.to_string_lossy().to_lowercase()) {
                        out.push(entry.path().to_string_lossy().into_owned());
                        if out.len() >= limit {
                            return out;
                        }
                    }
                }
            }
            subdirs.sort();
            stack.extend(subdirs.into_iter().rev().map(|d| (d, depth + 1)));
        }
        out
    })
    .await
    .map_err(|e| e.to_string())
}

/// "file2" before "file10", ignoring case.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    a.next();
                }
                let mut nb = String::new();
                while let Some(c) = b.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    b.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    atomic_write_with(path, |file| file.write_all(bytes))
}

fn atomic_write_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    // Saving through a symlink changes the file it points to and keeps the link.
    let linked = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let resolved = if linked {
        Some(fs::canonicalize(path)?)
    } else {
        None
    };
    let path = resolved.as_deref().unwrap_or(path);
    // Replacing the file would succeed on a Mac or Linux, since only its folder needs to be
    // writable, but a read-only file is one someone meant not to change.
    if fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the file is read-only. Use Save As to save a copy",
        ));
    }
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // A separate, exclusively created file for each save. A predictable name can be a symlink
    // to another document, or be shared by two saves in progress.
    let builder = tempfile::Builder::new();
    // Match ordinary new-file creation: the kernel applies the process umask.
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut builder = builder;
        builder.permissions(fs::Permissions::from_mode(0o666));
        builder
    };
    let mut tmp = builder.tempfile_in(dir)?;
    write(tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;
    if let Ok(meta) = fs::metadata(path) {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    // NamedTempFile removes partial writes and failed replacements when it is dropped.
    tmp.persist(path).map_err(|e| e.error)?;
    // So the rename itself survives a power cut.
    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Writes `bytes` to a brand-new `path` using a directory-local temp file and `persist_noclobber`
/// so concurrent exports cannot race past an existence check and overwrite an existing destination.
pub(crate) fn atomic_write_noclobber(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let builder = tempfile::Builder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut builder = builder;
        builder.permissions(fs::Permissions::from_mode(0o666));
        builder
    };
    let mut tmp = builder.tempfile_in(dir)?;
    tmp.as_file_mut().write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Why a save failed, in words; the original file is untouched in every case.
fn save_error(e: &std::io::Error) -> String {
    use std::io::ErrorKind;
    // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION: another app (Word, Excel) has the file open.
    if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) {
        return "it's open in another app. Close it there, then save again".into();
    }
    match e.kind() {
        ErrorKind::StorageFull => "the disk is full. Free up some space, then save again".into(),
        ErrorKind::ReadOnlyFilesystem => "the disk it's on is read-only. Use Save As to save a copy elsewhere".into(),
        ErrorKind::PermissionDenied if e.get_ref().is_some() => e.to_string(),
        ErrorKind::PermissionDenied if cfg!(windows) => "Windows didn't allow it. The file may be read-only or open in another app".into(),
        ErrorKind::PermissionDenied => "Writan isn't allowed to change files in that folder. Use Save As to save a copy elsewhere".into(),
        ErrorKind::NotFound => "its folder is gone. Use Save As to choose another place".into(),
        _ => e.to_string(),
    }
}

pub(crate) fn percent_decode(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|e| e.to_string())?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "Invalid path encoding")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "Invalid path encoding".into())
}

fn display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_unicode_paths() {
        assert_eq!(
            percent_decode("%2FUsers%2Fe%2FR%C3%A9sum%C3%A9.docx").unwrap(),
            "/Users/e/Résumé.docx"
        );
        assert_eq!(percent_decode("plain").unwrap(), "plain");
    }

    #[test]
    fn sorts_names_naturally() {
        let mut names = vec!["file10.md", "File2.md", "file1.md", "notes.md"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["file1.md", "File2.md", "file10.md", "notes.md"]);
    }

    #[test]
    fn writes_atomically() {
        let dir = std::env::temp_dir().join(format!("workspace-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.md");
        atomic_write(&file, b"one").unwrap();
        atomic_write(&file, b"two").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"two");
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "temp file left behind"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn new_saves_follow_umask_and_existing_saves_keep_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ordinary = dir.path().join("ordinary.txt");
        fs::write(&ordinary, b"ordinary").unwrap();
        let saved = dir.path().join("saved.txt");
        atomic_write(&saved, b"new").unwrap();
        assert_eq!(
            fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
            fs::metadata(&ordinary).unwrap().permissions().mode() & 0o777
        );
        fs::set_permissions(&saved, fs::Permissions::from_mode(0o640)).unwrap();
        atomic_write(&saved, b"replacement").unwrap();
        assert_eq!(
            fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn refuses_read_only_files() {
        let dir = std::env::temp_dir().join(format!("workspace-test-ro-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.md");
        fs::write(&file, b"one").unwrap();
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file, perms.clone()).unwrap();
        let err = atomic_write(&file, b"two").unwrap_err();
        assert!(save_error(&err).contains("read-only"));
        assert_eq!(fs::read(&file).unwrap(), b"one");
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&file, perms).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn partial_write_failure_keeps_original_and_removes_temp() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        fs::write(&file, b"original").unwrap();
        let err = atomic_write_with(&file, |tmp| {
            tmp.write_all(b"partial")?;
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
        })
        .unwrap_err();
        assert!(save_error(&err).contains("disk is full"));
        assert_eq!(fs::read(&file).unwrap(), b"original");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_removes_temp() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("folder");
        fs::create_dir(&destination).unwrap();
        assert!(atomic_write(&destination, b"document").is_err());
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_saves_never_mix_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0u8..8)
            .map(|i| {
                let file = file.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    atomic_write(&file, &vec![i; 128 * 1024]).unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let saved = fs::read(&file).unwrap();
        assert_eq!(saved.len(), 128 * 1024);
        assert!(saved.iter().all(|b| *b == saved[0]));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn predictable_temp_symlink_cannot_overwrite_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("other.md");
        let file = dir.path().join("notes.md");
        fs::write(&victim, b"other document").unwrap();
        let trap = dir.path().join(".notes.md.workspace-saving");
        std::os::unix::fs::symlink(&victim, &trap).unwrap();
        atomic_write(&file, b"new document").unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"other document");
        assert_eq!(fs::read(&file).unwrap(), b"new document");
        assert!(fs::symlink_metadata(trap).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_replace_a_broken_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("notes.md");
        std::os::unix::fs::symlink(dir.path().join("missing.md"), &link).unwrap();
        assert!(atomic_write(&link, b"document").is_err());
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }

    /// Needs a tiny file system to fill, in WRITAN_FULL_DIR. On Linux, without root:
    /// `unshare -rm sh -c 'mkdir -p /tmp/full && mount -t tmpfs -o size=64k tmpfs /tmp/full && WRITAN_FULL_DIR=/tmp/full cargo test full_disk -- --ignored'`
    #[test]
    #[ignore]
    fn full_disk_keeps_the_original_and_no_scraps() {
        let dir = PathBuf::from(std::env::var("WRITAN_FULL_DIR").expect("WRITAN_FULL_DIR"));
        let file = dir.join("a.md");
        fs::write(&file, b"original").unwrap();
        let err = atomic_write(&file, &vec![b'x'; 1 << 20]).unwrap_err();
        assert!(save_error(&err).contains("disk is full"), "{err}");
        assert_eq!(fs::read(&file).unwrap(), b"original");
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "partial temp file left behind"
        );
    }

    #[cfg(unix)]
    #[test]
    fn saves_through_symlinks() {
        let dir = std::env::temp_dir().join(format!("workspace-test-link-{}", std::process::id()));
        fs::create_dir_all(dir.join("real")).unwrap();
        let target = dir.join("real/a.md");
        let link = dir.join("a.md");
        fs::write(&target, b"one").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        atomic_write(&link, b"two").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"two");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn atomic_write_noclobber_rejects_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("copy.pdf");
        atomic_write_noclobber(&file, b"first").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"first");
        let err = atomic_write_noclobber(&file, b"second").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&file).unwrap(), b"first");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn https_agent_uses_native_tls_without_panicking() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let _ = listener.accept();
        });
        // Calling `ureq::get` here panics because `ureq` defaults to `TlsProvider::Rustls`
        // while only the `native-tls` feature is enabled. `http_agent()` should instead
        // attempt a native-tls handshake and return a normal `Err` when the peer closes.
        let result = http_agent()
            .get(&format!("https://127.0.0.1:{}/", addr.port()))
            .call();
        assert!(result.is_err());
    }
}
