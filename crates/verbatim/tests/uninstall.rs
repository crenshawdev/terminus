//! `verbatim uninstall`: what it takes back out, what it puts back, and what it
//! refuses to touch (INST-07, AC7).
//!
//! Every spawn here points `VERBATIM_BIN_DIR`, `CLAUDE_CONFIG_DIR`,
//! `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` and `HOME` at temporary
//! directories, for the reason `tests/install.rs` states: without them a test
//! would delete the `~/.local/bin/verbatim` and edit the settings files this
//! machine is actually running on.
//!
//! AC7's "restores it to its pre-install bytes" is asserted as a byte
//! comparison against the seed and never as a `jq` or `serde_json` comparison.
//! Those would pass on a file whose keys had been reordered and whose
//! indentation had changed, which is exactly the failure the whole `json_file`
//! module exists to prevent.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A `settings.json` shaped like the real one, and the same seed
/// `tests/install.rs` and `tests/doctor.rs` use: top-level keys in an order no
/// sort produces, a `hooks` object that already carries the user's own
/// `UserPromptSubmit` script, and an empty `SessionStart` array Claude Code
/// wrote itself - the one that must still be there when uninstall is done.
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

/// A `.claude.json` shaped like the real one: another server beside verbatim's
/// under `mcpServers`, and a `projects` object standing in for the 240 KB of
/// history the real file carries. No trailing newline, like the real one.
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
    std::fs::write(claude_dir.join("settings.json"), SETTINGS).unwrap();
    std::fs::write(claude_dir.join(".claude.json"), CLAUDE_JSON).unwrap();
    Fixture {
        _dir: dir,
        root,
        bin_dir,
        claude_dir,
    }
}

impl Fixture {
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

    fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("VERBATIM_BIN_DIR", &self.bin_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("VERBATIM_DATA_DIR", self.data_dir())
            .env("VERBATIM_CONFIG_DIR", self.root.join("config"))
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root);
        command
    }

    /// Run with stdin at end of file, which is what a command in a script sees.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs")
    }

    fn install(&self) -> &Self {
        let out = self.run(&["install", "--yes"]);
        assert!(out.status.success(), "install failed: {}", text(&out));
        self
    }

    /// One archived session, so the data directory is a real one rather than a
    /// path that was never created.
    fn ingest(&self) -> &Self {
        let project = self.claude_dir.join("projects").join("-data-code-x");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("44444444-4444-4444-8444-444444444444.jsonl"),
            verbatim_core::testkit::fixture_bytes("session-basic.jsonl"),
        )
        .unwrap();
        let out = self.run(&["ingest"]);
        assert!(out.status.success(), "ingest failed: {}", text(&out));
        self
    }

    fn uninstall(&self) -> Output {
        self.run(&["uninstall"])
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

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn bytes(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&bytes(path)).unwrap()
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

/// AC7's first half: a settings file untouched since install comes back byte
/// for byte, and `.claude.json` with it.
///
/// Byte for byte is the assertion because it is the only one that can fail for
/// the right reason. Both files went through verbatim's renderer on the way in,
/// so a `serde_json` comparison would pass even if the 29 keys came back in a
/// different order.
#[test]
fn install_then_uninstall_leaves_both_files_exactly_as_they_were() {
    let fixture = fixture();
    fixture.install();
    assert_ne!(
        bytes(&fixture.settings()),
        SETTINGS,
        "install wrote nothing, so this test proves nothing"
    );

    let out = fixture.uninstall();
    assert!(out.status.success(), "uninstall failed: {}", text(&out));

    assert_eq!(
        bytes(&fixture.settings()),
        SETTINGS,
        "settings.json did not come back to its pre-install bytes"
    );
    assert_eq!(
        bytes(&fixture.claude_json()),
        CLAUDE_JSON,
        ".claude.json did not come back to its pre-install bytes"
    );
    assert_eq!(
        fixture.backups(),
        Vec::<String>::new(),
        "a restored file left its backup behind"
    );
    assert!(
        !fixture.stable().exists(),
        "the copy at the stable path was left there"
    );
}

/// The `verbatim` key goes and every neighbour stays (AC7).
///
/// Named separately from the byte comparison above because it is the assertion
/// that still means something on a `.claude.json` the user has edited since:
/// the removal is keyed, not positional.
#[test]
fn only_verbatims_key_leaves_mcp_servers() {
    let fixture = fixture();
    fixture.install();
    let installed = json(&fixture.claude_json());
    assert_eq!(
        installed["mcpServers"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["context7", "verbatim"]
    );

    let out = fixture.uninstall();
    assert!(out.status.success(), "uninstall failed: {}", text(&out));

    let after = json(&fixture.claude_json());
    assert_eq!(
        after["mcpServers"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["context7"]
    );
    assert_eq!(after["projects"], installed["projects"]);
    assert_eq!(after["oauthAccount"], installed["oauthAccount"]);
}

/// D-13's whole point: a `/config` change made after install survives
/// uninstall, and the entries still go.
///
/// `theme` is the realistic one - `settings.json` carries 29 keys and Claude
/// Code's own UI writes most of them between an install and an uninstall - and
/// a restore that reverted it would be uninstall undoing a change it was never
/// asked about.
#[test]
fn an_edit_made_after_install_survives_and_the_entries_still_go() {
    let fixture = fixture();
    fixture.install();
    let edited = bytes(&fixture.settings()).replace("\"theme\": \"dark\"", "\"theme\": \"light\"");
    std::fs::write(fixture.settings(), &edited).unwrap();

    let output = fixture.uninstall();
    assert!(
        output.status.success(),
        "uninstall failed: {}",
        text(&output)
    );

    let after = json(&fixture.settings());
    assert_eq!(after["theme"], "light", "the user's own edit was reverted");
    assert!(
        our_entries(&after, &fixture.stable()).is_empty(),
        "a hook entry of verbatim's survived: {after:#}"
    );
    assert_eq!(
        after["hooks"]["UserPromptSubmit"].as_array().unwrap().len(),
        1,
        "the user's own UserPromptSubmit script was removed with ours"
    );
    assert_eq!(
        after["hooks"]["SessionStart"],
        serde_json::json!([]),
        "an empty array the user's own file already carried was dropped"
    );
    assert!(
        after["hooks"].get("PostCompact").is_none(),
        "an event key install created was left behind empty"
    );
    assert_eq!(
        fixture.backups(),
        ["settings.json.verbatim-backup"],
        "the backup for a file that changed since install must stay"
    );
    assert!(
        out(&output).contains("has changed since install"),
        "uninstall did not say which of the two happened:\n{}",
        text(&output)
    );
}

/// INST-07: the archive is not collateral, and its path is on the screen.
#[test]
fn the_data_directory_is_left_alone_and_its_path_printed() {
    let fixture = fixture();
    fixture.install().ingest();
    assert!(fixture.data_dir().is_dir());

    let output = fixture.uninstall();
    assert!(
        output.status.success(),
        "uninstall failed: {}",
        text(&output)
    );
    assert!(
        fixture.data_dir().is_dir(),
        "uninstall deleted the data directory"
    );
    assert!(
        out(&output).contains(&fixture.data_dir().display().to_string()),
        "uninstall did not print the data directory's path:\n{}",
        text(&output)
    );
}

/// A machine that is half wired is exactly the one uninstall is reached for.
///
/// One reported line per thing that is not there, exit 0, and the rest of the
/// work still done: the `.claude.json` half is cleaned up and the binary is
/// removed even though `settings.json` went missing.
#[test]
fn a_settings_file_deleted_since_install_is_one_reported_line() {
    let fixture = fixture();
    fixture.install();
    std::fs::remove_file(fixture.settings()).unwrap();

    let output = fixture.uninstall();
    assert_eq!(
        output.status.code(),
        Some(0),
        "a missing settings file is a state, not a failure:\n{}",
        text(&output)
    );
    assert!(
        out(&output).contains("not there"),
        "uninstall did not report the missing file:\n{}",
        text(&output)
    );
    assert!(
        !fixture.settings().exists(),
        "uninstall recreated a settings file the user had deleted"
    );
    assert_eq!(
        json(&fixture.claude_json())["mcpServers"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["context7"],
        "uninstall stopped at the missing file instead of carrying on"
    );
    assert!(!fixture.stable().exists(), "the binary was left behind");
}

/// Uninstalling twice is uninstalling once, and uninstalling a machine that
/// was never installed reports rather than fails.
#[test]
fn uninstall_on_a_machine_that_was_never_installed_changes_nothing() {
    let fixture = fixture();

    let output = fixture.uninstall();
    assert_eq!(
        output.status.code(),
        Some(0),
        "nothing to remove is not a failure:\n{}",
        text(&output)
    );
    assert_eq!(bytes(&fixture.settings()), SETTINGS);
    assert_eq!(bytes(&fixture.claude_json()), CLAUDE_JSON);

    fixture.install();
    assert!(fixture.uninstall().status.success());
    let second = fixture.uninstall();
    assert!(second.status.success(), "the second uninstall failed");
    assert_eq!(bytes(&fixture.settings()), SETTINGS);
    assert_eq!(bytes(&fixture.claude_json()), CLAUDE_JSON);
}

/// D-07 read backwards: a file at the stable path that is not a verbatim build
/// is somebody else's, and uninstall reports it rather than deleting it.
///
/// Not hypothetical - as of 2026-08-13 `~/.local/bin/verbatim` on this machine
/// is a different 12 MB program whose `--version` prints exactly what this
/// build's does.
#[test]
fn an_unmarked_file_at_the_stable_path_is_reported_and_kept() {
    let fixture = fixture();
    fixture.install();
    std::fs::write(fixture.stable(), b"#!/bin/sh\necho not verbatim\n").unwrap();

    let output = fixture.uninstall();
    assert!(
        output.status.success(),
        "uninstall failed: {}",
        text(&output)
    );
    assert!(
        fixture.stable().exists(),
        "uninstall deleted a program it did not put there"
    );
    assert_eq!(
        bytes(&fixture.stable()),
        "#!/bin/sh\necho not verbatim\n",
        "the file at the stable path was changed"
    );
    assert!(
        out(&output).contains("not a verbatim build"),
        "uninstall did not say why it kept the file:\n{}",
        text(&output)
    );
}

/// Uninstall is human-only: `--json` is an argument it does not have (D-24).
#[test]
fn uninstall_rejects_json() {
    let fixture = fixture();
    let output = fixture.run(&["uninstall", "--json"]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));
}
