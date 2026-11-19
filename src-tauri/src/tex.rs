//! The LaTeX plugin compiles with the TeX that's installed on the system (MacTeX on a Mac,
//! MiKTeX or TeX Live on Windows). This finds it, runs it, and installs it when it's missing.
//!
//! A compile copies the editor's text to `<cache>/tex/<tab>/<job>.tex` and runs the engine
//! from the document's own folder, so `\input` and `\includegraphics` find the files beside it,
//! while the .aux, .log and .pdf go to the cache instead of next to the user's file. The .aux
//! stays between compiles, so references settle without running TeX twice every time.

// Installing is for macOS and Windows only.
#![cfg_attr(
    not(any(target_os = "macos", windows)),
    allow(dead_code, unused_imports)
)]

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::http::header;
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Manager, Runtime};

const ENGINES: [&str; 3] = ["pdflatex", "xelatex", "lualatex"];
/// A runaway document (an endless loop in a macro) is stopped after this long.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RUNS: usize = 4;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TexStatus {
    installed: bool,
    distribution: Option<String>,
    engines: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    stage: &'static str,
    received: u64,
    total: u64,
}

/// Folders TeX may be in. An app started from the Dock or Start menu doesn't get the shell's
/// PATH (and one started before TeX was installed has an old one), so the usual places are
/// checked too.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push("/Library/TeX/texbin".into());
        dirs.extend(texlive_bins(
            Path::new("/usr/local/texlive"),
            "universal-darwin",
        ));
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Programs\MiKTeX\miktex\bin\x64"));
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(programs) = std::env::var_os(var) {
                dirs.push(PathBuf::from(programs).join(r"MiKTeX\miktex\bin\x64"));
            }
        }
        dirs.extend(texlive_bins(Path::new(r"C:\texlive"), "windows"));
        dirs.extend(texlive_bins(Path::new(r"C:\texlive"), "win64"));
        dirs.extend(texlive_bins(Path::new(r"C:\texlive"), "win32"));
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let platform = if cfg!(target_arch = "aarch64") {
            "aarch64-linux"
        } else {
            "x86_64-linux"
        };
        dirs.extend(texlive_bins(Path::new("/usr/local/texlive"), platform));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

/// `<root>/<year>/bin/<platform>`, newest year first.
fn texlive_bins(root: &Path, platform: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut years: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.len() == 4 && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    years.sort();
    years
        .into_iter()
        .rev()
        .map(|y| y.join("bin").join(platform))
        .collect()
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The folder holding a TeX program, and the program's path.
fn find(name: &str) -> Option<PathBuf> {
    let file = exe(name);
    search_dirs()
        .into_iter()
        .map(|dir| dir.join(&file))
        .find(|p| p.is_file())
}

/// A command that doesn't flash a console window on Windows.
fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // TeX's own programs (bibtex, kpsewhich) must be the ones beside the engine.
    if let Some(dir) = program.parent().filter(|d| !d.as_os_str().is_empty()) {
        let mut path = OsString::from(dir.as_os_str());
        if let Some(old) = std::env::var_os("PATH") {
            path.push(if cfg!(windows) { ";" } else { ":" });
            path.push(old);
        }
        cmd.env("PATH", path);
    }
    cmd
}

/// "TeX Live 2026" or "MiKTeX 25.12", from the first line of `pdflatex --version`.
fn distribution(engine: &Path) -> Option<String> {
    let output = command(engine)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.lines().next()?;
    let start = first.rfind('(')? + 1;
    let end = first[start..].find(')')? + start;
    Some(first[start..end].trim().to_string()).filter(|s| !s.is_empty())
}

/// Whether an engine is MiKTeX's, remembered per program so each compile doesn't ask again.
fn is_miktex(program: &Path) -> bool {
    static SEEN: std::sync::Mutex<Vec<(PathBuf, bool)>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, miktex)) = seen.iter().find(|(p, _)| p == program) {
        return *miktex;
    }
    let miktex = distribution(program).is_some_and(|d| d.contains("MiKTeX"));
    seen.push((program.to_path_buf(), miktex));
    miktex
}

