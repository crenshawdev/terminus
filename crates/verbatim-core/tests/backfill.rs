//! D-11 / ING-11: the bounded-parallelism pass lands where the sequential one
//! lands.
//!
//! The whole value of a second implementation of the walk is that it is not a
//! second implementation of the ingest, so the assertion that matters is
//! equality with `pass::run_with` over the same tree: every table's row count
//! (bar the post-walk set - see [`POST_WALK_TABLES`]), every session's blob
//! checksum, every watermark, and the archive digest that covers the blobs
//! themselves.
//!
//! `#![cfg(feature = "testkit")]` gates the file, which is load-bearing and also
//! a hazard `.planning/CAPTURE.md` records: `cargo test -p verbatim-core`
//! without the feature compiles this to an empty test binary and reports it
//! green. The command is
//! `cargo test -p verbatim-core --features testkit --test backfill`, and the
//! only thing that tells a real run from a vacuous one is the count in its
//! output - a self-test cannot say it, because a file that compiled to nothing
//! has no test left to run. Three live here, so `running 3 tests` is the line
//! to read, and it was read.

#![cfg(feature = "testkit")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::config::Config;
use verbatim_core::ingest::backfill;
use verbatim_core::ingest::pass::{self, PassOutcome, Summary};
use verbatim_core::store::{DB_FILE_NAME, TABLES};
use verbatim_core::{discover, testkit};

/// The worker count the plan's `Verify` names, and enough to be more than one.
const WORKERS: usize = 4;

/// The tables the post-walk half of a pass writes, which a backfill never runs.
///
/// `backfill::run_with` IS the walk and nothing more. `pass::run_with` runs the
/// same walk and then `feedback::drain` and `feedback::outcomes` after it - and
/// `outcomes` is what sets `session_meta.is_final`, which is the gate
/// `observe::observe_new` gets its sessions through. So these tables are
/// written by a step backfill deliberately does not take, and comparing their
/// row counts compares that deliberate asymmetry rather than the walk the two
/// implementations share.
///
/// `meta` is here for the same asymmetry one step further out: `pass::run_with`
/// runs the rolling snapshot step after `locked` returns (STOR-06, D-15) and
/// stamps `last_snapshot_at`, which backfill never writes. Only the row COUNT
/// was ever compared for this table - `archive_format` and `derived_schema` are
/// asserted where they are set, not here - so excluding it drops one count and
/// no value.
///
/// All of them, not just the one that fired. `decisions` and `labels` both read 0
/// against this fixture tree today, which is the only reason they never sprang
/// the trap `observations` just sprang; excluding `observations` alone would
/// move the trap to whichever of them a fixture change populates first. Every
/// other table, the checksums, the watermarks and the archive digest stay in
/// the comparison unchanged.
const POST_WALK_TABLES: &[&str] = &["observations", "decisions", "labels", "meta"];

/// How many copies of the fixture set the tree holds.
///
/// Three, so the tree has 33 transcripts against 4 workers: "more files than
/// workers" has to be true by a margin, and every worker needs a plausible
/// chance at a file for the thread-spread assertion to mean anything.
const COPIES: usize = 3;

struct Tree {
    _dir: tempfile::TempDir,
    root: PathBuf,
    claude_dir: PathBuf,
}

impl Tree {
    /// Every fixture transcript, [`COPIES`] times over, across one project
    /// directory per copy. Sidecars keep their `agent-*.jsonl` names and sit at
    /// the depth real ones sit at.
    fn build() -> Tree {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let claude_dir = root.join("claude");
        let projects = claude_dir.join("projects");
        std::fs::create_dir_all(&projects).unwrap();

        for copy in 0..COPIES {
            let project = projects.join(format!("-tmp-project-{copy}"));
            std::fs::create_dir_all(&project).unwrap();
            for (index, fixture) in testkit::TRANSCRIPT_FIXTURES.iter().enumerate() {
                let n = (copy * 100 + index) as u32;
                let dest = if fixture.contains('/') {
                    // A sidecar: `<project>/<sessionId>/subagents/agent-*.jsonl`.
                    project
                        .join(format!("{n:08x}-2222-4222-8222-222222222222"))
                        .join("subagents")
                        .join(format!("agent-{n}.jsonl"))
                } else {
                    project.join(format!("{n:08x}-1111-4111-8111-111111111111.jsonl"))
                };
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
            }
        }

        Tree {
            _dir: dir,
            root,
            claude_dir,
        }
    }

