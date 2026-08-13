//! The two files install writes, and the exact entries it puts in them.
//!
//! # Two files, not one (D-05)
//!
//! Hook entries go in `settings.json` under `.hooks`. The MCP registration goes
//! in `.claude.json` under `.mcpServers`. Measured 2026-08-13: `settings.json`
//! carries 29 top-level keys and no `mcpServers` among them, and the Claude Code
//! 2.1.231 settings schema carries every MCP key *except* that one, while
//! `~/.claude.json` `.mcpServers` holds the servers that are actually loaded. An
//! `mcpServers` block written into `settings.json` would be ignored, `doctor`
//! would report MCP registered, and recall would be unreachable from inside a
//! session with every check passing.
//!
//! Where those files are is Claude Code's own rule, read out of the 2.1.231
//! bundle: `$CLAUDE_CONFIG_DIR/settings.json` falling back to
//! `$HOME/.claude/settings.json`, and `$CLAUDE_CONFIG_DIR/.claude.json` falling
//! back to `$HOME/.claude.json`. `verbatim.toml`'s `roots` does not decide this:
//! it says which transcript trees to walk, not where Claude Code keeps its
//! settings.
//!
//! # Exec form, always (D-01, ING-10)
//!
//! `{"type":"command","command":"<stable path>","args":["hook","<event>"]}`.
//! The 2.1.231 schema documents `args` as "Argument list for exec form. When
//! present, `command` is resolved as an executable and spawned directly with
//! these arguments - no shell", and the same bundle carries the "requires bash
//! but Git Bash was not found" error the shell form produces on a Windows
//! machine without Git Bash. A shell-form entry also breaks outright on a home
//! directory containing a space, and reintroduces the whole claude-mem Windows
//! failure class the absolute-path choice exists to retire.
//!
//! # Merge, never replace
//!
//! Measured 2026-08-13, `.hooks` already holds seven event keys on this machine,
//! `.hooks.SessionStart` is `[]` and `.hooks.UserPromptSubmit` is a one-element
//! array carrying the user's own script. So: ensure the event key exists as an
//! array, append one group with `matcher: ""`, and leave every group already
//! there exactly where it was. Same for `.mcpServers`, keyed on `verbatim`, with
//! `context7` untouched beside it.
//!
//! # Idempotency and upgrade are the same rule (INST-05)
//!
//! An entry is verbatim's when its `command` equals the stable path. If one is
//! there for an event, nothing is written for that event - not a replacement,
//! not a reordering. Because the stable path never moves (D-08), an upgrade that
//! replaced the binary finds all four present and rewrites no byte of `hooks`.
//! That is the version-skew class this design exists to retire.
//!
//! Nothing here shells out to `claude mcp add -s user` (D-06). Install must not
//! depend on `claude` being on `PATH`, from a binary whose stated contract is no
//! shell and no subprocess, and `.claude.json` is 240 KB of the user's project
//! history and caches to lose if that call misbehaves.

use std::path::{Path, PathBuf};

use verbatim_core::config::{CLAUDE_CONFIG_DIR_ENV, DEFAULT_CLAUDE_DIR};

use super::json_file::Json;
use crate::cmd::{hook, Failure};

/// The key `.mcpServers` holds verbatim's server under, and the only key
/// uninstall ever removes from that object.
pub const MCP_SERVER_KEY: &str = "verbatim";

/// Claude Code's hook settings file.
pub fn settings_path() -> Result<PathBuf, Failure> {
    Ok(claude_config_dir()?.join("settings.json"))
}

/// Claude Code's `.claude.json`, where `.mcpServers` lives (D-19).
pub fn claude_json_path() -> Result<PathBuf, Failure> {
    match non_empty_var(CLAUDE_CONFIG_DIR_ENV) {
        Some(dir) => Ok(PathBuf::from(dir).join(".claude.json")),
        None => Ok(home_dir()?.join(".claude.json")),
    }
}

fn claude_config_dir() -> Result<PathBuf, Failure> {
    match non_empty_var(CLAUDE_CONFIG_DIR_ENV) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home_dir()?.join(DEFAULT_CLAUDE_DIR)),
    }
}