fn status() -> TexStatus {
    let engines: Vec<String> = ENGINES
        .iter()
        .filter(|e| find(e).is_some())
        .map(|e| e.to_string())
        .collect();
    let distribution = engines
        .first()
        .and_then(|e| find(e))
        .and_then(|p| distribution(&p));
    TexStatus {
        installed: !engines.is_empty(),
        distribution,
        engines,
    }
}

#[tauri::command]
pub async fn tex_status() -> TexStatus {
    tauri::async_runtime::spawn_blocking(status)
        .await
        .unwrap_or(TexStatus {
            installed: false,
            distribution: None,
            engines: Vec::new(),
        })
}

/// A tab id or file stem, reduced to characters that are safe in a folder name and to TeX.
fn safe_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// Runs a program to completion, killing it after COMPILE_TIMEOUT.
fn run(mut cmd: Command) -> Result<bool, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start TeX: {e}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.success());
        }
        if started.elapsed() > COMPILE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("TeX took more than two minutes and was stopped.".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn read_lossy(path: &Path) -> String {
    fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

fn needs_rerun(log: &str) -> bool {
    log.contains("Rerun to get")
        || log.contains("Label(s) may have changed")
        || log.contains("Please rerun LaTeX")
        || log.contains("Rerun LaTeX")
        || log.contains("(rerunfilecheck)")
}

fn compile(
    build_root: PathBuf,
    key: &str,
    source: &str,
    dir: Option<&str>,
    engine: &str,
    job: &str,
) -> Result<(Option<Vec<u8>>, PathBuf, String), String> {
    if !ENGINES.contains(&engine) {
        return Err(format!("Unknown TeX engine “{engine}”."));
    }
    let program = find(engine).ok_or_else(|| {
        if find("pdflatex").is_some() {
            format!("{engine} isn't installed. Use pdflatex, or add {engine} to your TeX.")
        } else {
            "TeX isn't installed.".to_string()
        }
    })?;
    let build = build_root.join(safe_name(key, "document"));
    fs::create_dir_all(&build).map_err(|e| e.to_string())?;
    let job = safe_name(job, "document");
    let tex = build.join(format!("{job}.tex"));
    fs::write(&tex, source).map_err(|e| e.to_string())?;
    let pdf = build.join(format!("{job}.pdf"));
    let log_path = build.join(format!("{job}.log"));
    // A failed compile mustn't hand back the last good PDF as if it were new.
    let _ = fs::remove_file(&pdf);

    let cwd = dir
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| build.clone());
    let miktex = is_miktex(&program);
    let latex = || {
        let mut cmd = command(&program);
        // One message per log line, so errors can be read back (TeX Live wraps at 79 otherwise).
        cmd.current_dir(&cwd)
            .env("max_print_line", "10000")
            .env("error_line", "254")
            .env("half_error_line", "238")
            .arg("-interaction=nonstopmode")
            .arg("-file-line-error")
            .arg(format!("-output-directory={}", build.display()));
        // \write18 may run only TeX's own safe list of programs, even if the TeX installation is set
        // to allow any (people do that for minted): opening a downloaded .tex mustn't run commands.
        cmd.arg(if miktex {
            "--restrict-write18"
        } else {
            "-shell-restricted"
        });
        if miktex {
            // Packages the document needs install by themselves instead of asking.
            cmd.arg("--enable-installer");
        }
        cmd.arg(&tex);
        cmd
    };

    let mut bib_done = false;
    for _ in 0..MAX_RUNS {
        run(latex())?;
        let log = read_lossy(&log_path);
        if !bib_done {
            bib_done = true;
            let aux = read_lossy(&build.join(format!("{job}.aux")));
            let bbl = build.join(format!("{job}.bbl"));
            let bcf = build.join(format!("{job}.bcf"));
            let biber = bcf.is_file() && (!bbl.is_file() || log.contains("Please (re)run Biber"));
            let bibtex = !bcf.is_file()
                && aux.contains("\\bibdata")
                && (!bbl.is_file() || log.contains("Citation") && log.contains("undefined"));
            if biber || bibtex {
                let tool = if biber { "biber" } else { "bibtex" };
                if let Some(program) = find(tool) {
                    let mut cmd = command(&program);
                    if biber {
                        cmd.current_dir(&build)
                            .arg("--input-directory")
                            .arg(&cwd)
                            .arg(&job);
                    } else {
                        // bibtex reads .bib files from where it runs; BIBINPUTS adds the document's folder.
                        let sep = if cfg!(windows) { ";" } else { ":" };
                        let mut inputs = OsString::from(cwd.as_os_str());
                        inputs.push(sep);
                        cmd.current_dir(&build).env("BIBINPUTS", inputs).arg(&job);
                    }
                    run(cmd)?;
                    continue;
                }
            }
        }
        if !needs_rerun(&log) {
            break;
        }
    }
    let log = read_lossy(&log_path);
    Ok((fs::read(&pdf).ok(), tex, log))
}

/// Compiles and answers in one binary message: the PDF's length (u32, little-endian; 0 for
/// none), the PDF, the compiled file's path (u32 length, UTF-8; the log names it in errors),
/// then the log as UTF-8.
#[tauri::command]
pub async fn tex_compile<R: Runtime>(
    app: AppHandle<R>,
    key: String,
    source: String,
    dir: Option<String>,
    engine: String,
    job: String,
) -> Result<Response, String> {
    let root = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("tex");
    let (pdf, file, log) = tauri::async_runtime::spawn_blocking(move || {
        compile(root, &key, &source, dir.as_deref(), &engine, &job)
    })
    .await
    .map_err(|e| e.to_string())??;
    let pdf = pdf.unwrap_or_default();
    let file = file.to_string_lossy();
    let mut out = Vec::with_capacity(8 + pdf.len() + file.len() + log.len());
    out.extend_from_slice(&(pdf.len() as u32).to_le_bytes());
    out.extend_from_slice(&pdf);
    out.extend_from_slice(&(file.len() as u32).to_le_bytes());
    out.extend_from_slice(file.as_bytes());
    out.extend_from_slice(log.as_bytes());
    Ok(Response::new(out))
}

/// A closed tab's build folder.
#[tauri::command]
pub fn tex_forget<R: Runtime>(app: AppHandle<R>, key: String) {
    if let Ok(root) = app.path().app_cache_dir() {
        let _ = fs::remove_dir_all(root.join("tex").join(safe_name(&key, "document")));
    }
}

// ── Installing

fn download_to(url: &str, to: &Path, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let failed = |_| "Couldn't download TeX. Check your connection and try again.".to_string();
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
    let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
    let mut file = fs::File::create(to).map_err(|e| e.to_string())?;
    let mut chunk = vec![0u8; 1024 * 1024];
    let mut received = 0u64;
    let mut last = Instant::now();
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|_| "The TeX download was interrupted.".to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&chunk[..n])
            .map_err(|e| format!("Couldn't save the TeX installer: {e}"))?;
        received += n as u64;
        if last.elapsed() > Duration::from_millis(200) {
            last = Instant::now();
            progress(received, total);
        }
    }
    progress(received, total);
    if total > 0 && received != total {
        return Err("The TeX download was interrupted.".into());
    }
    Ok(())
}

