//! Setting up AI agents to work with Writan (Settings › AI): adding Writan's MCP server
//! (`collab/mcp.rs`) and its skill to the agent someone picks.
//!
//! Each agent keeps these in its own place. Claude Code is told through its CLI, which keeps
//! its large state file itself. The others' config files are edited here, changing only
//! Writan's own entry and leaving everything else as it was:
//!
//! | Agent | MCP server | Skills |
//! | --- | --- | --- |
//! | Claude Code | `claude mcp add --transport http --scope user` | `~/.claude/skills` |
//! | Codex | `~/.codex/config.toml`, `[mcp_servers.writan] url` | `~/.agents/skills` |
//! | Antigravity | `~/.gemini/config/mcp_config.json`, `mcpServers.writan.serverUrl` | `~/.gemini/config/skills` |
//! | Grok Build | `~/.grok/config.toml`, `[mcp_servers.writan] url` | `~/.grok/skills` |
//! | Hermes Agent | `~/.hermes/config.yaml`, `mcp_servers.writan.url` | `~/.hermes/skills` |

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// The name Writan's server and skill go by in every agent.
const NAME: &str = "writan";

fn home() -> Result<PathBuf, String> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| "Couldn't find your home folder.".to_owned())
}

/// Where an agent's MCP config and skills live, under the home folder.
fn places(agent: &str) -> Option<(Option<&'static str>, &'static str)> {
    Some(match agent {
        "claude" => (None, ".claude/skills"),
        "codex" => (Some(".codex/config.toml"), ".agents/skills"),
        "antigravity" => (
            Some(".gemini/config/mcp_config.json"),
            ".gemini/config/skills",
        ),
        "grok" => (Some(".grok/config.toml"), ".grok/skills"),
        "hermes" => (Some(".hermes/config.yaml"), ".hermes/skills"),
        _ => return None,
    })
}

#[derive(Serialize)]
pub struct Installed {
    pub mcp: bool,
    pub skill: bool,
}

/// Whether Writan's MCP server and skill are set up in `agent`.
#[tauri::command]
pub fn agents_installed(agent: String) -> Result<Installed, String> {
    let (config, skills) = places(&agent).ok_or("Writan doesn't know that agent.")?;
    let home = home()?;
    let skill = home.join(skills).join(NAME).join("SKILL.md").is_file();
    let mcp = match config {
        Some(file) => std::fs::read_to_string(home.join(file)).is_ok_and(|t| has_server(file, &t)),
        None => std::fs::read_to_string(home.join(".claude.json")).is_ok_and(|t| {
            serde_json::from_str::<serde_json::Value>(&t)
                .is_ok_and(|v| v["mcpServers"].get(NAME).is_some())
        }),
    };
    Ok(Installed { mcp, skill })
}

/// Adds Writan's MCP server at `url` to `agent`, replacing an older one. Returns what it did.
#[tauri::command]
pub fn agents_install_mcp(agent: String, url: String) -> Result<String, String> {
    if !url.starts_with("http://") || url.contains(['"', '\'', '\\', ' ', '\n']) {
        return Err("That isn't a Writan MCP address.".into());
    }
    let (config, _) = places(&agent).ok_or("Writan doesn't know that agent.")?;
    let Some(file) = config else {
        return claude_add(&url);
    };
    let path = home()?.join(file);
    let before = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
    };
    let after = if file.ends_with(".toml") {
        set_toml_server(&before, &url)
    } else if file.ends_with(".json") {
        set_json_server(&before, &url)?
    } else {
        set_yaml_server(&before, &url)?
    };
    write(&path, &after)?;
    Ok(format!("Added to {}.", tilde(file)))
}

/// Puts Writan's skill (`SKILL.md`, from the page) where `agent` finds skills.
#[tauri::command]
pub fn agents_install_skill(agent: String, skill: String) -> Result<String, String> {
    let (_, skills) = places(&agent).ok_or("Writan doesn't know that agent.")?;
    let dir = home()?.join(skills).join(NAME);
    write(&dir.join("SKILL.md"), &skill)?;
    Ok(format!("Added to {}.", tilde(&format!("{skills}/{NAME}"))))
}

