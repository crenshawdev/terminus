//! AC3 / STOR-03: corruption localizes to named sessions.
//!
//! The fixture set here deliberately includes D-01's colliding pair -
//! `session-basic.jsonl` and `subagents/agent-alpha.jsonl`, which report the
//! *same* `sessionId`. That is what makes AC3's "and no other" clause able to
//! fail: a report keyed on the record's session id cannot distinguish the two,
//! so naming one would always name the other.
//!
//! Process-level assertions (exit code, stdout) live in
//! `crates/verbatim/tests/cli.rs`: `CARGO_BIN_EXE_verbatim` is defined only for
//! integration tests of the package that declares the bin, and this crate does
//! not depend on it.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{blob, ingest, testkit, verify, Store};

/// The three fixtures every test here ingests, the first two colliding on the
/// session id they report.
const FIXTURES: [&str; 3] = [
    "session-basic.jsonl",
    "subagents/agent-alpha.jsonl",
    "session-continuation.jsonl",
];

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    /// Session keys, in the order [`FIXTURES`] names them.
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
    fn store(&self) -> Store {
        Store::open(&self.data_dir).unwrap()
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// Flip one byte inside a session's blob column, with a direct UPDATE.
    fn flip_a_byte(&self, key: &str) {
        let conn = self.conn();
        let mut bytes: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [key],
                |r| r.get(0),
            )
            .unwrap();
        // Inside the compressed payload, past the header and block table.
        let at = bytes.len() / 2;
        bytes[at] ^= 0xff;
        conn.execute(
            "UPDATE sessions SET blob = ?1 WHERE session_key = ?2",
            rusqlite::params![bytes, key],
        )
        .unwrap();
    }

    /// Replace a session's stream with different bytes, validly compressed.
    ///
    /// The other half of the corruption space: `flip_a_byte` usually makes zstd
    /// refuse the block, which would let a `verify` that never hashed anything
    /// still pass AC3. This one decompresses perfectly and hashes differently,
    /// so only the BLAKE3 comparison can catch it (D-06).
    fn rewrite_the_stream(&self, key: &str) {
        let conn = self.conn();
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [key],
                |r| r.get(0),
            )
            .unwrap();
        let mut stream = blob::read_all(&bytes).unwrap();
        let at = stream.len() / 2;
        stream[at] = if stream[at] == b'x' { b'y' } else { b'x' };
        let rewritten = blob::write(&stream).unwrap();
        conn.execute(
            "UPDATE sessions SET blob = ?1 WHERE session_key = ?2",
            rusqlite::params![rewritten.bytes, key],
        )
        .unwrap();
    }
}

/// Session keys other than the ones named, so "and no other" can be checked
/// against the text the command actually prints.
fn others<'a>(keys: &'a [String], named: &[&str]) -> Vec<&'a String> {
    keys.iter().filter(|k| !named.contains(&k.as_str())).collect()
}

#[test]
fn a_clean_store_verifies() {
    let bench = bench();
    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.checked, 3);
    assert!(report.is_ok(), "{:?}", report.failures);
    assert_eq!(report.render(), "");
}

/// AC3. One flipped byte names one session and no other.
#[test]
fn one_corrupt_session_is_named_and_no_other() {
    let bench = bench();
    let corrupt = bench.keys[0].clone();
    bench.flip_a_byte(&corrupt);

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.checked, 3, "the walk visited every session");
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].session_key, corrupt);

    let text = report.render();
    assert!(text.contains(&corrupt), "{text}");
    for other in others(&bench.keys, &[&corrupt]) {
        assert!(!text.contains(other.as_str()), "named {other}: {text}");
    }
}

/// The corrupt session here is the sidecar, whose records report the *parent's*
/// session id. A report keyed on that id would name both files or the wrong
/// one; keyed on the transcript path it names exactly one (D-01).
#[test]
fn a_sidecar_and_its_parent_are_told_apart() {
    let bench = bench();
    let corrupt = bench.keys[1].clone();
    let parent = bench.keys[0].clone();

    // The premise: these two sessions really do report the same session id.
    let ids: Vec<String> = bench
        .conn()
        .prepare("SELECT session_id FROM session_meta WHERE session_key IN (?1, ?2)")
        .unwrap()
        .query_map([&corrupt, &parent], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1], "the fixture pair no longer collides");

    bench.rewrite_the_stream(&corrupt);
    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].session_key, corrupt);
    assert!(!report.render().contains(&parent), "{}", report.render());
}

/// A blob that will not decompress is a failure for its own session and must
/// not abort the walk, so the second corrupt session is still reported.
#[test]
fn two_corrupt_sessions_are_both_reported() {
    let bench = bench();
    let first = bench.keys[0].clone();
    let second = bench.keys[2].clone();
    bench.flip_a_byte(&first);
    bench.rewrite_the_stream(&second);

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.checked, 3);
    let named: Vec<&str> = report
        .failures
        .iter()
        .map(|f| f.session_key.as_str())
        .collect();
    assert!(named.contains(&first.as_str()), "{named:?}");
    assert!(named.contains(&second.as_str()), "{named:?}");
    assert_eq!(named.len(), 2);

    let text = report.render();
    for other in others(&bench.keys, &[&first, &second]) {
        assert!(!text.contains(other.as_str()), "named {other}: {text}");
    }
}

/// D-06 from the other side: only the BLAKE3 comparison catches a stream that
/// decompresses cleanly to the wrong bytes.
#[test]
fn a_valid_blob_holding_the_wrong_bytes_is_still_a_failure() {
    let bench = bench();
    let corrupt = bench.keys[2].clone();
    bench.rewrite_the_stream(&corrupt);

    // The premise: the replacement blob is perfectly readable.
    let bytes: Vec<u8> = bench
        .conn()
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [&corrupt],
            |r| r.get(0),
        )
        .unwrap();
    blob::read_all(&bytes).expect("the rewritten blob must decompress cleanly");

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.failures.len(), 1);
    assert!(
        report.failures[0].detail.contains("checksum mismatch"),
        "{:?}",
        report.failures[0]
    );
}

/// D-16. `integrity_check` is `doctor`'s signal (INST-06, phase 4) and never
/// appears here: a page-level complaint carries no session attribution and
/// would ride alongside the named session, breaking AC3's "and no other".
#[test]
fn the_report_never_mentions_integrity_check() {
    let bench = bench();
    bench.flip_a_byte(&bench.keys[0].clone());
    let report = verify::verify(&bench.store()).unwrap();
    assert!(!report.render().contains("integrity_check"));
}

/// The archive is two tables written in one transaction, so a blob with no
/// metadata row is not a state ingest can produce - and it is not a state
/// `verify` may pass over either.
#[test]
fn a_session_with_no_metadata_row_fails_rather_than_being_skipped() {
    let bench = bench();
    let orphan = bench.keys[0].clone();
    bench
        .conn()
        .execute("DELETE FROM session_meta WHERE session_key = ?1", [&orphan])
        .unwrap();

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.checked, 3);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].session_key, orphan);
}
