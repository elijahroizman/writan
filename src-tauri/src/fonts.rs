//! Fonts Writan offers to download instead of shipping, so the app stays small and each
//! font arrives only for people who want it. Michelangelus is free from Microsoft, but its license
//! lets people install it on their own devices without letting anyone pass it on, so each copy of
//! Writan downloads it from Microsoft's Download Center. The others are open-source (SIL Open
//! Font License, Ubuntu Font Licence) and come from Google Fonts' repository, pinned to one
//! commit, or their maker's release. Every file is checked against the hash Writan was tested
//! with and kept in the app's data folder.
//!
//! Microsoft's core fonts for the web (Arial, Times New Roman, Courier New…) are offered on Linux,
//! which lacks them. Microsoft lets anyone install them under its license but only as its original
//! 1990s installers, so Writan downloads those, as Debian's ttf-mscorefonts-installer does, and
//! unpacks the fonts from the cabinet inside each. Nothing downloads until the license is accepted.

use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, Runtime};

enum Source {
    /// A zip archive with the files somewhere inside it.
    Zip(&'static str),
    /// Each file on its own, at this address followed by its name.
    Files(&'static str),
    /// Microsoft's self-extracting installers, each a cabinet inside an .exe: name and SHA-256.
    /// Microsoft took them down in 2002; the SourceForge corefonts project keeps the originals,
    /// and the hashes are the ones Debian's ttf-mscorefonts-installer checks.
    Installers(&'static [(&'static str, &'static str)]),
}

struct ExtraFont {
    id: &'static str,
    /// Who the files come from, for error messages.
    maker: &'static str,
    source: Source,
    /// File name and its SHA-256.
    files: &'static [(&'static str, &'static str)],
    /// A license the person must accept before Writan downloads the font, kept beside it.
    license: Option<&'static str>,
}

/// Microsoft's End-User License Agreement for the core fonts, as each installer shows it.
const MICROSOFT_FONTS_EULA: &str = include_str!("../../src/state/microsoft-fonts-eula.txt");

/// SourceForge's redirector sends each request to a working mirror; the second path is the same
/// files by their folder, for when the short one stops resolving.
const COREFONTS_MIRRORS: &[&str] = &[
    "https://downloads.sourceforge.net/corefonts/",
    "https://downloads.sourceforge.net/project/corefonts/the%20fonts/final/",
];

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
        license: None,
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
        license: None,
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
        license: None,
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
        license: None,
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
        license: None,
    },
    ExtraFont {
        id: "instrument-serif",
        maker: "Google Fonts",
        source: Source::Files(google_fonts!("ofl/instrumentserif/")),
        files: &[
            ("InstrumentSerif-Regular.ttf", "498efd461f6ddfcb7a111bf9a565709d2085d48201d501ead960d93e84ffbb88"),
            ("InstrumentSerif-Italic.ttf", "08939b8bdf534afec24ae0ef5e03f948940cd9a8fe08e7fecbad040e62327385"),
        ],
        license: None,
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
        license: None,
    },
    ExtraFont {
        id: "ms-core-fonts",
        maker: "SourceForge",
        source: Source::Installers(&[
            ("andale32.exe", "0524fe42951adc3a7eb870e32f0920313c71f170c859b5f770d82b4ee111e970"),
            ("arial32.exe", "85297a4d146e9c87ac6f74822734bdee5f4b2a722d7eaa584b7f2cbf76f478f6"),
            ("arialb32.exe", "a425f0ffb6a1a5ede5b979ed6177f4f4f4fdef6ae7c302a7b7720ef332fec0a8"),
            ("comic32.exe", "9c6df3feefde26d4e41d4a4fe5db2a89f9123a772594d7f59afd062625cd204e"),
            ("courie32.exe", "bb511d861655dde879ae552eb86b134d6fae67cb58502e6ff73ec5d9151f3384"),
            ("georgi32.exe", "2c2c7dcda6606ea5cf08918fb7cd3f3359e9e84338dc690013f20cd42e930301"),
            ("impact32.exe", "6061ef3b7401d9642f5dfdb5f2b376aa14663f6275e60a51207ad4facf2fccfb"),
            ("times32.exe", "db56595ec6ef5d3de5c24994f001f03b2a13e37cee27bc25c58f6f43e8f807ab"),
            ("trebuc32.exe", "5a690d9bb8510be1b8b4fe49f1f2319651fe51bbe54775ddddd8ef0bd07fdac9"),
            ("verdan32.exe", "c1cb61255e363166794e47664e2f21af8e3a26cb6346eb8d2ae2fa85dd5aad96"),
            ("webdin32.exe", "64595b5abc1080fba8610c5c34fab5863408e806aafe84653ca8575bed17d75a"),
        ]),
        // Lowercase, as Debian installs them; the installers mix cases.
        files: &[
            ("andalemo.ttf", "48d9bc613917709d3b0e0f4a6d4fe33a5c544c5035dffe9e90bc11e50e822071"),
            ("arial.ttf", "35c0f3559d8db569e36c31095b8a60d441643d95f59139de40e23fada819b833"),
            ("arialbd.ttf", "4044aa6b5bebbc36980206b45b0aaaaa5681552a48bcadb41746d5d1d71fd7b4"),
            ("ariali.ttf", "70ade233175a6a6675e4501461af9326e6f78b1ffdf787ca0da5ab0fc8c9cfd6"),
            ("arialbi.ttf", "2f371cd9d96b3ac544519d85c16dc43ceacdfcea35090ee8ddf3ec5857c50328"),
            ("ariblk.ttf", "dad7c04acb26e23dfe4780e79375ca193ddaf68409317e81577a30674668830e"),
            ("comic.ttf", "b82c53776058f291382ff7e008d4675839d2dc21eb295c66391f6fb0655d8fc0"),
            ("comicbd.ttf", "873361465d994994762d0b9845c99fc7baa2a600442ea8db713a7dd19f8b0172"),
            ("cour.ttf", "6715838c52f813f3821549d3f645db9a768bd6f3a43d8f85a89cb6875a546c61"),
            ("courbd.ttf", "edf8a7c5bfcac2e1fe507faab417137cbddc9071637ef4648238d0768c921e02"),
            ("couri.ttf", "f3f6b09855b6700977e214aab5eb9e5be6813976a24f894bd7766e92c732fbe1"),
            ("courbi.ttf", "66dbfa20b534fba0e203da140fec7276a45a1069e424b1b9c35547538128bbe8"),
            ("georgia.ttf", "7d0bb20c632bb59e81a0885f573bd2173f71f73204de9058feb68ce032227072"),
            ("georgiab.ttf", "82d2fbadb88a8632d7f2e8ad50420c9fd2e7d3cbc0e90b04890213a711b34b93"),
            ("georgiai.ttf", "1523f19bda6acca42c47c50da719a12dd34f85cc2606e6a5af15a7728b377b60"),
            ("georgiaz.ttf", "c983e037d8e4e694dd0fb0ba2e625bca317d67a41da2dc81e46a374e53d0ec8a"),
            ("impact.ttf", "00f1fc230ac99f9b97ba1a7c214eb5b909a78660cb3826fca7d64c3af5a14848"),
            ("times.ttf", "4e98adeff8ccc8ef4e3ece8d4547e288ff85fdc9c7ca711a4599c234874bbe86"),
            ("timesbd.ttf", "4357b63cef20c01661a53c5dae70ffd20cb4765503aaed6d38b17a57c5a90bff"),
            ("timesi.ttf", "c25ae529b4cecdbca148b6ccb862ee0abad770af8b1fd29c8dba619d1b8da78a"),
            ("timesbi.ttf", "192e1b0d18e90334e999a99f8c32808d6a2e74b3698b8cd90c943c2249a46549"),
            ("trebuc.ttf", "ec3ffb302488251e1b67fb09dd578b364c5339e27c1cfb26eb627666236453d0"),
            ("trebucbd.ttf", "f65941f9487c0a0a3b7445996ecbbd24466d7ae76ea2a597ced55f438fa63838"),
            ("trebucit.ttf", "db56fdac7d3ba20b7aededcb6ee86c46687489d17b759e1708ea4e2d21e38410"),
            ("trebucbi.ttf", "c0a6bdf31f9f2953b2f08a0c1734c892bc825f0fb17c604d420f7acf203a213b"),
            ("verdana.ttf", "96ed14949ca4b7392cff235b9c41d55c125382abbe0c0d3c2b9dd66897cae0cb"),
            ("verdanab.ttf", "c8f5065ba91680f596af3b0378e2c3e713b95a523be3d56ae185ca2b8f5f0b23"),
            ("verdanai.ttf", "91b59186656f52972531a11433c866fd56e62ec4e61e2621a2dba70c8f19a828"),
            ("verdanaz.ttf", "698e220f48f4a40e77af7eb34958c8fd02f1e18c3ba3f365d93bfa2ed4474c80"),
            ("webdings.ttf", "10d099c88521b1b9e380b7690cbe47b54bb19396ca515358cfdc15ac249e2f5d"),
        ],
        license: Some(MICROSOFT_FONTS_EULA),
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
        Source::Installers(installers) => {
            let mut found = Vec::new();
            for (exe, hash) in installers {
                found.extend(uncab(font, exe, &download_installer(font, exe, hash)?)?);
            }
            Ok(found)
        }
    }
}

/// One of Microsoft's installers, from the first mirror that has the original.
fn download_installer(font: &ExtraFont, exe: &str, hash: &str) -> Result<Vec<u8>, String> {
    let mut error = String::new();
    for mirror in COREFONTS_MIRRORS {
        match download(font, &format!("{mirror}{exe}")) {
            Ok(bytes) if sha256_hex(&bytes) == hash => return Ok(bytes),
            // A mirror's error page, or a file someone altered: try the next, never unpack it.
            Ok(_) => error = format!("The download of {exe} wasn't Microsoft's original, so Writan didn't use it. Try again later."),
            Err(e) if error.is_empty() => error = e,
            Err(_) => {}
        }
    }
    Err(error)
}

/// The fonts in a self-extracting installer: the cabinet sits after the program that unpacks it,
/// so try each place its signature appears.
fn uncab(
    font: &ExtraFont,
    exe: &str,
    bytes: &[u8],
) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let starts = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"MSCF")
        .map(|(i, _)| i);
    for start in starts {
        let Ok(mut cab) = cab::Cabinet::new(Cursor::new(&bytes[start..])) else {
            continue;
        };
        let names: Vec<String> = cab
            .folder_entries()
            .flat_map(|folder| folder.file_entries().map(|f| f.name().to_string()))
            .collect();
        let mut found = Vec::new();
        for name in &names {
            let Some((file, _)) = font
                .files
                .iter()
                .find(|(f, _)| f.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            let mut data = Vec::new();
            cab.read_file(name)
                .and_then(|r| r.take(MAX_DOWNLOAD).read_to_end(&mut data))
                .map_err(|_| format!("Couldn't unpack {exe}."))?;
            found.push((*file, data));
        }
        if !found.is_empty() {
            return Ok(found);
        }
    }
    Err(format!("{exe} wasn't the font installer Writan expected."))
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

/// Downloads a font from its maker and keeps it for Writan's editors. A font with a license to
/// accept downloads only once the person has accepted it, and keeps a copy with the date.
#[tauri::command]
pub async fn install_font<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    accepted_license: Option<bool>,
) -> Result<(), String> {
    let font = font(&id)?;
    check_accepted(font, accepted_license)?;
    let dir = font_dir(&app, font.id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut files = fetch(font)?;
        if let Some(license) = font.license {
            files.push((
                LICENSE_FILE,
                acceptance(license, SystemTime::now()).into_bytes(),
            ));
        }
        install(&dir, font, files)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn check_accepted(font: &ExtraFont, accepted: Option<bool>) -> Result<(), String> {
    match font.license {
        Some(_) if accepted != Some(true) => {
            Err("Accept Microsoft's license to download these fonts.".into())
        }
        _ => Ok(()),
    }
}

/// Beside fonts downloaded under a license: the license, and when it was accepted.
const LICENSE_FILE: &str = "LICENSE-accepted.txt";

fn acceptance(license: &str, when: SystemTime) -> String {
    let days = when
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    let (y, m, d) = civil_date(days);
    format!("Accepted in Writan on {y:04}-{m:02}-{d:02}, before Writan downloaded these fonts.\n\n{license}")
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Whether the system already has Microsoft's core fonts, so Writan needn't offer them. Mac and
/// Windows ship them; on Linux, fontconfig lists them only if a package installed the real ones
/// (it doesn't count the look-alikes it substitutes).
#[tauri::command]
pub fn system_has_core_fonts() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    ["Arial", "Times New Roman", "Courier New"]
        .iter()
        .all(|family| {
            std::process::Command::new("fc-list")
                .arg(format!(":family={family}"))
                .arg("family")
                .output()
                .is_ok_and(|out| out.status.success() && !out.stdout.trim_ascii().is_empty())
        })
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

    fn core_fonts() -> &'static ExtraFont {
        font("ms-core-fonts").unwrap()
    }

    /// A self-extracting installer's shape: a program with a stray "MSCF" in it, then a cabinet.
    fn fake_installer(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut builder = cab::CabinetBuilder::new();
        let folder = builder.add_folder(cab::CompressionType::MsZip);
        for (name, _) in files {
            folder.add_file(*name);
        }
        let mut writer = builder.build(Cursor::new(Vec::new())).unwrap();
        let mut contents = files.iter();
        while let Some(mut file) = writer.next_file().unwrap() {
            file.write_all(contents.next().unwrap().1).unwrap();
        }
        let mut exe = b"MZ program text MSCF not a cabinet".to_vec();
        exe.extend(writer.finish().unwrap().into_inner());
        exe
    }

    #[test]
    fn unpacks_installers_by_name_in_any_case() {
        let exe = fake_installer(&[
            ("FONTINST.EXE", b"setup"),
            ("Arial.TTF", b"regular"),
            ("Arialbd.TTF", b"bold"),
        ]);
        let found = uncab(core_fonts(), "arial32.exe", &exe).unwrap();
        assert_eq!(
            found,
            vec![
                ("arial.ttf", b"regular".to_vec()),
                ("arialbd.ttf", b"bold".to_vec())
            ]
        );
        let err = uncab(core_fonts(), "arial32.exe", b"MZ no cabinet here MSCF").unwrap_err();
        assert!(err.contains("arial32.exe"), "{err}");
    }

    #[test]
    fn installer_fonts_that_dont_match_leave_nothing() {
        let dir = std::env::temp_dir().join(format!("workspace-core-fonts-{}", std::process::id()));
        let font = core_fonts();
        let names: Vec<String> = font.files.iter().map(|(n, _)| n.to_uppercase()).collect();
        let files: Vec<(&str, &[u8])> = names
            .iter()
            .map(|n| (n.as_str(), &b"not a font"[..]))
            .collect();
        let found = uncab(font, "all.exe", &fake_installer(&files)).unwrap();
        assert_eq!(found.len(), font.files.len());
        let err = install(&dir, font, found).unwrap_err();
        assert!(err.contains("changed"), "{err}");
        assert!(!dir.exists() && !dir.with_extension("installing").exists());
    }

    #[test]
    fn core_fonts_need_the_license_accepted() {
        assert!(check_accepted(core_fonts(), None).is_err());
        assert!(check_accepted(core_fonts(), Some(false)).is_err());
        assert!(check_accepted(core_fonts(), Some(true)).is_ok());
        assert!(check_accepted(&FONTS[0], None).is_ok());
        let record = acceptance(
            MICROSOFT_FONTS_EULA,
            UNIX_EPOCH + std::time::Duration::from_secs(19_723 * 86_400 + 5),
        );
        assert!(
            record.starts_with("Accepted in Writan on 2024-01-01"),
            "{record}"
        );
        assert!(record.contains("END-USER LICENSE AGREEMENT"));
    }

    #[test]
    fn calendar_dates() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(civil_date(20_777), (2026, 11, 20));
    }

    #[test]
    fn ids_and_files_are_unique() {
        for (i, font) in FONTS.iter().enumerate() {
            assert!(FONTS[..i].iter().all(|f| f.id != font.id), "{}", font.id);
            assert!(!font.files.is_empty());
            for (j, (name, _)) in font.files.iter().enumerate() {
                assert!(font.files[..j].iter().all(|(n, _)| n != name), "{name}");
            }
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
