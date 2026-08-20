//! `verbatim install`: what it writes, what it refuses to write, and what it
//! leaves exactly as it found it.
//!
//! Every spawn here points `VERBATIM_BIN_DIR`, `CLAUDE_CONFIG_DIR`,
//! `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` and `HOME` at temporary
//! directories. That is not tidiness: without `VERBATIM_BIN_DIR` a test would
//! copy a binary over the developer's real `~/.local/bin/verbatim`, and without
//! `CLAUDE_CONFIG_DIR` it would edit the `settings.json` and `.claude.json`
//! this machine is running on.
//!
//! The assertions are the ones AC4 and AC5 are written in, spelled in Rust
//! rather than in `jq`: "every other key byte-identical" is a comparison of the
//! file with verbatim's own additions removed against the file as it was
//! seeded, and "in its original position" is a comparison of the top-level key
//! order, which is why these tests read key order out of the raw text instead
//! of parsing into a map that would sort it.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

/// The events install writes an entry for. `cmd::hook::EVENTS` spelled again:
/// `verbatim` is a binary crate with no library target, so a test cannot name
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
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let bin_dir = root.join("bin");
    let claude_dir = root.join("claude");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Fixture {
        _dir: dir,
        root,
        bin_dir,
        claude_dir,
    }
}

impl Fixture {
    /// Where install will put this build.
    fn stable(&self) -> PathBuf {
        self.bin_dir.join(if cfg!(windows) {
            "verbatim.exe"
        } else {
            "verbatim"
        })
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
        self.command_at(Path::new(env!("CARGO_BIN_EXE_verbatim")), args)
    }

    /// The same environment, but from a named binary rather than the build's.
    ///
    /// Only the stable-path rerun needs it: running install *from* the copy it
    /// placed last time is the case where `current_exe()` stops resolving.
    fn command_at(&self, exe: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(exe);
        command
            .args(args)
            .env("VERBATIM_BIN_DIR", &self.bin_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("VERBATIM_DATA_DIR", self.root.join("data"))
            .env("VERBATIM_CONFIG_DIR", self.root.join("config"))
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root);
        command
    }

    /// Run with stdin at end of file, which is what a script that did not
    /// answer looks like.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs")
    }

    /// Run and write `answer` to the confirmation.
    fn answer(&self, args: &[&str], answer: &str) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary runs");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// Spawn install and read up to its confirmation, leaving the question
    /// unanswered.
    ///
    /// The pause is the point: it is the one moment install has decided
    /// everything it will do and has done none of it, which is where a check
    /// made before the answer and a write made after it come apart.
    // The child is waited on in `Paused::answer`, which is the only way to end
    // one; clippy cannot see across the return.
    #[allow(clippy::zombie_processes)]
    fn at_the_prompt(&self, args: &[&str]) -> Paused {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary runs");
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut shown = String::new();
        loop {
            let mut line = String::new();
            assert!(
                out.read_line(&mut line).unwrap() > 0,
                "install never asked:\n{shown}"
            );
            shown.push_str(&line);
            if line.contains("apply these changes?") {
                return Paused { child, out, shown };
            }
        }
    }

    fn backups(&self) -> Vec<String> {
        let mut found: Vec<String> = std::fs::read_dir(&self.claude_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".verbatim-backup"))
            .collect();
        found.sort();
        found
    }
}

/// An install stopped at its confirmation, with everything it printed so far.
struct Paused {
    child: Child,
    out: std::io::BufReader<std::process::ChildStdout>,
    shown: String,
}

impl Paused {
    /// The pid install is running under, and so the pid its temporary file
    /// names are built from.
    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Answer the question and collect what install did with the answer.
    fn answer(mut self, answer: &str) -> (std::process::ExitStatus, String) {
        self.child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        let mut rest = String::new();
        self.out.read_to_string(&mut rest).unwrap();
        let mut errors = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        let status = self.child.wait().unwrap();
        (status, format!("{}{rest}{errors}", self.shown))
    }
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
/// What an editor's "format document" leaves behind, and a file verbatim's
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

