//! OBS-01: the mechanical facts, computed off the parser and off nothing else.
//!
//! Every test here runs against a real store with real fixtures ingested into
//! it. Nothing is mocked and nothing is stubbed, because the claim under test
//! is precisely that these facts come out of the tables and the blob rather
//! than out of a model.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use terminus_core::observe::{self, Mechanical};
use terminus_core::store::DB_FILE_NAME;
use terminus_core::{ingest, testkit};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    /// Where the rooted fixtures' `cwd` values point: somewhere this test owns.
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        root,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// Archive one rooted fixture and answer with its session key.
    fn ingest(&self, fixture: &str) -> String {
        let path = testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root);
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{fixture}: {other:?}"),
        }
        key_of(&path)
    }

    /// Archive one transcript this test wrote itself.
    fn ingest_written(&self, name: &str, body: &str) -> String {
        let path = self.work.join(name);
        std::fs::write(&path, body).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{name}: {other:?}"),
        }
        key_of(&path)
    }

    fn observe(&self, session_key: &str) -> Mechanical {
        observe::observe(&self.conn(), session_key).expect("the facts must compute")
    }
}

fn key_of(path: &Path) -> String {
    path.canonicalize().unwrap().to_string_lossy().into_owned()
}

/// AC1's fact set, over the two fixtures that carry an edit and a failure.
#[test]
fn one_session_reports_its_files_tools_commands_errors_branch_and_duration() {
    let bench = bench();

    let edits = bench.observe(&bench.ingest("session-edits.jsonl"));
    let lantern = bench
        .root
        .join("project-alpha/crates/gizmo/lantern.rs")
        .to_string_lossy()
        .into_owned();
    assert!(
        edits.files_modified.contains(&lantern),
        "the file the Edit touched is not on the modified list: {:?}",
        edits.files_modified
    );
    assert!(
        edits.tools.contains(&"Edit".to_owned()),
        "{:?}",
        edits.tools
    );
    assert_eq!(edits.branch.as_deref(), Some("phase-5"));
    assert_eq!(edits.turns, 3, "three records, three turns");
    assert_eq!(
        edits.first_turn_at.as_deref(),
        Some("2026-08-13T09:00:00.000Z")
    );
    assert_eq!(edits.duration_seconds, Some(2), "09:00:00 to 09:00:02");
    assert_eq!(edits.compactions, 0);
    assert!(edits.truncated.is_empty(), "{:?}", edits.truncated);

    let errors = bench.observe(&bench.ingest("session-errors-a.jsonl"));
    assert!(
        errors
            .errors
            .iter()
            .any(|e| e.contains("Bash command failed")),
        "no error value came back: {:?}",
        errors.errors
    );
    assert!(
        errors.tools.contains(&"Bash".to_owned()),
        "{:?}",
        errors.tools
    );
    assert_eq!(errors.branch.as_deref(), Some("phase-3"));
}

/// D-04's whole reason for reading a blob: `entities` holds the BASENAME of a
/// command, so the arguments have to come from somewhere else.
///
/// The contrast is asserted on the same session, so this cannot pass by the
/// entity table happening to be empty.
#[test]
fn a_command_comes_back_with_its_arguments_and_not_merely_its_basename() {
    let bench = bench();
    let key = bench.ingest("session-errors-a.jsonl");
    let facts = bench.observe(&key);

    assert!(
        facts
            .commands
            .contains(&"cargo test -p verbatim-core --features testkit".to_owned()),
        "the command lost its arguments: {:?}",
        facts.commands
    );
    assert!(
        facts.commands.contains(&"sleep 600".to_owned()),
        "{:?}",
        facts.commands
    );

    // The premise: what SQL alone could have answered with.
    let stored: Vec<String> = bench
        .conn()
        .prepare(
            "SELECT DISTINCT e.value_norm FROM entities e
               JOIN turns t ON t.id = e.turn_id
              WHERE t.session_key = ?1 AND e.kind = 'command'",
        )
        .unwrap()
        .query_map([&key], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        stored,
        vec!["cargo".to_owned(), "sleep".to_owned()],
        "the entity table is supposed to hold basenames only"
    );
}

