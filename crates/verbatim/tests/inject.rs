//! Context injection at the process boundary: what Claude Code actually reads.
//!
//! These are spawns rather than library calls for the same reason
//! `crates/verbatim/tests/recall_cli.rs` gives: the properties under test are
//! the stdout bytes, the exit code and how long the process took, and none of
//! the three exists inside a library call. The wire shape is D-01's - one JSON
//! object carrying `hookSpecificOutput.hookEventName` and
//! `hookSpecificOutput.additionalContext` - and a hook that gets it wrong is a
//! red banner on every session start rather than a quiet no-op, so it is
//! asserted here against the event name that fired.
//!
//! Every spawn sets `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` **and**
//! `CLAUDE_CONFIG_DIR` at temporary directories: every hook spawns a bare
//! `verbatim ingest`, which walks the *configured* roots, so a spawn that set
//! only the data directory would walk the developer's live `~/.claude` tree.
//!
//! Every store is seeded through `testkit::copy_rooted_fixture_into` under a
//! root the test owns (`FIXTURE_ROOT_TOKEN`), never through the fixtures that
//! hardcode `/data/code/verbatim`: a project key has to be something this test
//! built, or the assertion is true only on a checkout at that literal path -
//! and true for the wrong reason on a machine where it exists.
//!
//! Deliberately NOT gated on the `testkit` cargo feature, the way
//! `tests/recall_cli.rs` is: that file needs the binary built with
//! `verbatim-core`'s fault-injection points in it, and this one needs only the
//! fixture helpers, which this crate's dev-dependency on
//! `verbatim-core/testkit` already provides. A gated file runs zero tests under
//! a bare `cargo test` and still reports green.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use verbatim_core::testkit;

/// The fixture whose `cwd` names `project-alpha` beneath the test's own root.
const FIXTURE: &str = "session-recall.jsonl";

/// Every directory a spawned `verbatim` may touch, all of them temporary.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    /// The root the rooted fixtures' `cwd` values were substituted with, and so
    /// the parent of every project directory a payload can name.
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        config_dir,
        claude_dir,
        root,
    }
}

impl Bench {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
        command
            .args(args)
            .current_dir(&self.work)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// Archive one rooted fixture through the binary's own ingest path, to
    /// completion, so the hook's own spawned pass finds nothing new.
    fn ingest(&self, fixture: &str) {
        let path = testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root);
        let out = self
            .command(&["ingest", path.to_str().unwrap()])
            .output()
            .expect("spawn the ingest");
        assert!(
            out.status.success(),
            "ingest {fixture}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The project directory one rooted fixture's `cwd` names.
    fn project(&self, fixture: &str) -> PathBuf {
        testkit::fixture_project(fixture, &self.root)
    }

    /// One hook invocation: the payload on stdin, then EOF, the way Claude Code
    /// writes it (`stdin.write(payload + "\n"); stdin.end()`).
    fn hook(&self, event: &str, payload: &Value) -> Output {
        let mut child = self
            .command(&["hook", event])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the hook");
        let line = format!("{payload}\n");
        child
            .stdin
            .take()
            .expect("the hook's stdin")
            .write_all(line.as_bytes())
            .expect("write the payload");
        child.wait_with_output().expect("wait for the hook")
    }
}

/// A payload of the shape `tests/fixtures/hooks/*.json` carry, standing in the
/// named directory.
fn payload(event: &str, cwd: &std::path::Path) -> Value {
    serde_json::json!({
        "session_id": "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55",
        "transcript_path": "/home/user/.claude/projects/-p/0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55.jsonl",
        "cwd": cwd.to_str().unwrap(),
        "hook_event_name": event,
        "source": "resume",
        "prompt": "where did we settle the retry budget",
    })
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one object on stdout, and the assertion that stdout held nothing else.
fn document(output: &Output) -> Value {
    let text = stdout(output);
    let mut lines = text.lines();
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("stdout was empty; stderr: {}", stderr(output)));
    assert_eq!(
        lines.next(),
        None,
        "stdout carried more than the one object: {text:?}"
    );
    serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("stdout is not one JSON object ({e}): {text:?}"))
}