/// Takes Writan's MCP server and skill back out of `agent`. Everything else stays.
#[tauri::command]
pub fn agents_remove(agent: String) -> Result<String, String> {
    let (config, skills) = places(&agent).ok_or("Writan doesn't know that agent.")?;
    let home = home()?;
    let mut gone: Vec<&str> = Vec::new();
    let dir = home.join(skills).join(NAME);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("Couldn't remove {}: {e}", dir.display()))?;
        gone.push("skill");
    }
    let mcp = match config {
        Some(file) => {
            let path = home.join(file);
            let before = std::fs::read_to_string(&path).unwrap_or_default();
            if !has_server(file, &before) {
                false
            } else {
                let after = if file.ends_with(".toml") {
                    remove_toml_server(&before)
                } else if file.ends_with(".json") {
                    remove_json_server(&before)?
                } else {
                    remove_yaml_server(&before)
                };
                write(&path, &after)?;
                true
            }
        }
        None => claude_remove()?,
    };
    if mcp {
        gone.insert(0, "MCP server");
    }
    Ok(match gone.len() {
        0 => "Nothing was set up.".into(),
        _ => format!("Removed its {}.", gone.join(" and ")),
    })
}

/// A new version's MCP address or skill isn't what an agent has: bring the ones that have it up
/// to date. Runs at startup, so after the app updates, every agent set up before follows.
#[tauri::command]
pub fn agents_sync(url: String, skill: String) -> Result<Vec<String>, String> {
    if !url.starts_with("http://") || url.contains(['"', '\'', '\\', ' ', '\n']) {
        return Err("That isn't a Writan MCP address.".into());
    }
    let home = home()?;
    let mut updated = Vec::new();
    for agent in ["claude", "codex", "antigravity", "grok", "hermes"] {
        let Some((config, skills)) = places(agent) else {
            continue;
        };
        let want = format!("{url}?agent={agent}");
        let stale = match config {
            Some(file) => std::fs::read_to_string(home.join(file))
                .ok()
                .is_some_and(|t| server_url(file, &t).is_some_and(|u| u != want)),
            None => std::fs::read_to_string(home.join(".claude.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v["mcpServers"][NAME]["url"].as_str().map(String::from))
                .is_some_and(|u| u != want),
        };
        if stale && agents_install_mcp(agent.into(), want).is_ok() {
            updated.push(format!("{agent}:mcp"));
        }
        let file = home.join(skills).join(NAME).join("SKILL.md");
        if file.is_file()
            && std::fs::read_to_string(&file).is_ok_and(|t| t != skill)
            && write(&file, &skill).is_ok()
        {
            updated.push(format!("{agent}:skill"));
        }
    }
    Ok(updated)
}

fn tilde(rel: &str) -> String {
    if cfg!(windows) {
        format!("%USERPROFILE%\\{}", rel.replace('/', "\\"))
    } else {
        format!("~/{rel}")
    }
}

/// Writes a file whole or not at all: a config an agent reads is never left half written. A
/// config that's a link to a file elsewhere (dotfiles kept in a repository) is written there.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let linked = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let path = &match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(_) if linked => {
            return Err(format!(
                "{} is a link to a file that isn't there, so Writan left it alone.",
                path.display()
            ))
        }
        Err(_) => path.to_path_buf(),
    };
    let dir = path.parent().ok_or("No folder to write in.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| format!("Couldn't write in {}: {e}", dir.display()))?;
    std::io::Write::write_all(&mut tmp, text.as_bytes())
        .map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    tmp.persist(path)
        .map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok(())
}

/// Claude Code keeps its servers in a big state file it writes itself, so its CLI adds and
/// removes this one. Apps opened from the Dock or Start menu don't get the terminal's PATH, so
/// on macOS and Linux it runs in a login shell.
fn claude_add(url: &str) -> Result<String, String> {
    let add = format!("claude mcp add --transport http --scope user {NAME} '{url}'");
    let script = format!("claude mcp remove --scope user {NAME} >/dev/null 2>&1; {add}");
    let output = if cfg!(windows) {
        let _ = Command::new("cmd")
            .args(["/C", &format!("claude mcp remove --scope user {NAME}")])
            .output();
        Command::new("cmd")
            .args(["/C", &add.replace('\'', "\"")])
            .output()
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        Command::new(shell).args(["-l", "-c", &script]).output()
    };
    match output {
        Ok(o) if o.status.success() => Ok("Added to Claude Code.".into()),
        Ok(o) => Err(format!(
            "Claude Code's command didn't work ({}). Run this in a terminal instead: {add}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!(
            "Couldn't run Claude Code's command ({e}). Run this in a terminal instead: {add}"
        )),
    }
}