    let first = fixture.run(&["install", "--yes"]);
    assert!(first.status.success(), "{}", text(&first));

    let stable = fixture.stable();
    let version = Command::new(&stable).arg("--version").output().unwrap();
    assert!(version.status.success(), "{}", text(&version));
    let ours = Command::new(env!("CARGO_BIN_EXE_verbatim"))
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        String::from_utf8_lossy(&ours.stdout),
        "the copy is not this build"
    );

    let second = fixture.run(&["install", "--yes"]);
    assert!(
        second.status.success(),
        "a second install over our own copy failed: {}",
        text(&second)
    );
}

/// D-07: a program that is not a verbatim build is refused, not overwritten,
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

    let output = fixture.run(&["install", "--yes"]);
    assert!(!output.status.success(), "install overwrote a stranger");
    let said = text(&output);
    assert!(
        said.contains(&fixture.stable().display().to_string()),
        "the refusal did not name the path: {said}"
    );
    assert!(
        said.contains("rm -f") || said.contains("del "),
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

    let output = fixture.run(&["install", "--yes"]);
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
}

/// A stale file at the temporary name install is about to use is stepped over,
/// not opened through.
///
/// Both temporaries are named from the process id, so a symlink left behind by
/// a crashed run - or one landed on by a pid that came round again - is enough
/// for `File::create` to truncate whatever it points at and for the rename that
/// follows to move the link rather than the file.
#[test]
#[cfg(unix)]
fn a_stale_temporary_name_is_stepped_over_rather_than_followed() {
    let fixture = fixture();
    fixture.seed();
    let paused = fixture.at_the_prompt(&["install"]);
    let pid = paused.pid();

    // One victim per temporary install writes, reached through the exact name
    // it builds out of its own pid.
    let victims = [
        (
            fixture.root.join("victim-binary"),
            fixture.bin_dir.join(format!(".verbatim-install-{pid}")),
        ),
        (
            fixture.root.join("victim-settings"),
            fixture
                .claude_dir
                .join(format!(".settings.json.verbatim-{pid}")),
        ),
    ];
    for (victim, stale) in &victims {
        std::fs::write(victim, b"not yours\n").unwrap();
        std::os::unix::fs::symlink(victim, stale).unwrap();
    }

    let (status, said) = paused.answer("y\n");
    assert!(status.success(), "{said}");
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
#[cfg(unix)]
fn a_symlink_at_the_backup_name_is_not_written_through() {
    let fixture = fixture();
    fixture.seed();
    let victim = fixture.root.join("not-a-backup");
    std::os::unix::fs::symlink(
        &victim,
        fixture.claude_dir.join("settings.json.verbatim-backup"),
    )
    .unwrap();

    let output = fixture.run(&["install", "--yes"]);
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

/// D-07 again, at the other end of the confirmation: a program that appears at
/// the stable path while install is waiting for an answer is refused, not
/// renamed over.
///
/// The check `run` makes is from before the question. The rename clobbers.
#[test]
fn a_binary_that_appears_during_the_confirmation_is_still_refused() {
    let fixture = fixture();
    fixture.seed();
    let paused = fixture.at_the_prompt(&["install"]);

    let foreign = b"#!/bin/sh\nexit 0\n";
    std::fs::write(fixture.stable(), foreign).unwrap();

    let (status, said) = paused.answer("y\n");
    assert!(!status.success(), "install overwrote a stranger: {said}");
    assert!(
        said.contains("is not a verbatim build"),
        "the refusal did not say what it found: {said}"
    );
    assert_eq!(std::fs::read(fixture.stable()).unwrap(), foreign);
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        SETTINGS
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        CLAUDE_JSON
    );
    assert!(fixture.backups().is_empty(), "a refusal wrote a backup");
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

    assert!(fixture.run(&["install", "--yes"]).status.success());
    let after_first = std::fs::read_to_string(fixture.settings()).unwrap();
    assert!(fixture.run(&["install", "--yes"]).status.success());

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
        vec![
            ".claude.json.verbatim-backup",
            "settings.json.verbatim-backup"
        ],
        "expected exactly one backup of each file"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_dir.join("settings.json.verbatim-backup")).unwrap(),
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
    assert!(fixture.run(&["install", "--yes"]).status.success());
    let hooks = read(&fixture.settings())["hooks"].clone();
    let raw = std::fs::read_to_string(fixture.settings()).unwrap();

    // The upgrade: different bytes at the stable path, still a verbatim build.
    let mut replacement = std::fs::read(fixture.stable()).unwrap();
    replacement.extend_from_slice(b"\0a later build\0");
    std::fs::write(fixture.stable(), &replacement).unwrap();

    assert!(fixture.run(&["install", "--yes"]).status.success());
    assert_eq!(read(&fixture.settings())["hooks"], hooks);
    assert_eq!(std::fs::read_to_string(fixture.settings()).unwrap(), raw);
}

