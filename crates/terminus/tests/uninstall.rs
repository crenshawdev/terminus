//! `terminus uninstall`: what it takes back out, what it puts back, and what it
//! refuses to touch (INST-07, AC7).
//!
//! Every spawn here points `TERMINUS_BIN_DIR`, `CLAUDE_CONFIG_DIR`,
//! `TERMINUS_DATA_DIR`, `TERMINUS_CONFIG_DIR` and `HOME` at temporary
//! directories, for the reason `tests/install.rs` states: without them a test
//! would delete the `~/.local/bin/terminus` and edit the settings files this
//! machine is actually running on. `PATH` is a fixture directory holding a
//! fake `claude` and nothing else, so no test can start the real one.
//!
//! AC7's "restores it to its pre-install bytes" is asserted as a byte
//! comparison against the seed and never as a `jq` or `serde_json` comparison.
//! Those would pass on a file whose keys had been reordered and whose
//! indentation had changed, which is exactly the failure the whole `json_file`
//! module exists to prevent.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use terminus_core::testkit::{self, FakeCall, NONESSENTIAL_TRAFFIC};

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

/// A `.claude.json` shaped like the real one: another server beside terminus's
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
    /// The whole of every spawned terminus's `PATH`.
    path_dir: PathBuf,
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
    let path_dir = root.join("path");
    std::fs::create_dir_all(&path_dir).unwrap();
    testkit::fake_claude(&path_dir, &root.join("claude.log"), "", 0);
    Fixture {
        _dir: dir,
        root,
        bin_dir,
        claude_dir,
        path_dir,
    }
}

impl Fixture {
    fn stable(&self) -> PathBuf {
        self.bin_dir.join("terminus")
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

    /// `PATH` holds the fake `claude` and nothing else, and the traffic
    /// variable is taken out so a logged value can only have come from
    /// terminus.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_terminus"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("PATH", &self.path_dir)
            .env_remove(NONESSENTIAL_TRAFFIC)
            .env("TERMINUS_BIN_DIR", &self.bin_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("TERMINUS_DATA_DIR", self.data_dir())
            .env("TERMINUS_CONFIG_DIR", self.root.join("config"))
            .env("HOME", &self.root);
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
        let out = self.run(&["install"]);
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
            terminus_core::testkit::fixture_bytes("session-basic.jsonl"),
        )
        .unwrap();
        let out = self.run(&["ingest"]);
        assert!(out.status.success(), "ingest failed: {}", text(&out));
        self
    }

    fn uninstall(&self) -> Output {
        self.run(&["uninstall"])
    }

    /// Run and type `answer` at the confirmation, the way a user at a terminal
    /// would.
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

    /// `terminus status`'s `size_bytes` for this fixture's store.
    fn status_size(&self) -> u64 {
        let output = self.run(&["status", "--json"]);
        assert!(output.status.success(), "status failed: {}", text(&output));
        let document: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("status emits one JSON document");
        document["data"]["size_bytes"].as_u64().unwrap()
    }

    /// The three files `--purge` counts, measured with no connection open.
    fn footprint(&self) -> u64 {
        ["terminus.db", "terminus.db-wal", "terminus.db-shm"]
            .iter()
            .filter_map(|name| std::fs::metadata(self.data_dir().join(name)).ok())
            .map(|meta| meta.len())
            .sum()
    }

    /// Every `claude` call terminus made, oldest first.
    fn calls(&self) -> Vec<FakeCall> {
        testkit::fake_claude_calls(&self.root.join("claude.log"))
    }

    /// Put a fake `claude` on `PATH` that exits with `status`.
    fn fake_claude(&self, status: i32) {
        testkit::fake_claude(&self.path_dir, &self.root.join("claude.log"), "", status);
    }