/// The MiKTeX installer Writan offers, and its SHA-256 from miktex.org/download. MiKTeX
/// doesn't code-sign it, so the checksum is what vouches for it. CTAN keeps old versions, so
/// this keeps working until an app update moves it on.
#[cfg_attr(not(windows), allow(dead_code))]
const MIKTEX: (&str, &str) = (
    "basic-miktex-25.12-x64.exe",
    "14b42dd9f4b4a7813a8bfd69c8f99316c2888cc4ee26f631f397e163d85d6c62",
);

/// MacTeX's packages are signed by its maintainer's Developer ID (Team ID RBGCY5RJWM).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MACTEX_SIGNER: &str = "Developer ID Installer: Richard Koch (RBGCY5RJWM)";

#[cfg_attr(not(windows), allow(dead_code))]
fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

// Package managers come first (Homebrew, winget, apt): they verify what they download, and the
// system can update or remove the result later. Without one, a Mac gets MacTeX's own signed
// installer and Windows gets MiKTeX's.

/// Runs a program to completion with its output captured; the error is the tail of what it said.
/// A program still running after `limit` is stopped, so a package manager waiting on something
/// can't leave Writan waiting forever.
fn run_captured(mut cmd: Command, what: &str, limit: Duration) -> Result<String, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start {what}: {e}"))?;
    // Each pipe is read on its own thread. A helper the program started can keep a pipe open
    // after the program exits, so that wait is short and bounded by the same deadline.
    let (sender, received) = std::sync::mpsc::channel::<(bool, String)>();
    let drain = |pipe: Option<Box<dyn Read + Send>>, is_out: bool| {
        let sender = sender.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            let _ = sender.send((is_out, String::from_utf8_lossy(&bytes).into_owned()));
        });
    };
    drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        true,
    );
    drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        false,
    );
    drop(sender);
    let started = Instant::now();
    let stopped = || {
        format!(
            "{what} didn't finish in {} minutes and was stopped.",
            limit.as_secs().div_ceil(60)
        )
    };
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(stopped());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let (mut out, mut err) = (String::new(), String::new());
    let exited = Instant::now();
    for _ in 0..2 {
        let left = limit
            .saturating_sub(started.elapsed())
            .min(Duration::from_secs(30).saturating_sub(exited.elapsed()));
        match received.recv_timeout(left) {
            Ok((true, text)) => out = text,
            Ok((false, text)) => err = text,
            // What it said is lost, but the exit status still tells whether it worked.
            Err(_) => break,
        }
    }
    if status.success() {
        return Ok(out);
    }
    let said = format!("{err}\n{out}");
    // winget draws progress bars with \r and spinner characters; keep the words.
    let tail: Vec<&str> = said
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|l| l.chars().any(char::is_alphanumeric))
        .collect();
    let tail = tail[tail.len().saturating_sub(4)..].join(" ");
    let code = status
        .code()
        .map(|c| format!(" (exit code {c:#x})"))
        .unwrap_or_default();
    Err(format!(
        "{what} failed{code}: {}",
        if tail.is_empty() {
            "no details"
        } else {
            tail.as_str()
        }
    ))
}

