//! AC6 / ING-02: a second ingest launched while the first holds `LOCK`.
//!
//! Ten consecutive contended runs must each exit 0 in under 50 ms and add no
//! row to any table. The tenth run proves nothing on its own, so the test ends
//! by releasing the lock and running once more: an ingest that never wrote
//! anything would satisfy "no rows changed" trivially, and the control is what
//! makes the ten silent runs mean something.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use verbatim_core::ingest::{self, Attempt};
use verbatim_core::store::{Store, DB_FILE_NAME, TABLES};
use verbatim_core::testkit;

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
    let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
    command
        .arg("ingest")
        .arg(transcript)
        .env("VERBATIM_DATA_DIR", data_dir);
    command
}

#[test]
fn a_contended_ingest_exits_zero_fast_and_writes_nothing() {
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

    for run in 0..10 {
        let started = Instant::now();
        let output = ingest_command(&data_dir, &transcript)
            .output()
            .expect("spawn verbatim ingest");
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
        assert_eq!(
            counts(&db),
            before,
            "contended run {run} wrote to the store"
        );
    }

    // The control. Without it every assertion above is satisfied by an ingest
    // that does nothing at all.
    drop(guard);
    let output = ingest_command(&data_dir, &transcript)
        .output()
        .expect("spawn verbatim ingest");
    assert!(
        output.status.success(),
        "the uncontended run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(
        counts(&db),
        before,
        "the uncontended run wrote nothing either, so the ten contended runs prove nothing"
    );
}

/// The lock dies with the process holding it, which is why it is an OS lock and
/// not a PID file (`DESIGN-BRIEF.md:135`): a killed ingest leaves no wedge.
#[test]
fn the_lock_is_released_by_process_death() {
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
