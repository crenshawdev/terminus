//! `terminus data move <path>` at the process boundary (STOR-07, AC5).
//!
//! **No spawn here sets `TERMINUS_DATA_DIR`.** The environment override outranks
//! the location pointer by design (D-06), so a bench that set it would pass
//! whether the pointer worked or not. Every spawn points the PLATFORM data-dir
//! variable at a temporary directory instead - `XDG_DATA_HOME`, `HOME` or
//! `LOCALAPPDATA` depending on the target - which is the same protection by the
//! only route that leaves the pointer load-bearing.
//!
//! The claims are about a process and cannot be made from inside a library
//! call: that a hook resolves the moved store, that a separately spawned MCP
//! server resolves it too, and that `~/.claude/settings.json` is byte-identical
//! across all of it.

#![cfg(feature = "testkit")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{json, Value};
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

/// A settings file with nothing of terminus's in it.
///
/// AC5's byte-identical claim is about the file the user has, and the move must
/// not touch it whether install ever ran or not: the hook entries are written
/// as `args: ["hook", <event>]` and the MCP entry as `args: ["mcp"]`, so no
/// store path is spelled anywhere in either (D-14) and there is nothing here a
/// relocation could invalidate.
const SETTINGS: &str = r#"{
  "cleanupPeriodDays": 7,
  "theme": "dark"
}
"#;

struct Bench {
    _dir: tempfile::TempDir,
    root: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    work: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let config_dir = root.join("config");
    let claude_dir = root.join("claude");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::write(claude_dir.join("settings.json"), SETTINGS).unwrap();
    Bench {
        _dir: dir,
        root,
        config_dir,
        claude_dir,
        work,
    }
}

impl Bench {
    /// Where the store lands before anything moves it: the platform location,
    /// resolved by the same rule `store::open::platform_data_dir` uses on this
    /// target.
    fn data_dir(&self) -> PathBuf {
        if cfg!(target_os = "windows") {
            self.root.join("local").join("terminus")
        } else if cfg!(target_os = "macos") {
            self.root
                .join("Library")
                .join("Application Support")
                .join("terminus")
        } else {
            self.root.join("share").join("terminus")
        }
    }

    fn pointer(&self) -> PathBuf {
        self.config_dir
            .join(terminus_core::store::LOCATION_FILE_NAME)
    }

    fn settings(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    /// Every spawn's environment, with `TERMINUS_DATA_DIR` explicitly REMOVED
    /// rather than merely unset by the bench: the developer running the suite
    /// may have it exported, and inheriting it would make the pointer moot.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_terminus"));
        command
            .args(args)
            .current_dir(&self.work)
            .env_remove("TERMINUS_DATA_DIR")
            .env("XDG_DATA_HOME", self.root.join("share"))
            .env("LOCALAPPDATA", self.root.join("local"))
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// Run with stdin at end of file, which is what a command in a script sees.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs")
    }

