//! `terminus install`: what it writes, what it refuses to write, and what it
//! leaves exactly as it found it.
//!
//! Every spawn here points `TERMINUS_BIN_DIR`, `CLAUDE_CONFIG_DIR`,
//! `TERMINUS_DATA_DIR`, `TERMINUS_CONFIG_DIR` and `HOME` at temporary
//! directories. That is not tidiness: without `TERMINUS_BIN_DIR` a test would
//! copy a binary over the developer's real `~/.local/bin/terminus`, and without
//! `CLAUDE_CONFIG_DIR` it would edit the `settings.json` and `.claude.json`
//! this machine is running on.
//!
//! `PATH` is a fixture directory holding a fake `claude` and nothing else, so
//! no test can start the real one.
//!
//! The assertions are the ones AC4 and AC5 are written in, spelled in Rust
//! rather than in `jq`: "every other key byte-identical" is a comparison of the
//! file with terminus's own additions removed against the file as it was
//! seeded, and "in its original position" is a comparison of the top-level key
//! order, which is why these tests read key order out of the raw text instead
//! of parsing into a map that would sort it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use terminus_core::testkit::{self, FakeCall, NONESSENTIAL_TRAFFIC};

/// The events install writes an entry for. `cmd::hook::EVENTS` spelled again:
/// `terminus` is a binary crate with no library target, so a test cannot name
/// the constant and has to agree with it.
const EVENTS: [&str; 4] = [
    "SessionStart",
    "UserPromptSubmit",
    "SessionEnd",
    "PostCompact",
];

/// A `settings.json` shaped like the real one: several top-level keys in an
/// order no sort produces, a `hooks` object that already holds the user's own
/// `UserPromptSubmit` script, and a low `cleanupPeriodDays` for the advisory.
const SETTINGS: &str = r#"{
  "cleanupPeriodDays": 7,
  "env": {
    "CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS": "50000"
  },
  "hooks": {
    "SessionStart": [],
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "$HOME/.claude/hooks/terse-answers.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "PreToolUse": []
  },
  "autoCompactEnabled": true,
  "theme": "dark"
}
"#;

/// A `.claude.json` shaped like the real one: an `mcpServers` object that
/// already holds another server, and a `projects` object standing in for the
/// 240 KB of history the real file carries.
const CLAUDE_JSON: &str = r#"{
  "numStartups": 412,
  "mcpServers": {
    "context7": {
      "type": "http",
      "url": "https://example.invalid/mcp"
    }
  },
  "projects": {
    "/data/code/verbatim": {
      "allowedTools": [],
      "history": []
    },
    "/data/code/other": {
      "allowedTools": []
    }
  },
  "oauthAccount": {
    "emailAddress": "someone@example.invalid"
  }
}"#;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    bin_dir: PathBuf,
    claude_dir: PathBuf,
    /// The whole of every spawned terminus's `PATH`: the fake `claude` and
    /// nothing else.
    path_dir: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let bin_dir = root.join("bin");
    let claude_dir = root.join("claude");
    let path_dir = root.join("path");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::create_dir_all(&path_dir).unwrap();
    let fixture = Fixture {
        _dir: dir,
        root,
        bin_dir,
        claude_dir,
        path_dir,
    };
    fixture.fake_claude("", 0);
    fixture
}

impl Fixture {
    /// Where install will put this build.
    fn stable(&self) -> PathBuf {
        self.bin_dir.join("terminus")
    }

    fn settings(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    fn claude_json(&self) -> PathBuf {
        self.claude_dir.join(".claude.json")
    }

    fn seed(&self) -> &Self {
        std::fs::write(self.settings(), SETTINGS).unwrap();
        std::fs::write(self.claude_json(), CLAUDE_JSON).unwrap();
        self
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_at(Path::new(env!("CARGO_BIN_EXE_terminus")), args)
    }

    /// The same environment, but from a named binary rather than the build's.
    ///
    /// Only the stable-path rerun needs it: running install *from* the copy it
    /// placed last time is the case where `current_exe()` stops resolving.
    ///
    /// `PATH` is the fixture's own directory and nothing else, because a system
    /// directory can hold a real `claude`. The traffic variable is taken out so
    /// a logged value can only have come from terminus.
    fn command_at(&self, exe: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(exe);
        command
            .args(args)
            .env("PATH", &self.path_dir)
            .env_remove(NONESSENTIAL_TRAFFIC)
            .env("TERMINUS_BIN_DIR", &self.bin_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("TERMINUS_DATA_DIR", self.root.join("data"))
            .env("TERMINUS_CONFIG_DIR", self.root.join("config"))
            .env("HOME", &self.root);
        command
    }

    /// Put a fake `claude` on the fixture's `PATH` that prints `stderr` and
    /// exits with `status`.
    fn fake_claude(&self, stderr: &str, status: i32) {
        testkit::fake_claude(&self.path_dir, &self.claude_log(), stderr, status);
    }

    fn claude_log(&self) -> PathBuf {
        self.root.join("claude.log")
    }

    /// Every `claude` call terminus made, oldest first.
    fn calls(&self) -> Vec<FakeCall> {
        testkit::fake_claude_calls(&self.claude_log())
    }

    /// Run with stdin at end of file, which is what a command in a script sees.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs")
    }

    fn backups(&self) -> Vec<String> {
        let mut found: Vec<String> = std::fs::read_dir(&self.claude_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".terminus-backup"))
            .collect();
        found.sort();
        found
    }
}

