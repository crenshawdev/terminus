//! AC3 / STOR-03: corruption localizes to named sessions.
//!
//! The fixture set here deliberately includes D-01's colliding pair -
//! `session-basic.jsonl` and `subagents/agent-alpha.jsonl`, which report the
//! *same* `sessionId`. That is what makes AC3's "and no other" clause able to
//! fail: a report keyed on the record's session id cannot distinguish the two,
//! so naming one would always name the other.
//!
//! Process-level assertions (exit code, stdout) live in
//! `crates/terminus/tests/cli.rs`: `CARGO_BIN_EXE_terminus` is defined only for
//! integration tests of the package that declares the bin, and this crate does
//! not depend on it.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use terminus_core::store::DB_FILE_NAME;
use terminus_core::{blob, ingest, testkit, verify, Store};

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

    /// Raise D-13's flag on one session, the way a pass over a shortened
    /// transcript would.
    fn flag_divergence(&self, key: &str) {
        let changed = self
            .conn()
            .execute(
                "UPDATE session_meta SET transcript_diverged = 1 WHERE session_key = ?1",
                [key],
            )
            .unwrap();
        assert_eq!(changed, 1, "no session_meta row for {key}");
    }

    /// Empty a session's blob and mark it, exactly as `retention::apply` does.
    ///
    /// A direct UPDATE for the reason `flag_divergence` is one: what retention
    /// does to reach this state is asserted in `tests/retention.rs`, and the
    /// question here is only what `verify` reports off the two columns.
    fn evict(&self, key: &str) {
        let conn = self.conn();
        conn.execute(
            "UPDATE sessions SET blob = x'' WHERE session_key = ?1",
            [key],
        )
        .unwrap();
        let changed = conn
            .execute(
                "UPDATE session_meta SET is_evicted = 1 WHERE session_key = ?1",
                [key],
            )
            .unwrap();
        assert_eq!(changed, 1, "no session_meta row for {key}");
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
    keys.iter()
        .filter(|k| !named.contains(&k.as_str()))
        .collect()
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

/// D-13, at the level `verify` owns: the flag the pass sets is a per-session
/// failure like every other, attributed by `session_key` and naming no other.
///
/// The flag is set with a direct UPDATE here on purpose. What the pass does to
/// a truncated file, and what it leaves the archive looking like afterwards, is
/// asserted end to end in `tests/compaction.rs`; the question here is only what
/// `verify` reports off the column, and setting it directly is what keeps the
/// two tests from proving the same half twice.
#[test]
fn a_diverged_session_is_named_and_no_other() {
    let bench = bench();
    let diverged = bench.keys[1].clone();
    bench.flag_divergence(&diverged);

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.checked, 3, "the walk visited every session");
    assert!(!report.is_ok(), "a divergence is a failure");
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].session_key, diverged);

    let text = report.render();
    assert!(text.contains(&diverged), "{text}");
    for other in others(&bench.keys, &[&diverged]) {
        assert!(!text.contains(other.as_str()), "named {other}: {text}");
    }
}

/// The message has to say the archive is intact, because the reflex `verify`
/// provokes is to re-run ingest and that is exactly what must not happen here:
/// re-reading a shortened file from offset 0 would overwrite archived bytes.
#[test]
fn the_divergence_says_the_archive_was_not_touched() {
    let bench = bench();
    bench.flag_divergence(&bench.keys[0].clone());

    let text = report_text(&bench);
    assert!(text.contains("shorter"), "{text}");
    assert!(text.contains("archive was left untouched"), "{text}");
    // Not a checksum complaint. The blob verified.
    assert!(!text.contains("checksum"), "{text}");
}