/// How long installing or removing TeX may take before Writan gives up on it.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// Whether a program answers `--version` (winget is an app alias, so it's found by running it).
#[cfg_attr(not(windows), allow(dead_code))]
fn runs(program: &str, arg: &str) -> bool {
    command(Path::new(program))
        .arg(arg)
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;

    pub const CASKS: [&str; 3] = ["mactex-no-gui", "mactex", "basictex"];

    pub fn brew() -> Option<PathBuf> {
        ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
    }

    /// Homebrew asks for the administrator password through SUDO_ASKPASS when it has no terminal.
    pub fn brew_command(brew: &Path, dir: &Path) -> Result<Command, String> {
        use std::os::unix::fs::PermissionsExt;
        let askpass = dir.join("askpass.sh");
        let script = "#!/bin/sh\nexec /usr/bin/osascript -e 'text returned of (display dialog \"Writan needs your password to change TeX.\" default answer \"\" with hidden answer with title \"Writan\")'\n";
        fs::write(&askpass, script).map_err(|e| e.to_string())?;
        fs::set_permissions(&askpass, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let mut cmd = command(brew);
        cmd.env("SUDO_ASKPASS", &askpass)
            .env("NONINTERACTIVE", "1")
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .env("HOMEBREW_NO_ANALYTICS", "1")
            .env("HOMEBREW_NO_INSTALL_CLEANUP", "1");
        Ok(cmd)
    }

    /// The cask Homebrew installed TeX from, if it did.
    pub fn installed_cask(brew: &Path) -> Option<&'static str> {
        let out = command(brew)
            .args(["list", "--cask", "-1"])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let list = String::from_utf8_lossy(&out.stdout);
        CASKS
            .into_iter()
            .find(|c| list.lines().any(|l| l.trim() == *c))
    }

    /// MacTeX's own installer leaves receipts named org.tug.mactex.*.
    pub fn has_mactex_receipt() -> bool {
        Command::new("/usr/sbin/pkgutil")
            .arg("--pkgs")
            .output()
            .is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.starts_with("org.tug.mactex"))
            })
    }

    /// The one `/usr/local/texlive/<year>` folder the TeX in use lives in (what /Library/TeX/texbin
    /// points into), and its year. Other years and texmf-local are someone else's to keep.
    pub fn texlive_year_dir() -> Option<(PathBuf, String)> {
        let bin = fs::canonicalize("/Library/TeX/texbin").ok()?;
        let year_dir = bin
            .ancestors()
            .find(|p| p.parent() == Some(Path::new("/usr/local/texlive")))?;
        let year = year_dir.file_name()?.to_str()?.to_string();
        (year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()))
            .then(|| (year_dir.to_path_buf(), year))
    }

    /// Runs shell commands as an administrator, after macOS asks for the password.
    pub fn admin(shell: &str) -> Result<(), String> {
        let escaped = shell.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("do shell script \"{escaped}\" with administrator privileges");
        let output = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&output.stderr);
        Err(if err.contains("-128") {
            "Cancelled.".into()
        } else {
            format!("{}", err.trim())
        })
    }
}