/// Whether a config file's text has Writan's server in it.
fn has_server(file: &str, text: &str) -> bool {
    if file.ends_with(".toml") {
        text.lines().any(is_our_table)
    } else if file.ends_with(".json") {
        serde_json::from_str::<serde_json::Value>(text)
            .is_ok_and(|v| v["mcpServers"].get(NAME).is_some())
    } else {
        yaml_entry(text).is_some_and(|e| e.found.is_some())
    }
}

/// A TOML key without its one pair of quotes, if it has them.
fn unquote(key: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = key.strip_prefix(q).and_then(|k| k.strip_suffix(q)) {
            return inner;
        }
    }
    key
}

/// Whether a TOML line opens Writan's table, `[mcp_servers.writan]`, with or without a comment
/// after it.
fn is_our_table(line: &str) -> bool {
    let code = line.split('#').next().unwrap_or("").trim();
    code.strip_prefix('[')
        .and_then(|l| l.strip_suffix(']'))
        .is_some_and(|name| {
            name.split('.')
                .map(|p| unquote(p.trim()))
                .eq(["mcp_servers", NAME])
        })
}

/// The address our server entry points at, if the config has one.
fn server_url(file: &str, text: &str) -> Option<String> {
    if file.ends_with(".toml") {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.iter().position(|l| is_our_table(l))?;
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map_or(lines.len(), |i| start + 1 + i);
        lines[start + 1..end].iter().find_map(|l| {
            let (k, v) = l.split('#').next()?.split_once('=')?;
            (unquote(k.trim()) == "url").then(|| unquote(v.trim()).to_owned())
        })
    } else if file.ends_with(".json") {
        serde_json::from_str::<serde_json::Value>(text).ok()?["mcpServers"][NAME]["serverUrl"]
            .as_str()
            .map(String::from)
    } else {
        let e = yaml_entry(text)?;
        let (start, stop) = e.found?;
        let lines: Vec<&str> = text.lines().collect();
        lines[start + 1..stop].iter().find_map(|l| {
            l.trim()
                .strip_prefix("url:")
                .map(|v| unquote(v.trim()).to_owned())
        })
    }
}

/// Our TOML table out of a config, leaving the rest — and a blank line where it sat, so tables
/// that were apart stay apart.
fn remove_toml_server(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| is_our_table(l)) else {
        return text.to_string();
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |i| start + 1 + i);
    let mut out: Vec<String> = lines[..start].iter().map(|l| l.to_string()).collect();
    // One blank line separates what came before from what follows; at the end, it goes too.
    if end == lines.len() && out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    let mut s = out.join("\n");
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Our server out of the JSON config, leaving the rest.
fn remove_json_server(text: &str) -> Result<String, String> {
    let mut v: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        format!("Antigravity's mcp_config.json isn't valid JSON ({e}), so Writan left it alone.")
    })?;
    if let Some(servers) = v.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        servers.remove(NAME);
        if servers.is_empty() {
            if let Some(root) = v.as_object_mut() {
                root.remove("mcpServers");
            }
        }
    }
    let mut s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    s.push('\n');
    Ok(s)
}