/// The arguments of `claude mcp add-json --scope user terminus <JSON>`, up to
/// the JSON.
const ADD: [&str; 5] = ["mcp", "add-json", "--scope", "user", "terminus"];

/// The arguments of `claude mcp remove --scope user terminus`.
const REMOVE: [&str; 5] = ["mcp", "remove", "--scope", "user", "terminus"];

/// The entry a logged add carried, after checking the call was the add and
/// carried the traffic variable.
fn added_entry(call: &FakeCall) -> serde_json::Value {
    assert_eq!(call.args.len(), 6, "{call:?}");
    assert_eq!(call.args[..5], ADD, "{call:?}");
    assert_eq!(call.traffic.as_deref(), Some("1"), "{call:?}");
    serde_json::from_str(&call.args[5]).expect("the add's last argument is JSON")
}

/// `type` `"stdio"`, `command` the stable path, `args` `["mcp"]`.
fn assert_current(entry: &serde_json::Value, stable: &Path) {
    assert_eq!(entry["type"], "stdio", "{entry}");
    assert_eq!(
        entry["command"],
        serde_json::Value::from(stable.display().to_string()),
        "{entry}"
    );
    assert_eq!(entry["args"], serde_json::json!(["mcp"]), "{entry}");
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The top-level keys of a JSON object as the file spells them, in order.
///
/// `serde_json::Value` is a `BTreeMap` and would sort them, which is exactly
/// the property under test, so this reads the raw text: a scan that tracks
/// brace depth and string state, and reports the key of every `"key":` at
/// depth 1.
fn key_order(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let bytes = text.as_bytes();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut at = 0usize;
    while at < bytes.len() {
        match bytes[at] {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            b'"' => {
                let start = at;
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                let literal = &text[start..(at + 1).min(text.len())];
                let mut after = at + 1;
                while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                    after += 1;
                }
                if depth == 1 && bytes.get(after) == Some(&b':') {
                    keys.push(literal.trim_matches('"').to_owned());
                }
            }
            _ => {}
        }
        at += 1;
    }
    keys
}

/// The same JSON, indented four spaces per level instead of two.
///
/// What an editor's "format document" leaves behind, and a file terminus's
/// renderer did not produce.
fn four_space(text: &str) -> String {
    text.lines()
        .map(|line| {
            let depth = line.len() - line.trim_start().len();
            format!("{}{}\n", " ".repeat(depth * 2), line.trim_start())
        })
        .collect()
}

