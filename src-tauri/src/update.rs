//! Update checks on the stable or nightly channel, the GitHub releases' notes, installs through
//! apt, dnf or pacman on Linux, and a macOS relaunch that replaces this process instead of
//! opening a second copy of the app.

use serde::Serialize;
use std::cmp::Ordering;

const RELEASES: &str = "https://api.github.com/repos/elijahroizman/writan/releases?per_page=50";

/// The nightly update feed, in the assets of the pre-release tagged `nightly`
/// (.github/workflows/nightly.yml). The stable feed is `plugins.updater.endpoints` in
/// tauri.conf.json. The Release workflow writes a new stable release to both.
const NIGHTLY_FEED: &str =
    "https://github.com/elijahroizman/writan/releases/download/nightly/latest.json";

/// Where a nightly's files are: the pre-release tagged `v<version>`.
#[cfg(any(target_os = "linux", test))]
const DOWNLOADS: &str = "https://github.com/elijahroizman/writan/releases/download";

/// Which releases this copy follows. The app keeps the choice (src/lib/updateChannel.ts).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Channel {
    Stable,
    Nightly,
}

impl Channel {
    fn parse(channel: Option<&str>) -> Self {
        match channel {
            Some("nightly") => Channel::Nightly,
            _ => Channel::Stable,
        }
    }
}

/// The date and time part of a nightly version (`1.0.1-nightly.202611280930`), or `None` for
/// any other version. Exactly that shape: the version goes into URLs and a root shell line.
fn nightly_stamp(version: &str) -> Option<(&str, &str)> {
    let (base, stamp) = version.split_once("-nightly.")?;
    let parts: Vec<&str> = base.split('.').collect();
    let plain = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    (plain && stamp.len() == 12 && stamp.bytes().all(|b| b.is_ascii_digit()))
        .then_some((base, stamp))
}

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

/// The release's version, for the releases `channel` follows. Drafts never reach the API. The
/// other pre-releases are the APT, dnf and pacman repositories, the nightly feed, plugins and
/// old dev builds, whose tags aren't versions, and the nightlies, which only the nightly channel
/// lists.
fn real_version(release: &GithubRelease, channel: Channel) -> Option<String> {
    if release.draft {
        return None;
    }
    let version = version_from_tag(&release.tag_name);
    if !version.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let nightly = nightly_stamp(&version).is_some();
    let listed = if release.prerelease {
        nightly && channel == Channel::Nightly
    } else {
        !nightly
    };
    listed.then_some(version)
}

/// Orders versions the way SemVer does: 0.7.2 is older than 0.7.10, and a pre-release such as
/// 1.0.1-nightly.202611280930 is older than 1.0.1 and newer than 1.0.0. A missing part of the
/// release counts as 0, so 0.11 is 0.11.0. Build metadata (`+…`) is ignored.
fn version_cmp(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.split_once('+').map_or(v, |(v, _)| v);
        let (release, pre) = match v.split_once('-') {
            Some((release, pre)) => (release, Some(pre)),
            None => (v, None),
        };
        (
            release.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            pre,
        )
    }
    let (ra, pa) = split(a);
    let (rb, pb) = split(b);
    for i in 0..ra.len().max(rb.len()) {
        match ra
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&rb.get(i).copied().unwrap_or(0))
        {
            Ordering::Equal => {}
            order => return order,
        }
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(pa), Some(pb)) => {
            let (mut ia, mut ib) = (pa.split('.'), pb.split('.'));
            loop {
                match (ia.next(), ib.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(x), Some(y)) => {
                        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            // Numeric identifiers sort before alphanumeric ones.
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => x.cmp(y),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                }
            }
        }
    }
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
/// then returns nothing). On the nightly channel the nightlies count as releases. Off the main
/// thread, which a slow network would freeze.
#[tauri::command]
pub async fn release_notes(
    app: tauri::AppHandle,
    channel: Option<String>,
) -> Result<ReleaseNotes, String> {
    let current = app.package_info().version.to_string();
    let channel = Channel::parse(channel.as_deref());
    tauri::async_runtime::spawn_blocking(move || fetch_release_notes(&current, channel))
        .await
        .map_err(|e| e.to_string())?
}

