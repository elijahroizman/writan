use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tauri::{Emitter, Manager};

const BASE_URL: &str = "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/87416418657359cb625c412a48b6e1d6d41c29bd/";

struct Pack {
    code: &'static str,
    size: u64,
    sha256: &'static str,
}

// Mirrored by src/lib/ocr/catalog.ts.
const PACKS: &[Pack] = &[
    Pack {
        code: "eng",
        size: 4113088,
        sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
    },
    Pack {
        code: "heb",
        size: 961404,
        sha256: "11f9e43ab227f786352a50f75c94c2e9906f1baba86d93276da19da7ce0904db",
    },
    Pack {
        code: "ara",
        size: 1432056,
        sha256: "e3206d3dc87fd50c24a0fb9f01838615911d25168f4e64415244b67d2bb3e729",
    },
    Pack {
        code: "fra",
        size: 1130365,
        sha256: "ced037562e8c80c13122dece28dd477d399af80911a28791a66a63ac1e3445ca",
    },
    Pack {
        code: "deu",
        size: 1525436,
        sha256: "19d219bbb6672c869d20a9636c6816a81eb9a71796cb93ebe0cb1530e2cdb22d",
    },
    Pack {
        code: "spa",
        size: 2294433,
        sha256: "6f2e04d02774a18f01bed44b1111f2cd7f3ba7ac9dc4373cd3f898a40ea6b464",
    },
    Pack {
        code: "ita",
        size: 2701314,
        sha256: "b8f89e1e785118dac4d51ae042c029a64edb5c3ee42ef73027a6d412748d8827",
    },
    Pack {
        code: "por",
        size: 1982756,
        sha256: "c4932b937207a9514b7514d518b931a99938c02a28a5a5a553f8599ed58b7deb",
    },
    Pack {
        code: "rus",
        size: 3861738,
        sha256: "e16e5e036cce1d9ec2b00063cf8b54472625b9e14d893a169e2b0dedeb4df225",
    },
    Pack {
        code: "chi_sim",
        size: 2469156,
        sha256: "a5fcb6f0db1e1d6d8522f39db4e848f05984669172e584e8d76b6b3141e1f730",
    },
    Pack {
        code: "jpn",
        size: 2471260,
        sha256: "1f5de9236d2e85f5fdf4b3c500f2d4926f8d9449f28f5394472d9e8d83b91b4d",
    },
    Pack {
        code: "kor",
        size: 1677415,
        sha256: "6b85e11d9bbf07863b97b3523b1b112844c43e713df8b66418a081fd1060b3b2",
    },
];

fn pack(code: &str) -> Result<&'static Pack, String> {
    PACKS
        .iter()
        .find(|pack| pack.code == code)
        .ok_or_else(|| "Unknown OCR language pack.".into())
}

fn directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("ocr").join("tessdata"))
        .map_err(|error| format!("Could not find OCR language pack storage: {error}"))
}

fn destination(dir: &Path, pack: &Pack) -> PathBuf {
    dir.join(format!("{}.traineddata", pack.code))
}

fn storage_error(error: io::Error) -> String {
    if error.kind() == io::ErrorKind::StorageFull {
        "The disk is full. Free up space and try downloading the language pack again.".into()
    } else {
        format!("Could not access OCR language pack storage: {error}")
    }
}

fn installed_meta(path: &Path) -> Result<Option<(u64, u128)>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            Ok(Some((meta.len(), mtime)))
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(storage_error(error)),
    }
}

fn installed_size(path: &Path) -> Result<Option<u64>, String> {
    Ok(installed_meta(path)?.map(|(size, _)| size))
}

type VerifiedMap = HashMap<PathBuf, (u64, u128, bool)>;

static VERIFIED_CACHE: LazyLock<Mutex<VerifiedMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn cached_verify(path: &Path, size: u64, mtime: u128, pack: &Pack) -> bool {
    if let Some(&(cached_size, cached_mtime, verified)) = VERIFIED_CACHE.lock().unwrap().get(path) {
        if cached_size == size && cached_mtime == mtime {
            return verified;
        }
    }
    let verified = verified_bytes(path, pack).is_ok();
    VERIFIED_CACHE
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), (size, mtime, verified));
    verified
}

fn matches_digest(pack: &Pack, size: u64, digest: Sha256) -> bool {
    size == pack.size && format!("{:x}", digest.finalize()) == pack.sha256
}