/// Every entry in `settings.json` whose command is the stable path.
fn our_entries(settings: &serde_json::Value, stable: &Path) -> Vec<serde_json::Value> {
    let want = serde_json::Value::from(stable.display().to_string());
    settings["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|events| events.values())
        .filter_map(|groups| groups.as_array())
        .flatten()
        .filter_map(|group| group["hooks"].as_array())
        .flatten()
        .filter(|entry| entry["command"] == want)
        .cloned()
        .collect()
}

/// The seeded file with everything install added taken back out: its four
/// groups, and the event keys it had to create to hold them.
///
/// The second half is the reason this takes `seeded`. `SessionEnd` and
/// `PostCompact` are not in the seeded `hooks` object at all, so install
/// creates them; `SessionStart` is there as an empty array and must come back
/// as one. "Every other key byte-identical" is a claim about the keys install
/// did not add, and telling those apart needs the file as it was.
fn without_ours(
    mut settings: serde_json::Value,
    stable: &Path,
    seeded: &serde_json::Value,
) -> serde_json::Value {
    let want = serde_json::Value::from(stable.display().to_string());
    let events: Vec<String> = settings["hooks"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    for event in events {
        let Some(groups) = settings["hooks"][&event].as_array_mut() else {
            continue;
        };
        groups.retain(|group| {
            !group["hooks"]
                .as_array()
                .is_some_and(|entries| entries.iter().any(|entry| entry["command"] == want))
        });
        if groups.is_empty() && seeded["hooks"].get(&event).is_none() {
            settings["hooks"].as_object_mut().unwrap().remove(&event);
        }
    }
    settings
}

// ---------------------------------------------------------------------------
// Task 1: the binary at the stable path
// ---------------------------------------------------------------------------

/// The copy is this build, it is executable, and a second run over it is fine.
#[test]
fn install_places_this_build_at_the_stable_path() {
    let fixture = fixture();
    fixture.seed();

    let first = fixture.run(&["install"]);
    assert!(first.status.success(), "{}", text(&first));

    let stable = fixture.stable();
    let version = fixture
        .command_at(&stable, &["--version"])
        .output()
        .unwrap();
    assert!(version.status.success(), "{}", text(&version));
    let ours = fixture.command(&["--version"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        String::from_utf8_lossy(&ours.stdout),
        "the copy is not this build"
    );

    let second = fixture.run(&["install"]);
    assert!(
        second.status.success(),
        "a second install over our own copy failed: {}",
        text(&second)
    );
}

/// D-07: a program that is not a terminus build is refused, not overwritten,
/// and the refusal names a command that makes the same install succeed.
#[test]
fn a_foreign_binary_at_the_stable_path_stops_everything() {
    let fixture = fixture();
    fixture.seed();

    // Something small, real and executable that is not this build.
    let foreign = b"#!/bin/sh\nexit 0\n";
    std::fs::write(fixture.stable(), foreign).unwrap();
    let settings_before = std::fs::read_to_string(fixture.settings()).unwrap();
    let claude_before = std::fs::read_to_string(fixture.claude_json()).unwrap();

    let output = fixture.run(&["install"]);
    assert!(!output.status.success(), "install overwrote a stranger");
    let said = text(&output);
    assert!(
        said.contains(&fixture.stable().display().to_string()),
        "the refusal did not name the path: {said}"
    );
    assert!(
        said.contains("rm -f"),
        "the refusal printed no command that fixes it: {said}"
    );

    assert_eq!(std::fs::read(fixture.stable()).unwrap(), foreign);
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        settings_before
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        claude_before
    );
    assert!(fixture.backups().is_empty(), "a refusal wrote a backup");
    assert_eq!(fixture.calls(), Vec::new(), "a refusal ran claude");
}

/// A settings file with a repeated key is refused, loudly, and nothing is
/// written.
///
/// `JSON.parse` - Claude Code's reader - takes the last of a repeated key.
/// Install's reader takes the first. Editing the wrong one would put four hook
/// entries in the copy Claude Code ignores and report success, so the file is
/// refused rather than guessed at.
#[test]
fn a_settings_file_with_a_repeated_key_is_refused() {
    let fixture = fixture();
    fixture.seed();
    let duplicated = SETTINGS.replace("  \"hooks\": {", "  \"hooks\": {},\n  \"hooks\": {");
    assert_eq!(
        duplicated.matches("\"hooks\": {").count(),
        2,
        "the fixture does not carry the duplicate under test"
    );
    std::fs::write(fixture.settings(), &duplicated).unwrap();

    let output = fixture.run(&["install"]);
    assert!(!output.status.success(), "install edited an ambiguous file");
    let said = text(&output);
    assert!(
        said.contains("more than one '.hooks' key"),
        "the refusal did not say which key: {said}"
    );

    assert!(!fixture.stable().exists(), "a refusal placed the binary");
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        duplicated
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        CLAUDE_JSON
    );
    assert!(fixture.backups().is_empty(), "a refusal wrote a backup");
    assert_eq!(fixture.calls(), Vec::new(), "a refusal ran claude");
}

/// A stale file at the temporary name install is about to use is stepped over,
/// not opened through.
///
/// Both temporaries are named from the process id, so a symlink left behind by
/// a crashed run - or one landed on by a pid that came round again - is enough
/// for `File::create` to truncate whatever it points at and for the rename that
/// follows to move the link rather than the file.
///
/// The pause is a shell that reads one line and then `exec`s install, so the
/// pid the test sees is the pid install runs under and the stale names can be
/// planted before install starts.
#[test]
fn a_stale_temporary_name_is_stepped_over_rather_than_followed() {
    let fixture = fixture();
    fixture.seed();
    let mut paused = fixture
        .command_at(
            Path::new("/bin/sh"),
            &[
                "-c",
                "read -r line; exec \"$0\" install",
                env!("CARGO_BIN_EXE_terminus"),
            ],
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the shell runs");
    let pid = paused.id();

    // One victim per temporary install writes, reached through the exact name
    // it builds out of its own pid.
    let victims = [
        (
            fixture.root.join("victim-binary"),
            fixture.bin_dir.join(format!(".terminus-install-{pid}")),
        ),
        (
            fixture.root.join("victim-settings"),
            fixture
                .claude_dir
                .join(format!(".settings.json.terminus-{pid}")),
        ),
    ];
    for (victim, stale) in &victims {
        std::fs::write(victim, b"not yours\n").unwrap();
        std::os::unix::fs::symlink(victim, stale).unwrap();
    }

    let mut go = paused.stdin.take().unwrap();
    go.write_all(b"\n").unwrap();
    drop(go);
    let output = paused.wait_with_output().unwrap();
    let said = text(&output);
    assert!(output.status.success(), "{said}");
    for (victim, _) in &victims {
        assert_eq!(
            std::fs::read(victim).unwrap(),
            b"not yours\n",
            "install wrote through a stale temporary name into {}",
            victim.display()
        );
    }

    // And it still did the whole job, under another name.
    assert!(fixture.stable().exists(), "{said}");
    assert_eq!(
        our_entries(&read(&fixture.settings()), &fixture.stable()).len(),
        4
    );
}

/// The backup's name is taken, never written through.
///
/// A dangling symlink at it is the case `Path::exists` answers wrongly: it
/// follows the link, finds nothing, and reports the name free - and the copy
/// that follows then creates the link's target, somewhere install was never
/// asked to write.
#[test]
fn a_symlink_at_the_backup_name_is_not_written_through() {
    let fixture = fixture();
    fixture.seed();
    let victim = fixture.root.join("not-a-backup");
    std::os::unix::fs::symlink(
        &victim,
        fixture.claude_dir.join("settings.json.terminus-backup"),
    )
    .unwrap();

    let output = fixture.run(&["install"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        !victim.exists(),
        "install wrote a backup through a symlink, into {}",
        victim.display()
    );

    // And it still did the job it was asked to do.
    assert_eq!(
        our_entries(&read(&fixture.settings()), &fixture.stable()).len(),
        4
    );
}

// ---------------------------------------------------------------------------
// Task 3: the four hook entries
// ---------------------------------------------------------------------------

/// AC4 and AC5 for `settings.json`: four entries after two runs, the user's own
/// group untouched, every other key where it was, one backup, and an upgrade
/// that rewrites no byte of `hooks`.
#[test]
fn two_installs_leave_one_hook_entry_per_event_and_change_nothing_else() {
    let fixture = fixture();
    fixture.seed();
    let before_keys = key_order(&fixture.settings());

    assert!(fixture.run(&["install"]).status.success());
    let after_first = std::fs::read_to_string(fixture.settings()).unwrap();
    assert!(fixture.run(&["install"]).status.success());

    let settings = read(&fixture.settings());
    let ours = our_entries(&settings, &fixture.stable());
    assert_eq!(ours.len(), 4, "expected one entry per event, got {ours:#?}");

    let mut events: Vec<String> = ours
        .iter()
        .map(|entry| entry["args"][1].as_str().unwrap().to_owned())
        .collect();
    events.sort();
    let mut expected: Vec<String> = EVENTS.iter().map(|e| (*e).to_owned()).collect();
    expected.sort();
    assert_eq!(events, expected);
    for entry in &ours {
        // Exec form (D-01): `args` present, and `hook` in front of the event.
        assert_eq!(entry["type"], "command");
        assert_eq!(entry["args"][0], "hook");
    }

    // The user's own script is still there, unmodified.
    let prompt = settings["hooks"]["UserPromptSubmit"].as_array().unwrap();
    assert_eq!(prompt.len(), 2);
    assert_eq!(
        prompt[0]["hooks"][0]["command"],
        serde_json::Value::from("$HOME/.claude/hooks/terse-answers.sh")
    );
    assert_eq!(prompt[0]["hooks"][0]["timeout"], serde_json::Value::from(5));

    // Everything install added, taken back out, is the file it was given.
    let seeded: serde_json::Value = serde_json::from_str(SETTINGS).unwrap();
    assert_eq!(without_ours(settings, &fixture.stable(), &seeded), seeded);
    assert_eq!(key_order(&fixture.settings()), before_keys);

    assert_eq!(
        fixture.backups(),
        vec!["settings.json.terminus-backup"],
        "expected exactly one backup, of settings.json"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_dir.join("settings.json.terminus-backup")).unwrap(),
        SETTINGS,
        "the backup is not the pre-install bytes"
    );

    // The second run rewrote nothing at all.
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        after_first,
        "a second install rewrote settings.json"
    );
}

/// INST-05: replacing the binary and rerunning install changes no byte of the
/// `hooks` object, because the path the entries name never moved.
#[test]
fn an_upgrade_rewrites_no_byte_of_the_hooks_object() {
    let fixture = fixture();
    fixture.seed();
    assert!(fixture.run(&["install"]).status.success());
    let hooks = read(&fixture.settings())["hooks"].clone();
    let raw = std::fs::read_to_string(fixture.settings()).unwrap();

    // The upgrade: different bytes at the stable path, still a terminus build.
    let mut replacement = std::fs::read(fixture.stable()).unwrap();
    replacement.extend_from_slice(b"\0a later build\0");
    std::fs::write(fixture.stable(), &replacement).unwrap();

    assert!(fixture.run(&["install"]).status.success());
    assert_eq!(read(&fixture.settings())["hooks"], hooks);
    assert_eq!(std::fs::read_to_string(fixture.settings()).unwrap(), raw);
}

/// INST-05: an entry that is terminus's and the wrong shape is repaired, not
/// counted as done.
///
/// A hand edit, or an entry an older terminus wrote with a different arg shape,
/// has the right command and the wrong `args`. Idempotence checked by presence
/// alone would call `SessionStart` handled and leave it with no working hook at
/// all - permanently, because every later install would agree.
#[test]
fn an_entry_of_ours_in_the_wrong_shape_is_repaired_rather_than_skipped() {
    let fixture = fixture();
    fixture.seed();
    // JSON needs any backslash in the path doubled; nothing else about the path
    // changes.
    let stable = fixture.stable().display().to_string().replace('\\', "\\\\");
    let seeded = format!(
        r#"{{
  "theme": "dark",
  "hooks": {{
    "SessionStart": [
      {{
        "matcher": "",
        "hooks": [
          {{
            "command": "{stable}",
            "args": [
              "hook",
              "SessionEnd"
            ],
            "timeout": 30
          }}
        ]
      }}
    ]
  }}
}}
"#
    );
    std::fs::write(fixture.settings(), &seeded).unwrap();

    assert!(fixture.run(&["install"]).status.success());

    let settings = read(&fixture.settings());
    let groups = settings["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "the repair appended a second group");
    let entry = &groups[0]["hooks"][0];
    assert_eq!(entry["type"], "command");
    assert_eq!(entry["args"][1], "SessionStart");
    assert_eq!(
        entry["timeout"],
        serde_json::Value::from(30),
        "a key beside ours was dropped"
    );
    // AC2 still holds: one entry per event, four in all.
    assert_eq!(our_entries(&settings, &fixture.stable()).len(), 4);

    // AC3 still holds: with everything in the intended shape, a second install
    // rewrites nothing.
    let after_first = std::fs::read_to_string(fixture.settings()).unwrap();
    assert!(fixture.run(&["install"]).status.success());
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        after_first,
        "a second install rewrote settings.json"
    );
}