    /// A user-level `terminus` entry in its current shape, as `claude mcp
    /// add-json` would leave it. Install then finds it current and runs nothing.
    fn seed_entry(&self) -> &Self {
        let mut seeded = json(&self.claude_json());
        seeded["mcpServers"]["terminus"] = serde_json::json!({
            "type": "stdio",
            "command": self.stable().display().to_string(),
            "args": ["mcp"],
        });
        std::fs::write(
            self.claude_json(),
            serde_json::to_string_pretty(&seeded).unwrap(),
        )
        .unwrap();
        self
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

/// The arguments of `claude mcp remove --scope user terminus`.
const REMOVE: [&str; 5] = ["mcp", "remove", "--scope", "user", "terminus"];

/// The remove as uninstall prints it for the user to run.
const REMOVE_COMMAND: &str = "claude mcp remove --scope user terminus";

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
/// for byte. `.claude.json` is never written by terminus, so with a `claude`
/// that writes nothing it is byte-identical after install, a rerun and
/// uninstall (AC2).
///
/// Byte for byte is the assertion because it is the only one that can fail for
/// the right reason. The settings file went through terminus's renderer on the
/// way in, so a `serde_json` comparison would pass even if the 29 keys came
/// back in a different order.
#[test]
fn install_then_uninstall_leaves_both_files_exactly_as_they_were() {
    let fixture = fixture();
    fixture.install();
    assert_ne!(
        bytes(&fixture.settings()),
        SETTINGS,
        "install wrote nothing, so this test proves nothing"
    );
    assert_eq!(bytes(&fixture.claude_json()), CLAUDE_JSON);
    fixture.install();
    assert_eq!(bytes(&fixture.claude_json()), CLAUDE_JSON);

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
        ".claude.json changed, and terminus never writes it"
    );
    assert_eq!(
        fixture.backups(),
        Vec::<String>::new(),
        "a restored file left its backup behind"
    );
    // Not `exists()`: it follows the link and reports false for a dangling one.
    assert_eq!(
        std::fs::symlink_metadata(fixture.stable())
            .map(|_| ())
            .map_err(|e| e.kind()),
        Err(std::io::ErrorKind::NotFound),
        "the link at the stable path was left there"
    );
    let versions = fixture
        .root
        .join(".local/lib/crenshawdev/terminus/versions");
    assert_eq!(
        std::fs::read(versions.join(env!("CARGO_PKG_VERSION")).join("terminus")).unwrap(),
        std::fs::read(env!("CARGO_BIN_EXE_terminus")).unwrap(),
        "uninstall took the version file"
    );
    let said = text(&out);
    assert!(
        said.contains(&versions.display().to_string()),
        "uninstall did not print the versions folder: {said}"
    );
}

/// D-13: with a user-level entry there, uninstall asks `claude` to remove it,
/// with `--scope user` and the traffic variable, and writes nothing itself.
///
/// The fake writes nothing, so `.claude.json` keeps the entry. That is the
/// proof terminus did not edit the file on the way out.
#[test]
fn only_the_terminus_key_leaves_mcp_servers() {
    let fixture = fixture();
    fixture.seed_entry();
    let seeded = bytes(&fixture.claude_json());

    fixture.install();
    assert!(
        fixture.calls().is_empty(),
        "install ran claude for an entry that was already current: {:?}",
        fixture.calls()
    );

    let output = fixture.uninstall();
    assert!(
        output.status.success(),
        "uninstall failed: {}",
        text(&output)
    );
    let calls = fixture.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].args, REMOVE, "{calls:?}");
    assert_eq!(calls[0].traffic.as_deref(), Some("1"), "{calls:?}");
    assert!(
        out(&output).contains("removed the mcp registration with claude mcp remove"),
        "{}",
        text(&output)
    );
    assert_eq!(bytes(&fixture.claude_json()), seeded);
}

/// D-13: when `claude` is not on `PATH`, or fails, the registration stays and
/// uninstall prints the command that removes it. The hook entries and the
/// binary still go, and the exit status says it is not done.
#[test]
fn a_registration_claude_cannot_remove_prints_the_command_and_the_rest_still_goes() {
    for missing in [true, false] {
        let fixture = fixture();
        fixture.seed_entry().install();
        if missing {
            std::fs::remove_file(fixture.path_dir.join("claude")).unwrap();
        } else {
            fixture.fake_claude(1);
        }

        let output = fixture.uninstall();
        let said = text(&output);
        assert_eq!(output.status.code(), Some(1), "{said}");
        assert!(out(&output).contains(REMOVE_COMMAND), "{said}");
        if missing {
            assert!(said.contains("claude is not on PATH"), "{said}");
        } else {
            assert_eq!(fixture.calls().last().unwrap().args, REMOVE);
        }
        assert!(
            our_entries(&json(&fixture.settings()), &fixture.stable()).is_empty(),
            "a hook entry of terminus's survived: {said}"
        );
        assert!(!fixture.stable().exists(), "the binary was left behind");
    }
}

