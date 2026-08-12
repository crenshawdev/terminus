//! The tree pass: one bad file does not stop it, and an excluded project is
//! never opened.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::config::Config;
use verbatim_core::discover;
use verbatim_core::ingest::pass::{self, PassOutcome, Summary};
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{ingest, testkit};

const PROJECT: &str = "-data-projects-cadence";
const SIBLING: &str = "-data-projects-cadence-research";
const WORKTREE: &str = "-data-projects-cadence--claude-worktrees-wt-a";
const SESSION_DIR: &str = "33333333-3333-4333-8333-333333333333";

/// A data directory plus a Claude config directory whose `projects` tree the
/// pass walks. Nothing here ever resolves a real transcript root.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        claude_dir,
    }
}

impl Bench {
    fn projects(&self) -> PathBuf {
        self.claude_dir.join("projects")
    }

    fn project(&self, name: &str) -> PathBuf {
        let dir = self.projects().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Copy a fixture into a project directory under a transcript-shaped name.
    fn place(&self, project: &str, name: &str, fixture: &str) -> PathBuf {
        let dest = self.project(project).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    fn config(&self, exclusions: &[&str]) -> Config {
        Config::from_parts(
            vec![self.claude_dir.clone()],
            exclusions.iter().map(|e| (*e).to_owned()).collect(),
        )
    }

    fn pass(&self, exclusions: &[&str]) -> Summary {
        match pass::run_with(&self.data_dir, &self.config(exclusions)).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

fn uuid(n: u8) -> String {
    format!("{n:08x}-1111-4111-8111-111111111111")
}

fn key(conn: &Connection, path: &Path) -> Option<String> {
    conn.query_row(
        "SELECT session_key FROM sessions WHERE session_key = ?1",
        [path.to_str().unwrap()],
        |r| r.get(0),
    )
    .ok()
}

fn turns_for(conn: &Connection, path: &Path) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM turns WHERE session_key = ?1",
        [path.to_str().unwrap()],
        |r| r.get(0),
    )
    .unwrap()
}

/// D-12, at the scale it matters. Six transcripts, two of them damaged in the
/// two ways phase 1 made fatal, one pass: the other four archive and the
/// summary names exactly the two that did not.
///
/// Either failure propagating would let one damaged session wedge every future
/// ingest of every other transcript in the tree - the failure gate fix
/// `3e5d9ff` closed for `reindex`, which costs far more here.
#[test]
fn one_damaged_transcript_does_not_stop_the_rest_of_the_tree() {
    let bench = bench();

    // Two files that will be damaged, ingested first so they have a
    // `session_meta` row and a watermark to damage.
    let orphaned = bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );
    let shrunk = bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(2)),
        "session-large-record.jsonl",
    );
    for path in [&orphaned, &shrunk] {
        match ingest::run(&bench.data_dir, path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{other:?}"),
        }
    }

    // Damage one: archived, with its metadata row gone. `Existing::read`
    // refuses rather than rewriting the blob from the tail.
    bench
        .conn()
        .execute(
            "DELETE FROM session_meta WHERE session_key = ?1",
            [orphaned.to_str().unwrap()],
        )
        .unwrap();
    // Damage the other: shorter on disk than its stored watermark. `read_tail`
    // refuses rather than guessing which bytes are still ours.
    std::fs::write(&shrunk, b"{}\n").unwrap();

    // Four healthy transcripts, none of them ingested yet.
    let healthy = vec![
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(3)),
            "session-basic.jsonl",
        ),
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(4)),
            "session-continuation.jsonl",
        ),
        bench.place(
            PROJECT,
            &format!("{SESSION_DIR}/subagents/agent-a.jsonl"),
            "subagents/agent-alpha.jsonl",
        ),
        bench.place(
            PROJECT,
            &format!("{SESSION_DIR}/subagents/workflows/wf_x/agent-deep.jsonl"),
            "subagents/workflows/wf_demo/agent-deep.jsonl",
        ),
    ];

    let summary = bench.pass(&[]);
    assert_eq!(summary.files_walked, 6);
    assert_eq!(summary.files_committed, 4);

    let mut failed: Vec<PathBuf> = summary.failures.iter().map(|(p, _)| p.clone()).collect();
    failed.sort();
    let mut expected = vec![orphaned.clone(), shrunk.clone()];
    expected.sort();
    assert_eq!(failed, expected, "exactly the two damaged files");

    let reasons: Vec<&str> = summary.failures.iter().map(|(_, r)| r.as_str()).collect();
    assert!(
        reasons.iter().any(|r| r.contains("session_meta")),
        "the orphaned session's reason must name what is missing: {reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("watermark")),
        "the short file's reason must name the watermark: {reasons:?}"
    );

    let conn = bench.conn();
    for path in &healthy {
        assert_eq!(key(&conn, path).as_deref(), path.to_str());
        assert!(
            turns_for(&conn, path) > 0,
            "{} archived no turns",
            path.display()
        );
    }
}