#[cfg(target_os = "macos")]
fn install(app: &AppHandle<impl Runtime>, variant: &str, dir: &Path) -> Result<(), String> {
    let emit = |stage, received, total| {
        let _ = app.emit(
            "tex-progress",
            Progress {
                stage,
                received,
                total,
            },
        );
    };
    if let Some(brew) = mac::brew() {
        emit("install", 0, 0);
        let cask = if variant == "basictex" {
            "basictex"
        } else {
            "mactex-no-gui"
        };
        let mut cmd = mac::brew_command(&brew, dir)?;
        cmd.args(["install", "--cask", cask]);
        return run_captured(cmd, "Homebrew", INSTALL_TIMEOUT).map(|_| ());
    }
    let name = match variant {
        "basictex" => "BasicTeX.pkg",
        _ => "MacTeX.pkg",
    };
    let pkg = dir.join(name);
    download_to(
        &format!("https://mirror.ctan.org/systems/mac/mactex/{name}"),
        &pkg,
        |r, t| emit("download", r, t),
    )?;
    emit("verify", 0, 0);
    // A signature macOS trusts, from MacTeX's own Developer ID.
    let check = Command::new("/usr/sbin/pkgutil")
        .arg("--check-signature")
        .arg(&pkg)
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&check.stdout);
    if !check.status.success()
        || !text.contains("signed by a developer certificate issued by Apple")
        || !text.contains(MACTEX_SIGNER)
    {
        return Err(
            "The TeX installer's signature didn't check out, so it wasn't installed.".into(),
        );
    }
    emit("install", 0, 0);
    // macOS asks for an administrator's password, then installs it without its wizard.
    let path = pkg.to_string_lossy().replace('\'', "'\\''");
    mac::admin(&format!("/usr/sbin/installer -pkg '{path}' -target /")).map_err(|e| {
        if e == "Cancelled." {
            "Installing TeX was cancelled.".into()
        } else {
            format!("Installing TeX failed: {e}")
        }
    })
}

