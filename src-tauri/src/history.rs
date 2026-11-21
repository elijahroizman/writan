//! Version history: a copy of a file each time Writan saves it, kept in the app's data folder.
//!
//! Git-style storage. Each distinct version's bytes are stored once, named by their SHA-256
//! (`history/objects/ab/cdef…`), so saving an unchanged file or two files with the same content
//! costs nothing. Each file has a log (`history/logs/<sha256 of its path>.json`) listing its
//! versions newest first. Older versions are thinned out the way Time Machine does it: every save
//! from the last hour, then one per 10 minutes for a day, one a day for a month, one a week after
//! that. Versions with a name are always kept.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, Runtime};

/// Logs are read, changed and written back under this lock, so two saves can't lose a version.
static LOCK: Mutex<()> = Mutex::new(());

const MAX_VERSIONS: usize = 300;
const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub hash: String,
    /// When it was saved, in ms since 1970.
    pub time: u64,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// "save"; "original" for the file as it was before Writan first saved over it; "outside"
    /// for a change another app (or a sync service) made before Writan saved over it.
    pub source: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Log {
    path: String,
    versions: Vec<Version>,
    /// The file's size and modified time just after Writan last saved it. While they still match,
    /// nothing else has changed it, so it needn't be read again before the next save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disk: Option<Stamp>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Stamp {
    size: u64,
    modified: u128,
}

/// The file's size and modified time (in ns), if it exists.
pub fn stamp(file: &Path) -> Option<Stamp> {
    let meta = fs::metadata(file).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(Stamp {
        size: meta.len(),
        modified,
    })
}

fn root<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("history"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn log_path(root: &Path, file: &str) -> PathBuf {
    root.join("logs")
        .join(format!("{}.json", sha256_hex(file.as_bytes())))
}

fn object_path(root: &Path, hash: &str) -> PathBuf {
    root.join("objects").join(&hash[..2]).join(&hash[2..])
}

fn read_log(root: &Path, file: &str) -> Log {
    fs::read(log_path(root, file))
        .ok()
        .and_then(|b| serde_json::from_slice::<Log>(&b).ok())
        .filter(|log| log.path == file)
        .unwrap_or_else(|| Log {
            path: file.to_string(),
            versions: Vec::new(),
            disk: None,
        })
}

fn write_log(root: &Path, log: &Log) -> std::io::Result<()> {
    let path = log_path(root, &log.path);
    fs::create_dir_all(path.parent().unwrap())?;
    if log.versions.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec(log).map_err(std::io::Error::other)?,
    )?;
    fs::rename(tmp, path)
}

fn store_object(root: &Path, bytes: &[u8]) -> std::io::Result<String> {
    let hash = sha256_hex(bytes);
    let path = object_path(root, &hash);
    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(tmp, &path)?;
    }
    Ok(hash)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn new_id(time: u64, hash: &str) -> String {
    format!("{time:x}-{}", &hash[..8])
}

/// Which versions to keep (newest first in, newest first out): see the module comment.
fn thin(versions: Vec<Version>, now: u64) -> Vec<Version> {
    let mut kept: Vec<Version> = Vec::new();
    // The bucket of the last kept version, per age band, so each bucket keeps its newest.
    let mut last_bucket: Option<(u8, u64)> = None;
    for v in versions {
        let age = now.saturating_sub(v.time);
        let bucket = if age < HOUR {
            None
        } else if age < DAY {
            Some((1, v.time / (10 * MINUTE)))
        } else if age < 30 * DAY {
            Some((2, v.time / DAY))
        } else {
            Some((3, v.time / (7 * DAY)))
        };
        let keep =
            v.label.is_some() || v.source != "save" || bucket.is_none() || bucket != last_bucket;
        if keep {
            if bucket.is_some() {
                last_bucket = bucket;
            }
            kept.push(v);
        }
    }
    // Over the limit, the oldest unnamed ones go first.
    while kept.len() > MAX_VERSIONS {
        match kept.iter().rposition(|v| v.label.is_none()) {
            Some(i) => {
                kept.remove(i);
            }
            None => break,
        }
    }
    kept
}