/// Our entry out of the YAML config, leaving the rest — and the `mcp_servers:` line itself if
/// nothing is left under it.
fn remove_yaml_server(text: &str) -> String {
    let Some(e) = yaml_entry(text) else {
        return text.to_string();
    };
    let Some((start, stop)) = e.found else {
        return text.to_string();
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.splice(start..stop, []);
    // Anything non-blank is left under `mcp_servers:`? Then its line stays.
    let end = out[e.servers + 1..]
        .iter()
        .position(|l| !blank(l) && indent_of(l) == 0)
        .map_or(out.len(), |i| e.servers + 1 + i);
    if !out[e.servers + 1..end].iter().any(|l| !blank(l)) {
        out.remove(e.servers);
    }
    let mut s = out.join("\n");
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Takes Writan's server out of Claude Code the way it went in. Returns whether it was there.
fn claude_remove() -> Result<bool, String> {
    let had = std::fs::read_to_string(home()?.join(".claude.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["mcpServers"].get(NAME).is_some());
    if !had {
        return Ok(false);
    }
    let remove = format!("claude mcp remove --scope user {NAME}");
    let output = if cfg!(windows) {
        Command::new("cmd").args(["/C", &remove]).output()
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        Command::new(shell).args(["-l", "-c", &remove]).output()
    };
    match output {
        Ok(o) if o.status.success() => Ok(true),
        Ok(o) => Err(format!(
            "Claude Code's command didn't work ({}). Run this in a terminal instead: {remove}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!(
            "Couldn't run Claude Code's command ({e}). Run this in a terminal instead: {remove}"
        )),
    }
}

/// A TOML config (Codex, Grok Build) with `[mcp_servers.writan]` holding just `url`. Everything
/// outside that table stays as it was.
fn set_toml_server(text: &str, url: &str) -> String {
    let header = format!("[mcp_servers.{NAME}]");
    let entry = format!("{header}\nurl = \"{url}\"\n");
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| is_our_table(l)) else {
        let sep = if text.is_empty() || text.ends_with("\n\n") {
            ""
        } else if text.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        return format!("{text}{sep}{entry}");
    };
    // The table runs to the next table header.
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |i| start + 1 + i);
    let mut out: Vec<String> = lines[..start].iter().map(|l| l.to_string()).collect();
    out.push(header);
    out.push(format!("url = \"{url}\""));
    if end < lines.len() {
        out.push(String::new());
    }
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Antigravity's JSON config with `mcpServers.writan = { serverUrl }`.
fn set_json_server(text: &str, url: &str) -> Result<String, String> {
    let mut v: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text).map_err(|e| {
            format!(
                "Antigravity's mcp_config.json isn't valid JSON ({e}), so Writan left it alone."
            )
        })?
    };
    let Some(root) = v.as_object_mut() else {
        return Err(
            "Antigravity's mcp_config.json isn't what Writan expects, so it left it alone.".into(),
        );
    };
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(
            "Antigravity's mcpServers isn't what Writan expects, so it left it alone.".into(),
        );
    };
    servers.insert(NAME.into(), serde_json::json!({ "serverUrl": url }));
    let mut s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    s.push('\n');
    Ok(s)
}