#[cfg(windows)]
fn install(app: &AppHandle<impl Runtime>, _variant: &str, dir: &Path) -> Result<(), String> {
    let emit = |stage, received, total| {
        let _ = app.emit(
            "tex-progress",
            Progress {
                stage,
                received,
                total,
            },
        );
    };
    let configure = || {
        // Missing packages install as a document needs them, without a dialog each time.
        if let Some(initexmf) = find("initexmf") {
            let _ = command(&initexmf)
                .arg("--set-config-value=[MPM]AutoInstall=1")
                .status();
        }
    };
    // winget first. If it isn't there or fails (no network for its sources, a broken App
    // Installer, policies on a work laptop), MiKTeX's own installer is the fallback.
    let mut winget_error = None;
    if runs("winget", "--version") {
        emit("install", 0, 0);
        let mut cmd = command(Path::new("winget"));
        // Just for this user, so Windows doesn't ask for an administrator.
        cmd.args([
            "install",
            "--id",
            "MiKTeX.MiKTeX",
            "-e",
            "--scope",
            "user",
            "--silent",
            "--disable-interactivity",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ]);
        match run_captured(cmd, "winget", INSTALL_TIMEOUT) {
            Ok(_) => {
                configure();
                return Ok(());
            }
            // It may have installed and still said no (already installed, a reboot pending).
            Err(_) if status().installed => {
                configure();
                return Ok(());
            }
            Err(e) => winget_error = Some(e),
        }
    }
    let with_winget = |e: String| match &winget_error {
        Some(w) => format!("{w}\n\nMiKTeX's own installer didn't work either: {e}"),
        None => e,
    };
    let (name, sha256) = MIKTEX;
    let setup = dir.join(name);
    let url = format!("https://mirror.ctan.org/systems/win32/miktex/setup/windows-x64/{name}");
    download_to(&url, &setup, |r, t| emit("download", r, t)).map_err(with_winget)?;
    emit("verify", 0, 0);
    if sha256_hex(&setup).map_err(with_winget)? != sha256 {
        return Err(with_winget(
            "The MiKTeX installer didn't match its published checksum, so it wasn't installed."
                .into(),
        ));
    }
    emit("install", 0, 0);
    // Just for this user, so Windows doesn't ask for an administrator.
    let mut cmd = command(&setup);
    cmd.args(["--unattended", "--private"]);
    run_captured(cmd, "The MiKTeX installer", INSTALL_TIMEOUT).map_err(with_winget)?;
    configure();
    Ok(())
}

/// What `apt` installs for each choice in the dialog: all of what documents commonly need, or the basics.
#[cfg(target_os = "linux")]
fn apt_packages(variant: &str) -> &'static str {
    match variant {
        "texlive-basic" => "texlive-latex-base texlive-latex-recommended texlive-fonts-recommended",
        _ => "texlive-latex-base texlive-latex-recommended texlive-latex-extra texlive-fonts-recommended texlive-xetex texlive-luatex biber",
    }
}

/// Runs a shell line as root through polkit's own password prompt.
#[cfg(target_os = "linux")]
pub(crate) fn pkexec(line: &str, what: &str) -> Result<(), String> {
    let mut cmd = Command::new("pkexec");
    cmd.args(["/bin/sh", "-c", line]);
    let output = cmd.stdin(Stdio::null()).output().map_err(|_| format!("Writan needs pkexec (polkit) to ask for your password. {what} needs doing in a terminal instead."))?;
    match output.status.code() {
        Some(0) => Ok(()),
        // 126 is polkit's "the password dialog was dismissed"; 127 is an error.
        Some(126) => Err(format!("{what} was cancelled.")),
        _ => {
            let err = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = err
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            let tail = tail[tail.len().saturating_sub(3)..].join(" ");
            Err(format!(
                "{what} failed: {}",
                if tail.is_empty() {
                    "pkexec couldn't authorize or run the command"
                } else {
                    tail.as_str()
                }
            ))
        }
    }
}