// ---------------------------------------------------------------------------
// Task 4: the MCP registration
// ---------------------------------------------------------------------------

/// Install places the MCP entry by running `claude mcp add-json`, and never
/// writes `.claude.json` itself.
///
/// The fake writes nothing, so the file is byte-identical after two installs,
/// and each install finds no entry and asks for the add once.
#[test]
fn two_installs_leave_claude_json_alone_and_each_asks_claude_for_the_add() {
    let fixture = fixture();
    fixture.seed();

    assert!(fixture.run(&["install"]).status.success());
    assert!(fixture.run(&["install"]).status.success());

    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        CLAUDE_JSON,
        "install wrote .claude.json"
    );
    assert!(
        !fixture
            .claude_dir
            .join(".claude.json.terminus-backup")
            .exists(),
        "install backed up a file it does not write"
    );
    let calls = fixture.calls();
    assert_eq!(calls.len(), 2, "{calls:#?}");
    for call in &calls {
        assert_current(&added_entry(call), &fixture.stable());
    }
}

/// An entry under terminus's key in the wrong shape is removed and placed
/// again, and the add keeps what the user put beside the three keys terminus
/// owns (D-02, D-06).
#[test]
fn a_stale_mcp_registration_is_removed_and_added_again() {
    let fixture = fixture();
    fixture.seed();
    let stale = CLAUDE_JSON.replace(
        "    \"context7\": {",
        "    \"terminus\": {\n      \"type\": \"sse\",\n      \"command\": \
         \"/old/bin/terminus\",\n      \"env\": {\n        \"KEEP\": \"me\"\n      }\n    },\n    \
         \"context7\": {",
    );
    std::fs::write(fixture.claude_json(), &stale).unwrap();

    let output = fixture.run(&["install"]);
    assert!(output.status.success(), "{}", text(&output));

    let calls = fixture.calls();
    assert_eq!(calls.len(), 2, "{calls:#?}");
    assert_eq!(calls[0].args, REMOVE);
    assert_eq!(calls[0].traffic.as_deref(), Some("1"));
    let entry = added_entry(&calls[1]);
    assert_current(&entry, &fixture.stable());
    assert_eq!(
        entry["env"]["KEEP"], "me",
        "a key beside ours was dropped: {entry}"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        stale
    );
}