/// Where `mcp_servers:` and Writan's entry under it are in a YAML file.
struct YamlEntry {
    /// The line with `mcp_servers:`.
    servers: usize,
    /// How far its entries are indented.
    indent: usize,
    /// Writan's entry: its first line and the line after its last.
    found: Option<(usize, usize)>,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn blank(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

fn yaml_entry(text: &str) -> Option<YamlEntry> {
    let lines: Vec<&str> = text.lines().collect();
    let servers = lines
        .iter()
        .position(|l| indent_of(l) == 0 && l.trim_end().starts_with("mcp_servers:"))?;
    let end = lines[servers + 1..]
        .iter()
        .position(|l| !blank(l) && indent_of(l) == 0)
        .map_or(lines.len(), |i| servers + 1 + i);
    let indent = lines[servers + 1..end]
        .iter()
        .find(|l| !blank(l))
        .map_or(2, |l| indent_of(l));
    let found = (servers + 1..end)
        .find(|&i| indent_of(lines[i]) == indent && lines[i].trim() == format!("{NAME}:"))
        .map(|start| {
            let stop = (start + 1..end)
                .find(|&i| !blank(lines[i]) && indent_of(lines[i]) <= indent)
                .unwrap_or(end);
            (start, stop)
        });
    Some(YamlEntry {
        servers,
        indent,
        found,
    })
}

/// Hermes Agent's YAML config with `mcp_servers.writan.url`. Only Writan's entry changes.
fn set_yaml_server(text: &str, url: &str) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(e) = yaml_entry(text) else {
        let sep = if text.is_empty() || text.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        return Ok(format!(
            "{text}{sep}mcp_servers:\n  {NAME}:\n    url: \"{url}\"\n"
        ));
    };
    let rest = lines[e.servers]
        .trim_end()
        .trim_start_matches("mcp_servers:")
        .trim();
    if !rest.is_empty() && !rest.starts_with('#') && rest != "{}" {
        return Err("Hermes's mcp_servers is written on one line, so Writan left config.yaml alone. Add it there yourself.".into());
    }
    let pad = " ".repeat(e.indent);
    let entry = [format!("{pad}{NAME}:"), format!("{pad}{pad}url: \"{url}\"")];
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match e.found {
        Some((start, stop)) => {
            out.splice(start..stop, entry);
        }
        None => {
            out[e.servers] = "mcp_servers:".into();
            out.splice(e.servers + 1..e.servers + 1, entry);
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://192.168.1.20:47811/mcp/tok?agent=codex";

    #[test]
    fn toml_gets_one_table_and_keeps_the_rest() {
        let added = set_toml_server("model = \"x\"\n\n[mcp_servers.other]\nurl = \"a\"\n", URL);
        assert_eq!(
            added,
            format!("model = \"x\"\n\n[mcp_servers.other]\nurl = \"a\"\n\n[mcp_servers.writan]\nurl = \"{URL}\"\n")
        );
        assert!(has_server("c.toml", &added));
        // Again replaces it, and a table after it stays.
        let with_after = format!("{added}\n[profiles.fast]\nmodel = \"y\"\n");
        let again = set_toml_server(&with_after, "http://h:1/mcp/t");
        assert_eq!(
            again,
            "model = \"x\"\n\n[mcp_servers.other]\nurl = \"a\"\n\n[mcp_servers.writan]\nurl = \"http://h:1/mcp/t\"\n\n[profiles.fast]\nmodel = \"y\"\n"
        );
        assert_eq!(
            set_toml_server("", URL),
            format!("[mcp_servers.writan]\nurl = \"{URL}\"\n")
        );
        // A comment after the table's name, or quotes in it, is still the same table.
        let commented = "[mcp_servers.writan] # Writan\nurl = \"old\"\n";
        assert_eq!(
            set_toml_server(commented, URL),
            format!("[mcp_servers.writan]\nurl = \"{URL}\"\n")
        );
        assert!(has_server("c.toml", "[ mcp_servers.\"writan\" ]\n"));
        assert!(!has_server("c.toml", "[mcp_servers.writan2]\n"));
        assert!(has_server("c.toml", "['mcp_servers'.'writan']\n"));
        // Quotes inside a quoted key are part of the name: that's another table.
        let other = "[\"'mcp_servers'\".writan]\nurl = \"a\"\n";
        assert!(!has_server("c.toml", other));
        assert!(set_toml_server(other, URL).starts_with(other));
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_config_is_written_where_it_points() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        std::fs::write(&real, "model = \"x\"\n").unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write(&link, "new\n").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new\n");
        // A link to nothing is left alone.
        let dangling = dir.path().join("gone.toml");
        std::os::unix::fs::symlink(dir.path().join("missing.toml"), &dangling).unwrap();
        assert!(write(&dangling, "x").is_err());
        assert!(std::fs::symlink_metadata(&dangling)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn json_gets_one_server_and_keeps_the_rest() {
        let out =
            set_json_server(r#"{"mcpServers":{"other":{"serverUrl":"a"}},"x":1}"#, URL).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["writan"]["serverUrl"], URL);
        assert_eq!(v["mcpServers"]["other"]["serverUrl"], "a");
        assert_eq!(v["x"], 1);
        assert!(set_json_server("", URL).unwrap().contains("serverUrl"));
        assert!(set_json_server("not json", URL).is_err());
    }

    #[test]
    fn server_url_reads_each_kind() {
        let toml = format!("model = \"x\"\n[mcp_servers.writan]\nurl = \"{URL}\"\n");
        assert_eq!(server_url("c.toml", &toml).as_deref(), Some(URL));
        let toml = format!("model = \"x\"\n[mcp_servers.writan] # ours\nurl = '{URL}' # the app\n");
        assert_eq!(server_url("c.toml", &toml).as_deref(), Some(URL));
        assert_eq!(server_url("c.toml", "model = \"x\"\n"), None);
        let json = format!("{{\"mcpServers\":{{\"writan\":{{\"serverUrl\":\"{URL}\"}}}}}}");
        assert_eq!(server_url("c.json", &json).as_deref(), Some(URL));
        assert_eq!(server_url("c.json", "{\"mcpServers\":{}}"), None);
        let yaml =
            format!("mcp_servers:\n  writan:\n    url: \"{URL}\"\n  other:\n    url: \"a\"\n");
        assert_eq!(server_url("c.yaml", &yaml).as_deref(), Some(URL));
        assert_eq!(
            server_url("c.yaml", "mcp_servers:\n  other:\n    url: \"a\"\n"),
            None
        );
    }

    #[test]
    fn toml_loses_only_our_table() {
        let added = set_toml_server("model = \"x\"\n\n[mcp_servers.other]\nurl = \"a\"\n", URL);
        let removed = remove_toml_server(&added);
        assert_eq!(
            removed,
            "model = \"x\"\n\n[mcp_servers.other]\nurl = \"a\"\n"
        );
        assert!(!has_server("c.toml", &removed));
        // In the middle, the spacing around it stays tidy.
        let middle = "[a]\nx = 1\n\n[mcp_servers.writan]\nurl = \"u\"\n\n[b]\ny = 2\n";
        assert_eq!(remove_toml_server(middle), "[a]\nx = 1\n\n[b]\ny = 2\n");
        // At the end, the blank line before it goes with it.
        let end = "[a]\nx = 1\n\n[mcp_servers.writan]\nurl = \"u\"\n";
        assert_eq!(remove_toml_server(end), "[a]\nx = 1\n");
        // Alone, the file comes out empty.
        assert_eq!(
            remove_toml_server("[mcp_servers.writan]\nurl = \"u\"\n"),
            ""
        );
        // Remove and add round-trip.
        assert_eq!(set_toml_server(&removed, URL), added);
    }

    #[test]
    fn json_loses_only_our_server() {
        let added =
            set_json_server(r#"{"mcpServers":{"other":{"serverUrl":"a"}},"x":1}"#, URL).unwrap();
        let removed = remove_json_server(&added).unwrap();
        let v: serde_json::Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(v["mcpServers"]["other"]["serverUrl"], "a");
        assert!(v["mcpServers"].get("writan").is_none());
        assert_eq!(v["x"], 1);
        // The only server: the mcpServers key goes too.
        let removed = remove_json_server(&set_json_server("", URL).unwrap()).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&removed).unwrap(),
            serde_json::json!({})
        );
        assert!(!has_server("c.json", &removed));
        assert!(remove_json_server("not json").is_err());
    }

    #[test]
    fn yaml_loses_only_our_entry() {
        let added = set_yaml_server(
            "model: x\nmcp_servers:\n    other:\n        url: \"a\"\n# end\ntools: []\n",
            URL,
        )
        .unwrap();
        let removed = remove_yaml_server(&added);
        assert_eq!(
            removed,
            "model: x\nmcp_servers:\n    other:\n        url: \"a\"\n# end\ntools: []\n"
        );
        assert!(!has_server("c.yaml", &removed));
        // The only entry: the mcp_servers line goes too.
        let removed = remove_yaml_server(&set_yaml_server("model: x\n", URL).unwrap());
        assert_eq!(removed, "model: x\n");
        // And re-adding after either works.
        assert!(has_server(
            "c.yaml",
            &set_yaml_server(&removed, URL).unwrap()
        ));
    }

    #[test]
    fn yaml_gets_one_server_and_keeps_the_rest() {
        let base = "model: x\nmcp_servers:\n    other:\n        url: \"a\"\n# end\ntools: []\n";
        let added = set_yaml_server(base, URL).unwrap();
        assert_eq!(
            added,
            format!("model: x\nmcp_servers:\n    writan:\n        url: \"{URL}\"\n    other:\n        url: \"a\"\n# end\ntools: []\n")
        );
        assert!(has_server("c.yaml", &added));
        let again = set_yaml_server(&added, "http://h:1/mcp/t").unwrap();
        assert_eq!(
            again,
            "model: x\nmcp_servers:\n    writan:\n        url: \"http://h:1/mcp/t\"\n    other:\n        url: \"a\"\n# end\ntools: []\n"
        );
        assert_eq!(
            set_yaml_server("model: x\n", URL).unwrap(),
            format!("model: x\nmcp_servers:\n  writan:\n    url: \"{URL}\"\n")
        );
        assert_eq!(
            set_yaml_server("mcp_servers: {}\n", URL).unwrap(),
            format!("mcp_servers:\n  writan:\n    url: \"{URL}\"\n")
        );
        assert!(set_yaml_server("mcp_servers: {a: {url: b}}\n", URL).is_err());
    }
}