    /// One archived session, through the binary's own ingest, so the directory
    /// that moves is one a real pass built - `snapshots` included, since
    /// snapshots run by default (STOR-06).
    fn ingest(&self) -> &Self {
        let project = self.claude_dir.join("projects").join("-data-code-x");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("44444444-4444-4444-8444-444444444444.jsonl"),
            testkit::fixture_bytes("session-basic.jsonl"),
        )
        .unwrap();
        let out = self.run(&["ingest"]);
        assert!(out.status.success(), "ingest failed: {}", text(&out));
        self
    }

    /// An undrained injection decision, which is the file a move that carried
    /// only `terminus.db` would strand (D-06). Its contents do not matter to the
    /// move; its presence in the directory does.
    fn decision(&self) -> PathBuf {
        let dir = self.data_dir().join("decisions");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session-under-test.json");
        std::fs::write(&path, "{\"fired\":false}\n").unwrap();
        path
    }

    fn status(&self) -> Value {
        let out = self.run(&["status", "--json"]);
        assert!(out.status.success(), "status failed: {}", text(&out));
        serde_json::from_slice(&out.stdout).expect("status emits one JSON document")
    }

    /// One `tools/call` against a freshly spawned MCP server, which resolves the
    /// store for itself - it is a separate process and inherits nothing but the
    /// environment above.
    fn call(&self, tool: &str, arguments: Value) -> Value {
        let mut child = self
            .command(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn terminus mcp");
        {
            let stdin = child.stdin.as_mut().expect("stdin is piped");
            for message in [
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {"name": "test", "version": "0"},
                    },
                }),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": tool, "arguments": arguments},
                }),
            ] {
                writeln!(stdin, "{message}").expect("write a request");
            }
        }
        drop(child.stdin.take());

        let out = child.wait_with_output().expect("the server exits");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let responses: Vec<Value> = stdout
            .lines()
            .map(|line| serde_json::from_str(line).expect("stdout is JSON-RPC"))
            .collect();
        assert_eq!(
            out.status.code(),
            Some(0),
            "the server did not exit 0: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(responses.len(), 2, "{responses:?}");
        let blocks = responses[1]["result"]["content"]
            .as_array()
            .unwrap_or_else(|| panic!("a tool result carries content: {}", responses[1]))
            .clone();
        serde_json::from_str(blocks[0]["text"].as_str().expect("a text block"))
            .expect("the tool result is one JSON document")
    }
}

