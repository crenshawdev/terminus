//! The seam ingest and reindex share.
//!
//! Two properties matter in this phase and neither is about what the FTS row
//! contains: re-deriving a turn is idempotent (D-10, which is what makes AC4's
//! rebuild safe), and the seam is the only writer, so calling it directly
//! reproduces exactly what ingest wrote.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::derive::{self, TurnRow};
use verbatim_core::ingest::{self, Outcome};
use verbatim_core::store::{schema, Store, DB_FILE_NAME};
use verbatim_core::{blob, parse, testkit};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    transcript: PathBuf,
    session_key: String,
    session_no: i64,
}

/// A store with `session-basic.jsonl` ingested.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let transcript = testkit::copy_fixture_into("session-basic.jsonl", &work);

    let pass = match ingest::run(&data_dir, &transcript).unwrap() {
        Outcome::Committed(pass) => pass,
        other => panic!("expected a committed pass, got {other:?}"),
    };
    let session_no = Connection::open(data_dir.join(DB_FILE_NAME))
        .unwrap()
        .query_row(
            "SELECT session_no FROM sessions WHERE session_key = ?1",
            [&pass.session_key],
            |r| r.get(0),
        )
        .unwrap();

    Bench {
        _dir: dir,
        data_dir,
        transcript,
        session_key: pass.session_key,
        session_no,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

fn matching(conn: &Connection, query: &str) -> Vec<i64> {
    conn.prepare("SELECT rowid FROM turns_fts WHERE turns_fts MATCH ?1 ORDER BY rowid")
        .unwrap()
        .query_map([query], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// One FTS row per turn, at the turn's own id.
#[test]
fn ingest_writes_one_fts_row_per_turn_at_the_turn_id() {
    let bench = bench();
    let conn = bench.conn();

    assert_eq!(count(&conn, "turns"), 8);
    assert_eq!(count(&conn, "turns_fts"), 8);

    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM turns ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let rowids: Vec<i64> = conn
        .prepare("SELECT rowid FROM turns_fts ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids, rowids, "rowid IS turns.id (D-10)");
}

/// A token that appears in exactly one turn across the whole fixture corpus
/// finds exactly that turn.
#[test]
fn a_unique_token_matches_exactly_one_turn() {
    let bench = bench();
    let conn = bench.conn();

    let hits = matching(&conn, testkit::UNIQUE_TOKEN);
    assert_eq!(hits.len(), 1, "`{}` is unique", testkit::UNIQUE_TOKEN);

    let (bytes, _) = testkit::read_turn(&conn, hits[0]);
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains(testkit::UNIQUE_TOKEN),
        "the matched turn does not contain the token: {text}"
    );
}

/// The eleven state record types produce no FTS row, so a token that only ever
/// appears in one of them is unfindable. This is D-03 seen from the index side.
#[test]
fn state_records_contribute_no_fts_row() {
    let bench = bench();
    let conn = bench.conn();
    let source = std::fs::read_to_string(&bench.transcript).unwrap();

    // `bridgeId` appears only in the `bridge-session` record, which is a state
    // record and gets no turn.
    assert!(source.contains("bridgeId"));
    assert!(
        matching(&conn, "bridgeId").is_empty(),
        "a state record reached the index"
    );
}

/// D-10's idempotence, which is what makes AC4's rebuild safe: re-deriving a
/// turn leaves the row count and the matching rowid unchanged.
#[test]
fn re_deriving_a_turn_changes_nothing() {
    let bench = bench();
    let conn = bench.conn();

    let before_count = count(&conn, "turns_fts");
    let before_hits = matching(&conn, testkit::UNIQUE_TOKEN);
    let before_rows = turn_rows(&conn);

    // Re-derive every turn from the blob, exactly as a rebuild would.
    let stream = {
        let raw: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [&bench.session_key],
                |r| r.get(0),
            )
            .unwrap();
        blob::read_all(&raw).unwrap()
    };
    let scan = parse::scan(&stream);
    let tx = conn.unchecked_transaction().unwrap();
    for (record, turn) in scan.turns() {
        derive::derive_turn(
            &tx,
            TurnRow {
                session_key: &bench.session_key,
                session_no: bench.session_no,
                turn,
                stream_offset: record.offset,
                byte_len: record.len,
                record: &stream[record.offset as usize..(record.offset + record.len) as usize],
            },
        )
        .unwrap();
    }
    tx.commit().unwrap();

    assert_eq!(count(&conn, "turns_fts"), before_count);
    assert_eq!(matching(&conn, testkit::UNIQUE_TOKEN), before_hits);
    assert_eq!(turn_rows(&conn), before_rows, "the seam is idempotent");
}

/// The seam called directly produces exactly the rows ingest wrote for that
/// turn - it is the same call, so a second way to write a derived row would
/// show up here as a difference.
#[test]
fn the_seam_reproduces_what_ingest_wrote_for_one_turn() {
    let bench = bench();
    let conn = bench.conn();
    let source = std::fs::read(&bench.transcript).unwrap();
    let scan = parse::scan(&source);
    let (record, turn) = scan.turns().next().unwrap();
    let id = schema::turn_id(bench.session_no, turn.turn_seq);

    let ingested = one_turn_row(&conn, id);
    conn.execute("DELETE FROM turns WHERE id = ?1", [id]).unwrap();
    conn.execute("DELETE FROM turns_fts WHERE rowid = ?1", [id])
        .unwrap();
    assert!(matching(&conn, testkit::UNIQUE_TOKEN).len() <= 1);

    let derived = derive::derive_turn(
        &conn,
        TurnRow {
            session_key: &bench.session_key,
            session_no: bench.session_no,
            turn,
            stream_offset: record.offset,
            byte_len: record.len,
            record: &source[record.offset as usize..(record.offset + record.len) as usize],
        },
    )
    .unwrap();

    assert_eq!(derived, id);
    assert_eq!(one_turn_row(&conn, id), ingested);
}

/// Phase 1 owns the entity tables and the rebuild path, not what fills them.
#[test]
fn no_entity_or_path_row_is_written_in_this_phase() {
    let bench = bench();
    let conn = bench.conn();
    assert_eq!(count(&conn, "entities"), 0);
    assert_eq!(count(&conn, "paths"), 0);
}

/// The seam must not open a transaction of its own: ingest commits the blob,
/// the turn rows and the watermark together (STOR-02), so a rollback of the
/// caller's transaction has to take the derived rows with it.
#[test]
fn the_seam_writes_inside_the_callers_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let conn = store.conn();
    conn.execute(
        "INSERT INTO sessions (session_key, session_no, blob) VALUES ('k', 0, x'00')",
        [],
    )
    .unwrap();

    let record = br#"{"type":"user","uuid":"u1","timestamp":"t","message":"quixotic"}"#;
    let scan = parse::scan(&[record.as_slice(), b"\n"].concat());
    let (_, turn) = scan.turns().next().unwrap();

    let tx = conn.unchecked_transaction().unwrap();
    let id = derive::derive_turn(
        &tx,
        TurnRow {
            session_key: "k",
            session_no: 0,
            turn,
            stream_offset: 0,
            byte_len: record.len() as u64,
            record,
        },
    )
    .unwrap();
    assert_eq!(matching(conn, "quixotic"), vec![id]);
    tx.rollback().unwrap();

    assert_eq!(count(conn, "turns"), 0, "the turn row survived a rollback");
    assert!(
        matching(conn, "quixotic").is_empty(),
        "the FTS row survived a rollback, so the seam committed on its own"
    );
}

fn turn_rows(conn: &Connection) -> Vec<(i64, i64, String, i64, i64)> {
    conn.prepare("SELECT id, turn_seq, record_type, stream_offset, byte_len FROM turns ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn one_turn_row(conn: &Connection, id: i64) -> (i64, i64, String, i64, i64) {
    turn_rows(conn)
        .into_iter()
        .find(|row| row.0 == id)
        .unwrap_or_else(|| panic!("no turn row {id}"))
}