/// An entry already current starts no `claude` at all (D-01).
#[test]
fn a_current_mcp_registration_runs_no_claude() {
    let fixture = fixture();
    fixture.seed();
    let current = CLAUDE_JSON.replace(
        "    \"context7\": {",
        &format!(
            "    \"terminus\": {{\"type\": \"stdio\", \"command\": {}, \"args\": [\"mcp\"]}},\n    \
             \"context7\": {{",
            serde_json::Value::from(fixture.stable().display().to_string())
        ),
    );
    std::fs::write(fixture.claude_json(), &current).unwrap();

    let output = fixture.run(&["install"]);
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(fixture.calls(), Vec::new(), "a current entry ran claude");
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        current
    );
}

/// What every incomplete install must still have done, and how it must say
/// so (D-05): the binary placed, the four hook entries merged, the backfill
/// started, `.claude.json` untouched, the command that finishes it, and exit 1.
fn assert_incomplete(fixture: &Fixture, output: &Output, claude_json: &str) -> String {
    let said = text(output);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(fixture.stable().exists(), "{said}");
    assert_eq!(
        our_entries(&read(&fixture.settings()), &fixture.stable()).len(),
        4,
        "{said}"
    );
    assert!(said.contains("incomplete"), "{said}");
    assert!(
        said.contains("terminus status"),
        "the backfill was not started: {said}"
    );
    assert!(
        said.lines()
            .any(|line| line.contains("claude mcp add-json --scope user terminus '")),
        "no command finishes the registration: {said}"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        claude_json
    );
    said
}

