//! The themes folder: color themes people and their agents publish for Writan, one JSON file each
//! (`<id>.json`), with an optional background image beside it (`<id>.webp`, `.png`, `.jpg`). The
//! page reads and checks the files (src/state/appearance.ts); this only reads and writes them, and
//! tells the page when the folder changes.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Files past this many are left out, so a folder full of junk can't slow the app.
const MAX_FILES: usize = 32;
/// The longest CSS (100,000 characters) in any JSON encoding, plus the colors.
const MAX_FILE_BYTES: u64 = 6 * 100_000 + 16 * 1024;
/// The page keeps every published theme in memory.
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024;
/// Earlier extensions win when a theme has more than one image.
const BACKGROUND_EXTENSIONS: [&str; 4] = ["webp", "png", "jpg", "jpeg"];
const MAX_BACKGROUND_BYTES: u64 = 16 * 1024 * 1024;

pub fn dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("themes"))
}

/// Lowercase letters, digits and hyphens, starting with a letter or digit, and not a word the
/// appearance setting uses.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && !matches!(id, "system" | "light" | "dark")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

#[derive(Serialize, PartialEq, Clone, Debug)]
pub struct Background {
    ext: String,
    revision: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ThemeEntry {
    id: String,
    text: String,
    background: Option<Background>,
}

#[derive(Serialize)]
pub struct Themes {
    dir: String,
    themes: Vec<ThemeEntry>,
}

/// A regular file's contents, if it is one (not a link, a pipe or a folder) and isn't too big.
fn read_regular(path: &Path, max: u64) -> Option<Vec<u8>> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > max {
        return None;
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(path)
        .ok()?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

fn revision(meta: &fs::Metadata) -> String {
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{}-{modified}", meta.len())
}

fn background(folder: &Path, id: &str) -> Option<Background> {
    BACKGROUND_EXTENSIONS.iter().find_map(|ext| {
        let meta = fs::symlink_metadata(folder.join(format!("{id}.{ext}"))).ok()?;
        (meta.file_type().is_file() && meta.len() <= MAX_BACKGROUND_BYTES).then(|| Background {
            ext: (*ext).to_owned(),
            revision: revision(&meta),
        })
    })
}

pub fn read_folder(folder: &Path) -> Vec<ThemeEntry> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    let mut themes = Vec::new();
    let mut total = 0;
    for name in names.into_iter().take(MAX_FILES) {
        let id = &name[..name.len() - ".json".len()];
        if !valid_id(id) {
            continue;
        }
        let Some(bytes) = read_regular(&folder.join(&name), MAX_FILE_BYTES) else {
            continue;
        };
        total += bytes.len() as u64;
        if total > MAX_TOTAL_BYTES {
            break;
        }
        // A UTF-8 byte order mark, which some editors write, isn't JSON.
        let text = String::from_utf8_lossy(&bytes)
            .trim_start_matches('\u{feff}')
            .to_owned();
        themes.push(ThemeEntry {
            id: id.to_owned(),
            text,
            background: background(folder, id),
        });
    }
    themes
}

#[tauri::command]
pub fn themes_read<R: Runtime>(app: AppHandle<R>) -> Result<Themes, String> {
    let folder = dir(&app)?;
    let _ = fs::create_dir_all(&folder);
    Ok(Themes {
        dir: folder.to_string_lossy().into_owned(),
        themes: read_folder(&folder),
    })
}

/// A published theme's background image.
#[tauri::command]
pub fn theme_background<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    ext: String,
) -> Result<tauri::ipc::Response, String> {
    if !valid_id(&id) || !BACKGROUND_EXTENSIONS.contains(&ext.as_str()) {
        return Err("No such background.".into());
    }
    read_regular(
        &dir(&app)?.join(format!("{id}.{ext}")),
        MAX_BACKGROUND_BYTES,
    )
    .map(tauri::ipc::Response::new)
    .ok_or_else(|| "The background image can't be read.".into())
}