/// The other fact SQL cannot answer: `parse::Record` reads `gitBranch` and no
/// commit field, so a commit subject only exists if the blob was walked.
#[test]
fn a_commit_subject_comes_off_the_session_stream() {
    let bench = bench();
    let key = bench.ingest_written(
        "33333333-3333-4333-8333-333333333333.jsonl",
        &transcript(&[
            "git status --porcelain",
            "git commit -m \"wire the observation step up\"",
        ]),
    );
    let facts = bench.observe(&key);

    assert_eq!(
        facts.commits,
        vec!["wire the observation step up".to_owned()],
        "commands were {:?}",
        facts.commands
    );
    assert_eq!(facts.commands.len(), 2, "{:?}", facts.commands);
}

/// One session's whole document, as it lands in the row.
#[test]
fn the_stored_document_carries_every_obs_01_fact_by_name() {
    let bench = bench();
    let facts = bench.observe(&bench.ingest("session-edits.jsonl"));
    let value = facts.to_json();
    let object = value.as_object().expect("the document is an object");

    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "branch",
            "commands",
            "commits",
            "compactions",
            "duration_seconds",
            "errors",
            "files_modified",
            "files_read",
            "first_turn_at",
            "last_turn_at",
            "tools",
            "truncated",
            "turns",
        ]
    );
}

/// One transcript whose assistant turns each run one Bash command.
fn transcript(commands: &[&str]) -> String {
    let mut body = String::new();
    for (n, command) in commands.iter().enumerate() {
        let record = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "cwd": "/data/code/verbatim",
            "sessionId": "33333333-3333-4333-8333-333333333333",
            "type": "assistant",
            "uuid": format!("33333333-3333-4333-8333-33333333333{n}"),
            "timestamp": format!("2026-08-13T09:0{n}:00.000Z"),
            "gitBranch": "phase-7",
            "message": {
                "role": "assistant",
                "model": "claude-opus-5",
                "content": [{
                    "type": "tool_use",
                    "id": format!("toolu_{n}"),
                    "name": "Bash",
                    "input": {"command": command},
                }],
            },
        });
        body.push_str(&record.to_string());
        body.push('\n');
    }
    body
}

// ---------------------------------------------------------------------------
// The pass step: one row per newly finalized session, written once.
// ---------------------------------------------------------------------------

/// The session that went quiet before the pass, and the one still being typed.
const IDLE: &str = "11111111-1111-4111-8111-111111111111";
const LIVE: &str = "22222222-2222-4222-8222-222222222222";

/// A data directory plus the Claude config directory whose `projects` tree a
/// pass walks. Nothing here resolves a real transcript root.
struct PassBench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude: PathBuf,
    root: PathBuf,
}

fn pass_bench() -> PassBench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude = dir.path().join("claude");
    let root = dir.path().join("root");
    std::fs::create_dir_all(claude.join("projects")).unwrap();
    PassBench {
        _dir: dir,
        data_dir,
        claude,
        root,
    }
}

/// A UTC timestamp `offset` from now, in the shape a transcript writes.
///
/// SQLite computes it because SQLite is what the idle rule compares against: a
/// formatter written here could differ from the one under test in exactly the
/// way that would make the comparison pass for the wrong reason.
fn ts(offset: &str) -> String {
    Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
            [offset],
            |r| r.get(0),
        )
        .unwrap()
}

impl PassBench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn pass(&self) -> terminus_core::ingest::pass::Summary {
        let config =
            terminus_core::config::Config::from_parts(vec![self.claude.clone()], Vec::new());
        match terminus_core::ingest::pass::run_with(&self.data_dir, &config).unwrap() {
            terminus_core::ingest::pass::PassOutcome::Ran(summary) => summary,
            terminus_core::ingest::pass::PassOutcome::LockHeld => {
                panic!("nothing else holds the lock")
            }
        }
    }

    /// Write one transcript into the tree a pass walks, with a chosen clock.
    fn transcript(&self, session: &str, at: &str, command: &str) {
        let cwd = self.root.join("project-alpha");
        std::fs::create_dir_all(&cwd).unwrap();
        let dir = self.claude.join("projects").join("project-alpha");
        std::fs::create_dir_all(&dir).unwrap();

        let record = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "cwd": cwd.to_string_lossy(),
            "sessionId": session,
            "type": "assistant",
            "uuid": format!("{session}-0"),
            "timestamp": at,
            "gitBranch": "phase-7",
            "message": {
                "role": "assistant",
                "model": "claude-opus-5",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_0",
                    "name": "Bash",
                    "input": {"command": command},
                }],
            },
        });
        std::fs::write(dir.join(format!("{session}.jsonl")), format!("{record}\n")).unwrap();
    }

    /// Every observation row, as `(session_id, generated_at, mechanical)`.
    fn rows(&self) -> Vec<(Option<String>, Option<String>, Option<String>)> {
        self.conn()
            .prepare(
                "SELECT o.session_id, o.generated_at, o.mechanical
                   FROM observations o ORDER BY o.session_key",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn runs(&self) -> i64 {
        self.conn()
            .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
            .unwrap()
    }
}

