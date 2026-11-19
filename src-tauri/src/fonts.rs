//! Fonts Writan offers to download instead of shipping, so the app stays small and each
//! font arrives only for people who want it. Michelangelus is free from Microsoft, but its license
//! lets people install it on their own devices without letting anyone pass it on, so each copy of
//! Writan downloads it from Microsoft's Download Center. The others are open-source (SIL Open
//! Font License, Ubuntu Font Licence) and come from Google Fonts' repository, pinned to one
//! commit, or their maker's release. Every file is checked against the hash Writan was tested
//! with and kept in the app's data folder.

use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, Runtime};

enum Source {
    /// A zip archive with the files somewhere inside it.
    Zip(&'static str),
    /// Each file on its own, at this address followed by its name.
    Files(&'static str),
}

struct ExtraFont {
    id: &'static str,
    /// Who the files come from, for error messages.
    maker: &'static str,
    source: Source,
    /// File name and its SHA-256.
    files: &'static [(&'static str, &'static str)],
}

/// A folder of Google Fonts' repository, at the commit these hashes were taken from.
macro_rules! google_fonts {
    ($folder:literal) => {
        concat!("https://raw.githubusercontent.com/google/fonts/47d54ca1ebb14fd864695f1d39dc734f39017c33/", $folder)
    };
}

const FONTS: &[ExtraFont] = &[
    ExtraFont {
        id: "michelangelus",
        maker: "Microsoft",
        source: Source::Zip("https://download.microsoft.com/download/e8912dd7-c7e6-4ca1-a36b-c121d3fb238f/Michelangelus%20Fonts.zip"),
        files: &[
            ("Michelangelus-Regular.ttf", "8e21078b09f6befa90ece78b55b8140d4a3b778eb48e68a7c899e02074429a4a"),
            ("Michelangelus-Italic.ttf", "36ef7b5a9a1f25c29c8ad3478d649fba2f64549bc50aab165dc55fb4418741da"),
            ("Michelangelus-Bold.ttf", "c5f07eb1d008b2ad1f75a9b5e8f91ba45827ef2b6d0f6989d0371a6168e1125f"),
            ("Michelangelus-BoldItalic.ttf", "f4815c2e4212be963d24c913c689dd2a97dc607e0e9ff0587f2ab90ad2641a52"),
        ],
    },
    ExtraFont {
        id: "ubuntu",
        maker: "Google Fonts",
        source: Source::Files(google_fonts!("ufl/ubuntu/")),
        files: &[
            ("Ubuntu-Regular.ttf", "3128df86a31805618436d0ae5651ba4285d0c9de0a39057d025f64ee33bceb64"),
            ("Ubuntu-Italic.ttf", "4ab857e72f781a8967a6e4a9ac8858fbd6b3a9f9782db349d4b62b78ed02860b"),
            ("Ubuntu-Bold.ttf", "679b5c1e09cab3156bb8ef529735f9382bf31ca7ac737382ab959297f8d82ad4"),
            ("Ubuntu-BoldItalic.ttf", "875d776e7f33c50b1d1b594791da0eba9865648f232f08bcba00bba9dfa01d96"),
        ],
    },
    ExtraFont {
        id: "jetbrains-mono",
        maker: "JetBrains",
        source: Source::Zip("https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip"),
        files: &[
            ("JetBrainsMono-Regular.ttf", "a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f"),
            ("JetBrainsMono-Italic.ttf", "9d0a1f7a708e6af183f1193b7e81d40da294f5c67682c085d8401c60aac8ded4"),
            ("JetBrainsMono-Bold.ttf", "5590990c82e097397517f275f430af4546e1c45cff408bde4255dad142479dcb"),
            ("JetBrainsMono-BoldItalic.ttf", "4039d5ce0ed225bf9c8b2c8c6436290ae2f356b7e90d70fa666227238324aa3b"),
        ],
    },
    ExtraFont {
        id: "atkinson-hyperlegible",
        maker: "Google Fonts",
        source: Source::Files(google_fonts!("ofl/atkinsonhyperlegible/")),
        files: &[
            ("AtkinsonHyperlegible-Regular.ttf", "7fb917c89019896d0b52ee84b7cbb3304c18cb90b19a62f5e32712bd23e97669"),
            ("AtkinsonHyperlegible-Italic.ttf", "021beda4d3c6edfc78872e436d74009f9a1bcb294331908fe5747c61d3dcc514"),
            ("AtkinsonHyperlegible-Bold.ttf", "5a3b0c8cc8ca545155150b4512a1fa248298df121c50d6557e651e61fbdab92f"),
            ("AtkinsonHyperlegible-BoldItalic.ttf", "7a46f1b0ef2f64b61065c1a6b5a11fcb9b019b44062d88ba121a083cc39e86bf"),
        ],
    },
    ExtraFont {
        id: "courier-prime",
        maker: "Google Fonts",
        source: Source::Files(google_fonts!("ofl/courierprime/")),
        files: &[
            ("CourierPrime-Regular.ttf", "72f793376f8e2841656bf21d77a5de010f2929bd6956a22ee848ad0c7eb978af"),
            ("CourierPrime-Italic.ttf", "f1b9a5829789f7e56432a9f3bc7665ef4531dbba1c112639e48bef39621a006b"),
            ("CourierPrime-Bold.ttf", "ff1f38786c849d1c41fa8e447960abdb2bd75fdfb0cfcdeb524fad65a5af3638"),
            ("CourierPrime-BoldItalic.ttf", "3355273f3ea6d6362658f9564f4e83d59e156cf1a182e3c1592b5c5943627dcc"),
        ],
    },
    ExtraFont {
        id: "instrument-serif",
        maker: "Google Fonts",
        source: Source::Files(google_fonts!("ofl/instrumentserif/")),
        files: &[
            ("InstrumentSerif-Regular.ttf", "498efd461f6ddfcb7a111bf9a565709d2085d48201d501ead960d93e84ffbb88"),
            ("InstrumentSerif-Italic.ttf", "08939b8bdf534afec24ae0ef5e03f948940cd9a8fe08e7fecbad040e62327385"),
        ],
    },
    ExtraFont {
        id: "comic-neue",
        maker: "Google Fonts",
        source: Source::Files(google_fonts!("ofl/comicneue/")),
        files: &[
            ("ComicNeue-Regular.ttf", "a0ee5a37c8b27c4db0700137d928598b1e23b0089e1546a8961909176b779360"),
            ("ComicNeue-Italic.ttf", "e06bfd1552f5c9464c5665733ffd69239b0593885dbb9e059688a5900f78cf98"),
            ("ComicNeue-Bold.ttf", "3e7e5fccfd7e0788f317b43312151c1bd5cf058c9697a8d83eac3939050bd61e"),
            ("ComicNeue-BoldItalic.ttf", "5c312c2a2fa64eee82f3b87fcfab8f3b12a5e59b043124401d322eb323cfbf16"),
        ],
    },
];

/// The biggest download (JetBrains Mono's release) is under 6 MB; anything far bigger isn't it.
const MAX_DOWNLOAD: u64 = 20 * 1024 * 1024;

fn font(id: &str) -> Result<&'static ExtraFont, String> {
    FONTS
        .iter()
        .find(|f| f.id == id)
        .ok_or_else(|| "Unknown font".to_string())
}

fn font_dir<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("fonts")
        .join(id))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The extra fonts whose files are all on disk.
