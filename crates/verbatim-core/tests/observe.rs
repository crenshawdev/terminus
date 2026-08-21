//! OBS-01: the mechanical facts, computed off the parser and off nothing else.
//!
//! Every test here runs against a real store with real fixtures ingested into
//! it. Nothing is mocked and nothing is stubbed, because the claim under test
//! is precisely that these facts come out of the tables and the blob rather
//! than out of a model.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::observe::{self, Mechanical};
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{ingest, testkit};

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