/// What the updater plugin's JS `Update` class is made from: the plugin's own `check` command
/// returns the same fields, with the `Update` put in the webview's resource table under `rid`.
/// The plugin's `download`, `install` and `close` then find it there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadata {
    rid: tauri::ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    raw_json: serde_json::Value,
}

/// Checks the channel's update feed. The plugin's JS `check()` always reads the feed in
/// tauri.conf.json, so this builds the updater with the nightly feed when that's the channel.
/// Either way the plugin installs only a version newer than this copy, so moving from nightly to
/// stable waits for the next stable release newer than the installed nightly.
#[tauri::command]
pub async fn check_update<R: tauri::Runtime>(
    webview: tauri::Webview<R>,
    channel: Option<String>,
    timeout: Option<u64>,
) -> Result<Option<UpdateMetadata>, String> {
    use tauri::Manager;
    use tauri_plugin_updater::UpdaterExt;
    let mut builder = webview.updater_builder();
    if Channel::parse(channel.as_deref()) == Channel::Nightly {
        let feed = NIGHTLY_FEED
            .parse()
            .map_err(|_| "Bad update feed".to_string())?;
        builder = builder.endpoints(vec![feed]).map_err(|e| e.to_string())?;
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(std::time::Duration::from_millis(timeout));
    }
    let updater = builder.build().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let date = update.date.and_then(|d| {
        d.format(&time::format_description::well_known::Rfc3339)
            .ok()
    });
    Ok(Some(UpdateMetadata {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        date,
        body: update.body.clone(),
        raw_json: update.raw_json.clone(),
        rid: webview.resources_table().add(update),
    }))
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

fn fetch_release_notes(current: &str, channel: Channel) -> Result<ReleaseNotes, String> {
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
    notes_from_list(current, &releases, truncated, channel)
        .ok_or_else(|| "Couldn't read the release notes.".to_string())
}

fn notes_from_list(
    current: &str,
    releases: &[GithubRelease],
    truncated: bool,
    channel: Channel,
) -> Option<ReleaseNotes> {
    let mut versions: Vec<(String, &GithubRelease)> = releases
        .iter()
        .filter_map(|r| real_version(r, channel).map(|v| (v, r)))
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

/// Writan's signing key as the Arch package installs it (scripts/arch-package.sh), for
/// `pacman-key --populate`. pacman-install.sh checks every package against this key alone.
#[cfg(target_os = "linux")]
const PACMAN_KEYRING: &str = "/usr/share/pacman/keyrings/writan.gpg";

/// The root shell script that downloads an Arch package and its signature, checks them against
/// Writan's key with gpgv, and installs the file with `pacman -U`.
#[cfg(any(target_os = "linux", test))]
const PACMAN_INSTALL: &str = include_str!("../linux/pacman-install.sh");

/// The command pkexec runs to install the Arch package at `url`, checked against `keyring`.
/// `pacman -U <url>` checks a download's signature at pacman.conf's RemoteFileSigLevel, which can
/// be Optional, and against any key pacman trusts, so the script checks it itself first.
#[cfg(any(target_os = "linux", test))]
fn pacman_install_line(url: &str, keyring: &std::path::Path) -> String {
    format!(
        "set -- {} {}\n{PACMAN_INSTALL}",
        sh_quote(std::path::Path::new(url)),
        sh_quote(keyring)
    )
}

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
/// source), `manager` says apt, dnf or pacman is there, and `repo` says Writan's own repository
/// is there and on as well, so the package manager can install new stable versions. Nightlies
/// install from their release by URL and need only `manager`.
#[derive(Serialize)]
pub struct LinuxPackage {
    format: Option<&'static str>,
    manager: bool,
    repo: bool,
}

const NO_PACKAGE: LinuxPackage = LinuxPackage {
    format: None,
    manager: false,
    repo: false,
};

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

/// The nightly's package file on its pre-release, for this format and machine:
/// `Writan_1.0.1-nightly.202611280930_amd64.deb`, `Writan-1.0.1-0.nightly.202611280930.x86_64.rpm`
/// or `writan-1.0.1nightly.202611280930-1-x86_64.pkg.tar.zst`. The versions inside the packages
/// differ, since dpkg, rpm and pacman each order pre-releases their own way, and each sorts the
/// nightly before the stable 1.0.1 (docs/internals/releasing.md). `None` for anything that isn't a
/// nightly version on x86_64 or aarch64.
#[cfg(any(target_os = "linux", test))]
fn nightly_package_url(version: &str, format: &str, arch: &str) -> Option<String> {
    let (base, stamp) = nightly_stamp(version)?;
    let file = match (format, arch) {
        ("deb", "x86_64") => format!("Writan_{version}_amd64.deb"),
        ("deb", "aarch64") => format!("Writan_{version}_arm64.deb"),
        ("rpm", "x86_64" | "aarch64") => format!("Writan-{base}-0.nightly.{stamp}.{arch}.rpm"),
        ("pacman", "x86_64" | "aarch64") => {
            format!("writan-{base}nightly.{stamp}-1-{arch}.pkg.tar.zst")
        }
        _ => return None,
    };
    Some(format!("{DOWNLOADS}/v{version}/{file}"))
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
        let manager = Path::new("/usr/bin/apt-get").is_file();
        LinuxPackage {
            format: Some("deb"),
            manager,
            repo: repo && manager,
        }
    } else if owned_by("rpm", &["-qf"]) {
        let repo = std::fs::read_to_string(existing_file(DNF_REPO, DNF_REPO_OLD))
            .is_ok_and(|repo| dnf_repo_enabled(&repo));
        let manager = Path::new("/usr/bin/dnf").is_file();
        LinuxPackage {
            format: Some("rpm"),
            manager,
            repo: repo && manager,
        }
    } else if owned_by("pacman", &["-Qqo"]) {
        let repo =
            std::fs::read_to_string(PACMAN_CONF).is_ok_and(|conf| pacman_repo_enabled(&conf));
        let manager = Path::new("/usr/bin/pacman").is_file();
        LinuxPackage {
            format: Some("pacman"),
            manager,
            repo: repo && manager,
        }
    } else {
        NO_PACKAGE
    }
}

/// How this copy was installed on Linux, and whether apt, dnf or pacman can update it.
#[tauri::command]
pub async fn linux_package() -> LinuxPackage {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(linux_package_now)
            .await
            .unwrap_or(NO_PACKAGE)
    }
    #[cfg(not(target_os = "linux"))]
    {
        NO_PACKAGE
    }
}

