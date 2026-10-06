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
//! back to `$HOME/.claude.json`. `terminus.toml`'s `roots` does not decide this:
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
//! there exactly where it was. Same for `.mcpServers`, keyed on `terminus`, with
//! `context7` untouched beside it.
//!
//! # Idempotency and upgrade are the same rule (INST-05)
//!
//! Two questions, and they are not the same one. **Whose** an entry is: it is
//! terminus's when its `command` is the stable path, and anything else is
//! somebody's and is never touched, read or written. **What shape** it is in:
//! `type: "command"`, and `args` naming this event. A second install finds its
//! four entries in the intended shape and writes nothing at all - not a
//! replacement, not a reordering - so an upgrade that replaced the binary
//! rewrites no byte of `hooks`, which is the version-skew class this design
//! exists to retire.
//!
//! Idempotence checked by presence alone would be a trap: an entry with the
//! right command and no `args`, or `args` naming the wrong event - a hand edit,
//! or an older terminus's shape - would make install skip that event and leave
//! it with no working hook at all, for good. So a terminus entry that is the
//! wrong shape is repaired in place, keeping its position and any key beside it
//! the user added, such as a `timeout`. Same rule under `.mcpServers.terminus`:
//! a registration pointing at a binary path that has moved is corrected rather
//! than left to rot, because "keeping itself current with no user action" is
//! exactly what fails when it is not.
//!
//! Nothing here shells out to `claude mcp add -s user` (D-06). Install must not
//! depend on `claude` being on `PATH`, from a binary whose stated contract is no
//! shell and no subprocess, and `.claude.json` is 240 KB of the user's project
//! history and caches to lose if that call misbehaves.

use std::path::{Path, PathBuf};

use terminus_core::config::{CLAUDE_CONFIG_DIR_ENV, DEFAULT_CLAUDE_DIR};

use super::json_file::Json;
use crate::cmd::{hook, Failure};

/// The key `.mcpServers` holds terminus's server under, and the only key
/// uninstall ever removes from that object.
pub const MCP_SERVER_KEY: &str = "terminus";

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

/// Put each event's entry in the shape terminus writes, adding the ones that
/// are not there, and change nothing else.
pub fn add_hooks(root: &mut Json, stable: &Path) -> Result<(), Failure> {
    let stable = stable.display().to_string();
    let hooks = root.entry("hooks", Json::object())?;
    for event in hook::EVENTS {
        let entries = hooks.entry(event, Json::array())?;
        if holds_ours(entries, &stable) {
            repair_entries(entries, &stable, event)?;
            continue;
        }
        let mut group = Json::object();
        // The shape every group in a real `settings.json` has: an empty matcher
        // means every invocation of the event.
        group.entry("matcher", Json::string(""))?;
        let mut inner = Json::array();
        inner.push(hook_entry(&stable, event)?)?;
        group.entry("hooks", inner)?;

        entries.push(group)?;
    }
    Ok(())
}

/// The entry terminus writes for one event, in exec form (D-01).
fn hook_entry(stable: &str, event: &str) -> Result<Json, Failure> {
    let mut entry = Json::object();
    entry.entry("type", Json::string("command"))?;
    entry.entry("command", Json::string(stable))?;
    entry.entry("args", hook_args(event))?;
    Ok(entry)
}

fn hook_args(event: &str) -> Json {
    Json::Array(vec![Json::string("hook"), Json::string(event)])
}

/// Does one of this event's groups hold an entry of terminus's?
///
/// Ownership is the command and only the command: an entry whose `command` is
/// the stable path is terminus's however wrong the rest of it is, and an entry
/// whose command is anything else is not read past that field.
fn holds_ours(entries: &Json, stable: &str) -> bool {
    entries
        .items()
        .iter()
        .filter_map(|group| group.get("hooks"))
        .flat_map(|inner| inner.items())
        .any(|entry| entry.get("command").and_then(Json::as_str).as_deref() == Some(stable))
}

/// Bring this event's own entries up to the intended shape.
///
/// In place, so an entry keeps its position and any key beside it - a `timeout`
/// the user added stays. Assigning a value it already holds is a no-op all the
/// way to the bytes, which is what leaves an upgrade rewriting nothing (AC3).
fn repair_entries(entries: &mut Json, stable: &str, event: &str) -> Result<(), Failure> {
    for group in entries.items_mut() {
        let Some(inner) = group.get_mut("hooks") else {
            continue;
        };
        for entry in inner.items_mut() {
            if entry.get("command").and_then(Json::as_str).as_deref() != Some(stable) {
                continue;
            }
            set(entry, "type", Json::string("command"))?;
            set(entry, "args", hook_args(event))?;
        }
    }
    Ok(())
}

/// Register the MCP server, or put the registration back in the shape terminus
/// writes.
pub fn add_mcp_server(root: &mut Json, stable: &Path) -> Result<(), Failure> {
    let stable = stable.display().to_string();
    let servers = root.entry("mcpServers", Json::object())?;
    let server = servers.entry(MCP_SERVER_KEY, Json::object())?;
    if !matches!(server, Json::Object(_)) {
        // Under terminus's own key, and not a server registration at all. There
        // is nothing in it to keep.
        *server = Json::object();
    }
    // The three fields terminus owns, and no others: a `command` naming a
    // binary path that has moved is corrected, and an `env` block the user put
    // beside it survives.
    set(server, "type", Json::string("stdio"))?;
    set(server, "command", Json::string(&stable))?;
    set(server, "args", Json::Array(vec![Json::string("mcp")]))?;
    Ok(())
}