/// Objects no log mentions any more.
fn collect_garbage(root: &Path, candidates: &[String]) {
    if candidates.is_empty() {
        return;
    }
    let mut used = std::collections::HashSet::new();
    if let Ok(entries) = fs::read_dir(root.join("logs")) {
        for entry in entries.flatten() {
            if let Some(log) = fs::read(entry.path())
                .ok()
                .and_then(|b| serde_json::from_slice::<Log>(&b).ok())
            {
                used.extend(log.versions.into_iter().map(|v| v.hash));
            }
        }
    }
    for hash in candidates {
        if !used.contains(hash) {
            let _ = fs::remove_file(object_path(root, hash));
        }
    }
}

fn replace_versions(root: &Path, mut log: Log, versions: Vec<Version>) -> std::io::Result<()> {
    let removed: Vec<String> = log
        .versions
        .iter()
        .filter(|v| !versions.iter().any(|k| k.hash == v.hash))
        .map(|v| v.hash.clone())
        .collect();
    log.versions = versions;
    write_log(root, &log)?;
    collect_garbage(root, &removed);
    Ok(())
}

/// The file on disk before Writan saves over it: the first time, so the version you opened can
/// always come back, and whenever something else changed it since Writan's last save (another
/// app, a sync service), so that change isn't lost either.
pub fn remember_original(root: &Path, file: &Path) {
    let key = file.to_string_lossy();
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let log = read_log(root, &key);
    if !log.versions.is_empty() && (log.disk.is_none() || log.disk == stamp(file)) {
        return;
    }
    let Ok(bytes) = fs::read(file) else { return };
    let source = if log.versions.is_empty() {
        "original"
    } else {
        "outside"
    };
    let _ = add_version(root, log, &bytes, source, None, file);
}

/// A version just saved; `disk` is the file's stamp right after the save.
pub fn record(
    root: &Path,
    file: &Path,
    bytes: &[u8],
    label: Option<String>,
    disk: Option<Stamp>,
) -> std::io::Result<()> {
    let key = file.to_string_lossy();
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut log = read_log(root, &key);
    log.disk = disk;
    add_version(root, log, bytes, "save", label, file)
}

fn add_version(
    root: &Path,
    mut log: Log,
    bytes: &[u8],
    source: &str,
    label: Option<String>,
    file: &Path,
) -> std::io::Result<()> {
    let hash = store_object(root, bytes)?;
    let time = now_ms();
    if let Some(latest) = log.versions.first_mut() {
        if latest.hash == hash {
            // The same bytes again: only a new name, if one came with it.
            if label.is_some() {
                latest.label = label;
            }
            return write_log(root, &log);
        }
    }
    let version = Version {
        id: new_id(time, &hash),
        hash,
        time,
        size: bytes.len() as u64,
        label,
        source: source.into(),
    };
    log.path = file.to_string_lossy().into_owned();
    let mut versions = log.versions.clone();
    versions.insert(0, version);
    let versions = thin(versions, time);
    replace_versions(root, log, versions)
}

/// A file's versions, newest first.
#[tauri::command(async)]
pub fn history_list<R: Runtime>(app: AppHandle<R>, path: String) -> Result<Vec<Version>, String> {
    let root = root(&app)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Ok(read_log(&root, &path).versions)
}

/// One version's bytes, as a raw response.
#[tauri::command(async)]
pub fn history_read<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    id: String,
) -> Result<Response, String> {
    let root = root(&app)?;
    let log = {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        read_log(&root, &path)
    };
    let version = log
        .versions
        .iter()
        .find(|v| v.id == id)
        .ok_or("That version is gone.")?;
    fs::read(object_path(&root, &version.hash))
        .map(Response::new)
        .map_err(|e| format!("Couldn’t read that version: {e}"))
}