/// The injected text of a document that carries one.
fn additional_context(document: &Value) -> &str {
    let inner = document
        .get("hookSpecificOutput")
        .unwrap_or_else(|| panic!("no hookSpecificOutput in {document}"));
    inner
        .get("additionalContext")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no additionalContext in {document}"))
}

fn empty_stdout(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} exited {:?}: {}",
        output.status.code(),
        stderr(output)
    );
    assert!(
        output.stdout.is_empty(),
        "{what} wrote to stdout: {:?}",
        stdout(output)
    );
}

// ---------------------------------------------------------------------------
// The wire shape (D-01)

/// The seam, end to end: a `SessionStart` in a project the archive knows writes
/// one object, under its own event name, naming what is indexed for it.
#[test]
fn session_start_writes_one_object_naming_what_is_indexed_for_the_project() {
    let bench = bench();
    bench.ingest(FIXTURE);

    let output = bench.hook(
        "SessionStart",
        &payload("SessionStart", &bench.project(FIXTURE)),
    );
    assert!(
        output.status.success(),
        "SessionStart exited {:?}: {}",
        output.status.code(),
        stderr(&output)
    );

    let document = document(&output);
    assert_eq!(
        document
            .get("hookSpecificOutput")
            .and_then(|o| o.get("hookEventName"))
            .and_then(Value::as_str),
        Some("SessionStart"),
        "the harness throws `Hook returned incorrect event name` on a mismatch: {document}"
    );

    // The counts the store actually holds, read independently of the code that
    // rendered them.
    let conn = rusqlite::Connection::open_with_flags(
        bench.data_dir.join(verbatim_core::store::DB_FILE_NAME),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open the seeded store");
    let sessions: i64 = conn
        .query_row("SELECT count(*) FROM session_meta", [], |r| r.get(0))
        .unwrap();
    let turns: i64 = conn
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    assert!(sessions > 0 && turns > 0, "the fixture archived nothing");

    let text = additional_context(&document);
    assert!(
        text.contains(&format!("{sessions} session")) && text.contains(&format!("{turns} turn")),
        "the brief names neither count ({sessions} sessions, {turns} turns): {text:?}"
    );
    assert!(
        text.contains(bench.project(FIXTURE).to_str().unwrap()),
        "the brief does not name the project it counted: {text:?}"
    );
    // INJ-01's other half: the model has to be told the tools exist, and the
    // names have to be the ones `cmd::mcp::tools` registers.
    for tool in ["recall_search", "recall_context", "recall_get"] {
        assert!(
            text.contains(tool),
            "the brief never names {tool}: {text:?}"
        );
    }
}

/// Neither event has an `additionalContext` variant in the harness's
/// discriminated union, so an object under either name is a schema failure the
/// user sees as a red banner. They write nothing, against a store with history.
#[test]
fn session_end_and_post_compact_write_nothing_against_a_store_with_history() {
    let bench = bench();
    bench.ingest(FIXTURE);

    for event in ["SessionEnd", "PostCompact"] {
        let output = bench.hook(event, &payload(event, &bench.project(FIXTURE)));
        empty_stdout(&output, event);
    }
}

/// D-12's failure mode, made visible: a `cwd` no archived project covers is
/// `Reason::UnknownProject`, which is silence and exit 0 rather than the whole
/// archive.
#[test]
fn a_cwd_no_archived_project_covers_writes_nothing() {
    let bench = bench();
    bench.ingest(FIXTURE);

    let output = bench.hook("SessionStart", &payload("SessionStart", &bench.work));
    empty_stdout(&output, "SessionStart in an unindexed directory");
}

/// The ordinary state of a machine that has installed verbatim and not yet
/// ingested: no store to read, so nothing to say.
///
/// The `cwd` is a project the archive *would* know if anything were archived,
/// which is what makes this about the missing store rather than about scoping.
#[test]
fn a_machine_with_no_archive_writes_nothing() {
    let bench = bench();
    // The project directory without the transcript: `copy_rooted_fixture_into`
    // creates it, and nothing has been ingested.
    std::fs::create_dir_all(bench.project(FIXTURE)).unwrap();

    let output = bench.hook(
        "SessionStart",
        &payload("SessionStart", &bench.project(FIXTURE)),
    );
    empty_stdout(&output, "SessionStart with no store");
}
