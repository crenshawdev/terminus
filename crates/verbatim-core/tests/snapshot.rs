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
use verbatim_core::config::{Config, DEFAULT_SNAPSHOT_INTERVAL_HOURS};
use verbatim_core::ingest::lock::{self, Attempt};
use verbatim_core::ingest::pass::{self, PassOutcome, Summary};
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

// ---------------------------------------------------------------------------
// The schedule: on by default, at most once per interval, outside the lock
// (STOR-06, D-15)

/// A data directory plus a Claude tree for the pass to walk, so the snapshot
/// step is reached the way a hook reaches it rather than by being called.
struct PassBench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
}

fn pass_bench() -> PassBench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    let project = claude_dir.join("projects").join("-data-projects-cadence");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(
        testkit::fixture_path("session-basic.jsonl"),
        project.join("11111111-1111-4111-8111-111111111111.jsonl"),
    )
    .unwrap();

    PassBench {
        _dir: dir,
        data_dir,
        claude_dir,
    }
}

impl PassBench {
    fn config(&self) -> Config {
        Config::from_parts(vec![self.claude_dir.clone()], Vec::new())
    }

    fn pass(&self) -> Summary {
        match pass::run_with(&self.data_dir, &self.config()).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn snapshots(&self) -> Vec<String> {
        let dir = self.data_dir.join(snapshot::DIR_NAME);
        if dir.is_dir() {
            listed(&dir)
        } else {
            Vec::new()
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.data_dir).unwrap()
    }

    fn stamp(&self) -> Option<i64> {
        self.store().meta_int(pass::META_LAST_SNAPSHOT).unwrap()
    }

    fn runs(&self) -> i64 {
        self.store()
            .conn()
            .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
            .unwrap()
    }
}

/// Two claims that are only worth anything together: the first pass takes a
/// snapshot with nothing configuring it, and the second one immediately after
/// does not. Without the second, "on by default" is a machine that writes an
/// archive-sized file on every prompt.
#[test]
fn the_first_pass_snapshots_and_the_next_one_inside_the_interval_does_not() {
    let bench = pass_bench();

    let first = bench.pass();
    assert!(first.snapshot.notes.is_empty(), "{:?}", first.snapshot);
    assert!(first.snapshot.taken.is_some());
    let after_first = bench.snapshots();
    assert_eq!(after_first.len(), 1, "{after_first:?}");
    let stamped = bench.stamp().expect("the pass stamped `meta`");

    let second = bench.pass();
    assert!(second.snapshot.notes.is_empty(), "{:?}", second.snapshot);
    assert_eq!(
        second.snapshot.taken, None,
        "a second pass inside the interval took another snapshot"
    );
    assert_eq!(bench.snapshots(), after_first);
    assert_eq!(
        bench.stamp(),
        Some(stamped),
        "an undue pass moved the timestamp forward, which would slide the interval"
    );
}

/// The gate is the timestamp and nothing else: backdate it past the interval
/// and the next pass copies again.
#[test]
fn a_pass_past_the_interval_takes_the_next_snapshot() {
    let bench = pass_bench();
    bench.pass();
    let first = bench.snapshots();
    assert_eq!(first.len(), 1);

    let stamped = bench.stamp().unwrap();
    // One second past 24 hours, so the boundary itself is exercised rather than
    // a week-old timestamp that any comparison would pass.
    let backdated = stamped - (DEFAULT_SNAPSHOT_INTERVAL_HOURS as i64 * 3_600) - 1;
    let store = bench.store();
    store
        .set_meta_int(pass::META_LAST_SNAPSHOT, backdated)
        .unwrap();
    drop(store);

    let summary = bench.pass();

    assert!(summary.snapshot.taken.is_some(), "{:?}", summary.snapshot);
    let both = bench.snapshots();
    assert_eq!(both.len(), 2, "{both:?}");
    assert_eq!(both[0], first[0], "the older snapshot was not kept");
    // Against the backdated value and not against `stamped`: the stamp is unix
    // SECONDS and two passes in one test land inside the same second, so
    // "moved forward from a day ago" is the claim and "moved forward from the
    // first pass" is a statement about the clock's resolution.
    assert!(
        bench.stamp().unwrap() > backdated,
        "the interval was not restamped"
    );
}

/// The retained count is a config key and the prune runs in the pass, so a
/// store configured to keep one keeps one however many intervals elapse.
#[test]
fn the_pass_prunes_to_the_configured_count() {
    let bench = pass_bench();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(verbatim_core::config::CONFIG_FILE_NAME),
        format!(
            "roots = ['{}']\n[snapshot]\nkeep = 1\n",
            bench.claude_dir.display()
        ),
    )
    .unwrap();
    let config = Config::load_from(dir.path()).unwrap();

