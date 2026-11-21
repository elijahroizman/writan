//! The GitHub releases' notes, and a macOS relaunch that replaces this process
//! instead of opening a second copy of the app.

use serde::Serialize;
use std::cmp::Ordering;

const RELEASES: &str = "https://api.github.com/repos/elijahroizman/writan/releases?per_page=50";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNotes {
    version: String,
    name: String,
    notes: String,
    url: String,
    published_at: Option<String>,
    /// Every release newer than this copy, newest first. Empty when this copy is current.
    releases: Vec<ReleaseEntry>,
    /// The missed-version list is incomplete: the page cap was hit or a later page failed.
    /// Still older missed versions are on GitHub.
    truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseEntry {
    version: String,
    notes: String,
    url: String,
}

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

fn version_from_tag(tag: &str) -> String {
    match tag.strip_prefix('v') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_string(),
        _ => tag.to_string(),
    }
}

/// The release's version, for real releases only. Drafts never reach the API; pre-releases are
/// the pacman `arch/*` repositories, and a tag that isn't a version isn't one either.
fn real_version(release: &GithubRelease) -> Option<String> {
    if release.draft || release.prerelease {
        return None;
    }
    let version = version_from_tag(&release.tag_name);
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then_some(version)
}

/// Orders versions (both like 0.7.2); a missing part counts as 0, so 0.11 is 0.11.0.
fn version_cmp(a: &str, b: &str) -> Ordering {
    let pa: Vec<u64> = a
        .split(['.', '-'])
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    let pb: Vec<u64> = b
        .split(['.', '-'])
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    for i in 0..pa.len().max(pb.len()) {
        match pa
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&pb.get(i).copied().unwrap_or(0))
        {
            Ordering::Equal => {}
            order => return order,
        }
    }
    Ordering::Equal
}

/// GitHub's `rel="next"` pagination link, if there is another page.
fn next_page(link: Option<&str>) -> Option<String> {
    link?.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        (rel.trim() == "rel=\"next\"").then(|| url.trim().trim_matches(['<', '>']).to_string())
    })
}

fn release_name(release: &GithubRelease, version: &str) -> String {
    match release.name.as_deref() {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => format!("Writan {version}"),
    }
}

