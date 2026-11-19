//! The latest GitHub release, and a macOS relaunch that replaces this process
//! instead of opening a second copy of the app.

use serde::Serialize;

const LATEST_RELEASE: &str = "https://api.github.com/repos/elijahroizman/writan/releases/latest";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNotes {
    version: String,
    name: String,
    notes: String,
    url: String,
    published_at: Option<String>,
}

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
}

fn version_from_tag(tag: &str) -> String {
    match tag.strip_prefix('v') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_string(),
        _ => tag.to_string(),
    }
}

fn notes_from_release(release: GithubRelease) -> ReleaseNotes {
    let version = version_from_tag(&release.tag_name);
    let name = release
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Writan {version}"));
    ReleaseNotes {
        version,
        name,
        notes: release.body.unwrap_or_default(),
        url: release.html_url,
        published_at: release.published_at,
    }
}

/// The latest GitHub release: version, notes and page. This is the changelog the
/// update panel shows, including when this copy is already current (the updater
/// feed then returns nothing). Off the main thread, which a slow network would freeze.
#[tauri::command]
pub async fn latest_release() -> Result<ReleaseNotes, String> {
    tauri::async_runtime::spawn_blocking(fetch_latest_release)
        .await
        .map_err(|e| e.to_string())?
}

fn fetch_latest_release() -> Result<ReleaseNotes, String> {
    let mut response = crate::files::http_agent()
        .get(LATEST_RELEASE)
        .header("User-Agent", "Writan")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|_| "Couldn't reach GitHub. Check your connection and try again.".to_string())?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(200 * 1024)
        .read_to_vec()
        .map_err(|_| "Couldn't read the release notes.".to_string())?;
    let release: GithubRelease = serde_json::from_slice(&bytes)
        .map_err(|_| "Couldn't read the release notes.".to_string())?;
    Ok(notes_from_release(release))
}

/// Where the .deb's install script puts Writan's APT repository (src-tauri/linux/postinst).
/// The old path is what packages wrote before the rename. Upgrade moves it, and a file that
/// has not moved yet still counts.
#[cfg(target_os = "linux")]
const APT_SOURCES: &str = "/etc/apt/sources.list.d/writan.list";
#[cfg(target_os = "linux")]
const APT_SOURCES_OLD: &str = "/etc/apt/sources.list.d/workspace.list";

/// Where the .rpm's install script puts Writan's dnf repository (src-tauri/linux/rpm-postinst).
#[cfg(target_os = "linux")]
const DNF_REPO: &str = "/etc/yum.repos.d/writan.repo";
#[cfg(target_os = "linux")]
const DNF_REPO_OLD: &str = "/etc/yum.repos.d/workspace.repo";

/// Prefer the renamed repository file, while accepting an installation not yet migrated.
#[cfg(target_os = "linux")]
fn existing_file<'a>(primary: &'a str, legacy: &'a str) -> &'a str {
    if std::path::Path::new(primary).is_file() {
        primary
    } else {
        legacy
    }
}

/// How this Linux copy was installed: `format` is "deb" or "rpm" (none for a build run from
/// source), and `repo` says Writan's own repository is there and on, so the package manager
/// can install new versions.
#[derive(Serialize)]
pub struct LinuxPackage {
    format: Option<&'static str>,
    repo: bool,
}

/// Query one real package name; a missing command or package is treated as not installed.
#[cfg(target_os = "linux")]
fn query_installed(format: &str, name: &str) -> Option<String> {
    let output = match format {
        "deb" => std::process::Command::new("dpkg-query")
            .args(["-W", "-f=${Version}", name])
            .output(),
        _ => std::process::Command::new("rpm")
            .args(["-q", "--qf", "%{VERSION}-%{RELEASE}", name])
            .output(),
    };
    output
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The installed package version. `writan` is the package name. `workspace` is the previous name,
/// still present until dnf obsoletes it or apt moves it to the empty transitional package.
#[cfg(target_os = "linux")]
fn installed_version(format: &str) -> Option<String> {
    query_installed(format, "writan").or_else(|| query_installed(format, "workspace"))
}

/// Whether the dnf repo file has Writan's section switched on (`enabled=1`, the default).
#[cfg(target_os = "linux")]
fn dnf_repo_enabled(repo: &str) -> bool {
    let mut in_section = false;
    let mut enabled = true;
    let mut found = false;
    for line in repo.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == "[writan]" || line == "[workspace]";
            found |= in_section;
        } else if in_section {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "enabled" {
                    enabled = matches!(value.trim(), "1" | "true" | "yes" | "True");
                }
            }
        }
    }
    found && enabled
}