    fn config(&self) -> Config {
        Config::from_parts(vec![self.claude_dir.clone()], Vec::new())
    }

    fn data(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn transcripts(&self) -> Vec<PathBuf> {
        discover::discover(&self.config()).transcripts
    }
}

/// Everything two passes over one tree must agree on.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    /// One count per table in `store::TABLES`, in that order, minus
    /// [`POST_WALK_TABLES`].
    counts: Vec<(String, i64)>,
    /// The per-session blob checksum, keyed by session.
    checksums: BTreeMap<String, String>,
    watermarks: BTreeMap<String, i64>,
    /// A digest over every `sessions` and `session_meta` row, blobs included.
    archive: String,
}

fn snapshot(data_dir: &Path) -> Snapshot {
    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    Snapshot {
        counts: TABLES
            .iter()
            .filter(|table| !POST_WALK_TABLES.contains(*table))
            .map(|table| {
                let count: i64 = conn
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap_or_else(|e| panic!("count {table}: {e}"));
                ((*table).to_owned(), count)
            })
            .collect(),
        checksums: pairs(&conn, "SELECT session_key, hex(checksum) FROM session_meta"),
        watermarks: pairs(&conn, "SELECT transcript_path, byte_offset FROM watermarks"),
        archive: testkit::archive_digest(&conn),
    }
}