/// AC5's ingest half. An excluded project records zero opens of anything
/// beneath it and adds no row to any table, while the sibling whose encoded
/// name merely extends the excluded one is ingested normally.
///
/// The counted-open log is queried by path prefix rather than by total, because
/// a total could reach the expected number by opening every excluded file and
/// skipping an equal number elsewhere.
#[test]
fn an_excluded_project_is_never_opened_and_adds_no_row() {
    let bench = bench();

    let excluded = vec![
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(1)),
            "session-basic.jsonl",
        ),
        bench.place(
            PROJECT,
            &format!("{SESSION_DIR}/subagents/agent-a.jsonl"),
            "subagents/agent-alpha.jsonl",
        ),
        bench.place(
            WORKTREE,
            &format!("{}.jsonl", uuid(2)),
            "session-continuation.jsonl",
        ),
    ];
    let kept = bench.place(
        SIBLING,
        &format!("{}.jsonl", uuid(3)),
        "session-large-record.jsonl",
    );

    let summary = bench.pass(&["/data/projects/cadence"]);
    assert_eq!(summary.files_walked, 1, "only the sibling was walked");
    assert_eq!(summary.files_committed, 1);
    assert!(summary.failures.is_empty(), "{:?}", summary.failures);

    for project in [PROJECT, WORKTREE] {
        let dir = bench.projects().join(project).canonicalize().unwrap();
        assert!(
            discover::opened::under(&dir).is_empty(),
            "{project} was opened: {:?}",
            discover::opened::under(&dir)
        );
    }

    let conn = bench.conn();
    for path in &excluded {
        let key = path.to_str().unwrap();
        for (table, column) in [
            ("sessions", "session_key"),
            ("session_meta", "session_key"),
            ("turns", "session_key"),
            ("watermarks", "transcript_path"),
        ] {
            let rows: i64 = conn
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE {column} = ?1"),
                    [key],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(rows, 0, "{table} holds a row for an excluded transcript");
        }
    }
    let fts: i64 = conn
        .query_row("SELECT count(*) FROM turns_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        fts,
        turns_for(&conn, &kept),
        "turns_fts holds rows the excluded projects contributed"
    );

    // The sibling is there in full, and was opened.
    assert_eq!(key(&conn, &kept).as_deref(), kept.to_str());
    assert!(turns_for(&conn, &kept) > 0);
    assert!(!discover::opened::under(&kept).is_empty());
}

/// A pass over a tree it has already ingested walks every file and commits
/// none: nothing is past its watermark. The steady state of a hook-spawned run.
#[test]
fn a_second_pass_over_an_unchanged_tree_commits_nothing() {
    let bench = bench();
    bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );
    bench.place(
        PROJECT,
        &format!("{SESSION_DIR}/subagents/agent-a.jsonl"),
        "subagents/agent-alpha.jsonl",
    );

    let first = bench.pass(&[]);
    assert_eq!((first.files_walked, first.files_committed), (2, 2));

    let second = bench.pass(&[]);
    assert_eq!(second.files_walked, 2);
    assert_eq!(second.files_committed, 0);
    assert_eq!(second.files_unchanged(), 2);
    assert!(second.failures.is_empty());
}

/// The lock is taken once for the whole pass, not once per file.
#[test]
fn a_pass_reports_the_lock_rather_than_waiting_for_it() {
    let bench = bench();
    bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );

    let guard = match verbatim_core::ingest::lock::try_acquire(&bench.data_dir).unwrap() {
        verbatim_core::ingest::Attempt::Acquired(guard) => guard,
        other => panic!("the lock should have been free: {other:?}"),
    };
    assert_eq!(
        pass::run_with(&bench.data_dir, &bench.config(&[])).unwrap(),
        PassOutcome::LockHeld
    );
    drop(guard);

    let summary = bench.pass(&[]);
    assert_eq!(summary.files_committed, 1);
}