#[cfg(target_os = "linux")]
fn linux_package_now() -> LinuxPackage {
    use std::path::Path;
    // The binary's path says which package put it there: dpkg and rpm both list their files.
    let exe = std::env::current_exe().ok();
    let owned_by = |cmd: &str, args: &[&str]| {
        exe.as_ref().is_some_and(|exe| {
            std::process::Command::new(cmd)
                .args(args)
                .arg(exe)
                .output()
                .is_ok_and(|o| o.status.success())
        })
    };
    if owned_by("dpkg-query", &["-S"]) {
        let repo = std::fs::read_to_string(existing_file(APT_SOURCES, APT_SOURCES_OLD))
            .is_ok_and(|list| list.lines().any(|l| l.trim_start().starts_with("deb ")));
        LinuxPackage {
            format: Some("deb"),
            repo: repo && Path::new("/usr/bin/apt-get").is_file(),
        }
    } else if owned_by("rpm", &["-qf"]) {
        let repo = std::fs::read_to_string(existing_file(DNF_REPO, DNF_REPO_OLD))
            .is_ok_and(|repo| dnf_repo_enabled(&repo));
        LinuxPackage {
            format: Some("rpm"),
            repo: repo && Path::new("/usr/bin/dnf").is_file(),
        }
    } else {
        LinuxPackage {
            format: None,
            repo: false,
        }
    }
}

/// How this copy was installed on Linux, and whether apt or dnf can update it.
#[tauri::command]
pub async fn linux_package() -> LinuxPackage {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(linux_package_now)
            .await
            .unwrap_or(LinuxPackage {
                format: None,
                repo: false,
            })
    }
    #[cfg(not(target_os = "linux"))]
    {
        LinuxPackage {
            format: None,
            repo: false,
        }
    }
}