/// Two independent facts about one session, and the graver one is not hidden by
/// the lesser. A diverged transcript says nothing about whether the blob is
/// intact, so a store where both are wrong must report both.
#[test]
fn a_diverged_session_with_a_damaged_blob_reports_both() {
    let bench = bench();
    let broken = bench.keys[0].clone();
    bench.flag_divergence(&broken);
    // `rewrite_the_stream` and not `flip_a_byte`: this fixture is small enough
    // that a flipped byte often still decompresses, so which of the two blob
    // verdicts fires would be incidental. This one always reaches the hash.
    bench.rewrite_the_stream(&broken);

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.failures.len(), 2, "{:?}", report.failures);
    assert!(report.failures.iter().all(|f| f.session_key == broken));

    let text = report.render();
    assert!(text.contains("checksum mismatch"), "{text}");
    assert!(text.contains("archive was left untouched"), "{text}");
    for other in others(&bench.keys, &[&broken]) {
        assert!(!text.contains(other.as_str()), "named {other}: {text}");
    }
}

/// The control the new signal needs: clearing the column returns the store to
/// silent. Without this, a flag that could never be cleared would still pass
/// every assertion above.
#[test]
fn clearing_the_flag_makes_the_store_clean_again() {
    let bench = bench();
    let diverged = bench.keys[2].clone();
    bench.flag_divergence(&diverged);
    assert!(!verify::verify(&bench.store()).unwrap().is_ok());

    bench
        .conn()
        .execute(
            "UPDATE session_meta SET transcript_diverged = NULL WHERE session_key = ?1",
            [&diverged],
        )
        .unwrap();

    let report = verify::verify(&bench.store()).unwrap();
    assert!(report.is_ok(), "{:?}", report.failures);
    assert_eq!(report.render(), "");
}

/// D-03. An evicted session's blob was emptied on purpose, so `verify` counts
/// it and says nothing about it - while a genuinely corrupt blob beside it is
/// still named, and only it.
///
/// Without the arm this is the failure mode: `blob::read_all` finds no header
/// in zero bytes, every evicted session reports "blob does not decompress", and
/// a store whose retention policy is doing exactly what it was told reads as
/// wholesale corruption.
#[test]
fn an_evicted_session_is_counted_and_never_named_while_a_corrupt_one_still_is() {
    let bench = bench();
    let evicted = bench.keys[0].clone();
    let corrupt = bench.keys[2].clone();
    bench.evict(&evicted);
    bench.rewrite_the_stream(&corrupt);

    // The premise: the emptied blob really is unreadable, so the arm is what
    // keeps it out of the report rather than the bytes happening to survive.
    let bytes: Vec<u8> = bench
        .conn()
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [&evicted],
            |r| r.get(0),
        )
        .unwrap();
    assert!(bytes.is_empty());
    assert!(blob::read_all(&bytes).is_err());

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(
        report.checked, 3,
        "an evicted session is still a session the walk visited"
    );
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert_eq!(report.failures[0].session_key, corrupt);

    let text = report.render();
    assert!(!text.contains(&evicted), "named the evicted session: {text}");
    for other in others(&bench.keys, &[&corrupt]) {
        assert!(!text.contains(other.as_str()), "named {other}: {text}");
    }
}

/// A store whose every session has been evicted has no findings at all, which
/// is what makes the case above a claim about the arm and not about the one
/// corrupt session drowning out the rest.
#[test]
fn a_store_of_evicted_sessions_verifies_clean() {
    let bench = bench();
    for key in bench.keys.clone() {
        bench.evict(&key);
    }

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.checked, 3);
    assert!(report.is_ok(), "{:?}", report.failures);
    assert_eq!(report.render(), "");
}

/// The divergence check stays independent of the eviction arm, exactly as it is
/// independent of the checksum verdict. "The file on disk is shorter than what
/// was archived" is a statement about the FILE, and emptying the blob answers
/// nothing about the file.
#[test]
fn an_evicted_session_can_still_report_a_divergence() {
    let bench = bench();
    let both = bench.keys[1].clone();
    bench.evict(&both);
    bench.flag_divergence(&both);

    let report = verify::verify(&bench.store()).unwrap();
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert_eq!(report.failures[0].session_key, both);
    let text = report.render();
    assert!(text.contains("archive was left untouched"), "{text}");
    // And not a word about the blob, which is the arm doing its job underneath.
    assert!(!text.contains("does not decompress"), "{text}");
    assert!(!text.contains("checksum"), "{text}");
}

fn report_text(bench: &Bench) -> String {
    verify::verify(&bench.store()).unwrap().render()
}
