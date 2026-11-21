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

fn tilde(rel: &str) -> String {
    if cfg!(windows) {
        format!("%USERPROFILE%\\{}", rel.replace('/', "\\"))
    } else {
        format!("~/{rel}")
    }
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("Couldn't write {}: {e}", path.display()))
}

/// Claude Code keeps its servers in a big state file it writes itself, so its CLI adds this
/// one. Apps opened from the Dock or Start menu don't get the terminal's PATH, so on macOS and
/// Linux it runs in a login shell.
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
        text.lines()
            .any(|l| l.trim() == format!("[mcp_servers.{NAME}]"))
    } else if file.ends_with(".json") {
        serde_json::from_str::<serde_json::Value>(text)
            .is_ok_and(|v| v["mcpServers"].get(NAME).is_some())
    } else {
        yaml_entry(text).is_some_and(|e| e.found.is_some())
    }
}

/// A TOML config (Codex, Grok Build) with `[mcp_servers.writan]` holding just `url`. Everything
/// outside that table stays as it was.
fn set_toml_server(text: &str, url: &str) -> String {
    let header = format!("[mcp_servers.{NAME}]");
    let entry = format!("{header}\nurl = \"{url}\"\n");
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == header) else {
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