/// D-15: a `.claude.json.terminus-backup` an older install left is reported
/// and left alone. Putting it back would roll back Claude Code's own state.
#[test]
fn an_old_claude_json_backup_is_reported_and_left_alone() {
    let fixture = fixture();
    let backup = fixture.claude_dir.join(".claude.json.terminus-backup");
    let old = "{\n  \"numStartups\": 3\n}\n";
    std::fs::write(&backup, old).unwrap();
    fixture.install();

    let output = fixture.uninstall();
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        out(&output).contains(&backup.display().to_string()),
        "uninstall did not print the old backup's path:\n{}",
        text(&output)
    );
    assert_eq!(bytes(&backup), old);
    assert_eq!(bytes(&fixture.claude_json()), CLAUDE_JSON);
}

/// D-15 when `~/.claude.json` does not parse: the old backup is still
/// reported, since a broken live file is when the user most needs to know the
/// old copy is there, and neither file is touched.
#[test]
fn an_old_claude_json_backup_is_reported_when_claude_json_does_not_parse() {
    let fixture = fixture();
    let backup = fixture.claude_dir.join(".claude.json.terminus-backup");
    let old = "{\n  \"numStartups\": 3\n}\n";
    std::fs::write(&backup, old).unwrap();
    fixture.install();
    std::fs::write(fixture.claude_json(), "{").unwrap();

    let output = fixture.uninstall();
    assert!(
        out(&output).contains(&backup.display().to_string()),
        "uninstall did not print the old backup's path:\n{}",
        text(&output)
    );
    assert_eq!(bytes(&backup), old);
    assert_eq!(bytes(&fixture.claude_json()), "{");
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
        "a hook entry of terminus's survived: {after:#}"
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
        ["settings.json.terminus-backup"],
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
/// work still done: the registration is removed through `claude` and the
/// binary is removed even though `settings.json` went missing.
#[test]
fn a_settings_file_deleted_since_install_is_one_reported_line() {
    let fixture = fixture();
    fixture.seed_entry().install();
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
        fixture.calls().last().map(|call| call.args.clone()),
        Some(REMOVE.map(String::from).to_vec()),
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
    assert!(
        fixture.calls().is_empty(),
        "uninstall ran claude with no registration there: {:?}",
        fixture.calls()
    );

    fixture.install();
    assert!(fixture.uninstall().status.success());
    let second = fixture.uninstall();
    assert!(second.status.success(), "the second uninstall failed");
    assert_eq!(bytes(&fixture.settings()), SETTINGS);
    assert_eq!(bytes(&fixture.claude_json()), CLAUDE_JSON);
}

/// D-07 read backwards: a file at the stable path that is not a terminus build
/// is somebody else's, and uninstall reports it rather than deleting it.
///
/// Not hypothetical - as of 2026-08-13 `~/.local/bin/verbatim`, the stable path
/// before the rename, was a different 12 MB program on this machine whose
/// `--version` printed exactly what this build's did then.
#[test]
fn an_unmarked_file_at_the_stable_path_is_reported_and_kept() {
    let fixture = fixture();
    fixture.install();
    std::fs::write(fixture.stable(), b"#!/bin/sh\necho not terminus\n").unwrap();

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
        "#!/bin/sh\necho not terminus\n",
        "the file at the stable path was changed"
    );
    assert!(
        out(&output).contains("not a terminus build"),
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

/// The number `--purge` printed, out of its own last line.
fn deleted_bytes(output: &Output) -> u64 {
    let line = out(output)
        .lines()
        .find(|line| line.trim_start().starts_with("deleted "))
        .unwrap_or_else(|| panic!("--purge printed no deletion line:\n{}", text(output)))
        .to_owned();
    line.split_whitespace()
        .nth(1)
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("no byte count in {line:?}"))
}

/// The archive is the product, so the question is asked and an unanswerable
/// prompt is a refusal rather than an assumed yes (INST-08, D-20).
///
/// This is the shape a script gets: `--purge` with stdin closed and no `--yes`.
/// Exit non-zero, and every byte still there.
#[test]
fn purge_with_nothing_to_read_refuses_and_keeps_the_archive() {
    let fixture = fixture();
    fixture.install().ingest();
    let before = fixture.footprint();
    assert!(before > 0, "the fixture has no store to purge");

    let output = fixture.run(&["uninstall", "--purge"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unanswerable purge must not exit 0:\n{}",
        text(&output)
    );
    assert!(
        fixture.data_dir().join("terminus.db").is_file(),
        "the archive was deleted without an answer"
    );
    assert_eq!(fixture.footprint(), before, "the store was written to");
    // Task 5's work still ran: the purge refusal is about the archive alone.
    assert!(!fixture.stable().exists());
}

/// Answering the question with anything but yes keeps the archive and exits 0:
/// a declined delete is a decision, not a failure.
#[test]
fn answering_no_keeps_the_archive_and_exits_zero() {
    let fixture = fixture();
    fixture.install().ingest();

    let output = fixture.answer(&["uninstall", "--purge"], "n\n");
    assert_eq!(
        output.status.code(),
        Some(0),
        "declining is not a failure:\n{}",
        text(&output)
    );
    assert!(
        fixture.data_dir().join("terminus.db").is_file(),
        "the archive went anyway"
    );
    assert!(
        out(&output).contains("the archive was kept"),
        "uninstall did not say the archive was kept:\n{}",
        text(&output)
    );
}

/// The size and the path are shown before the question, and `--yes` deletes.
///
/// The byte count is the footprint on disk - the same three files
/// `terminus status` reports and the same rule - and it is asserted against a
/// measurement this test takes for itself. `status`'s own number is larger and
/// checked as an upper bound instead: it measures while its connection is open,
/// and the `-shm` it counts exists only for the life of that connection.
#[test]
fn purge_with_yes_prints_the_size_and_the_path_then_deletes() {
    let fixture = fixture();
    fixture.install().ingest();
    let footprint = fixture.footprint();
    let reported = fixture.status_size();
    assert!(footprint > 0);
    assert!(
        footprint <= reported,
        "status counts the same files plus the sidecars its own connection makes: \
         {footprint} > {reported}"
    );

    let output = fixture.run(&["uninstall", "--purge", "--yes"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "purge failed:\n{}",
        text(&output)
    );
    let shown = out(&output);
    assert!(
        shown.contains(&fixture.data_dir().display().to_string()),
        "the path was not shown:\n{shown}"
    );
    assert!(
        shown.contains(&footprint.to_string()),
        "the size was not shown before the question:\n{shown}"
    );
    assert_eq!(deleted_bytes(&output), footprint);
    assert!(
        !fixture.data_dir().exists(),
        "the data directory is still there"
    );
}

/// The size is on the screen before the question, never after it.
///
/// Ordering is the property: a user is asked to approve the deletion of a
/// number they have already seen.
#[test]
fn the_size_is_shown_before_the_question() {
    let fixture = fixture();
    fixture.install().ingest();
    let footprint = fixture.footprint();

    let output = fixture.answer(&["uninstall", "--purge"], "n\n");
    let shown = out(&output);
    let size_at = shown
        .find(&footprint.to_string())
        .unwrap_or_else(|| panic!("the size was never shown:\n{shown}"));
    let question_at = shown
        .find("delete it?")
        .unwrap_or_else(|| panic!("nothing was asked:\n{shown}"));
    assert!(
        size_at < question_at,
        "the size was shown after the question:\n{shown}"
    );
}

/// A pass that is mid-transaction never has the store deleted out from under
/// it: `--purge` takes the ingest lock first.
///
/// The lock is held by this test process, which is what a detached ingest
/// spawned by a hook looks like from outside.
#[test]
fn purge_refuses_while_an_ingest_holds_the_lock() {
    let fixture = fixture();
    fixture.install().ingest();

    let held = match terminus_core::ingest::lock::try_acquire(&fixture.data_dir()).unwrap() {
        terminus_core::ingest::lock::Attempt::Acquired(held) => held,
        terminus_core::ingest::lock::Attempt::Held => panic!("nothing else should hold this lock"),
    };

    let output = fixture.run(&["uninstall", "--purge", "--yes"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "purge deleted a store another process was writing:\n{}",
        text(&output)
    );
    assert!(
        fixture.data_dir().join("terminus.db").is_file(),
        "the archive went while the lock was held"
    );
    drop(held);

    // And it goes once the pass is done.
    let output = fixture.run(&["uninstall", "--purge", "--yes"]);
    assert!(output.status.success(), "purge failed:\n{}", text(&output));
    assert!(!fixture.data_dir().exists());
}

/// `--purge` with no store is a state, not a failure, and deletes nothing.
#[test]
fn purge_before_the_first_ingest_deletes_nothing() {
    let fixture = fixture();
    fixture.install();

    let output = fixture.run(&["uninstall", "--purge", "--yes"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a machine before its first ingest is not a failure:\n{}",
        text(&output)
    );
    assert!(
        out(&output).contains("nothing to purge"),
        "uninstall did not say why it deleted nothing:\n{}",
        text(&output)
    );
}