#[tauri::command]
pub fn installed_fonts<R: Runtime>(app: AppHandle<R>) -> Vec<&'static str> {
    FONTS
        .iter()
        .filter(|f| {
            font_dir(&app, f.id)
                .is_ok_and(|dir| f.files.iter().all(|(name, _)| dir.join(name).is_file()))
        })
        .map(|f| f.id)
        .collect()
}

fn download(font: &ExtraFont, url: &str) -> Result<Vec<u8>, String> {
    let failed = |_| {
        format!(
            "Couldn't download the font from {}. Check your connection and try again.",
            font.maker
        )
    };
    let mut response = crate::files::download_agent()
        .get(url)
        .header("User-Agent", "Writan")
        .call()
        .map_err(failed)?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .reader()
        .read_to_end(&mut bytes)
        .map_err(|_| "The font download was interrupted.".to_string())?;
    Ok(bytes)
}

/// The font's files from its maker, by name, not yet checked.
fn fetch(font: &ExtraFont) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    match font.source {
        Source::Zip(url) => unzip(font, &download(font, url)?),
        Source::Files(base) => font
            .files
            .iter()
            .map(|(name, _)| Ok((*name, download(font, &format!("{base}{name}"))?)))
            .collect(),
    }
}

fn unzip(font: &ExtraFont, archive: &[u8]) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).map_err(|_| {
        format!(
            "{}'s download wasn't the font archive Writan expected.",
            font.maker
        )
    })?;
    let mut found = Vec::new();
    for (name, _) in font.files {
        // Files can sit in a folder inside the archive; match on the file name alone.
        let index = (0..zip.len())
            .find(|&i| {
                zip.by_index(i)
                    .is_ok_and(|f| f.name().rsplit('/').next() == Some(*name))
            })
            .ok_or_else(|| format!("{name} is missing from {}'s download.", font.maker))?;
        let mut bytes = Vec::new();
        zip.by_index(index)
            .map_err(|e| e.to_string())?
            .take(MAX_DOWNLOAD)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        found.push((*name, bytes));
    }
    Ok(found)
}