/// Installs `version` through apt, dnf or pacman, after polkit asks for the password.
///
/// A stable version comes from Writan's APT, dnf or pacman repository. Only Writan's own
/// repository is read, so a broken or slow mirror elsewhere can't stop it. pacman has no way to
/// refresh one repository, and `pacman -Sy writan` would leave the rest of an Arch system partly
/// upgraded, so it installs `version`'s package file from Writan's repository with `pacman -U`.
/// The root shell downloads the package and its signature first and checks them against Writan's
/// key alone (linux/pacman-install.sh), since pacman's own check of a URL follows
/// RemoteFileSigLevel, which can be Optional. `pacman -Syu` brings the same package with the
/// system's other updates.
///
/// A nightly comes from its pre-release, since the repositories only hold stable releases.
/// pacman installs it the same way, from beside its signature. For a .deb or .rpm
/// this downloads the package and checks its signature against the updater key (the one Mac and
/// Windows updates are checked against). The root shell copies the file somewhere only root can
/// write and checks its SHA-256 again before apt or dnf installs it, so nothing can swap it on
/// the way.
#[tauri::command]
pub async fn package_upgrade(app: tauri::AppHandle, version: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let pubkey = crate::plugins::updater_pubkey(&app)
            .map_err(|_| "This build can't check update signatures.".to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            let package = linux_package_now();
            let arch = std::env::consts::ARCH;
            let nightly = version.as_deref().filter(|v| nightly_stamp(v).is_some());
            // The download stays until the install is done.
            let mut _download = None;
            // `stale` is the error when the package manager succeeds but the version didn't change.
            let (format, line, stale) = match (package.format, nightly) {
                (Some(format @ ("deb" | "rpm")), Some(version)) if package.manager => {
                    let url = nightly_package_url(version, format, arch)
                        .ok_or("There's no nightly package for this machine.")?;
                    let (dir, path, sha256) = download_signed(&url, &pubkey)?;
                    _download = Some(dir);
                    let name = url.rsplit('/').next().unwrap_or_default();
                    let install = if format == "deb" {
                        "DEBIAN_FRONTEND=noninteractive apt-get install -y \"$d/$name\""
                    } else {
                        "dnf -y install \"$d/$name\""
                    };
                    (
                        format,
                        format!(
                            "set -e; name={}; d=$(mktemp -d); trap 'rm -rf \"$d\"' EXIT; chmod 755 \"$d\"; cp -- {} \"$d/$name\"; echo \"{sha256}  $d/$name\" | sha256sum -c --quiet; {install}",
                            sh_quote(std::path::Path::new(name)),
                            sh_quote(&path),
                        ),
                        "The nightly didn't install. Try again in a few minutes.",
                    )
                }
                (Some("pacman"), Some(version)) if package.manager => {
                    let url = nightly_package_url(version, "pacman", arch)
                        .ok_or("There's no nightly package for this machine.")?;
                    (
                        "pacman",
                        pacman_install_line(&url, std::path::Path::new(PACMAN_KEYRING)),
                        "The nightly didn't install. Try again in a few minutes.",
                    )
                }
                (Some("deb"), None) if package.repo => {
                    let sources = existing_file(APT_SOURCES, APT_SOURCES_OLD);
                    (
                        "deb",
                        format!(
                            "export DEBIAN_FRONTEND=noninteractive; apt-get update -o Dir::Etc::sourcelist={sources} -o Dir::Etc::sourceparts=- -o APT::Get::List-Cleanup=0 && apt-get install -y writan"
                        ),
                        "The new version isn't in Writan's APT repository yet. Try again in a few minutes.",
                    )
                }
                // -y also accepts the repository's key the first time dnf sees it.
                // The section id was `workspace` before the rename. The file that is on disk decides it.
                (Some("rpm"), None) if package.repo => {
                    let repo_id = if std::path::Path::new(DNF_REPO).is_file() { "writan" } else { "workspace" };
                    ("rpm", format!("dnf -y --refresh --repo={repo_id} upgrade writan"), "The new version isn't in Writan's dnf repository yet. Try again in a few minutes.")
                }
                (Some("pacman"), None) if package.repo => {
                    let url = version
                        .as_deref()
                        .and_then(|v| pacman_package_url(v, arch))
                        .ok_or("pacman can't install this version from Writan's repository.")?;
                    ("pacman", pacman_install_line(&url, std::path::Path::new(PACMAN_KEYRING)), "The new version isn't in Writan's pacman repository yet. Try again in a few minutes.")
                }
                _ => return Err("This copy of Writan didn't come with its update repository.".into()),
            };
            let before = installed_version(format);
            crate::tex::pkexec(&line, "Updating Writan")?;
            // apt and dnf succeed without doing anything when the repository hasn't got the
            // release yet (the Linux build finishes after the Mac and Windows ones).
            if before.is_some() && installed_version(format) == before {
                return Err(stale.into());
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, version);
        Err("Updates through apt, dnf or pacman are for Linux.".into())
    }
}