fn verified_bytes(path: &Path, pack: &Pack) -> Result<Vec<u8>, String> {
    let integrity_error = || format!("The {} OCR pack is damaged. Download it again.", pack.code);
    match installed_size(path)? {
        Some(size) if size == pack.size => {}
        Some(_) => return Err(integrity_error()),
        None => return Err(format!("Download the {} OCR pack first.", pack.code)),
    }
    let mut bytes = Vec::with_capacity(pack.size as usize);
    fs::File::open(path)
        .and_then(|file| file.take(pack.size + 1).read_to_end(&mut bytes))
        .map_err(storage_error)?;
    let mut digest = Sha256::new();
    digest.update(&bytes);
    if !matches_digest(pack, bytes.len() as u64, digest) {
        return Err(integrity_error());
    }
    Ok(bytes)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPack {
    code: &'static str,
    size: u64,
    sha256_verified: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInventory {
    installed: Vec<InstalledPack>,
    total_bytes: u64,
}

#[derive(Clone, Serialize)]
struct PackProgress {
    code: &'static str,
    received: u64,
    total: u64,
}

fn storage_usage(dir: &Path) -> Result<u64, String> {
    PACKS.iter().try_fold(0, |sum, pack| {
        Ok(sum + installed_size(&destination(dir, pack))?.unwrap_or(0))
    })
}

fn inventory(dir: &Path) -> Result<PackInventory, String> {
    let mut inventory = PackInventory {
        installed: Vec::new(),
        total_bytes: 0,
    };
    for pack in PACKS {
        let path = destination(dir, pack);
        if let Some((size, mtime)) = installed_meta(&path)? {
            inventory.total_bytes += size;
            inventory.installed.push(InstalledPack {
                code: pack.code,
                size,
                sha256_verified: cached_verify(&path, size, mtime, pack),
            });
        }
    }
    Ok(inventory)
}

static DOWNLOADS: LazyLock<Mutex<HashMap<&'static str, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct Download {
    code: &'static str,
    cancelled: Arc<AtomicBool>,
}

impl Download {
    fn begin(code: &'static str) -> Result<Self, String> {
        let mut downloads = DOWNLOADS.lock().unwrap();
        if downloads.contains_key(code) {
            return Err(format!("The {code} OCR pack is already downloading."));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        downloads.insert(code, cancelled.clone());
        Ok(Self { code, cancelled })
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        DOWNLOADS.lock().unwrap().remove(self.code);
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("OCR pack download cancelled.".into())
    } else {
        Ok(())
    }
}

fn stage_pack(
    dir: &Path,
    pack: &Pack,
    mut input: impl Read,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<tempfile::NamedTempFile, String> {
    check_cancelled(cancelled)?;
    fs::create_dir_all(dir).map_err(storage_error)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(storage_error)?;
    let mut digest = Sha256::new();
    let mut received = 0;
    let mut buffer = [0; 32 * 1024];
    let integrity_error = || {
        format!(
            "The {} OCR pack download failed verification. Try downloading it again.",
            pack.code
        )
    };
    loop {
        check_cancelled(cancelled)?;
        let read = input.read(&mut buffer);
        check_cancelled(cancelled)?;
        let count = read.map_err(|error| format!("OCR pack download failed: {error}"))?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > pack.size {
            return Err(integrity_error());
        }
        digest.update(&buffer[..count]);
        tmp.write_all(&buffer[..count]).map_err(storage_error)?;
        progress(received);
    }
    if !matches_digest(pack, received, digest) {
        return Err(integrity_error());
    }
    tmp.as_file().sync_all().map_err(storage_error)?;
    Ok(tmp)
}

fn remove_pack(dir: &Path, pack: &Pack) -> Result<(), String> {
    let downloads = DOWNLOADS.lock().unwrap();
    if downloads.contains_key(pack.code) {
        return Err(
            "Wait for the OCR pack download to finish or cancel it before removing the pack."
                .into(),
        );
    }
    let path = destination(dir, pack);
    VERIFIED_CACHE.lock().unwrap().remove(&path);
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result.map_err(storage_error),
    }
}

#[tauri::command(async)]
pub fn ocr_list_packs(app: tauri::AppHandle) -> Result<PackInventory, String> {
    inventory(&directory(&app)?)
}

#[tauri::command]
pub async fn ocr_download_pack(app: tauri::AppHandle, code: String) -> Result<(), String> {
    let pack = pack(&code)?;
    let dir = directory(&app)?;
    let download = Download::begin(pack.code)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut last_emit = Instant::now();
        let mut emit_progress = |received: u64, force: bool| {
            if !force && last_emit.elapsed() < Duration::from_millis(75) {
                return;
            }
            last_emit = Instant::now();
            let _ = app.emit(
                "ocr-pack-progress",
                PackProgress {
                    code: pack.code,
                    received,
                    total: pack.size,
                },
            );
        };
        emit_progress(0, true);
        check_cancelled(&download.cancelled)?;
        let response = crate::files::download_agent()
            .get(format!("{BASE_URL}{}.traineddata", pack.code))
            .call();
        check_cancelled(&download.cancelled)?;
        let response = response.map_err(|error| format!("OCR pack download failed: {error}"))?;
        let tmp = stage_pack(
            &dir,
            pack,
            response.into_body().into_reader(),
            &download.cancelled,
            |received| emit_progress(received, received >= pack.size),
        )?;
        // Serialize the final rename with cancellation and removal.
        let _downloads = DOWNLOADS.lock().unwrap();
        check_cancelled(&download.cancelled)?;
        let dest = destination(&dir, pack);
        tmp.persist(&dest)
            .map_err(|error| storage_error(error.error))?;
        if let Ok(Some((size, mtime))) = installed_meta(&dest) {
            VERIFIED_CACHE
                .lock()
                .unwrap()
                .insert(dest, (size, mtime, true));
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("OCR pack download failed: {error}"))?
}

#[tauri::command]
pub fn ocr_cancel_download(code: String) -> Result<(), String> {
    let pack = pack(&code)?;
    if let Some(cancelled) = DOWNLOADS.lock().unwrap().get(pack.code) {
        cancelled.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command(async)]
pub fn ocr_remove_pack(app: tauri::AppHandle, code: String) -> Result<(), String> {
    let pack = pack(&code)?;
    remove_pack(&directory(&app)?, pack)
}

#[tauri::command(async)]
pub fn ocr_read_pack(app: tauri::AppHandle, code: String) -> Result<tauri::ipc::Response, String> {
    let pack = pack(&code)?;
    verified_bytes(&destination(&directory(&app)?, pack), pack).map(tauri::ipc::Response::new)
}

#[tauri::command(async)]
pub fn ocr_storage_usage(app: tauri::AppHandle) -> Result<u64, String> {
    storage_usage(&directory(&app)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BYTES: &[u8] = b"abc";
    const FIXTURE: Pack = Pack {
        code: "eng",
        size: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    };

    #[test]
    fn ocr_allowlist_rejects_unknown_codes_and_paths() {
        assert_eq!(PACKS.len(), 12);
        for allowed in PACKS {
            assert!(pack(allowed.code).is_ok());
            assert!(allowed.size > 0);
            assert_eq!(allowed.sha256.len(), 64);
            assert!(allowed.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        for invalid in [
            "../eng",
            "eng/../ara",
            "eng\\..",
            "ENG",
            "eng.traineddata",
            "osd",
            "",
        ] {
            assert!(pack(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn ocr_verified_download_and_offline_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = destination(dir.path(), &FIXTURE);
        let mut progress = Vec::new();
        let tmp = stage_pack(dir.path(), &FIXTURE, BYTES, &AtomicBool::new(false), |n| {
            progress.push(n)
        })
        .unwrap();
        assert!(!path.exists());
        tmp.persist(&path).unwrap();
        assert_eq!(progress, [3]);
        assert_eq!(verified_bytes(&path, &FIXTURE).unwrap(), BYTES);
        assert_eq!(dir.path().read_dir().unwrap().count(), 1);
        fs::write(&path, b"abd").unwrap();
        assert!(verified_bytes(&path, &FIXTURE).is_err());
        fs::write(&path, b"ab").unwrap();
        assert!(verified_bytes(&path, &FIXTURE).is_err());
    }

    #[test]
    fn ocr_failed_verification_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        for invalid in [b"abd".as_slice(), b"ab", b"abcd"] {
            let result = stage_pack(
                dir.path(),
                &FIXTURE,
                invalid,
                &AtomicBool::new(false),
                |_| {},
            );
            assert!(result.unwrap_err().contains("failed verification"));
            assert_eq!(dir.path().read_dir().unwrap().count(), 0);
        }
    }

    #[test]
    fn ocr_cancel_aborts_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let download = Download::begin("heb").unwrap();
        assert!(Download::begin("heb").is_err());
        let result = stage_pack(dir.path(), &FIXTURE, BYTES, &download.cancelled, |n| {
            if n > 0 {
                ocr_cancel_download("heb".into()).unwrap();
            }
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert_eq!(dir.path().read_dir().unwrap().count(), 0);
        drop(download);
        assert!(Download::begin("heb").is_ok());
    }

    #[test]
    fn ocr_network_error_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        struct FailedBody;
        impl Read for FailedBody {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::ConnectionReset))
            }
        }
        let result = stage_pack(
            dir.path(),
            &FIXTURE,
            FailedBody,
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(result.unwrap_err().contains("download failed"));
        assert_eq!(dir.path().read_dir().unwrap().count(), 0);
        assert!(storage_error(io::ErrorKind::StorageFull.into()).contains("disk is full"));
    }

    #[test]
    fn ocr_remove_and_storage_usage() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(storage_usage(&dir.path().join("missing")).unwrap(), 0);
        fs::write(destination(dir.path(), &FIXTURE), BYTES).unwrap();
        fs::write(destination(dir.path(), pack("ara").unwrap()), [0; 6]).unwrap();
        fs::write(dir.path().join("unlisted.traineddata"), [0; 10]).unwrap();
        let inventory = inventory(dir.path()).unwrap();
        assert_eq!(inventory.total_bytes, 9);
        assert_eq!(inventory.installed.len(), 2);
        assert_eq!(inventory.installed[0].code, "eng");
        assert!(!inventory.installed[0].sha256_verified);
        remove_pack(dir.path(), &FIXTURE).unwrap();
        remove_pack(dir.path(), &FIXTURE).unwrap();
        assert_eq!(storage_usage(dir.path()).unwrap(), 6);
    }
}