/// Writes `<id>.json`, replacing the file at once so the folder never holds half a theme.
pub fn publish(folder: &Path, id: &str, text: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err(format!("“{id}” can't name a theme. Use lowercase letters, digits and hyphens, and not system, light or dark."));
    }
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err("The theme is too big.".into());
    }
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let target = folder.join(format!("{id}.json"));
    if fs::symlink_metadata(&target).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(format!(
            "{} isn't a file, so the theme can't replace it.",
            target.display()
        ));
    }
    // The page only reads names ending in .json, so it never sees this one.
    let staging = folder.join(format!(".{id}.json.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&staging);
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .and_then(|()| fs::rename(&staging, &target));
    if let Err(e) = written {
        let _ = fs::remove_file(&staging);
        return Err(e.to_string());
    }
    Ok(target)
}

#[tauri::command]
pub fn theme_publish<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    text: String,
) -> Result<String, String> {
    let path = publish(&dir(&app)?, &id, &text)?;
    // Tell the page now rather than at the next look at the folder.
    let _ = app.emit("themes-changed", ());
    Ok(path.to_string_lossy().into_owned())
}

/// Deletes `<id>.json` and its background images. The JSON goes last, so if an image can't be
/// deleted the theme is still listed and can be deleted again.
pub fn remove(folder: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err(format!("“{id}” can't name a theme."));
    }
    for ext in BACKGROUND_EXTENSIONS
        .into_iter()
        .chain(std::iter::once("json"))
    {
        let path = folder.join(format!("{id}.{ext}"));
        // Only files; a folder or link with the name is left alone.
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn theme_delete<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    remove(&dir(&app)?, &id)?;
    let _ = app.emit("themes-changed", ());
    Ok(())
}

/// What the folder holds, cheaply: each name with its size and time.
fn signature(folder: &Path) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().into_string().ok()?;
            if name.starts_with('.') {
                return None;
            }
            Some((name, revision(&e.metadata().ok()?)))
        })
        .collect();
    out.sort();
    out
}

/// Looks at the folder every second and tells the page when anything in it changed. Polling
/// works the same on every system and in folders a watcher can't follow (network drives).
pub fn watch<R: Runtime>(app: AppHandle<R>) {
    let Ok(folder) = dir(&app) else { return };
    let _ = std::thread::Builder::new()
        .name("themes".into())
        .spawn(move || {
            let mut last = signature(&folder);
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let now = signature(&folder);
                if now != last {
                    last = now;
                    let _ = app.emit("themes-changed", ());
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "writan-themes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ids() {
        assert!(valid_id("nightfall"));
        assert!(valid_id("solarized-2"));
        for bad in [
            "",
            "dark",
            "light",
            "system",
            "-x",
            "Night",
            "a/b",
            "../x",
            "a.b",
            &"a".repeat(49),
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn publishes_and_reads_back() {
        let d = temp();
        publish(&d, "dusk", r#"{"name":"Dusk"}"#).unwrap();
        publish(&d, "dusk", r#"{"name":"Dusk 2"}"#).unwrap();
        fs::write(d.join("Bad Name.json"), "{}").unwrap();
        fs::write(d.join("dusk.png"), [1, 2, 3]).unwrap();
        fs::write(d.join("notes.txt"), "x").unwrap();
        let read = read_folder(&d);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, "dusk");
        assert_eq!(read[0].text, r#"{"name":"Dusk 2"}"#);
        assert_eq!(read[0].background.as_ref().unwrap().ext, "png");
        // Nothing left over from staging.
        assert!(fs::read_dir(&d).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        assert!(publish(&d, "../escape", "{}").is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn removes_a_theme_and_its_background() {
        let d = temp();
        publish(&d, "dusk", r#"{"name":"Dusk"}"#).unwrap();
        publish(&d, "dawn", r#"{"name":"Dawn"}"#).unwrap();
        fs::write(d.join("dusk.jpg"), [1, 2, 3]).unwrap();
        remove(&d, "dusk").unwrap();
        let read = read_folder(&d);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, "dawn");
        assert!(!d.join("dusk.jpg").exists());
        // Already gone is fine; a bad id isn't.
        remove(&d, "dusk").unwrap();
        assert!(remove(&d, "../dawn").is_err());
        assert!(d.join("dawn.json").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn skips_oversized_files() {
        let d = temp();
        fs::write(
            d.join("huge.json"),
            vec![b' '; (MAX_FILE_BYTES + 1) as usize],
        )
        .unwrap();
        assert!(read_folder(&d).is_empty());
        let _ = fs::remove_dir_all(&d);
    }
}