/// Installs the newest Writan from its APT or dnf repository, after polkit asks for the
/// password. Only Writan's own repository is refreshed, so a broken or slow mirror elsewhere
/// can't stop it.
#[tauri::command]
pub async fn package_upgrade() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(|| {
            let package = linux_package_now();
            let (format, line, what) = match package.format {
                Some("deb") if package.repo => {
                    let sources = existing_file(APT_SOURCES, APT_SOURCES_OLD);
                    (
                        "deb",
                        format!(
                            "export DEBIAN_FRONTEND=noninteractive; apt-get update -o Dir::Etc::sourcelist={sources} -o Dir::Etc::sourceparts=- -o APT::Get::List-Cleanup=0 && apt-get install -y writan"
                        ),
                        "APT",
                    )
                }
                // -y also accepts the repository's key the first time dnf sees it.
                // The section id was `workspace` before the rename. The file that is on disk decides it.
                Some("rpm") if package.repo => {
                    let repo_id = if std::path::Path::new(DNF_REPO).is_file() { "writan" } else { "workspace" };
                    ("rpm", format!("dnf -y --refresh --repo={repo_id} upgrade writan"), "dnf")
                }
                _ => return Err("This copy of Writan didn't come with its update repository.".into()),
            };
            let before = installed_version(format);
            crate::tex::pkexec(&line, "Updating Writan")?;
            // apt and dnf succeed without doing anything when the repository hasn't got the
            // release yet (the Linux build finishes after the Mac and Windows ones).
            if before.is_some() && installed_version(format) == before {
                return Err(format!("The new version isn't in Writan's {what} repository yet. Try again in a few minutes."));
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("Updates through apt or dnf are for Linux.".into())
    }
}

/// Quote `path` as one shell word.
#[cfg(any(target_os = "macos", test))]
fn sh_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// Quit, then open this same bundle once the process is gone.
///
/// Tauri's relaunch runs `open -n` while this process is still alive. `-n` means
/// a new instance, so macOS shows a second window and a second Dock icon (the
/// running app and the one just opened). The helper waits until this process has
/// exited, then `open`s the bundle without `-n`. Launch Services starts one copy
/// in the same Dock slot, and the window-state plugin has already saved the frame
/// on `RunEvent::Exit`.
#[tauri::command]
pub fn relaunch_in_place(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        relaunch_macos(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("This restart is only used on macOS.".into())
    }
}

#[cfg(target_os = "macos")]
fn relaunch_macos(app: tauri::AppHandle) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    // The path recorded at launch. After the updater swaps the bundle, this path
    // is the new app; resolving `current_exe` again can follow the old inode into
    // the temp directory the previous copy was moved to.
    let bundle = launch_bundle().ok_or("Writan is not running from an app bundle.")?;
    let pid = std::process::id();
    // A short pause after the process dies lets Launch Services drop the old
    // registration. Opening any sooner focuses the copy that is still quitting.
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.15; done; sleep 0.5; exec /usr/bin/open {}",
        sh_quote(&bundle)
    );

    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Its own process group, so nothing sent to ours on the way out reaches it.
    // The shell then `exec`s `open`, which asks Launch Services to start the app.
    cmd.process_group(0);
    cmd.spawn()
        .map_err(|_| "Couldn't reopen Writan.".to_string())?;
    app.exit(0);
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_bundle() -> Option<std::path::PathBuf> {
    LAUNCH_BUNDLE.get().cloned()
}

#[cfg(target_os = "macos")]
static LAUNCH_BUNDLE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Remember the bundle we were launched from, before an update swaps it aside.
#[cfg(target_os = "macos")]
pub fn remember_bundle() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bundle) = crate::launch_services::bundle_app_dir(&exe) {
            let _ = LAUNCH_BUNDLE.set(bundle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_version_prefix() {
        assert_eq!(version_from_tag("v0.3.0"), "0.3.0");
        assert_eq!(version_from_tag("0.3.0"), "0.3.0");
        assert_eq!(version_from_tag("nightly"), "nightly");
    }

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(
            sh_quote(std::path::Path::new("/Applications/Writan.app")),
            "'/Applications/Writan.app'"
        );
        assert_eq!(
            sh_quote(std::path::Path::new("/tmp/O'Hare.app")),
            "'/tmp/O'\\''Hare.app'"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_the_dnf_repo_switch() {
        let repo = "[writan]\nname=Writan\nbaseurl=https://example.com/\nenabled=1\n";
        assert!(dnf_repo_enabled(repo));
        assert!(!dnf_repo_enabled(&repo.replace("enabled=1", "enabled=0")));
        assert!(dnf_repo_enabled("[workspace]\nname=Writan\nenabled=1\n"));
        assert!(!dnf_repo_enabled("[other]\nenabled=1\n"));
        assert!(!dnf_repo_enabled(
            "[workspace]\nenabled=0\n[other]\nenabled=1\n"
        ));
    }

    #[test]
    fn reads_release_notes() {
        let release = GithubRelease {
            tag_name: "v0.4.0".into(),
            name: Some("Writan v0.4.0".into()),
            body: Some("## Changes\n\n- Fixes the Dock icon".into()),
            html_url: "https://github.com/elijahroizman/writan/releases/tag/v0.4.0".into(),
            published_at: Some("2026-10-16T00:00:00Z".into()),
        };
        let notes = notes_from_release(release);
        assert_eq!(notes.version, "0.4.0");
        assert!(notes.notes.contains("Dock icon"));
        assert_eq!(notes.published_at.as_deref(), Some("2026-10-16T00:00:00Z"));
    }
}