/// No `claude` on `PATH` leaves the registration, and only the registration,
/// undone.
#[test]
fn without_claude_on_path_the_install_is_incomplete() {
    let fixture = fixture();
    fixture.seed();
    std::fs::remove_file(fixture.path_dir.join("claude")).unwrap();

    let output = fixture.run(&["install"]);
    let said = assert_incomplete(&fixture, &output, CLAUDE_JSON);
    assert!(said.contains("not on PATH"), "{said}");
}

/// A `claude` that fails is reported by the first line of what it said.
#[test]
fn a_claude_that_fails_leaves_the_install_incomplete_with_its_first_line() {
    let fixture = fixture();
    fixture.seed();
    fixture.fake_claude("first\nsecond", 1);

    let output = fixture.run(&["install"]);
    let said = assert_incomplete(&fixture, &output, CLAUDE_JSON);
    // The cause is the first line exactly, and the second is nowhere. Not a
    // bare `contains("second")`: the backfill estimate can say "under a
    // second".
    assert!(
        said.lines()
            .any(|line| line.contains("mcp server") && line.ends_with("(not registered: first)")),
        "{said}"
    );
    assert!(!said.lines().any(|line| line.trim() == "second"), "{said}");
}

/// A `.claude.json` that does not parse starts no `claude`, and no longer
/// stops the rest of the install.
#[test]
fn an_unreadable_claude_json_leaves_the_install_incomplete_and_runs_no_claude() {
    let fixture = fixture();
    fixture.seed();
    std::fs::write(fixture.claude_json(), "{").unwrap();

    let output = fixture.run(&["install"]);
    assert_incomplete(&fixture, &output, "{");
    assert_eq!(fixture.calls(), Vec::new());
}

// ---------------------------------------------------------------------------
// Install asks nothing (phase 3 D-08, D-12)
// ---------------------------------------------------------------------------

/// D-12: a settings file terminus's renderer did not produce is rewritten in
/// full, two-space indented, without a question. The backup holds the original
/// bytes and the summary says what happened to the file.
#[test]
fn a_differently_formatted_file_is_rewritten_in_full_and_the_summary_says_so() {
    let fixture = fixture();
    fixture.seed();
    std::fs::write(fixture.settings(), four_space(SETTINGS)).unwrap();

    let output = fixture.run(&["install"]);
    assert!(output.status.success(), "{}", text(&output));
    let said = text(&output);
    assert!(!said.contains("[Y/n]"), "install asked:\n{said}");
    assert!(
        said.contains("rewritten in full in two-space form, keys and values unchanged"),
        "the summary did not say the file was rewritten:\n{said}"
    );
    assert!(
        said.lines()
            .any(|line| line.contains("the original bytes are in")
                && line.contains("settings.json.terminus-backup")),
        "the summary did not say where the original is:\n{said}"
    );

    let after = std::fs::read_to_string(fixture.settings()).unwrap();
    assert!(
        after.contains("\n  \"theme\": \"dark\""),
        "the file was not rewritten in two-space form:\n{after}"
    );
    assert_eq!(
        our_entries(&read(&fixture.settings()), &fixture.stable()).len(),
        4
    );
    // The backup is still the bytes that were there before any of this.
    assert_eq!(
        std::fs::read_to_string(fixture.claude_dir.join("settings.json.terminus-backup")).unwrap(),
        four_space(SETTINGS)
    );
}

/// AC1: `--yes` and `-y` are misuse now that there is nothing to say yes to.
/// Exit 2, and nothing is placed, written or run.
#[test]
fn install_takes_no_yes_flag() {
    for flag in ["--yes", "-y"] {
        let fixture = fixture();
        fixture.seed();
        let output = fixture.run(&["install", flag]);
        assert_eq!(output.status.code(), Some(2), "{flag}: {}", text(&output));
        assert!(!fixture.stable().exists(), "{flag}");
        assert_eq!(
            std::fs::read_to_string(fixture.settings()).unwrap(),
            SETTINGS,
            "{flag}"
        );
        assert_eq!(fixture.calls(), Vec::new(), "{flag}");
    }
}