fn pairs<V: rusqlite::types::FromSql + Ord>(conn: &Connection, sql: &str) -> BTreeMap<String, V> {
    conn.prepare(sql)
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, V>(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn sequential(tree: &Tree, data_dir: &Path) -> Summary {
    match pass::run_with(data_dir, &tree.config()).unwrap() {
        PassOutcome::Ran(summary) => summary,
        PassOutcome::LockHeld => panic!("nothing else holds the lock in this test"),
    }
}

fn parallel(tree: &Tree, data_dir: &Path) -> backfill::Report {
    backfill::run_with(data_dir, &tree.config(), WORKERS).unwrap()
}

fn ran(report: &backfill::Report) -> &Summary {
    match &report.outcome {
        PassOutcome::Ran(summary) => summary,
        PassOutcome::LockHeld => panic!("nothing else holds the lock in this test"),
    }
}

/// The claim the whole module exists for: the two passes are the same pass.
#[test]
fn the_pipeline_lands_on_the_same_store_as_the_sequential_pass() {
    let tree = Tree::build();
    let files = tree.transcripts().len();
    assert_eq!(
        files,
        COPIES * testkit::TRANSCRIPT_FIXTURES.len(),
        "the tree is not the one this test builds"
    );

    let sequential_dir = tree.data("sequential");
    let parallel_dir = tree.data("parallel");
    let reference = sequential(&tree, &sequential_dir);
    let report = parallel(&tree, &parallel_dir);
    let summary = ran(&report);

    assert_eq!(summary.files_walked, reference.files_walked);
    assert_eq!(summary.files_committed, reference.files_committed);
    assert_eq!(summary.bytes_read, reference.bytes_read);
    assert_eq!(summary.turns_added, reference.turns_added);
    assert_eq!(summary.failures, reference.failures);

    // A name in POST_WALK_TABLES that no longer names a table would exclude
    // nothing and read as if it did, so the exclusion has to be checked against
    // the same list the counts are built from.
    for table in POST_WALK_TABLES {
        assert!(
            TABLES.contains(table),
            "POST_WALK_TABLES names {table}, which is not a table in store::TABLES"
        );
    }

    let expected = snapshot(&sequential_dir);
    let actual = snapshot(&parallel_dir);
    assert_eq!(
        actual.counts, expected.counts,
        "the two passes disagree on a table's row count"
    );
    assert_eq!(
        actual.checksums, expected.checksums,
        "the two passes disagree on a session's blob checksum"
    );
    assert_eq!(
        actual.watermarks, expected.watermarks,
        "the two passes disagree on a watermark"
    );
    assert_eq!(
        actual.archive, expected.archive,
        "the two passes archived different bytes"
    );

    // Without this the whole comparison is satisfied by a pipeline that runs
    // everything on the thread that called it.
    assert_eq!(report.workers, WORKERS);
    assert!(
        report.threads_used > 1,
        "the pipeline prepared {files} files on {} thread(s), so nothing was parallel",
        report.threads_used
    );
    assert!(
        report.threads_used <= WORKERS,
        "the pipeline used {} threads for {WORKERS} workers",
        report.threads_used
    );
}

/// More files than workers, and not one of them left behind.
#[test]
fn a_tree_with_more_files_than_workers_leaves_none_unprocessed() {
    let tree = Tree::build();
    let transcripts = tree.transcripts();
    assert!(
        transcripts.len() > WORKERS,
        "the tree must hold more files than there are workers"
    );

    let data_dir = tree.data("parallel");
    let report = parallel(&tree, &data_dir);
    let summary = ran(&report);

    assert_eq!(summary.files_walked, transcripts.len());
    assert_eq!(summary.failures, Vec::new());
    assert_eq!(summary.files_committed, transcripts.len());

    // A watermark per file is the durable proof, and the thing a resumed pass
    // reads: a file the pipeline dropped on the floor has none.
    let marks = snapshot(&data_dir).watermarks;
    for path in &transcripts {
        let key = path.to_str().unwrap();
        assert!(
            marks.contains_key(key),
            "no watermark for {key}: the pipeline never processed it"
        );
    }
    assert_eq!(marks.len(), transcripts.len());

    // One `runs` row for the whole pass, not one per file (D-10).
    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    let runs: i64 = conn
        .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(runs, 1, "a backfill must leave exactly one runs row");
}

/// A worker that panics is one file's failure, not the pass's deadlock.
///
/// The drain loop waits for exactly as many `Done` messages as it sent, and its
/// disconnect arm only fires once EVERY worker is gone. Before `panic_error`,
/// one worker panicking inside `prepare` therefore hung the whole backfill: the
/// message never arrived, the surviving workers held `done_rx` open so the
/// disconnect arm never fired, and the scope could not join to re-raise the
/// panic because its own closure was what was blocked.
///
/// The assertion is a WALL CLOCK one, because the defect's symptom is that the
/// call never returns - there is no wrong value to compare against. The pass is
/// run on its own thread and the result collected with `recv_timeout`, so the
/// failure mode is a reported timeout rather than a test binary that hangs
/// forever and takes CI with it.
#[test]
fn one_worker_panicking_is_a_skipped_file_and_not_a_wedged_pass() {
    // Generous: this tree takes well under a second, and the bound only has to
    // tell "returned" from "never returns".
    const BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

    let tree = Tree::build();
    // Arm the fault: one more transcript, in a project of its own, whose name
    // is the one `fault::panic_preparing` fires on.
    let armed = tree
        .claude_dir
        .join("projects")
        .join("-tmp-project-panic")
        .join("deadbeef-2222-4222-8222-222222222222")
        .join("subagents")
        .join(verbatim_core::ingest::fault::PANIC_PREPARING);
    std::fs::create_dir_all(armed.parent().unwrap()).unwrap();
    std::fs::copy(
        testkit::fixture_path(testkit::TRANSCRIPT_FIXTURES[0]),
        &armed,
    )
    .unwrap();

    let expected = COPIES * testkit::TRANSCRIPT_FIXTURES.len() + 1;
    assert_eq!(
        tree.transcripts().len(),
        expected,
        "the armed transcript is not in the tree, so this test would pass vacuously"
    );

    let data_dir = tree.data("panic");
    let config = tree.config();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // The panic is caught inside the worker, so this is the ordinary
        // `Ok` path - a `Result` crossing the channel, not a caught unwind.
        let _ = tx.send(backfill::run_with(&data_dir, &config, WORKERS));
    });

    let report = rx
        .recv_timeout(BUDGET)
        .expect("the backfill never returned: a panicking worker wedged the pass")
        .expect("a per-file panic is not a failure of the pass as a whole");
    let summary = ran(&report);

    assert_eq!(
        summary.files_walked, expected,
        "every file is walked, the armed one included"
    );
    assert_eq!(
        summary.files_committed,
        expected - 1,
        "every file but the armed one commits"
    );

    let failures: Vec<_> = summary
        .failures
        .iter()
        .filter(|(path, _)| path == &armed)
        .collect();
    assert_eq!(
        failures.len(),
        1,
        "the armed file is recorded exactly once, as a failure: {:?}",
        summary.failures
    );
    assert!(
        failures[0].1.contains("panicked"),
        "the recorded reason says what happened: {}",
        failures[0].1
    );
}