/// INST-05: an entry that is verbatim's and the wrong shape is repaired, not
/// counted as done.
///
/// A hand edit, or an entry an older verbatim wrote with a different arg shape,
/// has the right command and the wrong `args`. Idempotence checked by presence
/// alone would call `SessionStart` handled and leave it with no working hook at
/// all - permanently, because every later install would agree.
#[test]
fn an_entry_of_ours_in_the_wrong_shape_is_repaired_rather_than_skipped() {
    let fixture = fixture();
    fixture.seed();
    // JSON needs the Windows separators doubled; nothing else about the path
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

    assert!(fixture.run(&["install", "--yes"]).status.success());

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
    assert!(fixture.run(&["install", "--yes"]).status.success());
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        after_first,
        "a second install rewrote settings.json"
    );
}

// ---------------------------------------------------------------------------
// Task 4: the MCP registration
// ---------------------------------------------------------------------------

/// AC4 for `.claude.json`: `context7` keeps its place, `verbatim` is added once
/// however many times install runs, and nothing else in the file moves.
#[test]
fn two_installs_leave_one_mcp_entry_beside_the_servers_already_there() {
    let fixture = fixture();
    fixture.seed();
    let before_keys = key_order(&fixture.claude_json());

    assert!(fixture.run(&["install", "--yes"]).status.success());
    assert!(fixture.run(&["install", "--yes"]).status.success());

    let claude = read(&fixture.claude_json());
    let servers = claude["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 2);
    assert_eq!(
        servers["verbatim"]["command"],
        serde_json::Value::from(fixture.stable().display().to_string())
    );
    assert_eq!(servers["verbatim"]["args"][0], "mcp");
    assert_eq!(servers["verbatim"]["type"], "stdio");
    assert_eq!(
        servers["context7"]["url"],
        serde_json::Value::from("https://example.invalid/mcp")
    );

    // `del(.mcpServers.verbatim)` is the file it was given.
    let mut stripped = claude.clone();
    stripped["mcpServers"]
        .as_object_mut()
        .unwrap()
        .remove("verbatim");
    let seeded: serde_json::Value = serde_json::from_str(CLAUDE_JSON).unwrap();
    assert_eq!(stripped, seeded);
    assert_eq!(key_order(&fixture.claude_json()), before_keys);

    // `mcpServers` keeps its own key order: context7 was there first.
    let raw = std::fs::read_to_string(fixture.claude_json()).unwrap();
    assert!(
        raw.find("\"context7\"").unwrap() < raw.find("\"verbatim\"").unwrap(),
        "the server object was reordered"
    );
    // The file it read had no trailing newline, and neither has the one it wrote.
    assert!(raw.ends_with('}'), "a trailing newline appeared");
}

/// A registration of verbatim's that names a binary path which has moved is
/// brought up to date.
///
/// This is the phase goal at its narrowest: "keeping itself current with no
/// user action" is exactly what fails if a registration under verbatim's own
/// key is left alone whatever it holds.
#[test]
fn a_stale_mcp_registration_is_brought_up_to_date() {
    let fixture = fixture();
    fixture.seed();
    let stale = CLAUDE_JSON.replace(
        "    \"context7\": {",
        "    \"verbatim\": {\n      \"type\": \"sse\",\n      \"command\": \
         \"/old/bin/verbatim\",\n      \"env\": {\n        \"KEEP\": \"me\"\n      }\n    },\n    \
         \"context7\": {",
    );
    std::fs::write(fixture.claude_json(), &stale).unwrap();

    assert!(fixture.run(&["install", "--yes"]).status.success());

    let servers = read(&fixture.claude_json())["mcpServers"].clone();
    let ours = &servers["verbatim"];
    assert_eq!(
        ours["command"],
        serde_json::Value::from(fixture.stable().display().to_string()),
        "the registration still names a binary that is not there"
    );
    assert_eq!(ours["type"], "stdio");
    assert_eq!(ours["args"][0], "mcp");
    assert_eq!(
        ours["env"]["KEEP"],
        serde_json::Value::from("me"),
        "a key beside ours was dropped"
    );
    // And the server that is not ours is exactly as it was.
    assert_eq!(
        servers["context7"]["url"],
        serde_json::Value::from("https://example.invalid/mcp")
    );
    assert_eq!(servers.as_object().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// Task 5: the diff, the one confirmation, and --yes
// ---------------------------------------------------------------------------

/// Nothing to read and no `--yes`: refuse, name `--yes`, and write nothing -
/// not the settings files, and not the binary either.
#[test]
fn without_an_answer_and_without_yes_install_refuses_and_writes_nothing() {
    let fixture = fixture();
    fixture.seed();

    let output = fixture.run(&["install"]);
    assert!(!output.status.success());
    let said = text(&output);
    assert!(said.contains("--yes"), "{said}");

    assert!(!fixture.stable().exists(), "a refusal placed the binary");
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        SETTINGS
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        CLAUDE_JSON
    );
    assert!(fixture.backups().is_empty());
}

/// A declined confirmation is a success that changed nothing, and it says so.
#[test]
fn a_declined_confirmation_changes_nothing_and_exits_zero() {
    let fixture = fixture();
    fixture.seed();

    let output = fixture.answer(&["install"], "n\n");
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("nothing was changed"));

    assert!(!fixture.stable().exists());
    assert_eq!(
        std::fs::read_to_string(fixture.settings()).unwrap(),
        SETTINGS
    );
    assert_eq!(
        std::fs::read_to_string(fixture.claude_json()).unwrap(),
        CLAUDE_JSON
    );
    assert!(fixture.backups().is_empty());
}

/// An accepted confirmation writes all three, and the diff it showed first
/// carried the change rather than the file.
#[test]
fn an_accepted_confirmation_writes_all_three_after_showing_the_diff() {
    let fixture = fixture();
    fixture.seed();

    let output = fixture.answer(&["install"], "y\n");
    assert!(output.status.success(), "{}", text(&output));
    let said = text(&output);
    let shown = said
        .split("apply these changes?")
        .next()
        .expect("the confirmation is asked once");
    assert_eq!(
        said.matches("apply these changes?").count(),
        1,
        "install asked more than once: {said}"
    );
    assert!(
        shown.contains("   + ") && shown.contains("PostCompact"),
        "no hook diff was shown: {shown}"
    );
    assert!(
        shown.contains("\"verbatim\": {"),
        "no mcp diff was shown: {shown}"
    );
    // A `-` line is only ever a reshaping install caused - `"SessionStart": []`
    // becoming a populated array, `"PreToolUse": []` gaining a comma. None of
    // them may carry a line the user wrote.
    assert!(
        !shown
            .lines()
            .any(|line| line.starts_with("   - ") && line.contains("terse-answers")),
        "the diff removes the user's own hook: {shown}"
    );
    // And it is the change, not the file: neither of these is anywhere near
    // an insertion point.
    for untouched in ["oauthAccount", "numStartups", "\"theme\""] {
        assert!(
            !shown.contains(untouched),
            "the diff printed {untouched}, which nothing changed: {shown}"
        );
    }

    assert!(fixture.stable().exists());
    assert_eq!(
        our_entries(&read(&fixture.settings()), &fixture.stable()).len(),
        4
    );
    assert!(read(&fixture.claude_json())["mcpServers"]["verbatim"].is_object());
}

/// INST-03: the diff is the file's own bytes against what will land on top of
/// them, so a settings file verbatim's renderer did not produce shows the whole
/// rewrite it is about to get. A diff of two re-rendered copies would show a
/// four-line insertion and then reformat the file.
#[test]
fn a_differently_formatted_file_shows_the_rewrite_it_will_get() {
    let fixture = fixture();
    fixture.seed();
    std::fs::write(fixture.settings(), four_space(SETTINGS)).unwrap();

    let output = fixture.run(&["install", "--yes"]);
    assert!(output.status.success(), "{}", text(&output));
    let said = text(&output);
    let shown = said
        .split("apply these changes?")
        .next()
        .expect("the confirmation is asked once");

    // The line as the file spells it, removed; the line as install will write
    // it, added. Neither appears in a diff taken between two rendered copies,
    // because nothing in `theme` changed - only its indentation did.
    assert!(
        shown.contains("   -     \"theme\": \"dark\""),
        "the diff hid the reformatting of a line it is about to rewrite:\n{shown}"
    );
    assert!(
        shown.contains("   +   \"theme\": \"dark\""),
        "the diff did not show the line it will write:\n{shown}"
    );
    assert!(
        shown.contains("rewrites all of it"),
        "the diff did not say why it is the whole file:\n{shown}"
    );

    // And what landed is what it showed.
    let after = std::fs::read_to_string(fixture.settings()).unwrap();
    assert!(
        after.contains("\n  \"theme\": \"dark\""),
        "the file was not rewritten the way the diff said it would be"
    );
    assert_eq!(
        our_entries(&read(&fixture.settings()), &fixture.stable()).len(),
        4
    );
    // The backup is still the bytes that were there before any of this.
    assert_eq!(
        std::fs::read_to_string(fixture.claude_dir.join("settings.json.verbatim-backup")).unwrap(),
        four_space(SETTINGS)
    );
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

/// INST-04: a low `cleanupPeriodDays` is reported and auto-compact is
/// recommended, and `--yes` changes neither.
#[test]
fn install_reports_the_two_settings_it_never_changes() {
    let fixture = fixture();
    fixture.seed();
    let before = read(&fixture.settings());

    let output = fixture.run(&["install", "--yes"]);
    assert!(output.status.success(), "{}", text(&output));
    let said = text(&output);
    assert!(said.contains("cleanupPeriodDays"), "{said}");
    assert!(said.contains("autoCompactEnabled"), "{said}");

    let after = read(&fixture.settings());
    assert_eq!(after["cleanupPeriodDays"], before["cleanupPeriodDays"]);
    assert_eq!(after["autoCompactEnabled"], before["autoCompactEnabled"]);
    assert_eq!(after["cleanupPeriodDays"], serde_json::Value::from(7));
}

/// The closing summary names where things went, and the paths it names are
/// real: the data directory is `status`'s store's parent, and each backup it
/// printed exists.
#[test]
fn the_summary_names_the_data_directory_and_the_backups_it_wrote() {
    let fixture = fixture();
    fixture.seed();

    let output = fixture.run(&["install", "--yes"]);
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
    assert_eq!(fixture.backups().len(), 2);
}

/// AC8/ING-11, the case a user actually hits: `~/.local/bin/verbatim install`.
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
            verbatim_core::testkit::fixture_path(fixture_name),
            project.join(name),
        )
        .unwrap();
    };

    transcript(
        "11111111-1111-4111-8111-111111111111.jsonl",
        "session-basic.jsonl",
    );
    assert!(fixture.run(&["install", "--yes"]).status.success());
    wait_for_sessions(&fixture, 1);

    // A second session, so the rerun has work to find and the assertion is
    // about this install rather than the first one.
    transcript(
        "22222222-2222-4222-8222-222222222222.jsonl",
        "session-continuation.jsonl",
    );

    let output = fixture
        .command_at(&fixture.stable(), &["install", "--yes"])
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
        .join(verbatim_core::store::DB_FILE_NAME);
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