#[cfg(target_os = "linux")]
fn install(app: &AppHandle<impl Runtime>, variant: &str, _dir: &Path) -> Result<(), String> {
    if !Path::new("/usr/bin/apt-get").is_file() {
        return Err("Writan installs TeX with apt, which this system doesn't have. Install TeX Live with your package manager.".into());
    }
    let _ = app.emit(
        "tex-progress",
        Progress {
            stage: "install",
            received: 0,
            total: 0,
        },
    );
    let line = format!(
        "export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y {}",
        apt_packages(variant)
    );
    pkexec(&line, "Installing TeX")
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn install(_app: &AppHandle<impl Runtime>, _variant: &str, _dir: &Path) -> Result<(), String> {
    Err("Install TeX Live with your package manager (for example, texlive-latex-extra).".into())
}

/// Downloads and installs TeX: `mactex` or `basictex` on a Mac, MiKTeX on Windows, `texlive` or
/// `texlive-basic` on Debian and Ubuntu.
#[tauri::command]
pub async fn tex_install<R: Runtime>(
    app: AppHandle<R>,
    variant: String,
) -> Result<TexStatus, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("tex-installer");
    tauri::async_runtime::spawn_blocking(move || {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let result = install(&app, &variant, &dir);
        let _ = fs::remove_dir_all(&dir);
        result?;
        Ok(status())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Uninstalling: only what Writan knows how to take away cleanly.

/// How TeX would be removed from this computer ("Homebrew", "winget", "apt", "MacTeX's installer"),
/// or none if it isn't there or wasn't put there by something Writan can undo.
fn uninstall_method() -> Option<&'static str> {
    if !status().installed {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        if mac::brew().is_some_and(|b| mac::installed_cask(&b).is_some()) {
            return Some("Homebrew");
        }
        if mac::has_mactex_receipt() && mac::texlive_year_dir().is_some() {
            return Some("MacTeX’s installer");
        }
    }
    #[cfg(windows)]
    {
        // Only a MiKTeX winget itself knows by id: one from MiKTeX's own installer may not match.
        let mut list = command(Path::new("winget"));
        list.args([
            "list",
            "--id",
            "MiKTeX.MiKTeX",
            "--exact",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]);
        if run_captured(list, "winget", Duration::from_secs(60))
            .is_ok_and(|out| out.contains("MiKTeX"))
        {
            return Some("winget");
        }
    }
    #[cfg(target_os = "linux")]
    {
        let installed = Command::new("dpkg-query")
            .args(["-W", "-f=${Status}", "texlive-binaries"])
            .output();
        if installed
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("install ok installed"))
            && Path::new("/usr/bin/apt-get").is_file()
        {
            return Some("apt");
        }
    }
    None
}

#[tauri::command]
pub async fn tex_uninstall_method() -> Option<&'static str> {
    tauri::async_runtime::spawn_blocking(uninstall_method)
        .await
        .ok()
        .flatten()
}

fn uninstall(app: &AppHandle<impl Runtime>, dir: &Path) -> Result<(), String> {
    let _ = app.emit(
        "tex-progress",
        Progress {
            stage: "install",
            received: 0,
            total: 0,
        },
    );
    match uninstall_method() {
        #[cfg(target_os = "macos")]
        Some("Homebrew") => {
            let brew = mac::brew().ok_or("Homebrew isn't installed.")?;
            let cask = mac::installed_cask(&brew).ok_or("Homebrew didn't install this TeX.")?;
            let mut cmd = mac::brew_command(&brew, dir)?;
            cmd.args(["uninstall", "--cask", cask]);
            run_captured(cmd, "Homebrew", INSTALL_TIMEOUT).map(|_| ())
        }
        #[cfg(target_os = "macos")]
        Some("MacTeX’s installer") => {
            let _ = dir;
            // Just the year in use (and the links and apps MacTeX adds); other years and texmf-local stay.
            let (year_dir, year) = mac::texlive_year_dir()
                .ok_or("Writan couldn't tell which TeX Live folder MacTeX installed.")?;
            let year_dir = year_dir.to_string_lossy().replace('\'', "'\\''");
            mac::admin(&format!("/bin/rm -rf '{year_dir}' /Library/TeX /Applications/TeX; for p in $(/usr/sbin/pkgutil --pkgs | /usr/bin/grep '^org.tug.mactex.*{year}'); do /usr/sbin/pkgutil --forget $p; done"))
                .map_err(|e| if e == "Cancelled." { "Removing TeX was cancelled.".into() } else { format!("Removing TeX failed: {e}") })
        }
        #[cfg(windows)]
        Some("winget") => {
            let _ = dir;
            let mut cmd = command(Path::new("winget"));
            cmd.args([
                "uninstall",
                "--id",
                "MiKTeX.MiKTeX",
                "--exact",
                "--silent",
                "--disable-interactivity",
                "--accept-source-agreements",
            ]);
            run_captured(cmd, "winget", INSTALL_TIMEOUT).map(|_| ())
        }
        #[cfg(target_os = "linux")]
        Some("apt") => {
            let _ = dir;
            // Removing the base takes every TeX Live package that needs it along.
            pkexec("export DEBIAN_FRONTEND=noninteractive; apt-get remove -y texlive-base texlive-binaries texlive-latex-base", "Removing TeX")
        }
        _ => {
            let _ = dir;
            Err("Writan can't remove this TeX. Use the program that installed it.".into())
        }
    }
}