/// One row per finalized session, none for a session still open, and a second
/// pass changes nothing.
///
/// The insert-only rule is what the second half is about, and it is not
/// tidiness: `terminus observations regenerate` is the only rebuild path (D-02)
/// precisely so a pass cannot discard a judgment half somebody paid for.
#[test]
fn a_pass_observes_every_newly_finalized_session_exactly_once() {
    let bench = pass_bench();
    bench.transcript(IDLE, &ts("-10 hours"), "cargo test -p terminus-core");
    bench.transcript(LIVE, &ts("-1 minutes"), "cargo build --release");

    let first = bench.pass();
    assert_eq!(first.observations.written, 1, "{:?}", first.observations);
    assert!(
        first.observations.notes.is_empty(),
        "{:?}",
        first.observations
    );

    let rows = bench.rows();
    assert_eq!(rows.len(), 1, "the live session got an observation too");
    assert_eq!(rows[0].0.as_deref(), Some(IDLE));
    let facts: serde_json::Value = serde_json::from_str(rows[0].2.as_deref().unwrap()).unwrap();
    assert_eq!(facts["branch"], "phase-7");
    assert_eq!(facts["turns"], 1);
    assert_eq!(facts["commands"][0], "cargo test -p terminus-core");
    assert_eq!(bench.runs(), 1, "the pass must write its runs row");

    let second = bench.pass();
    assert_eq!(second.observations.written, 0, "a pass rewrote a row");
    assert_eq!(bench.rows(), rows, "a second pass moved an observation");
    assert_eq!(bench.runs(), 2, "the second pass wrote no runs row");
}

/// A session that goes quiet between two passes is observed on the pass that
/// closes it, not on the one after: the step runs after `feedback::outcomes`,
/// which is what sets `is_final`.
#[test]
fn a_session_is_observed_on_the_pass_that_finalizes_it() {
    let bench = pass_bench();
    bench.transcript(LIVE, &ts("-1 minutes"), "cargo build --release");
    assert_eq!(bench.pass().observations.written, 0);
    assert!(bench.rows().is_empty());

    // The same session, now idle. Rewritten wholesale with an older clock,
    // which is what makes `finalize` close it on the next pass.
    bench
        .conn()
        .execute(
            "UPDATE session_meta SET last_turn_at = ?1",
            [ts("-10 hours")],
        )
        .unwrap();

    let summary = bench.pass();
    assert_eq!(summary.outcomes.finalized, 1, "the premise: it closed here");
    assert_eq!(
        summary.observations.written, 1,
        "{:?}",
        summary.observations
    );
    assert_eq!(bench.rows().len(), 1);
}

/// An excluded project's sessions are never observed, including sessions
/// archived before the exclusion was configured (ING-08): this step reads a
/// blob, and "excluded means never read" is the whole of the read half.
#[test]
fn an_excluded_project_gets_no_observation() {
    let bench = pass_bench();
    bench.transcript(IDLE, &ts("-10 hours"), "cargo test -p terminus-core");
    assert_eq!(bench.pass().observations.written, 1);

    bench
        .conn()
        .execute("DELETE FROM observations", [])
        .unwrap();
    let excluded = terminus_core::config::Config::from_parts(
        vec![bench.claude.clone()],
        vec![bench
            .root
            .join("project-alpha")
            .to_string_lossy()
            .into_owned()],
    );
    let observed = terminus_core::observe::observe_new(&bench.conn(), &excluded);
    assert_eq!(observed.written, 0, "{observed:?}");
    assert!(bench.rows().is_empty());
}