// The release workflow ends every release's body with this pointer to CHANGELOG.md and the
// install instructions. In a list of missed versions each entry links to GitHub instead.
fn strip_trailer(body: &str) -> String {
    body.lines()
        .filter(|line| !line.trim_start().starts_with("Earlier versions:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// The GitHub releases' notes: the latest release's own notes plus `releases`, every release
/// newer than this copy, which the update panel lists under per-version headings. This is the
/// changelog the panel shows, including when this copy is already current (the updater feed
/// then returns nothing). Off the main thread, which a slow network would freeze.
#[tauri::command]
pub async fn release_notes(app: tauri::AppHandle) -> Result<ReleaseNotes, String> {
    let current = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || fetch_release_notes(&current))
        .await
        .map_err(|e| e.to_string())?
}

fn fetch_page(url: &str) -> Result<(Vec<GithubRelease>, Option<String>), String> {
    let mut response = crate::files::http_agent()
        .get(url)
        .header("User-Agent", "Writan")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|_| "Couldn't reach GitHub. Check your connection and try again.".to_string())?;
    let next = next_page(response.headers().get("link").and_then(|v| v.to_str().ok()));
    // A 50-release page is under ~1.5 MB at the sizes GitHub sends today; asset metadata can
    // grow, so the limit leaves room rather than sitting just above the current response.
    let bytes = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_vec()
        .map_err(|_| "Couldn't read the release notes.".to_string())?;
    let page = serde_json::from_slice(&bytes)
        .map_err(|_| "Couldn't read the release notes.".to_string())?;
    Ok((page, next))
}

fn fetch_release_notes(current: &str) -> Result<ReleaseNotes, String> {
    let mut releases = Vec::new();
    let mut url = Some(RELEASES.to_string());
    // The list runs newest first but not by version: a maintenance release for an old version
    // lands on the first page, so every page is read. Ten pages of 50 is a bound, not a filter;
    // `truncated` says it was hit.
    for _ in 0..10 {
        let Some(page_url) = url.clone() else { break };
        match fetch_page(&page_url) {
            Ok((page, next)) => {
                releases.extend(page);
                url = next;
            }
            // A later page can fail without losing the releases already fetched. Leaving `url`
            // set marks the list truncated.
            Err(e) if releases.is_empty() => return Err(e),
            Err(_) => break,
        }
    }
    let truncated = url.is_some();
    notes_from_list(current, &releases, truncated)
        .ok_or_else(|| "Couldn't read the release notes.".to_string())
}

fn notes_from_list(
    current: &str,
    releases: &[GithubRelease],
    truncated: bool,
) -> Option<ReleaseNotes> {
    let mut versions: Vec<(String, &GithubRelease)> = releases
        .iter()
        .filter_map(|r| real_version(r).map(|v| (v, r)))
        .collect();
    versions.sort_by(|a, b| version_cmp(&b.0, &a.0));
    let (latest_version, latest) = versions.first()?;
    Some(ReleaseNotes {
        version: latest_version.clone(),
        name: release_name(latest, latest_version),
        notes: latest.body.clone().unwrap_or_default(),
        url: latest.html_url.clone(),
        published_at: latest.published_at.clone(),
        releases: versions
            .iter()
            .filter(|(version, _)| version_cmp(version, current).is_gt())
            .map(|(version, release)| ReleaseEntry {
                version: version.clone(),
                notes: strip_trailer(release.body.as_deref().unwrap_or_default()),
                url: release.html_url.clone(),
            })
            .collect(),
        truncated,
    })
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

/// Where the Arch package's install script adds Writan's pacman repository (scripts/arch-package.sh).
#[cfg(target_os = "linux")]
const PACMAN_CONF: &str = "/etc/pacman.conf";

/// The pacman repository: one release per architecture, holding the newest package and its signature.
#[cfg(target_os = "linux")]
const PACMAN_REPO: &str = "https://github.com/elijahroizman/writan/releases/download/arch";

/// Prefer the renamed repository file, while accepting an installation not yet migrated.
#[cfg(target_os = "linux")]
fn existing_file<'a>(primary: &'a str, legacy: &'a str) -> &'a str {
    if std::path::Path::new(primary).is_file() {
        primary
    } else {
        legacy
    }
}

/// How this Linux copy was installed: `format` is "deb", "rpm" or "pacman" (none for a build run from
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
        "pacman" => std::process::Command::new("pacman")
            .args(["-Q", name])
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

/// The installed package version (for pacman, `writan <version>`). `writan` is the package name.
/// `workspace` is the previous name, still present until dnf obsoletes it or apt moves it to the
/// empty transitional package. The Arch package was never called that.
#[cfg(target_os = "linux")]
fn installed_version(format: &str) -> Option<String> {
    if format == "pacman" {
        return query_installed(format, "writan");
    }
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

/// Whether pacman.conf has Writan's `[writan]` section, not commented out. Removing the package
/// takes the section out again, and commenting it out is how updates are turned off.
#[cfg(target_os = "linux")]
fn pacman_repo_enabled(conf: &str) -> bool {
    conf.lines().any(|line| line.trim() == "[writan]")
}

/// The Arch package file for a release, in the pacman repository. Only plain `x.y.z` versions:
/// the version goes into the command pkexec runs, and pre-releases aren't published there.
#[cfg(target_os = "linux")]
fn pacman_package_url(version: &str, arch: &str) -> Option<String> {
    let parts: Vec<&str> = version.split('.').collect();
    let plain = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let arch = match arch {
        "x86_64" | "aarch64" => arch,
        _ => return None,
    };
    plain.then(|| format!("{PACMAN_REPO}/{arch}/writan-{version}-1-{arch}.pkg.tar.zst"))
}

#[cfg(target_os = "linux")]
fn linux_package_now() -> LinuxPackage {
    use std::path::Path;
    // The binary's path says which package put it there: dpkg, rpm and pacman all list their files.
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
    } else if owned_by("pacman", &["-Qqo"]) {
        let repo =
            std::fs::read_to_string(PACMAN_CONF).is_ok_and(|conf| pacman_repo_enabled(&conf));
        LinuxPackage {
            format: Some("pacman"),
            repo: repo && Path::new("/usr/bin/pacman").is_file(),
        }
    } else {
        LinuxPackage {
            format: None,
            repo: false,
        }
    }
}

/// How this copy was installed on Linux, and whether apt, dnf or pacman can update it.
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

/// Installs the newest Writan from its APT, dnf or pacman repository, after polkit asks for the
/// password. Only Writan's own repository is read, so a broken or slow mirror elsewhere can't
/// stop it.
///
/// pacman has no way to refresh one repository, and `pacman -Sy writan` would leave the rest of an
/// Arch system partly upgraded, so it installs `version`'s package file from Writan's repository
/// with `pacman -U`, which checks its signature against the key the package added to pacman's
/// keyring. `pacman -Syu` brings the same package with the system's other updates.
#[tauri::command]
pub async fn package_upgrade(version: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(move || {
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
                Some("pacman") if package.repo => {
                    let url = version
                        .as_deref()
                        .and_then(|v| pacman_package_url(v, std::env::consts::ARCH))
                        .ok_or("pacman can't install this version from Writan's repository.")?;
                    ("pacman", format!("pacman -U --noconfirm {url}"), "pacman")
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
        let _ = version;
        Err("Updates through apt, dnf or pacman are for Linux.".into())
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
    fn reads_the_pacman_repo_section() {
        assert!(pacman_repo_enabled(
            "[core]\nInclude = x\n\n[writan]\nSigLevel = Required\n"
        ));
        assert!(pacman_repo_enabled("  [writan]  \n"));
        assert!(!pacman_repo_enabled("#[writan]\n#Server = x\n"));
        assert!(!pacman_repo_enabled("[core]\n"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn builds_the_pacman_package_url() {
        assert_eq!(
            pacman_package_url("0.10.11", "aarch64").as_deref(),
            Some("https://github.com/elijahroizman/writan/releases/download/arch/aarch64/writan-0.10.11-1-aarch64.pkg.tar.zst")
        );
        assert!(pacman_package_url("0.10.11", "x86_64").is_some());
        assert!(pacman_package_url("0.11.0-beta.1", "x86_64").is_none());
        assert!(pacman_package_url("1.2.3; rm -rf /", "x86_64").is_none());
        assert!(pacman_package_url("1..3", "x86_64").is_none());
        assert!(pacman_package_url("1.2.3", "riscv64").is_none());
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

    fn release(tag: &str, name: &str, body: &str, prerelease: bool) -> GithubRelease {
        GithubRelease {
            tag_name: tag.into(),
            name: Some(name.into()),
            body: Some(body.into()),
            html_url: format!("https://github.com/elijahroizman/writan/releases/tag/{tag}"),
            published_at: Some("2026-10-16T00:00:00Z".into()),
            draft: false,
            prerelease,
        }
    }

    #[test]
    fn orders_versions() {
        assert_eq!(version_cmp("0.11.4", "0.11.3"), Ordering::Greater);
        assert_eq!(version_cmp("0.11.4", "0.11.4"), Ordering::Equal);
        assert_eq!(version_cmp("0.9.9", "0.11.0"), Ordering::Less);
        assert_eq!(version_cmp("0.11", "0.11.0"), Ordering::Equal);
    }

    #[test]
    fn lists_the_missed_versions() {
        let releases = vec![
            release("arch/x86_64", "pacman repository (x86_64)", "repo", true),
            release("docs", "Documentation", "not a version", false),
            release(
                "v0.11.2",
                "Writan v0.11.2",
                "- B\n\nEarlier versions: [CHANGELOG.md](x).",
                false,
            ),
            release(
                "v0.11.4",
                "Writan v0.11.4",
                "- A\n\nEarlier versions: [CHANGELOG.md](x).",
                false,
            ),
            release(
                "v0.11.3",
                "Writan v0.11.3",
                "- C\n\nEarlier versions: [CHANGELOG.md](x).",
                false,
            ),
        ];
        let notes = notes_from_list("0.11.2", &releases, false).unwrap();
        assert_eq!(notes.version, "0.11.4");
        assert_eq!(notes.name, "Writan v0.11.4");
        assert!(notes.notes.contains("Earlier versions:"));
        let missed: Vec<&str> = notes.releases.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(missed, ["0.11.4", "0.11.3"]);
        assert!(!notes.releases[0].notes.contains("Earlier versions:"));
        assert!(notes.releases[0].notes.contains("- A"));
    }

    #[test]
    fn no_missed_versions_when_current() {
        let releases = vec![release("v0.11.4", "Writan v0.11.4", "- A", false)];
        let notes = notes_from_list("0.11.4", &releases, false).unwrap();
        assert!(notes.releases.is_empty());
        assert!(notes_from_list("0.11.4", &[], false).is_none());
    }

    #[test]
    fn reads_the_next_page_link() {
        let link = concat!(
            "<https://api.github.com/x?page=2>; rel=\"next\", ",
            "<https://api.github.com/x?page=9>; rel=\"last\""
        );
        assert_eq!(
            next_page(Some(link)).as_deref(),
            Some("https://api.github.com/x?page=2")
        );
        assert_eq!(
            next_page(Some("<https://api.github.com/x?page=1>; rel=\"prev\"")),
            None
        );
        assert_eq!(next_page(None), None);
    }
}