/// Wait for `child` under a deadline, so an install that blocks on stdin fails
/// the test rather than hanging it.
fn finish_within(mut child: Child, seconds: u64) -> Output {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "install was still running after {seconds}s, waiting on stdin:\n{}",
                text(&output)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

/// AC1: install reads nothing from stdin. A run with stdin closed and a run
/// with stdin open and never written both finish, ask nothing, and leave the
/// same files and the same `claude` calls behind.
///
/// No test can attach a terminal. Install has no `confirm` call and no
/// terminal check, so a terminal takes the same path as these two runs.
#[test]
fn install_reads_nothing_from_stdin() {
    let closed = fixture();
    closed.seed();
    let held = fixture();
    held.seed();

    let first = finish_within(
        closed
            .command(&["install"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        30,
    );
    let mut child = held
        .command(&["install"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Held open, never written, and dropped only once install has exited.
    let open_stdin = child.stdin.take();
    let second = finish_within(child, 30);
    drop(open_stdin);

    for (fixture, output) in [(&closed, &first), (&held, &second)] {
        let said = text(output);
        assert!(output.status.success(), "{said}");
        assert!(!said.contains("[Y/n]") && !said.contains("[y/N]"), "{said}");
        assert!(
            !said.lines().any(|line| line.trim_end().ends_with('?')),
            "install asked a question:\n{said}"
        );
        assert!(fixture.stable().exists());
    }

    let root = |fixture: &Fixture, text: String| {
        text.replace(&fixture.root.display().to_string(), "<root>")
    };
    let settings = |fixture: &Fixture| {
        root(
            fixture,
            std::fs::read_to_string(fixture.settings()).unwrap(),
        )
    };
    let calls = |fixture: &Fixture| {
        let calls: Vec<Vec<String>> = fixture
            .calls()
            .into_iter()
            .map(|call| {
                assert_eq!(call.traffic.as_deref(), Some("1"));
                call.args
                    .into_iter()
                    .map(|arg| root(fixture, arg))
                    .collect()
            })
            .collect();
        calls
    };
    assert_eq!(settings(&closed), settings(&held));
    assert_eq!(
        std::fs::read(closed.stable()).unwrap(),
        std::fs::read(held.stable()).unwrap()
    );
    assert_eq!(calls(&closed), calls(&held));
    assert_eq!(calls(&closed).len(), 1, "{:?}", calls(&closed));
}

/// D-24: `install` is human-only. `--json` is misuse, exit 2.
#[test]
fn install_takes_no_json_flag() {
    let fixture = fixture();
    fixture.seed();
    let output = fixture.run(&["install", "--json"]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
    assert!(!fixture.stable().exists());
}

// ---------------------------------------------------------------------------
// Task 6: the advisories, and the settings install never changes
// ---------------------------------------------------------------------------

/// INST-04 and D-11: the receipt states `cleanupPeriodDays` at every value,
/// says what is already gone, and prints the line that raises it, and install
/// changes neither it nor `autoCompactEnabled`. It also names every file the
/// run touched and where to watch the backfill.
#[test]
fn install_reports_the_two_settings_it_never_changes() {
    let seven = "  \"cleanupPeriodDays\": 7,\n";
    assert!(SETTINGS.contains(seven));
    let seeds = [
        (SETTINGS.to_owned(), "cleanupPeriodDays is 7.", 7),
        (
            SETTINGS.replace(seven, "  \"cleanupPeriodDays\": 3650,\n"),
            "cleanupPeriodDays is 3650.",
            3650,
        ),
        (
            SETTINGS.replace(seven, ""),
            "cleanupPeriodDays is unset, so Claude Code's default of 30 applies.",
            30,
        ),
    ];

    for (settings, stated, days) in seeds {
        let fixture = fixture();
        fixture.seed();
        std::fs::write(fixture.settings(), &settings).unwrap();
        let before = read(&fixture.settings());

        let output = fixture.run(&["install"]);
        let said = text(&output);
        assert!(output.status.success(), "{said}");

        assert!(said.contains(stated), "{said}");
        assert!(
            said.contains(&format!("older than {days} days are already gone")),
            "{said}"
        );
        assert!(
            said.contains(&format!(
                "set \"cleanupPeriodDays\": 3650 in {}",
                fixture.settings().display()
            )),
            "{said}"
        );
        assert!(said.contains("autoCompactEnabled"), "{said}");

        let backup = fixture.claude_dir.join("settings.json.terminus-backup");
        assert!(backup.exists());
        for named in [
            fixture.stable(),
            fixture.settings(),
            backup,
            fixture.claude_json(),
        ] {
            assert!(
                said.contains(&named.display().to_string()),
                "the receipt does not name {}: {said}",
                named.display()
            );
        }
        assert!(said.contains("terminus status"), "{said}");

        let after = read(&fixture.settings());
        assert_eq!(after["cleanupPeriodDays"], before["cleanupPeriodDays"]);
        assert_eq!(after["autoCompactEnabled"], before["autoCompactEnabled"]);
    }
}

/// D-10: with a transcript on disk, the receipt carries the backfill estimate.
#[test]
fn the_receipt_carries_the_backfill_estimate() {
    let fixture = fixture();
    fixture.seed();
    let project = fixture.claude_dir.join("projects").join("-tmp-receipt");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(
        testkit::fixture_path("session-basic.jsonl"),
        project.join("11111111-1111-4111-8111-111111111111.jsonl"),
    )
    .unwrap();

    let output = fixture.run(&["install"]);
    let said = text(&output);
    assert!(output.status.success(), "{said}");
    assert!(said.contains("transcript(s)"), "{said}");
    assert!(said.contains("not yet archived"), "{said}");
    assert!(said.contains("terminus status"), "{said}");

    // The backfill it started runs detached; let it finish before the
    // fixture's directory goes.
    wait_for_sessions(&fixture, 1);
}

/// The closing summary names where things went, and the paths it names are
/// real: the data directory is `status`'s store's parent, and each backup it
/// printed exists.
#[test]
fn the_summary_names_the_data_directory_and_the_backups_it_wrote() {
    let fixture = fixture();
    fixture.seed();

    let output = fixture.run(&["install"]);
    let said = text(&output);
    let data_dir = said
        .lines()
        .find_map(|line| line.trim().strip_prefix("data directory"))
        .expect("the summary names the data directory")
        .trim()
        .to_owned();

    // `status` has to open a store, so ingest once first. `status --json`
    // reports the store file; its parent is what install printed.
    assert!(fixture.run(&["ingest"]).status.success());
    let status = fixture.run(&["status", "--json"]);
    let document: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("status emits one document");
    let store = PathBuf::from(document["data"]["store"].as_str().unwrap());
    assert_eq!(store.parent().unwrap(), Path::new(&data_dir));

    for line in said.lines() {
        if let Some(path) = line.trim().strip_prefix("backup ") {
            assert!(
                Path::new(path.trim()).exists(),
                "the summary named a backup that is not there: {path}"
            );
        }
    }
    assert_eq!(fixture.backups().len(), 1);
}

/// AC8/ING-11, the case a user actually hits: `~/.local/bin/terminus install`.
///
/// `binary::place` renames a new binary over the stable path, which unlinks the
/// inode the running process was exec'd from. Install then asks for a backfill.
/// Spawning `current_exe()` at that point spawns a deleted file and fails
/// ENOENT, so the backfill install has just announced never starts and the
/// store does not grow - silently, because a backfill cannot fail an install.
/// The rerun install is exactly what doctor's own fix line tells a user to run.
#[test]
fn install_from_the_stable_path_still_starts_its_backfill() {
    let fixture = fixture();
    fixture.seed();

    let project = fixture.claude_dir.join("projects").join("-tmp-install");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = |name: &str, fixture_name: &str| {
        std::fs::copy(
            terminus_core::testkit::fixture_path(fixture_name),
            project.join(name),
        )
        .unwrap();
    };

    transcript(
        "11111111-1111-4111-8111-111111111111.jsonl",
        "session-basic.jsonl",
    );
    assert!(fixture.run(&["install"]).status.success());
    wait_for_sessions(&fixture, 1);

    // A second session, so the rerun has work to find and the assertion is
    // about this install rather than the first one.
    transcript(
        "22222222-2222-4222-8222-222222222222.jsonl",
        "session-continuation.jsonl",
    );

    let output = fixture
        .command_at(&fixture.stable(), &["install"])
        .stdin(Stdio::null())
        .output()
        .expect("the placed binary runs");
    let said = text(&output);
    assert!(output.status.success(), "the rerun install failed: {said}");
    assert!(
        !said.contains("the backfill could not be started"),
        "install spawned a binary it had just replaced: {said}"
    );

    wait_for_sessions(&fixture, 2);
}

/// Block until the store holds `expected` sessions, or fail.
fn wait_for_sessions(fixture: &Fixture, expected: i64) {
    let db = fixture
        .root
        .join("data")
        .join(terminus_core::store::DB_FILE_NAME);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let count = || -> i64 {
        if !db.exists() {
            return 0;
        }
        let Ok(conn) = rusqlite::Connection::open(&db) else {
            return 0;
        };
        conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap_or(0)
    };
    while count() < expected {
        assert!(
            std::time::Instant::now() < deadline,
            "the detached backfill never archived {expected} sessions: {} after 30s",
            count()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