/// `object[key] = value`, in place when the key is there and appended when it
/// is not.
fn set(object: &mut Json, key: &str, value: Json) -> Result<(), Failure> {
    let slot = object.entry(key, value.clone())?;
    *slot = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::install::json_file::Document;

    fn stable() -> PathBuf {
        PathBuf::from("/home/someone/.local/bin/terminus")
    }

    fn parsed(text: &str) -> Json {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, text).unwrap();
        Document::read(&path).unwrap().value().clone()
    }

    /// Terminus's own entries among one event's groups - the ones whose
    /// `command` is the stable path, whatever else they say.
    fn ours<'a>(groups: &'a Json, stable: &str) -> Vec<&'a Json> {
        groups
            .items()
            .iter()
            .filter_map(|group| group.get("hooks"))
            .flat_map(|inner| inner.items())
            .filter(|entry| entry.get("command").and_then(Json::as_str).as_deref() == Some(stable))
            .collect()
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
            let mine = ours(hooks.get(event).unwrap(), &stable().display().to_string());
            assert_eq!(mine.len(), 1, "{event}");
            assert_eq!(
                mine[0].get("type").and_then(Json::as_str).unwrap(),
                "command"
            );
            assert_eq!(
                mine[0].get("args").unwrap().items()[1].as_str().unwrap(),
                *event
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
        assert_eq!(servers.keys(), ["context7", "terminus"]);
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
                .get("terminus")
                .unwrap()
                .get("command")
                .and_then(Json::as_str)
                .unwrap(),
            stable().display().to_string()
        );
    }

    /// An entry of terminus's that is the wrong shape is repaired where it
    /// stands, not skipped and not duplicated.
    ///
    /// Presence is not the test. This entry has the right command and the wrong
    /// event in its `args`, which is what a hand edit or an older terminus
    /// leaves; matching on the command alone would call `SessionStart` done and
    /// leave it with no working hook at all.
    #[test]
    fn an_entry_of_ours_in_the_wrong_shape_is_repaired_in_place() {
        let mut root = parsed(
            r#"{
  "hooks": {
    "SessionStart": [
      {"matcher": "", "hooks": [{"command": "/home/someone/.local/bin/terminus",
                                 "args": ["hook", "SessionEnd"], "timeout": 30}]}
    ]
  }
}"#,
        );
        add_hooks(&mut root, &stable()).unwrap();

        let groups = root.get("hooks").unwrap().get("SessionStart").unwrap();
        assert_eq!(
            groups.items().len(),
            1,
            "the repair appended a second group"
        );
        let mine = ours(groups, &stable().display().to_string());
        assert_eq!(mine.len(), 1);
        assert_eq!(
            mine[0].get("type").and_then(Json::as_str).unwrap(),
            "command"
        );
        assert_eq!(
            mine[0].get("args").unwrap().items()[1].as_str().unwrap(),
            "SessionStart"
        );
        assert_eq!(
            mine[0].get("timeout").and_then(Json::as_i64),
            Some(30),
            "a key the user put beside ours was dropped"
        );
    }

    /// An entry that is not terminus's is not read past its command, whatever
    /// shape it is in.
    #[test]
    fn an_entry_that_is_not_ours_is_left_exactly_as_it_is() {
        let text = r#"{
  "hooks": {
    "SessionStart": [
      {"matcher": "", "hooks": [{"command": "/opt/someone-else/terminus", "args": ["hook"]}]}
    ]
  }
}"#;
        let mut root = parsed(text);
        add_hooks(&mut root, &stable()).unwrap();

        let groups = root.get("hooks").unwrap().get("SessionStart").unwrap();
        assert_eq!(groups.items().len(), 2, "ours should have been appended");
        assert_eq!(
            groups.items()[0],
            parsed(text)
                .get("hooks")
                .unwrap()
                .get("SessionStart")
                .unwrap()
                .items()[0],
            "the entry that is not ours was changed"
        );
    }

    /// A registration of ours pointing at a binary path that has moved is
    /// corrected, and what the user put beside it survives.
    #[test]
    fn a_stale_mcp_registration_is_brought_up_to_date() {
        let mut root = parsed(
            r#"{"mcpServers": {"terminus": {"type": "sse", "command": "/old/bin/terminus",
             "env": {"KEEP": "me"}}}}"#,
        );
        add_mcp_server(&mut root, &stable()).unwrap();

        let server = root.get("mcpServers").unwrap().get("terminus").unwrap();
        assert_eq!(server.get("type").and_then(Json::as_str).unwrap(), "stdio");
        assert_eq!(
            server.get("command").and_then(Json::as_str).unwrap(),
            stable().display().to_string()
        );
        assert_eq!(
            server.get("args").unwrap().items()[0].as_str().unwrap(),
            "mcp"
        );
        assert_eq!(
            server.get("env").unwrap().get("KEEP").unwrap().as_str(),
            Some("me".to_owned()),
            "a key the user put beside ours was dropped"
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