/// Checks every file against its hash, then puts them in `dir` together.
fn install(dir: &PathBuf, font: &ExtraFont, found: Vec<(&str, Vec<u8>)>) -> Result<(), String> {
    for (name, hash) in font.files {
        let bytes = found
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b)
            .ok_or_else(|| format!("{name} is missing from {}'s download.", font.maker))?;
        if sha256_hex(bytes) != *hash {
            return Err(format!("{} has changed this font since this version of Writan. Update Writan and try again.", font.maker));
        }
    }
    let staging = dir.with_extension("installing");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    for (name, bytes) in found {
        fs::write(staging.join(name), bytes).map_err(|e| format!("Couldn't save the font: {e}"))?;
    }
    let _ = fs::remove_dir_all(dir);
    fs::rename(&staging, dir).map_err(|e| format!("Couldn't save the font: {e}"))
}

/// Downloads a font from its maker and keeps it for Writan's editors.
#[tauri::command]
pub async fn install_font<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let font = font(&id)?;
    let dir = font_dir(&app, font.id)?;
    tauri::async_runtime::spawn_blocking(move || install(&dir, font, fetch(font)?))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn remove_font<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let dir = font_dir(&app, font(&id)?.id)?;
    match fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("Couldn't remove the font: {e}"))
        }
        _ => Ok(()),
    }
}

/// One face's bytes, for the editors to register.
#[tauri::command]
pub fn font_file<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    file: String,
) -> Result<Response, String> {
    let font = font(&id)?;
    let (name, _) = font
        .files
        .iter()
        .find(|(name, _)| *name == file)
        .ok_or("Unknown font file")?;
    let bytes = fs::read(font_dir(&app, font.id)?.join(name)).map_err(|e| e.to_string())?;
    Ok(Response::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_files_that_dont_match() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("workspace-fonts-{}", std::process::id()));
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, _) in FONTS[0].files {
                zip.start_file(format!("Michelangelus Fonts/{name}"), opts)
                    .unwrap();
                zip.write_all(b"not a font").unwrap();
            }
            zip.finish().unwrap();
        }
        let err = install(&dir, &FONTS[0], unzip(&FONTS[0], buf.get_ref()).unwrap()).unwrap_err();
        assert!(err.contains("changed"), "{err}");
        assert!(!dir.exists());
    }

    #[test]
    fn ids_and_files_are_unique() {
        for (i, font) in FONTS.iter().enumerate() {
            assert!(FONTS[..i].iter().all(|f| f.id != font.id), "{}", font.id);
            assert!(!font.files.is_empty());
        }
    }

    /// Fetches every font from its maker: `cargo test downloads_every_font -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_every_font() {
        let mut failed = Vec::new();
        for font in FONTS {
            let dir = std::env::temp_dir().join(format!(
                "workspace-fonts-real-{}-{}",
                std::process::id(),
                font.id
            ));
            match fetch(font).and_then(|files| install(&dir, font, files)) {
                Ok(()) => assert!(font.files.iter().all(|(name, _)| dir.join(name).is_file())),
                Err(e) => failed.push(format!("{}: {e}", font.id)),
            }
            let _ = fs::remove_dir_all(&dir);
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}