/// Removes TeX by the same means as `uninstall_method` says.
#[tauri::command]
pub async fn tex_uninstall<R: Runtime>(app: AppHandle<R>) -> Result<TexStatus, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("tex-installer");
    tauri::async_runtime::spawn_blocking(move || {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let result = uninstall(&app, &dir);
        let _ = fs::remove_dir_all(&dir);
        result?;
        Ok(status())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn reports_what_a_failed_program_said() {
        let mut ok = Command::new("sh");
        ok.args(["-c", "echo done"]);
        assert_eq!(
            run_captured(ok, "sh", Duration::from_secs(10))
                .unwrap()
                .trim(),
            "done"
        );
        let mut failing = Command::new("sh");
        failing.args(["-c", "echo progress; echo 'No package found' >&2; exit 3"]);
        let err = run_captured(failing, "winget", Duration::from_secs(10)).unwrap_err();
        assert!(
            err.starts_with("winget failed (exit code 0x3)") && err.contains("No package found"),
            "{err}"
        );
        let mut slow = Command::new("sh");
        slow.args(["-c", "sleep 5"]);
        assert!(run_captured(slow, "sh", Duration::from_millis(300))
            .unwrap_err()
            .contains("was stopped"));
    }

    #[test]
    fn hashes_files() {
        let path = std::env::temp_dir().join(format!("ws-sha-{}", std::process::id()));
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_hex(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn names_are_safe() {
        assert_eq!(safe_name("../../etc", "document"), "etc");
        assert_eq!(safe_name("My Thesis (final)", "document"), "MyThesisfinal");
        assert_eq!(safe_name("…", "document"), "document");
    }

    /// Needs TeX on the PATH: `cargo test compiles_with_tex -- --ignored`.
    #[test]
    #[ignore]
    fn compiles_with_tex() {
        let root = std::env::temp_dir().join(format!("ws-tex-test-{}", std::process::id()));
        let dir = root.join("doc");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("chapter.tex"),
            "\\section{Included}\\label{sec:inc}\nFrom a file beside the document.\n",
        )
        .unwrap();
        fs::write(dir.join("refs.bib"), "@book{knuth, author={Donald Knuth}, title={The TeXbook}, year={1984}, publisher={Addison-Wesley}}\n").unwrap();
        let source = "\\documentclass{article}\n\\begin{document}\nSee section~\\ref{sec:inc} and \\cite{knuth}.\n\\input{chapter}\n\\bibliographystyle{plain}\\bibliography{refs}\n\\end{document}\n";
        let (pdf, file, log) = compile(
            root.join("build"),
            "tab1",
            source,
            dir.to_str(),
            "pdflatex",
            "My Doc",
        )
        .unwrap();
        assert!(pdf.is_some_and(|p| p.starts_with(b"%PDF")), "{log}");
        assert!(file.ends_with("MyDoc.tex"));
        // References settled (a rerun) and the citation resolved (bibtex found refs.bib).
        assert!(!log.contains("undefined"), "{log}");
        assert!(
            !dir.join("MyDoc.aux").exists(),
            "nothing is written beside the document"
        );

        let (pdf, _, log) = compile(root.join("build"), "tab1", "\\documentclass{article}\n\\begin{document}\nText.\n\\nosuchcommand\n\\end{document}\n", dir.to_str(), "pdflatex", "My Doc").unwrap();
        assert!(log.contains(":4: Undefined control sequence"), "{log}");
        assert!(pdf.is_some());

        let (pdf, _, _) = compile(
            root.join("build"),
            "tab1",
            "\\documentclass{article}\n\\begin{document}\n\\input{missing}\n",
            dir.to_str(),
            "pdflatex",
            "My Doc",
        )
        .unwrap();
        assert!(
            pdf.is_none(),
            "a fatal error makes no PDF, and the last one isn't handed back"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sees_reruns() {
        assert!(needs_rerun(
            "LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right."
        ));
        assert!(!needs_rerun("Output written on main.pdf (1 page)."));
    }
}