    let mut taken = 0;
    for _ in 0..3 {
        match pass::run_with(&bench.data_dir, &config).unwrap() {
            PassOutcome::Ran(summary) => {
                if summary.snapshot.taken.is_some() {
                    taken += 1;
                }
            }
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
        // Age the stamp out, so each pass is due.
        let store = bench.store();
        store.set_meta_int(pass::META_LAST_SNAPSHOT, 0).unwrap();
    }

    assert_eq!(taken, 3, "a due pass declined to snapshot");
    assert_eq!(
        bench.snapshots().len(),
        1,
        "the prune left more than the configured count"
    );
}

/// `enabled = false` is the off switch, and it costs nothing: no directory, no
/// copy, no `meta` row.
#[test]
fn snapshots_turned_off_write_nothing_at_all() {
    let bench = pass_bench();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(verbatim_core::config::CONFIG_FILE_NAME),
        format!(
            "roots = ['{}']\n[snapshot]\nenabled = false\n",
            bench.claude_dir.display()
        ),
    )
    .unwrap();
    let config = Config::load_from(dir.path()).unwrap();

    let summary = match pass::run_with(&bench.data_dir, &config).unwrap() {
        PassOutcome::Ran(summary) => summary,
        PassOutcome::LockHeld => panic!("nothing else holds the lock"),
    };

    assert_eq!(summary.snapshot, Default::default());
    assert!(!bench.data_dir.join(snapshot::DIR_NAME).exists());
    assert_eq!(bench.stamp(), None);
}

/// A snapshot that cannot be written is a NOTE. The archive is the work: the
/// pass still ran, still committed, still wrote its `runs` row, and the copy of
/// what is already safely stored is the only thing missing.
#[cfg(unix)]
#[test]
fn a_snapshot_that_cannot_be_written_is_a_note_and_not_a_failed_pass() {
    use std::os::unix::fs::PermissionsExt;

    let bench = pass_bench();
    let snapshots = bench.data_dir.join(snapshot::DIR_NAME);
    std::fs::create_dir_all(&snapshots).unwrap();
    std::fs::set_permissions(&snapshots, std::fs::Permissions::from_mode(0o500)).unwrap();

    let outcome = pass::run_with(&bench.data_dir, &bench.config()).unwrap();
    std::fs::set_permissions(&snapshots, std::fs::Permissions::from_mode(0o755)).unwrap();

    let summary = match outcome {
        PassOutcome::Ran(summary) => summary,
        PassOutcome::LockHeld => panic!("nothing else holds the lock"),
    };

    if summary.snapshot.taken.is_some() {
        // Running as root defeats the mode bits; the assertions below would
        // then be about nothing.
        println!("skipped: this process can write into a 0500 directory");
        return;
    }
    assert_eq!(summary.files_committed, 1, "the pass did not do its work");
    assert_eq!(bench.runs(), 1, "the pass wrote no `runs` row");
    assert_eq!(
        summary.snapshot.notes.len(),
        1,
        "{:?}",
        summary.snapshot.notes
    );
    assert!(
        summary.snapshot.notes[0].contains("could not be snapshotted"),
        "{:?}",
        summary.snapshot.notes
    );
    assert_eq!(
        bench.stamp(),
        None,
        "a failed snapshot stamped the interval, buying itself a day of silence"
    );
}