/// The largest nightly package Writan downloads. Today's are under 60 MB.
#[cfg(target_os = "linux")]
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;

/// Downloads a nightly package and the minisign signature beside it (`<file>.sig`), checks them
/// against the updater key, and saves the package in a new private directory. Returns the
/// directory, which removes itself when dropped, the file's path and its SHA-256.
#[cfg(target_os = "linux")]
fn download_signed(
    url: &str,
    pubkey: &str,
) -> Result<(tempfile::TempDir, std::path::PathBuf, String), String> {
    use sha2::Digest;
    use std::io::Read;
    let fetch = |url: &str, limit: u64| -> Result<Vec<u8>, String> {
        let failed = "Couldn't download the update. Check your connection and try again.";
        let mut response = crate::files::download_agent()
            .get(url)
            .header("User-Agent", "Writan")
            .call()
            .map_err(|_| failed.to_string())?;
        let mut bytes = Vec::new();
        response
            .body_mut()
            .with_config()
            .limit(limit)
            .reader()
            .read_to_end(&mut bytes)
            .map_err(|_| failed.to_string())?;
        Ok(bytes)
    };
    let signature = fetch(&format!("{url}.sig"), 64 * 1024)?;
    let package = fetch(url, MAX_PACKAGE)?;
    let name = url.rsplit('/').next().unwrap_or_default();
    crate::plugins::check_signature(&package, &signature, pubkey, name)
        .map_err(|_| "The update's signature doesn't match. It wasn't installed.".to_string())?;
    let dir = tempfile::Builder::new()
        .prefix("writan-update-")
        .tempdir()
        .map_err(|e| format!("Couldn't save the update: {e}"))?;
    let path = dir.path().join(name);
    std::fs::write(&path, &package).map_err(|e| format!("Couldn't save the update: {e}"))?;
    let sha256: String = sha2::Sha256::digest(&package)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((dir, path, sha256))
}

