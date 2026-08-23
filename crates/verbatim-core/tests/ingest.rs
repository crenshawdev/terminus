//! Ingesting one named transcript: AC7's byte fidelity, AC1's block counts,
//! and ING-01's resume from a mid-line watermark.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::ingest::{self, Outcome};
use verbatim_core::store::{schema, Store, DB_FILE_NAME, TABLES};
use verbatim_core::{blob, testkit};

/// A temp data dir plus a work dir the fixtures are copied into, so the
/// transcript path under test is never the repository's own.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
    }
}

impl Bench {
    fn copy(&self, fixture: &str) -> PathBuf {
        testkit::copy_fixture_into(fixture, &self.work)
    }

    fn ingest(&self, path: &Path) -> Outcome {
        ingest::run(&self.data_dir, path).unwrap_or_else(|e| panic!("ingest {path:?}: {e}"))
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

fn committed(outcome: Outcome) -> ingest::Pass {
    match outcome {
        Outcome::Committed(pass) => pass,
        other => panic!("expected a committed pass, got {other:?}"),
    }
}

fn counts(conn: &Connection) -> Vec<(String, i64)> {
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

fn stream(conn: &Connection, session_key: &str) -> Vec<u8> {
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [session_key],
            |r| r.get(0),
        )
        .unwrap();
    blob::read_all(&bytes).unwrap()
}

fn watermark(conn: &Connection, key: &str) -> i64 {
    conn.query_row(
        "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
        [key],
        |r| r.get(0),
    )
    .unwrap()
}

/// AC7. Every line of the fixture, including the eleven state record types that
/// get no turn row, reads back from the blob byte-identical to the source.
#[test]
fn the_blob_reproduces_the_transcript_byte_for_byte() {
    let bench = bench();
    let path = bench.copy("session-basic.jsonl");
    let source = std::fs::read(&path).unwrap();

    let pass = committed(bench.ingest(&path));
    let conn = bench.conn();

    assert_eq!(stream(&conn, &pass.session_key), source, "AC7");
    assert_eq!(pass.watermark, source.len() as u64);
    assert_eq!(watermark(&conn, &pass.session_key), source.len() as i64);

    // The state records are in the blob and produce no turn row (D-03/D-13).
    let text = String::from_utf8(stream(&conn, &pass.session_key)).unwrap();
    for kind in [
        "user",
        "assistant",
        "attachment",
        "system",
        "last-prompt",
        "ai-title",
        "mode",
        "permission-mode",
        "file-history-snapshot",
        "queue-operation",
        "bridge-session",
        "file-history-delta",
        "agent-name",
        "agent-setting",
        "frame-link",
    ] {
        assert!(
            text.contains(&format!("\"type\":\"{kind}\"")),
            "{kind} missing from the archived stream"
        );
    }

    let turns: i64 = conn
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(turns, 8, "eight turn records, fifteen record types");
    assert_eq!(pass.turns_added, 8);
}

/// The session is keyed on the transcript file, never on the record's session
/// id (D-01): the sidecar reports its parent's `sessionId` and must not collide.
#[test]
fn a_sidecar_reporting_its_parents_session_id_gets_its_own_row() {
    let bench = bench();
    let parent = bench.copy("session-basic.jsonl");
    let sidecar = bench.copy("subagents/agent-alpha.jsonl");

    let parent_pass = committed(bench.ingest(&parent));
    let sidecar_pass = committed(bench.ingest(&sidecar));
    let conn = bench.conn();

    assert_ne!(parent_pass.session_key, sidecar_pass.session_key);
    assert_eq!(
        stream(&conn, &parent_pass.session_key),
        std::fs::read(&parent).unwrap(),
        "the sidecar overwrote the parent's blob"
    );

    // The colliding half: both rows report the same session_id.
    let ids: Vec<String> = conn
        .prepare("SELECT session_id FROM session_meta ORDER BY session_key")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(
        ids[0], ids[1],
        "the fixture pair must collide on session_id"
    );
}

/// AC1 / STOR-01. A turn that fits in one block costs one block; the 200 KB
/// record costs the four it occupies.
#[test]
fn reading_a_turn_decompresses_only_the_blocks_it_occupies() {
    let bench = bench();
    let path = bench.copy("session-large-record.jsonl");
    let pass = committed(bench.ingest(&path));
    let conn = bench.conn();

    let session_no: i64 = conn
        .query_row(
            "SELECT session_no FROM sessions WHERE session_key = ?1",
            [&pass.session_key],
            |r| r.get(0),
        )
        .unwrap();

    let source = std::fs::read(&path).unwrap();
    let lines: Vec<&[u8]> = source.split(|b| *b == b'\n').collect();

    // Turn 0 is the oversized record, first line of the file, at offset 0.
    let (bytes, blocks) = testkit::read_turn(&conn, schema::turn_id(session_no, 0));
    assert_eq!(bytes, lines[0]);
    assert!(bytes.len() > 3 * blob::BLOCK_SIZE, "the big record is big");
    assert_eq!(blocks, 4, "AC1: N blocks for a turn spanning N");

    // Turn 1 is small and sits inside one block.
    let (bytes, blocks) = testkit::read_turn(&conn, schema::turn_id(session_no, 1));
    assert_eq!(bytes, lines[1]);
    assert!(bytes.len() < blob::BLOCK_SIZE);
    assert_eq!(blocks, 1, "AC1: one block for a turn that fits in one");
}

/// Every turn's row addresses its own record's bytes, for every fixture. This
/// is the general form of AC1's two spot checks.
#[test]
fn every_turn_row_addresses_its_own_record() {
    let bench = bench();
    for fixture in testkit::TRANSCRIPT_FIXTURES {
        let path = bench.copy(fixture);
        bench.ingest(&path);
    }
    let conn = bench.conn();

    let rows: Vec<(i64, String, i64, i64)> = conn
        .prepare("SELECT id, session_key, stream_offset, byte_len FROM turns ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(rows.len() >= 12, "every fixture contributed turns");

    for (id, session_key, offset, len) in rows {
        let (bytes, _) = testkit::read_turn(&conn, id);
        let source = stream(&conn, &session_key);
        assert_eq!(
            bytes,
            &source[offset as usize..(offset + len) as usize],
            "turn {id} does not address its own bytes"
        );
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("turn {id} is not one whole JSON record: {e}"));
        assert!(value.get("uuid").is_some() && value.get("timestamp").is_some());
    }
}

/// A rerun over an unchanged file adds no row to any table, not even `runs`.
#[test]
fn a_rerun_on_an_unchanged_file_writes_nothing() {
    let bench = bench();
    let path = bench.copy("session-basic.jsonl");
    committed(bench.ingest(&path));

    let before = counts(&bench.conn());
    assert_eq!(bench.ingest(&path), Outcome::UpToDate);
    assert_eq!(counts(&bench.conn()), before);
}

/// ING-01. Ingest a transcript cut mid-record, let it grow, ingest again: the
/// result must equal a single pass over the completed file.
#[test]
fn resuming_from_a_mid_line_watermark_converges_on_a_single_pass() {
    let bench = bench();
    let complete = testkit::fixture_bytes("session-basic.jsonl");
    let partial = testkit::fixture_bytes(testkit::TRUNCATED_FIXTURE);

    let growing = bench.work.join("growing.jsonl");
    std::fs::write(&growing, &partial).unwrap();
    let one_shot = bench.work.join("one-shot.jsonl");
    std::fs::write(&one_shot, &complete).unwrap();

    // Pass one stops at the last complete record, short of the file.
    let first = committed(bench.ingest(&growing));
    assert!(first.watermark < partial.len() as u64, "D-14");
    assert_eq!(
        stream(&bench.conn(), &first.session_key),
        &complete[..first.watermark as usize],
        "the partial record must not be archived"
    );

    // The file grows: the rest of the cut line, then the remaining records.
    std::fs::write(&growing, &complete).unwrap();
    let second = committed(bench.ingest(&growing));
    let reference = committed(bench.ingest(&one_shot));
    let conn = bench.conn();

    assert_eq!(second.watermark, reference.watermark);
    assert_eq!(second.watermark, complete.len() as u64);

    // Byte-identical blobs, not merely equal streams: an append that
    // recompressed the whole session would still decompress correctly.
    let blob_of = |key: &str| -> Vec<u8> {
        conn.query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [key],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        blob_of(&second.session_key),
        blob_of(&reference.session_key),
        "the incrementally built blob differs from the one-shot blob"
    );

    let turns_of = |key: &str| -> Vec<(i64, String, Option<String>, String, i64, i64)> {
        conn.prepare(
            "SELECT turn_seq, uuid, parent_uuid, record_type, stream_offset, byte_len
             FROM turns WHERE session_key = ?1 ORDER BY turn_seq",
        )
        .unwrap()
        .query_map([key], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    assert_eq!(
        turns_of(&second.session_key),
        turns_of(&reference.session_key)
    );
    assert_eq!(turns_of(&second.session_key).len(), 8);

    assert_eq!(
        watermark(&conn, &second.session_key),
        watermark(&conn, &reference.session_key)
    );
}

/// The same transcript reached through a symlink is one session, not two
/// (`DESIGN-BRIEF.md:406`; CONTEXT's third flagged assumption, resolved as
/// "canonicalize").
#[cfg(unix)]
#[test]
fn two_paths_to_one_transcript_produce_one_session() {
    let bench = bench();
    let path = bench.copy("session-basic.jsonl");
    let link_dir = bench.work.join("link");
    std::os::unix::fs::symlink(&bench.work, &link_dir).unwrap();
    let through_link = link_dir.join("session-basic.jsonl");

    let first = committed(bench.ingest(&path));
    assert_eq!(bench.ingest(&through_link), Outcome::UpToDate);

    let conn = bench.conn();
    let sessions: i64 = conn
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 1, "the symlinked path opened a second session");
    assert_eq!(
        first.session_key,
        path.canonicalize().unwrap().to_str().unwrap()
    );
}

/// D-11. `continues_from` comes from the record's foreign `session_id`, which
/// is not the file's own `sessionId`.
#[test]
fn a_continuation_records_the_session_it_continues() {
    let bench = bench();
    let basic = bench.copy("session-basic.jsonl");
    let continuation = bench.copy("session-continuation.jsonl");
    let basic_pass = committed(bench.ingest(&basic));
    let pass = committed(bench.ingest(&continuation));
    let conn = bench.conn();

    let (own, continues): (String, Option<String>) = conn
        .query_row(
            "SELECT session_id, continues_from FROM session_meta WHERE session_key = ?1",
            [&pass.session_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let predecessor: String = conn
        .query_row(
            "SELECT session_id FROM session_meta WHERE session_key = ?1",
            [&basic_pass.session_key],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(continues.as_deref(), Some(predecessor.as_str()));
    assert_ne!(own, predecessor, "its own sessionId is a different signal");
}

/// D-11's fallback: a continuation carrying no foreign `session_id` is linked
/// through the first turn's `parentUuid`, resolved to the session that holds
/// that record so the column stays in one namespace. No fixture carries this
/// shape, so the test builds one.
#[test]
fn a_continuation_without_a_foreign_session_id_falls_back_to_parent_uuid() {
    let bench = bench();
    let basic = bench.copy("session-basic.jsonl");
    let basic_pass = committed(bench.ingest(&basic));

    let last_uuid: String = bench
        .conn()
        .query_row(
            "SELECT uuid FROM turns WHERE session_key = ?1 ORDER BY turn_seq DESC LIMIT 1",
            [&basic_pass.session_key],
            |r| r.get(0),
        )
        .unwrap();

    let forked = bench.work.join("forked.jsonl");
    std::fs::write(
        &forked,
        format!(
            "{{\"type\":\"user\",\"uuid\":\"bbbbbbbb-0000-4000-8000-000000000001\",\
              \"parentUuid\":\"{last_uuid}\",\"timestamp\":\"2026-08-12T17:00:00.000Z\",\
              \"sessionId\":\"33333333-3333-4333-8333-333333333333\"}}\n"
        ),
    )
    .unwrap();

    let pass = committed(bench.ingest(&forked));
    let continues: Option<String> = bench
        .conn()
        .query_row(
            "SELECT continues_from FROM session_meta WHERE session_key = ?1",
            [&pass.session_key],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        continues.as_deref(),
        Some("11111111-1111-4111-8111-111111111111"),
        "the fallback must name a session, not a message uuid"
    );
}

/// A transcript that is not a transcript (D-12) still archives: it produces a
/// session and no turns, and nothing errors.
#[test]
fn a_non_transcript_jsonl_archives_with_no_turns() {
    let bench = bench();
    let path = bench.copy(testkit::NON_TRANSCRIPT_FIXTURE);
    let pass = committed(bench.ingest(&path));
    let conn = bench.conn();

    assert_eq!(pass.turns_added, 0);
    assert_eq!(
        stream(&conn, &pass.session_key),
        std::fs::read(&path).unwrap()
    );
    let turns: i64 = conn
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(turns, 0);
}

/// The checksum in `session_meta` is BLAKE3 over the uncompressed session bytes
/// (D-06), and the append path is handed it to verify before it writes.
#[test]
fn the_stored_checksum_is_blake3_over_the_uncompressed_stream() {
    let bench = bench();
    let path = bench.copy("session-basic.jsonl");
    let pass = committed(bench.ingest(&path));
    let conn = bench.conn();

    let (checksum, len): (Vec<u8>, i64) = conn
        .query_row(
            "SELECT checksum, uncompressed_len FROM session_meta WHERE session_key = ?1",
            [&pass.session_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let source = std::fs::read(&path).unwrap();
    assert_eq!(checksum, blake3::hash(&source).as_bytes().to_vec());
    assert_eq!(len, source.len() as i64);
}

/// STOR-02's premise, checked at the level this test can reach: the store is
/// opened for writing exactly once per pass and every table moves together.
/// The kill harness (AC2) is the real proof; this catches an ingest that wrote
/// a session without its watermark.
#[test]
fn a_pass_moves_the_session_its_turns_and_its_watermark_together() {
    let bench = bench();
    let path = bench.copy("session-basic.jsonl");
    let before = {
        drop(Store::open(&bench.data_dir).unwrap());
        counts(&bench.conn())
    };
    committed(bench.ingest(&path));
    let after = counts(&bench.conn());

    let moved: Vec<&str> = before
        .iter()
        .zip(after.iter())
        .filter(|(b, a)| b.1 != a.1)
        .map(|(b, _)| b.0.as_str())
        .collect();
    assert_eq!(
        moved,
        vec![
            "sessions",
            "session_meta",
            "turns",
            "turns_fts",
            // `session-basic.jsonl` carries a `Bash` tool_use, so phase 3's
            // extractor fills `entities` on the same pass. `paths` stays
            // absent: that command line names no file.
            "entities",
            "watermarks",
            "runs"
        ],
        "a pass must move exactly these tables"
    );
}

/// An archived session whose `session_meta` row is missing is DAMAGED, not
/// absent, and re-ingesting it must refuse rather than start over.
///
/// The lookup used an inner join, so "no metadata row" and "never ingested"
/// were the same answer. A pass over a grown transcript then wrote the tail as
/// the whole blob - destroying every archived byte before it - allocated a
/// second `session_no` that no longer matched the `sessions` row, and left the
/// watermark ahead of the committed blob, which is the one state STOR-02 says
/// a store must never reach. It exited 0, and `verify` certified it, because
/// the checksum had been minted over the truncated stream. The obvious
/// response to `verify` naming a session - re-run ingest - was what destroyed
/// the archive.
#[test]
fn a_session_that_lost_its_metadata_row_is_refused_rather_than_re_ingested() {
    let bench = bench();
    let path = testkit::copy_fixture_into("session-truncated.jsonl", &bench.work);
    match ingest::run(&bench.data_dir, &path).unwrap() {
        Outcome::Committed(_) => {}
        other => panic!("{other:?}"),
    }

    let db = bench.data_dir.join(DB_FILE_NAME);
    let (blob_before, watermark_before, session_no_before) = {
        let conn = Connection::open(&db).unwrap();
        conn.execute("DELETE FROM session_meta", []).unwrap();
        let blob: Vec<u8> = conn
            .query_row("SELECT blob FROM sessions", [], |r| r.get(0))
            .unwrap();
        let watermark: i64 = conn
            .query_row("SELECT byte_offset FROM watermarks", [], |r| r.get(0))
            .unwrap();
        let session_no: i64 = conn
            .query_row("SELECT session_no FROM sessions", [], |r| r.get(0))
            .unwrap();
        (blob, watermark, session_no)
    };

    // Grow the transcript, exactly as a live session grows between passes.
    std::fs::write(&path, testkit::fixture_bytes("session-basic.jsonl")).unwrap();
    let err = ingest::run(&bench.data_dir, &path)
        .expect_err("re-ingesting a session with no metadata row must be refused");
    let message = err.to_string();
    assert!(
        message.contains("session_meta"),
        "the refusal must name what is missing: {message}"
    );

    // The archive is exactly as it was.
    let conn = Connection::open(&db).unwrap();
    let blob_after: Vec<u8> = conn
        .query_row("SELECT blob FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!(
        blob_after == blob_before,
        "the refused pass rewrote the blob"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1,
        "the refused pass added a second session row"
    );
    assert_eq!(
        conn.query_row("SELECT session_no FROM sessions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        session_no_before
    );
    assert_eq!(
        conn.query_row("SELECT byte_offset FROM watermarks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        watermark_before,
        "the refused pass moved the watermark past the committed blob"
    );
}
