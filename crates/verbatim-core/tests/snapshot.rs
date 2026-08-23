//! STOR-06: a consistent copy of the store, taken without stopping an ingest,
//! and a prune that keeps only the newest few.
//!
//! The comparison here is deliberately NOT built out of
//! `crates/verbatim/tests/crash.rs`'s `Snapshot` struct: phase 2's summary
//! records that it excludes `compaction_boundaries`, so a reused comparison is
//! blind on exactly the table a boundary-carrying fixture exercises. These
//! tests read the snapshot as a store instead - `verify::verify` over every
//! blob, and the session keys themselves.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::ingest::lock::{self, Attempt};
use verbatim_core::store::{snapshot, DB_FILE_NAME};
use verbatim_core::{ingest, testkit, verify, Store};

/// Three fixtures, committed before anything below runs.
const FIXTURES: [&str; 3] = [
    "session-basic.jsonl",
    "session-compacted.jsonl",
    "session-continuation.jsonl",
];

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    keys: Vec<String>,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let keys = FIXTURES
        .iter()
        .map(|fixture| {
            let path = testkit::copy_fixture_into(fixture, &work);
            match ingest::run(&data_dir, &path).unwrap() {
                ingest::Outcome::Committed(pass) => pass.session_key,
                other => panic!("{fixture}: {other:?}"),
            }
        })
        .collect();

    Bench {
        _dir: dir,
        data_dir,
        keys,
    }
}

impl Bench {
    fn snapshots(&self) -> PathBuf {
        self.data_dir.join(snapshot::DIR_NAME)
    }

    /// Snapshot files in the directory, by name - which is by time.
    fn listed(&self) -> Vec<String> {
        listed(&self.snapshots())
    }
}

fn listed(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Open a snapshot file as a store: it is copied into a directory of its own
/// under the store's name, because the resolver names one database per data
/// directory.
fn reopened(snapshot_file: &Path, into: &Path) -> Store {
    std::fs::create_dir_all(into).unwrap();
    std::fs::copy(snapshot_file, into.join(DB_FILE_NAME)).unwrap();
    Store::open(into).expect("a snapshot opens as a store")
}

fn session_keys(store: &Store) -> Vec<String> {
    store
        .conn()
        .prepare("SELECT session_key FROM sessions ORDER BY session_no")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The whole of AC6 in one test, because the three claims are only worth
/// anything about the SAME file: a copy taken while a pass owns the store opens
/// as a store, passes `verify` with no findings, and holds every session
/// committed before the open transaction began and none of the uncommitted one.
///
/// The ingest lock is held by a second handle for the duration, so this is not
/// a snapshot of a quiet store that happens to work. `File::try_lock` is
/// `flock(2)` with `LOCK_EX | LOCK_NB` on Unix and `LockFileEx` on Windows, and
/// both conflict between two open file descriptions whether or not they belong
/// to the same process - so a snapshot that quietly took the lock would find it
/// held and fail here rather than pass.
#[test]
fn a_snapshot_taken_mid_pass_opens_verifies_and_excludes_the_uncommitted_session() {
    let bench = bench();

    let held = match lock::try_acquire(&bench.data_dir).unwrap() {
        Attempt::Acquired(guard) => guard,
        Attempt::Held => panic!("nothing else holds the ingest lock"),
    };

    // A writer mid-transaction, exactly as a pass is between two files.
    let mut writer = Connection::open(bench.data_dir.join(DB_FILE_NAME)).unwrap();
    let tx = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "INSERT INTO sessions (session_key, session_no, blob) VALUES ('mid-pass', 9999, x'00')",
        [],
    )
    .unwrap();

    let file = snapshot::take(&bench.data_dir).expect("a snapshot is taken mid-pass");
    assert!(file.is_file(), "{} was not written", file.display());

    // Only now, so the copy above was demonstrably taken against an open write
    // transaction rather than after it.
    tx.commit().unwrap();
    drop(writer);
    drop(held);

    let opened = bench._dir.path().join("reopened");
    let store = reopened(&file, &opened);

    let report = verify::verify(&store).expect("the snapshot verifies");
    assert!(
        report.failures.is_empty(),
        "the snapshot reported findings: {:?}",
        report.failures
    );
    assert_eq!(report.checked, bench.keys.len());

    let keys = session_keys(&store);
    assert_eq!(keys, bench.keys, "the snapshot lost a committed session");
    assert!(
        !keys.iter().any(|k| k == "mid-pass"),
        "the snapshot picked up a session that had not committed"
    );
}

/// The other half of "opens as a store": SQLite's own answer about the file.
#[test]
fn a_snapshot_passes_sqlites_own_integrity_check() {
    let bench = bench();
    let file = snapshot::take(&bench.data_dir).unwrap();

    let conn = Connection::open(&file).unwrap();
    let answer: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(answer, "ok");
}

/// The prune keeps the newest by NAME, which is why the name carries the
/// instant: a filesystem mtime is rewritten by a copy, a restore or a `tar -x`,
/// and the snapshot the user most wants back is the one they just moved.
#[test]
fn pruning_keeps_the_newest_and_removes_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let snapshots = dir.path().join(snapshot::DIR_NAME);
    std::fs::create_dir_all(&snapshots).unwrap();

    let names: Vec<String> = (1..=5)
        .map(|n| format!("verbatim-2026082{n}T101010.000Z.db"))
        .collect();
    // Written oldest-name last, so a prune that read creation order rather than
    // the name would keep the wrong three.
    for name in names.iter().rev() {
        std::fs::write(snapshots.join(name), b"not really a database").unwrap();
    }

    let removed = snapshot::prune(&snapshots, 3).unwrap();

    assert_eq!(removed, 2);
    assert_eq!(listed(&snapshots), names[2..].to_vec());
}

/// The directory sits inside the user's data directory, so the prune removes
/// what this module wrote and nothing else - including the temporary name a
/// killed process leaves behind, which is why it is not spelled like a
/// snapshot.
#[test]
fn pruning_leaves_a_file_it_did_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let snapshots = dir.path().join(snapshot::DIR_NAME);
    std::fs::create_dir_all(&snapshots).unwrap();

    for name in [
        "verbatim-20260801T101010.000Z.db",
        "verbatim-20260802T101010.000Z.db",
        "notes.txt",
        ".verbatim-20260803T101010.000Z.db.tmp",
    ] {
        std::fs::write(snapshots.join(name), b"x").unwrap();
    }

    let removed = snapshot::prune(&snapshots, 1).unwrap();

    assert_eq!(removed, 1);
    assert_eq!(
        listed(&snapshots),
        vec![
            ".verbatim-20260803T101010.000Z.db.tmp".to_owned(),
            "notes.txt".to_owned(),
            "verbatim-20260802T101010.000Z.db".to_owned(),
        ]
    );
}

/// A snapshot lands inside the data directory, so relocating the data directory
/// carries the snapshots with it and `uninstall --purge` removes them.
#[test]
fn snapshots_land_inside_the_data_directory() {
    let bench = bench();
    let file = snapshot::take(&bench.data_dir).unwrap();

    assert_eq!(file.parent().unwrap(), bench.snapshots());
    assert_eq!(bench.listed().len(), 1);
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("verbatim-") && name.ends_with(".db") && name.len() == 32,
        "unexpected snapshot name {name}"
    );
}