#[cfg(not(windows))]
fn home_dir() -> Result<PathBuf, Failure> {
    non_empty_var("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Failure::Operational("HOME is not set".to_owned()))
}

#[cfg(windows)]
fn home_dir() -> Result<PathBuf, Failure> {
    non_empty_var("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| Failure::Operational("USERPROFILE is not set".to_owned()))
}

fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Add the four hook entries that are missing, and nothing else.
pub fn add_hooks(root: &mut Json, stable: &Path) -> Result<(), Failure> {
    let stable = stable.display().to_string();
    let hooks = root.entry("hooks", Json::object())?;
    for event in hook::EVENTS {
        let entries = hooks.entry(event, Json::array())?;
        if holds_command(entries, &stable) {
            continue;
        }
        let mut entry = Json::object();
        entry.entry("type", Json::string("command"))?;
        entry.entry("command", Json::string(&stable))?;
        let mut args = Json::array();
        args.push(Json::string("hook"))?;
        args.push(Json::string(event))?;
        entry.entry("args", args)?;

        let mut group = Json::object();
        // The shape every group in a real `settings.json` has: an empty matcher
        // means every invocation of the event.
        group.entry("matcher", Json::string(""))?;
        let mut inner = Json::array();
        inner.push(entry)?;
        group.entry("hooks", inner)?;

        entries.push(group)?;
    }
    Ok(())
}

/// Is one of this event's groups already ours?
fn holds_command(entries: &Json, stable: &str) -> bool {
    entries.items().iter().any(|group| {
        group
            .get("hooks")
            .map(|inner| {
                inner.items().iter().any(|entry| {
                    entry.get("command").and_then(Json::as_str).as_deref() == Some(stable)
                })
            })
            .unwrap_or(false)
    })
}

/// Register the MCP server, unless it is already registered.
pub fn add_mcp_server(root: &mut Json, stable: &Path) -> Result<(), Failure> {
    let stable = stable.display().to_string();
    let servers = root.entry("mcpServers", Json::object())?;
    if servers.get(MCP_SERVER_KEY).is_some() {
        return Ok(());
    }
    let mut server = Json::object();
    server.entry("type", Json::string("stdio"))?;
    server.entry("command", Json::string(&stable))?;
    let mut args = Json::array();
    args.push(Json::string("mcp"))?;
    server.entry("args", args)?;
    servers.entry(MCP_SERVER_KEY, server)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::install::json_file::Document;

    fn stable() -> PathBuf {
        PathBuf::from("/home/someone/.local/bin/verbatim")
    }

    fn parsed(text: &str) -> Json {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, text).unwrap();
        Document::read(&path).unwrap().value().clone()
    }

    /// Four entries, exec form, and the user's own `UserPromptSubmit` group
    /// still first in its array.
    #[test]
    fn the_four_entries_are_appended_beside_what_was_there() {
        let mut root = parsed(
            r#"{
  "theme": "dark",
  "hooks": {
    "UserPromptSubmit": [
      {"matcher": "", "hooks": [{"type": "command", "command": "$HOME/mine.sh", "timeout": 5}]}
    ]
  }
}"#,
        );
        add_hooks(&mut root, &stable()).unwrap();

        let hooks = root.get("hooks").unwrap();
        for event in hook::EVENTS {
            let groups = hooks.get(event).unwrap();
            assert!(
                holds_command(groups, &stable().display().to_string()),
                "{event}"
            );
        }
        let prompt = hooks.get("UserPromptSubmit").unwrap().items();
        assert_eq!(prompt.len(), 2, "the user's own group must survive");
        assert_eq!(
            prompt[0].get("hooks").unwrap().items()[0]
                .get("command")
                .and_then(Json::as_str)
                .unwrap(),
            "$HOME/mine.sh"
        );
        let ours = &prompt[1].get("hooks").unwrap().items()[0];
        assert_eq!(ours.get("type").and_then(Json::as_str).unwrap(), "command");
        let args = ours.get("args").unwrap().items();
        assert_eq!(args[0].as_str().unwrap(), "hook");
        assert_eq!(args[1].as_str().unwrap(), "UserPromptSubmit");
    }

    /// Applying the edit twice is applying it once: this is INST-05 in one
    /// assertion, because an upgrade is exactly a second application.
    #[test]
    fn a_second_application_changes_nothing() {
        let mut root = parsed("{}");
        add_hooks(&mut root, &stable()).unwrap();
        add_mcp_server(&mut root, &stable()).unwrap();
        let once = root.clone();
        add_hooks(&mut root, &stable()).unwrap();
        add_mcp_server(&mut root, &stable()).unwrap();
        assert_eq!(once, root);
    }

    /// `context7` keeps its place and its value.
    #[test]
    fn another_server_is_left_alone() {
        let mut root = parsed(
            r#"{"mcpServers": {"context7": {"type": "http", "url": "https://example.invalid"}}}"#,
        );
        add_mcp_server(&mut root, &stable()).unwrap();
        let servers = root.get("mcpServers").unwrap();
        assert_eq!(servers.keys(), ["context7", "verbatim"]);
        assert_eq!(
            servers
                .get("context7")
                .unwrap()
                .get("url")
                .and_then(Json::as_str)
                .unwrap(),
            "https://example.invalid"
        );
        assert_eq!(
            servers
                .get("verbatim")
                .unwrap()
                .get("command")
                .and_then(Json::as_str)
                .unwrap(),
            stable().display().to_string()
        );
    }

    /// A `.hooks` that is not an object is a malformed settings file, not a
    /// thing to overwrite.
    #[test]
    fn a_hooks_key_that_is_not_an_object_is_operational() {
        let mut root = parsed(r#"{"hooks": []}"#);
        match add_hooks(&mut root, &stable()) {
            Err(Failure::Operational(_)) => {}
            other => panic!("expected an operational failure, got {other:?}"),
        }
    }
}