/// Name a version (or clear its name with null). Named versions are never thinned out.
#[tauri::command]
pub fn history_label<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    id: String,
    label: Option<String>,
) -> Result<(), String> {
    let root = root(&app)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut log = read_log(&root, &path);
    let version = log
        .versions
        .iter_mut()
        .find(|v| v.id == id)
        .ok_or("That version is gone.")?;
    version.label = label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    write_log(&root, &log).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_delete<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    id: String,
) -> Result<(), String> {
    let root = root(&app)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let log = read_log(&root, &path);
    let versions = log
        .versions
        .iter()
        .filter(|v| v.id != id)
        .cloned()
        .collect();
    replace_versions(&root, log, versions).map_err(|e| e.to_string())
}

/// How much space every file's history takes, in bytes.
#[tauri::command]
pub fn history_usage<R: Runtime>(app: AppHandle<R>) -> Result<u64, String> {
    fn size(dir: &Path) -> u64 {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| match e.metadata() {
                        Ok(m) if m.is_dir() => size(&e.path()),
                        Ok(m) => m.len(),
                        Err(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    Ok(size(&root(&app)?))
}

/// Forget every version of every file.
#[tauri::command]
pub fn history_clear_all<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let root = root(&app)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match fs::remove_dir_all(&root) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Where versions live, for files.rs's save path.
pub fn history_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    root(app).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(time: u64, label: Option<&str>) -> Version {
        Version {
            id: format!("{time}"),
            hash: format!("{time:064}"),
            time,
            size: 1,
            label: label.map(String::from),
            source: "save".into(),
        }
    }

    #[test]
    fn thins_older_versions() {
        let now = 100 * DAY;
        // Every minute for two hours, newest first.
        let versions: Vec<Version> = (0..120).map(|m| v(now - m * MINUTE, None)).collect();
        let kept = thin(versions, now);
        // The last hour's 60 stay; the hour before keeps one per 10 minutes (6 or 7 buckets).
        let older = kept.iter().filter(|x| now - x.time >= HOUR).count();
        assert_eq!(kept.iter().filter(|x| now - x.time < HOUR).count(), 60);
        assert!((6..=7).contains(&older), "kept {older}");
    }

    #[test]
    fn keeps_named_versions() {
        let now = 100 * DAY;
        let versions = vec![
            v(now - 2 * DAY, None),
            v(now - 2 * DAY - MINUTE, Some("Draft 1")),
            v(now - 2 * DAY - 2 * MINUTE, None),
        ];
        let kept = thin(versions, now);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|x| x.label.as_deref() == Some("Draft 1")));
    }

    #[test]
    fn records_dedupes_and_restores() {
        let root = std::env::temp_dir().join(format!("workspace-history-{}", std::process::id()));
        let file = root.join("notes.md");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, b"first").unwrap();
        remember_original(&root, &file);
        record(&root, &file, b"second", None, None).unwrap();
        record(&root, &file, b"second", Some("Named".into()), None).unwrap();
        let log = read_log(&root, &file.to_string_lossy());
        assert_eq!(log.versions.len(), 2);
        assert_eq!(log.versions[0].label.as_deref(), Some("Named"));
        assert_eq!(log.versions[1].source, "original");
        assert_eq!(
            fs::read(object_path(&root, &log.versions[1].hash)).unwrap(),
            b"first"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keeps_a_change_made_outside_before_saving_over_it() {
        let root =
            std::env::temp_dir().join(format!("workspace-history-outside-{}", std::process::id()));
        let file = root.join("notes.md");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, b"first").unwrap();
        remember_original(&root, &file);
        fs::write(&file, b"mine").unwrap();
        record(&root, &file, b"mine", None, stamp(&file)).unwrap();
        // Saving again with nothing changed outside adds nothing.
        remember_original(&root, &file);
        assert_eq!(read_log(&root, &file.to_string_lossy()).versions.len(), 2);
        // Another app rewrites it; the next save keeps that version first.
        fs::write(&file, b"theirs, longer").unwrap();
        remember_original(&root, &file);
        let log = read_log(&root, &file.to_string_lossy());
        assert_eq!(log.versions.len(), 3);
        assert_eq!(log.versions[0].source, "outside");
        assert_eq!(
            fs::read(object_path(&root, &log.versions[0].hash)).unwrap(),
            b"theirs, longer"
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
