//! Recovery at the top of every run (ING-03, D-24).
//!
//! Every case here is a state a killed pass can leave behind, and the assertion
//! is always the same shape: the watermark comes back to what the committed
//! blob holds, and the archive is untouched.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use terminus_core::config::Config;
use terminus_core::ingest::pass::{self, PassOutcome, Summary};
use terminus_core::recover;
use terminus_core::store::DB_FILE_NAME;
use terminus_core::{ingest, testkit};

const PROJECT: &str = "-data-projects-cadence";
const ORPHAN: &str = "/no/such/transcript.jsonl";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
    /// The one transcript, already archived.
    transcript: PathBuf,
}

/// A store holding exactly one fully-ingested transcript, inside a tree the
/// pass can walk.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    let project = claude_dir.join("projects").join(PROJECT);
    std::fs::create_dir_all(&project).unwrap();

    let transcript = project.join("11111111-1111-4111-8111-111111111111.jsonl");
    std::fs::copy(testkit::fixture_path("session-basic.jsonl"), &transcript).unwrap();
    let transcript = transcript.canonicalize().unwrap();

    match ingest::run(&data_dir, &transcript).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("{other:?}"),
    }

    Bench {
        _dir: dir,
        data_dir,
        claude_dir,
        transcript,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn key(&self) -> String {
        self.transcript.to_str().unwrap().to_owned()
    }

    fn config(&self) -> Config {
        Config::from_parts(vec![self.claude_dir.clone()], vec![])
    }

    fn pass(&self) -> Summary {
        match pass::run_with(&self.data_dir, &self.config()).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    /// The state a killed pass leaves: a watermark past what the blob holds,
    /// and a watermark for a path nothing archived.
    fn damage(&self) -> (i64, Vec<u8>, Vec<u8>) {
        let conn = self.conn();
        let committed: i64 = conn
            .query_row(
                "SELECT uncompressed_len FROM session_meta WHERE session_key = ?1",
                [self.key()],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE watermarks SET byte_offset = ?2 WHERE transcript_path = ?1",
            rusqlite::params![self.key(), committed + 4096],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO watermarks (transcript_path, byte_offset) VALUES (?1, 8192)",
            [ORPHAN],
        )
        .unwrap();

        let blob: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [self.key()],
                |r| r.get(0),
            )
            .unwrap();
        let checksum: Vec<u8> = conn
            .query_row(
                "SELECT checksum FROM session_meta WHERE session_key = ?1",
                [self.key()],
                |r| r.get(0),
            )
            .unwrap();
        (committed, blob, checksum)
    }

    fn watermarks(&self) -> Vec<(String, i64)> {
        self.conn()
            .prepare("SELECT transcript_path, byte_offset FROM watermarks ORDER BY transcript_path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn assert_repaired(&self, committed: i64, blob: &[u8], checksum: &[u8]) {
        assert_eq!(
            self.watermarks(),
            vec![(self.key(), committed)],
            "the watermark must come back to the bytes the blob holds, and the \
             orphan must be gone"
        );

        let conn = self.conn();
        let blob_after: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [self.key()],
                |r| r.get(0),
            )
            .unwrap();
        let checksum_after: Vec<u8> = conn
            .query_row(
                "SELECT checksum FROM session_meta WHERE session_key = ?1",
                [self.key()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(blob_after == blob, "recovery rewrote the blob");
        assert_eq!(checksum_after, checksum, "recovery moved the checksum");
    }
}

/// The tree pass recovers before it walks, and says what it repaired.
#[test]
fn a_pass_lowers_an_overrun_watermark_and_drops_an_orphaned_one() {
    let bench = bench();
    let (committed, blob, checksum) = bench.damage();

    let summary = bench.pass();

    assert_eq!(summary.recovery.lowered.len(), 1);
    assert_eq!(summary.recovery.lowered[0].session_key, bench.key());
    assert_eq!(summary.recovery.lowered[0].from, (committed + 4096) as u64);
    assert_eq!(summary.recovery.lowered[0].to, committed as u64);
    assert_eq!(summary.recovery.removed.len(), 1);
    assert_eq!(summary.recovery.removed[0].session_key, ORPHAN);
    assert_eq!(summary.recovery.removed[0].byte_offset, 8192);
    assert!(summary.recovery.repaired());

    bench.assert_repaired(committed, &blob, &checksum);
    assert!(summary.failures.is_empty(), "{:?}", summary.failures);
}

/// ING-03 says every run, and single-file `terminus ingest <path.jsonl>` is a
/// run. Wiring only the pass would leave this entry point resuming from an
/// offset the archive never reached, losing the gap from the blob in silence.
#[test]
fn single_file_ingest_recovers_before_it_reads_the_transcript() {
    let bench = bench();
    let (committed, blob, checksum) = bench.damage();

    // Without recovery this is the call that fails or corrupts: the watermark
    // sits past the end of the file on disk.
    match ingest::run(&bench.data_dir, &bench.transcript).unwrap() {
        ingest::Outcome::UpToDate | ingest::Outcome::Committed(_) => {}
        other => panic!("{other:?}"),
    }

    bench.assert_repaired(committed, &blob, &checksum);
}

/// The report names both repairs, which is what lets the pass put them in its
/// `runs` row.
#[test]
fn the_report_names_every_repair() {
    let bench = bench();
    let (committed, _, _) = bench.damage();

    let (store, report) = recover::recover(&bench.data_dir).unwrap();
    drop(store);

    let lines = report.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains(&bench.key()) && lines[0].contains(&committed.to_string()));
    assert!(lines[1].contains(ORPHAN) && lines[1].contains("8192"));
}

/// A store needing no repair is left alone by either entry point. Recovery that
/// churned every watermark on every run would be indistinguishable from one
/// that never fired.
#[test]
fn a_store_needing_no_repair_has_no_watermark_touched() {
    let bench = bench();
    let before = bench.watermarks();
    assert_eq!(before.len(), 1);

    let summary = bench.pass();
    assert!(!summary.recovery.repaired(), "{:?}", summary.recovery);
    assert_eq!(bench.watermarks(), before);

    match ingest::run(&bench.data_dir, &bench.transcript).unwrap() {
        ingest::Outcome::UpToDate => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(bench.watermarks(), before);
}

/// A zero watermark claims no bytes: it is what a pass writes for a transcript
/// with no complete record yet, so removing it would be churn rather than
/// repair.
#[test]
fn a_zero_watermark_naming_no_session_is_left_alone() {
    let bench = bench();
    bench
        .conn()
        .execute(
            "INSERT INTO watermarks (transcript_path, byte_offset) VALUES (?1, 0)",
            [ORPHAN],
        )
        .unwrap();
    let before = bench.watermarks();

    let summary = bench.pass();
    assert!(!summary.recovery.repaired(), "{:?}", summary.recovery);
    assert_eq!(bench.watermarks(), before);
}

/// A session archived with no `session_meta` row is the damaged state phase 1
/// refuses to guess at and `verify` reports. Recovery must not touch its
/// watermark either - that would be the same guess, made somewhere quieter.
#[test]
fn a_session_with_no_metadata_row_keeps_its_watermark() {
    let bench = bench();
    bench
        .conn()
        .execute(
            "DELETE FROM session_meta WHERE session_key = ?1",
            [bench.key()],
        )
        .unwrap();
    let before = bench.watermarks();

    let (store, report) = recover::recover(&bench.data_dir).unwrap();
    drop(store);

    assert!(!report.repaired(), "{report:?}");
    assert_eq!(bench.watermarks(), before);
}
