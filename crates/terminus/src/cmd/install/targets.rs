//! The file install writes, the file it only reads, and the exact entries that
//! belong in each.
//!
//! # One file written, one read (D-05)
//!
//! Hook entries go in `settings.json` under `.hooks`, and install writes that
//! file itself. The MCP entry lives in `.claude.json` under `.mcpServers`.
//! Measured 2026-08-13: `settings.json` carries 29 top-level keys and no
//! `mcpServers` among them, and the Claude Code 2.1.231 settings schema carries
//! every MCP key *except* that one, while `~/.claude.json` `.mcpServers` holds
//! the servers that are actually loaded. An `mcpServers` block written into
//! `settings.json` would be ignored, `doctor` would report MCP registered, and
//! recall would be unreachable from inside a session with every check passing.
//!
//! Install only reads `.claude.json`. It places the MCP entry by running
//! `claude mcp` ([`super::claude`]), so Claude Code stays that file's only
//! writer. The read is what decides whether anything runs at all: an entry
//! already in the shape [`mcp_entry_is_current`] accepts means no `claude`
//! process starts (phase 3 D-01).
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
//! there exactly where it was.
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
//! the user added, such as a `timeout`.
//!
//! The MCP entry follows the same rule under `.mcpServers.terminus` (phase 3
//! D-02). Any user-level entry under that key is terminus's to replace, and one
//! naming a binary path that has moved is corrected rather than left to rot.
//! [`mcp_entry`] keeps every key the user put beside the three terminus owns,
//! such as `env`, in the JSON handed to `claude mcp add-json`.
//!
//! # A subprocess, never a shell (phase 3 D-03)
//!
//! v0.1.0's D-06 rule kept install off `claude mcp` with a contract of "no
//! shell and no subprocess". That rule is gone. Phase 3 D-03 replaces it: no
//! shell, and `claude` run as a subprocess by the absolute path found on
//! `PATH`, so `.claude.json` is changed only by the program that owns it.

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

fn home_dir() -> Result<PathBuf, Failure> {
    non_empty_var("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Failure::Operational("HOME is not set".to_owned()))
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

/// Is the entry found at `.mcpServers.terminus` already the one install would
/// place (D-01)?
///
/// `type` is `"stdio"`, `command` decodes to the stable path and `args` is
/// exactly `["mcp"]`. Any other key is ignored. Doctor's `mcp_server` check
/// asks the same question.
pub fn mcp_entry_is_current(found: &Json, stable: &Path) -> bool {
    let args = found.get("args").map(Json::items).unwrap_or(&[]);
    found.get("type").and_then(Json::as_str).as_deref() == Some("stdio")
        && found.get("command").and_then(Json::as_str) == Some(stable.display().to_string())
        && args.len() == 1
        && args[0].as_str().as_deref() == Some("mcp")
}

/// The entry install places, given what was found at `.mcpServers.terminus`
/// (D-02).
///
/// A found object keeps every key it has, with the three terminus owns set in
/// place. Anything else under the key holds nothing to keep, and the entry is
/// those three alone.
pub fn mcp_entry(found: Option<&Json>, stable: &Path) -> Json {
    let mut entry = match found {
        Some(object @ Json::Object(_)) => object.clone(),
        _ => Json::object(),
    };
    let owned = [
        ("type", Json::string("stdio")),
        ("command", Json::string(&stable.display().to_string())),
        ("args", Json::Array(vec![Json::string("mcp")])),
    ];
    for (key, value) in owned {
        set(&mut entry, key, value).expect("the entry is an object, which holds any key");
    }
    entry
}

/// The command that places `entry` by hand:
/// `claude mcp add-json --scope user terminus '<compact JSON>'`.
pub fn add_command(entry: &Json) -> String {
    format!(
        "claude mcp add-json --scope user {MCP_SERVER_KEY} {}",
        shell_quote(&entry.compact())
    )
}

/// The command that finishes a registration install could not complete,
/// built from the entry found under `.mcpServers.terminus`, or from none.
///
/// With an entry there it starts with the remove, because `add-json` refuses
/// a name that already exists (D-06).
pub fn finishing_command(found: Option<&Json>, stable: &Path) -> String {
    let add = add_command(&mcp_entry(found, stable));
    match found {
        Some(_) => format!("claude mcp remove --scope user {MCP_SERVER_KEY} && {add}"),
        None => add,
    }
}

/// `text` as one single-quoted POSIX shell word.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
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
        let once = root.clone();
        add_hooks(&mut root, &stable()).unwrap();
        assert_eq!(once, root);
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

    /// D-01: the three owned keys decide, and a key beside them does not.
    #[test]
    fn a_current_entry_is_the_three_owned_keys_whatever_is_beside_them() {
        let current = parsed(
            r#"{"type": "stdio", "command": "/home/someone/.local/bin/terminus",
               "args": ["mcp"], "env": {"KEEP": "me"}}"#,
        );
        assert!(mcp_entry_is_current(&current, &stable()));

        for wrong in [
            r#"{"type": "sse", "command": "/home/someone/.local/bin/terminus", "args": ["mcp"]}"#,
            r#"{"type": "stdio", "command": "/old/bin/terminus", "args": ["mcp"]}"#,
            r#"{"type": "stdio", "command": "/home/someone/.local/bin/terminus",
                "args": ["mcp", "x"]}"#,
        ] {
            assert!(!mcp_entry_is_current(&parsed(wrong), &stable()), "{wrong}");
        }
    }

    /// D-02: the entry placed over a found object keeps what the user put
    /// there, and sets the three owned keys.
    #[test]
    fn the_entry_placed_over_a_found_one_keeps_its_other_keys() {
        let found =
            parsed(r#"{"type": "sse", "command": "/old/bin/terminus", "env": {"KEEP": "me"}}"#);
        let entry = mcp_entry(Some(&found), &stable());
        assert!(mcp_entry_is_current(&entry, &stable()));
        assert_eq!(
            entry.get("env").unwrap().get("KEEP").unwrap().as_str(),
            Some("me".to_owned()),
            "a key the user put beside ours was dropped"
        );

        let fresh = mcp_entry(None, &stable());
        assert_eq!(fresh.keys(), ["type", "command", "args"]);
    }

    /// The quoted JSON survives a shell exactly, even with a `'` in the path.
    #[test]
    fn the_finishing_command_quotes_the_entry_for_a_shell() {
        let stable = PathBuf::from("/home/o'brien/.local/bin/terminus");
        let command = finishing_command(None, &stable);
        let quoted = command
            .strip_prefix("claude mcp add-json --scope user terminus ")
            .unwrap_or_else(|| panic!("not the add: {command}"));

        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf %s {quoted}"))
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            mcp_entry(None, &stable).compact()
        );
    }

    /// An entry still under the key is removed first, or the add would refuse
    /// the name.
    #[test]
    fn the_finishing_command_for_a_wrong_entry_removes_it_first() {
        let found = parsed(r#"{"type": "sse", "command": "/old/bin/terminus"}"#);
        let command = finishing_command(Some(&found), &stable());
        assert!(
            command.starts_with("claude mcp remove --scope user terminus && "),
            "{command}"
        );
        assert!(
            command.contains("&& claude mcp add-json --scope user terminus '"),
            "{command}"
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