fn text(out: &Output) -> String {
    format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn count(data_dir: &Path, table: &str) -> i64 {
    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn listed(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// AC5's first half: everything moves, the old directory is gone, the pointer
/// names the new one, and the settings file was never opened.
#[test]
fn the_whole_directory_moves_and_the_settings_file_does_not_change() {
    let bench = bench();
    bench.ingest();
    let decision = bench.decision();
    let source = bench.data_dir();
    let destination = bench.root.join("elsewhere").join("terminus");

    let before = bench.status();
    let settings_before = std::fs::read(bench.settings()).unwrap();
    let snapshots_before = listed(&source.join("snapshots"));
    assert_eq!(snapshots_before.len(), 1, "the pass took no snapshot");

    let out = bench.run(&["data", "move", destination.to_str().unwrap(), "--yes"]);
    assert!(out.status.success(), "the move failed: {}", text(&out));

    // (a) the database, its sidecars, the lock, the injection state and the
    // snapshots are all at the new path, and the old directory is gone.
    assert!(
        destination.join(DB_FILE_NAME).is_file(),
        "{:?}",
        listed(&destination)
    );
    assert!(
        destination.join("LOCK").is_file(),
        "{:?}",
        listed(&destination)
    );
    assert!(destination
        .join("decisions")
        .join(decision.file_name().unwrap())
        .is_file());
    assert_eq!(listed(&destination.join("snapshots")), snapshots_before);
    assert!(!source.exists(), "the old data directory is still there");

    // The pointer holds one absolute path and a trailing newline, which is what
    // `store::open::data_dir` trims.
    let pointer = std::fs::read_to_string(bench.pointer()).unwrap();
    assert_eq!(pointer.trim(), destination.to_string_lossy());

    // (b) the next command resolves the new location on its own, and finds the
    // same archive there.
    let after = bench.status();
    assert_eq!(
        after["data"]["store"].as_str().unwrap(),
        destination.join(DB_FILE_NAME).to_string_lossy()
    );
    assert_eq!(after["data"]["sessions"], before["data"]["sessions"]);
    assert_eq!(after["data"]["turns"], before["data"]["turns"]);

    // (d) settings.json is byte-identical: not re-serialized, not reordered,
    // not touched at all.
    assert_eq!(
        std::fs::read(bench.settings()).unwrap(),
        settings_before,
        "the move rewrote settings.json"
    );
}

/// AC5's second half: the two components that resolve the store for themselves
/// - a hook spawn and an MCP server - both land on the moved one.
#[test]
fn a_hook_and_an_mcp_server_both_answer_off_the_new_location() {
    let bench = bench();
    bench.ingest();
    let source = bench.data_dir();
    let destination = bench.root.join("elsewhere").join("terminus");
    let out = bench.run(&["data", "move", destination.to_str().unwrap(), "--yes"]);
    assert!(out.status.success(), "the move failed: {}", text(&out));

    let runs_before = count(&destination, "runs");

    // The hook resolves the data directory on every event and then spawns a
    // detached pass, so what proves where it looked is a `runs` row appearing in
    // the moved store - and no store reappearing at the platform location.
    let mut child = bench
        .command(&["hook", "SessionStart"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&testkit::fixture_bytes("hooks/session-start.json"))
        .unwrap();
    let hooked = child.wait_with_output().unwrap();
    assert!(
        hooked.status.success(),
        "the hook failed: {}",
        text(&hooked)
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while count(&destination, "runs") == runs_before {
        assert!(
            Instant::now() < deadline,
            "the hook's ingest never wrote a runs row into the moved store"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !source.exists(),
        "the hook created a second store at the platform location"
    );

    // The MCP server is a separate process that resolves the store itself.
    // `project: "*"` opts out of the working-directory scoping, which this
    // bench's temporary work directory would otherwise narrow to nothing.
    let document = bench.call("recall_search", json!({"query": "brillig", "project": "*"}));
    let hits = document["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("a search result carries hits: {document}"));
    assert!(
        !hits.is_empty(),
        "the mcp server found nothing at the moved store: {document}"
    );
}

/// (e) With no answer to read, the warning has still been printed and nothing
/// has moved.
///
/// The refusal is [`install::confirm`]'s, unchanged: a move that assumed yes
/// when stdin was a closed pipe would relocate an archive nobody asked to
/// relocate.
#[test]
fn without_an_answer_the_warning_is_printed_and_nothing_moves() {
    let bench = bench();
    bench.ingest();
    let source = bench.data_dir();
    let destination = bench.root.join("elsewhere");

    let out = bench.run(&["data", "move", destination.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("network filesystem"),
        "the network-filesystem warning was not printed: {stdout}"
    );

    assert!(
        source.join(DB_FILE_NAME).is_file(),
        "the store moved anyway"
    );
    assert!(!destination.exists(), "the destination was written to");
    assert!(!bench.pointer().exists(), "the pointer was written anyway");
}

/// (f) A destination that already holds something is refused before anything is
/// copied, and before the question is asked.
#[test]
fn a_destination_that_already_holds_files_is_refused() {
    let bench = bench();
    bench.ingest();
    let source = bench.data_dir();
    let destination = bench.root.join("occupied");
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("someone-elses.txt"), "hands off\n").unwrap();

    let out = bench.run(&["data", "move", destination.to_str().unwrap(), "--yes"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));

    assert_eq!(listed(&destination), vec!["someone-elses.txt".to_owned()]);
    assert!(
        source.join(DB_FILE_NAME).is_file(),
        "the store moved anyway"
    );
    assert!(!bench.pointer().exists(), "the pointer was written anyway");
}

/// The two-word form, held to the `observations regenerate` rule: the verb comes
/// first or not at all, and a second word that is not the verb is misuse (exit
/// 2) rather than a move of something the user did not name.
#[test]
fn the_verb_is_required_and_nothing_else_is_accepted() {
    let bench = bench();
    let destination = bench.root.join("elsewhere");

    for args in [
        vec!["data"],
        vec!["data", "shove", destination.to_str().unwrap()],
        vec!["data", "move"],
        vec!["data", "move", destination.to_str().unwrap(), "--json"],
    ] {
        let out = bench.run(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", text(&out));
    }
    assert!(!destination.exists(), "a misuse created the destination");
}
