//! AC6 / ING-02: a second ingest launched while the first holds `LOCK`.
//!
//! Ten consecutive contended runs must each exit 0 and add no row to any table.
//! The tenth run proves nothing on its own, so the test ends by releasing the
//! lock and running once more: an ingest that never wrote anything would
//! satisfy "no rows changed" trivially, and the control is what makes the ten
//! silent runs mean something.
//!
//! That each of them exits in under 50 ms is a test of its own, so CI can skip
//! it on the macOS runners, where starting any process at all has taken longer
//! than the budget (`.github/workflows/test.yml`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use terminus_core::ingest::{self, Attempt, IngestLock};
use terminus_core::store::{Store, DB_FILE_NAME, TABLES};
use terminus_core::testkit;

/// ING-02's budget. The hook path spawns this process; a contended run must
/// cost one file open and one lock syscall, never a wait on SQLite's busy
/// handler (D-15).
const BUDGET: Duration = Duration::from_millis(50);

/// Row counts for every table a pass could touch, in a stable order.
fn counts(path: &Path) -> Vec<(String, i64)> {
    let conn = Connection::open(path).expect("open the store read side");
    TABLES
        .iter()
        .map(|table| {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap_or_else(|e| panic!("count {table}: {e}"));
            ((*table).to_owned(), count)
        })
        .collect()
}

fn ingest_command(data_dir: &Path, transcript: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_terminus"));
    command
        .arg("ingest")
        .arg(transcript)
        .env("TERMINUS_DATA_DIR", data_dir);
    command
}

/// A store, a transcript to ingest into it, and the lock held over both.
struct Contended {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    transcript: PathBuf,
    db: PathBuf,
    before: Vec<(String, i64)>,
    guard: IngestLock,
}

fn contended() -> Contended {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let transcript = testkit::copy_fixture_into("session-basic.jsonl", &work);

    // The store exists before the race, so "no row was added" is a comparison
    // against a real table set rather than against a missing file.
    drop(Store::open(&data_dir).expect("create the store"));
    let db = data_dir.join(DB_FILE_NAME);
    let before = counts(&db);

    let guard = match ingest::lock::try_acquire(&data_dir).unwrap() {
        Attempt::Acquired(guard) => guard,
        Attempt::Held => panic!("nothing else holds the lock in this test"),
    };
    assert!(guard.path().is_file(), "the LOCK file is created by us");

    Contended {
        _dir: dir,
        data_dir,
        transcript,
        db,
        before,
        guard,
    }
}

#[test]
fn a_contended_ingest_exits_zero_and_writes_nothing() {
    let race = contended();

    for run in 0..10 {
        let output = ingest_command(&race.data_dir, &race.transcript)
            .output()
            .expect("spawn terminus ingest");

        assert!(
            output.status.success(),
            "contended run {run} exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            counts(&race.db),
            race.before,
            "contended run {run} wrote to the store"
        );
    }

    // The control. Without it every assertion above is satisfied by an ingest
    // that does nothing at all.
    drop(race.guard);
    let output = ingest_command(&race.data_dir, &race.transcript)
        .output()
        .expect("spawn terminus ingest");
    assert!(
        output.status.success(),
        "the uncontended run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(
        counts(&race.db),
        race.before,
        "the uncontended run wrote nothing either, so the ten contended runs prove nothing"
    );
}

/// ING-02's budget, over the same ten runs.
#[test]
fn a_contended_ingest_exits_inside_the_budget() {
    let race = contended();

    for run in 0..10 {
        let started = Instant::now();
        let output = ingest_command(&race.data_dir, &race.transcript)
            .output()
            .expect("spawn terminus ingest");
        let elapsed = started.elapsed();

        assert!(
            output.status.success(),
            "contended run {run} exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            elapsed < BUDGET,
            "contended run {run} took {elapsed:?}, over the {BUDGET:?} budget"
        );
    }
}

/// A second attempt fails at once while the guard is held, and the lock is free
/// again once the guard is dropped. This covers the drop inside one process and
/// kills nothing.
///
/// The lock is an OS lock and not a PID file because it dies with the process
/// holding it, so a killed ingest leaves no wedge (`DESIGN-BRIEF.md:135`). Real
/// ingest processes are killed, and the next ingest still runs, in
/// `killing_ingest_anywhere_leaves_a_store_that_converges` in
/// `crates/terminus/tests/crash.rs`.
#[test]
fn a_held_lock_refuses_a_second_attempt_and_frees_when_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");

    {
        let guard = match ingest::lock::try_acquire(&data_dir).unwrap() {
            Attempt::Acquired(guard) => guard,
            Attempt::Held => panic!("a fresh data dir cannot be locked already"),
        };
        assert!(
            matches!(ingest::lock::try_acquire(&data_dir).unwrap(), Attempt::Held),
            "a second attempt must fail immediately rather than wait"
        );
        drop(guard);
    }

    assert!(
        matches!(
            ingest::lock::try_acquire(&data_dir).unwrap(),
            Attempt::Acquired(_)
        ),
        "the lock must be available again once released"
    );
}