/// Quote `path` as one shell word.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
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
        assert_eq!(version_cmp("1.0.0+build.5", "1.0.0"), Ordering::Equal);
    }

    #[test]
    fn orders_nightlies_between_stable_releases() {
        let nightly = "1.0.1-nightly.202611280930";
        assert_eq!(version_cmp(nightly, "1.0.1"), Ordering::Less);
        assert_eq!(version_cmp("1.0.1", nightly), Ordering::Greater);
        assert_eq!(version_cmp(nightly, "1.0.0"), Ordering::Greater);
        assert_eq!(version_cmp(nightly, "0.19.0"), Ordering::Greater);
        assert_eq!(
            version_cmp(nightly, "1.0.1-nightly.202611281200"),
            Ordering::Less
        );
        assert_eq!(
            version_cmp("1.0.1-nightly.202612010000", "1.0.1-nightly.202611302359"),
            Ordering::Greater
        );
        assert_eq!(version_cmp(nightly, nightly), Ordering::Equal);
        // The old comparison read 1.0.1-nightly.x as 1.0.1.x, newer than 1.0.1.
        assert_eq!(version_cmp("1.0.1-nightly.1", "1.0.1.5"), Ordering::Less);
    }

    #[test]
    fn orders_prerelease_identifiers_like_semver() {
        // The example list from semver.org, oldest first.
        let order = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ];
        for pair in order.windows(2) {
            assert_eq!(version_cmp(pair[0], pair[1]), Ordering::Less, "{pair:?}");
            assert_eq!(version_cmp(pair[1], pair[0]), Ordering::Greater, "{pair:?}");
        }
    }

    #[test]
    fn reads_nightly_versions() {
        assert_eq!(
            nightly_stamp("1.0.1-nightly.202611280930"),
            Some(("1.0.1", "202611280930"))
        );
        assert_eq!(nightly_stamp("1.0.1"), None);
        assert_eq!(nightly_stamp("1.0.1-nightly.2026112809"), None);
        assert_eq!(nightly_stamp("1.0.1-nightly.20261128093x"), None);
        assert_eq!(nightly_stamp("1.0-nightly.202611280930"), None);
        assert_eq!(nightly_stamp("1.0.1-beta.1"), None);
        assert_eq!(nightly_stamp("1.0.1-nightly.202611280930;id"), None);
    }

    #[test]
    fn builds_the_nightly_package_urls() {
        let v = "1.0.1-nightly.202611280930";
        let base =
            "https://github.com/elijahroizman/writan/releases/download/v1.0.1-nightly.202611280930";
        assert_eq!(
            nightly_package_url(v, "deb", "x86_64").unwrap(),
            format!("{base}/Writan_1.0.1-nightly.202611280930_amd64.deb")
        );
        assert_eq!(
            nightly_package_url(v, "deb", "aarch64").unwrap(),
            format!("{base}/Writan_1.0.1-nightly.202611280930_arm64.deb")
        );
        assert_eq!(
            nightly_package_url(v, "rpm", "aarch64").unwrap(),
            format!("{base}/Writan-1.0.1-0.nightly.202611280930.aarch64.rpm")
        );
        assert_eq!(
            nightly_package_url(v, "pacman", "x86_64").unwrap(),
            format!("{base}/writan-1.0.1nightly.202611280930-1-x86_64.pkg.tar.zst")
        );
        assert!(nightly_package_url("1.0.1", "deb", "x86_64").is_none());
        assert!(nightly_package_url(v, "deb", "riscv64").is_none());
        assert!(nightly_package_url(v, "snap", "x86_64").is_none());
    }

    #[test]
    fn builds_the_pacman_install_command() {
        let line = pacman_install_line(
            "https://example.com/it's/writan-1.0.0-1-x86_64.pkg.tar.zst",
            std::path::Path::new("/usr/share/pacman/keyrings/writan.gpg"),
        );
        let (args, script) = line.split_once('\n').unwrap();
        assert_eq!(
            args,
            "set -- 'https://example.com/it'\\''s/writan-1.0.0-1-x86_64.pkg.tar.zst' '/usr/share/pacman/keyrings/writan.gpg'"
        );
        assert_eq!(script, PACMAN_INSTALL);
        // Never a URL straight to pacman, which would check it at RemoteFileSigLevel.
        assert!(!line.contains("pacman -U --noconfirm \"$url\""));
        assert!(line.contains("gpgv --homedir"));
    }

    /// Runs linux/pacman-install.sh with a stand-in for pacman, a throwaway Writan key and
    /// file:// URLs: it installs a package signed with that key, and refuses one with no
    /// signature, a signature of another file, or a good signature from a key that isn't Writan's.
    /// scripts/check-linux-packages.sh runs the same script on a real pacman. Needs gpg and curl.
    #[cfg(target_os = "linux")]
    #[test]
    fn pacman_install_checks_the_signature_against_writans_key() {
        use std::path::Path;
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let home = root.join("gnupg");
        std::fs::create_dir(&home).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let gpg = |args: &[&str]| {
            let status = Command::new("gpg")
                .arg("--homedir")
                .arg(&home)
                .args(["--batch", "--quiet", "--passphrase", ""])
                .args(args)
                .status()
                .expect("this test needs gpg");
            assert!(status.success(), "gpg {args:?} failed");
        };
        gpg(&[
            "--quick-gen-key",
            "Writan test <writan@example.com>",
            "ed25519",
            "sign",
            "never",
        ]);
        gpg(&[
            "--quick-gen-key",
            "Someone else <other@example.com>",
            "ed25519",
            "sign",
            "never",
        ]);
        let keyring = root.join("writan.gpg");
        gpg(&[
            "--armor",
            "--output",
            keyring.to_str().unwrap(),
            "--export",
            "writan@example.com",
        ]);
        let name = "writan-1.0.0-1-x86_64.pkg.tar.zst";
        let make = |case: &str, signer: Option<&str>, signed: &[u8]| {
            let case_dir = root.join(case);
            std::fs::create_dir(&case_dir).unwrap();
            let package = case_dir.join(name);
            std::fs::write(&package, b"the package").unwrap();
            if let Some(signer) = signer {
                let other = case_dir.join("signed");
                std::fs::write(&other, signed).unwrap();
                let sig = case_dir.join(format!("{name}.sig"));
                gpg(&[
                    "--local-user",
                    signer,
                    "--output",
                    sig.to_str().unwrap(),
                    "--detach-sign",
                    other.to_str().unwrap(),
                ]);
            }
            format!("file://{}", package.display())
        };
        let good = make("good", Some("writan@example.com"), b"the package");
        let unsigned = make("unsigned", None, b"");
        let wrong = make("wrong", Some("writan@example.com"), b"another package");
        let stranger = make("stranger", Some("other@example.com"), b"the package");
        let _ = Command::new("gpgconf")
            .arg("--homedir")
            .arg(&home)
            .args(["--kill", "gpg-agent"])
            .status();

        // pacman, standing in: it notes what it was asked to install.
        let bin = root.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let log = root.join("pacman.log");
        std::fs::write(
            bin.join("pacman"),
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$PACMAN_LOG\"\ncmp -s \"$3\" \"$PACKAGE\"\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("pacman"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let run = |url: &str| {
            let _ = std::fs::remove_file(&log);
            let output = Command::new("sh")
                .arg("-c")
                .arg(pacman_install_line(url, &keyring))
                .env("PATH", &path)
                .env("PACMAN_LOG", &log)
                .env("PACKAGE", root.join("good").join(name))
                .output()
                .unwrap();
            let installed = std::fs::read_to_string(&log).ok();
            (
                output.status.success(),
                installed,
                String::from_utf8_lossy(&output.stderr).into_owned(),
            )
        };

        let (ok, installed, err) = run(&good);
        assert!(ok, "the signed package didn't install: {err}");
        let installed = installed.unwrap();
        assert!(installed.starts_with("-U --noconfirm /"), "{installed}");
        assert!(
            installed.trim_end().ends_with(&format!("/{name}")),
            "{installed}"
        );
        assert!(
            !installed.contains("file://"),
            "pacman got the URL: {installed}"
        );
        // The download's directory is gone afterwards.
        let downloaded = installed.trim_end().trim_start_matches("-U --noconfirm ");
        assert!(!Path::new(downloaded).exists());

        for (case, url, says) in [
            (
                "no signature",
                &unsigned,
                "Couldn't download the update or its signature",
            ),
            (
                "a signature of another file",
                &wrong,
                "doesn't match Writan's key",
            ),
            (
                "another key's signature",
                &stranger,
                "doesn't match Writan's key",
            ),
        ] {
            let (ok, installed, err) = run(url);
            assert!(!ok, "a package with {case} installed");
            assert_eq!(installed, None, "pacman ran for a package with {case}");
            assert!(err.contains(says), "{case}: {err}");
        }
    }

    #[test]
    fn lists_nightlies_only_on_the_nightly_channel() {
        let releases = vec![
            release("nightly", "Nightly update feed", "feed", true),
            release("apt", "APT repository", "repo", true),
            release("plugin-latex-1.0.2", "Plugin: latex 1.0.2", "zip", true),
            release("dev-0.19.1-dev.4", "Writan Dev 0.19.1-dev.4", "dmg", true),
            release("v1.0.0", "Writan v1.0.0", "- Stable", false),
            release(
                "v1.0.1-nightly.202611280930",
                "Writan Nightly",
                "- First",
                true,
            ),
            release(
                "v1.0.1-nightly.202611291015",
                "Writan Nightly",
                "- Second",
                true,
            ),
        ];
        let stable = notes_from_list("0.19.0", &releases, false, Channel::Stable).unwrap();
        assert_eq!(stable.version, "1.0.0");
        let missed: Vec<&str> = stable.releases.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(missed, ["1.0.0"]);

        let nightly = notes_from_list(
            "1.0.1-nightly.202611280930",
            &releases,
            false,
            Channel::Nightly,
        )
        .unwrap();
        assert_eq!(nightly.version, "1.0.1-nightly.202611291015");
        let missed: Vec<&str> = nightly
            .releases
            .iter()
            .map(|r| r.version.as_str())
            .collect();
        assert_eq!(missed, ["1.0.1-nightly.202611291015"]);

        // A nightly user who moved back to stable sees no release newer than their copy.
        let back = notes_from_list(
            "1.0.1-nightly.202611291015",
            &releases,
            false,
            Channel::Stable,
        )
        .unwrap();
        assert_eq!(back.version, "1.0.0");
        assert!(back.releases.is_empty());
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
        let notes = notes_from_list("0.11.2", &releases, false, Channel::Stable).unwrap();
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
        let notes = notes_from_list("0.11.4", &releases, false, Channel::Stable).unwrap();
        assert!(notes.releases.is_empty());
        assert!(notes_from_list("0.11.4", &[], false, Channel::Stable).is_none());
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
