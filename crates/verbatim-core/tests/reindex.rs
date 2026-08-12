//! AC4 / STOR-04: the derived tables come back from the blobs alone.
//!
//! The comparison is byte-identical JSON for a fixed query set, and it carries
//! turn ids rather than counts: a rebuild that renumbered turns would return
//! the same number of hits pointing at different records, which is precisely
//! what D-10's deterministic `turns.id` exists to prevent.
//!
//! `verbatim reindex` invoked as a *command* is asserted in
//! `crates/verbatim/tests/cli.rs`; this crate cannot spawn the binary.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::store::{
    schema, Store, DB_FILE_NAME, DERIVED_SCHEMA, META_ARCHIVE_FORMAT, META_DERIVED_SCHEMA,
};
use verbatim_core::{ingest, reindex, testkit};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
}

/// A store with every transcript fixture ingested.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    for fixture in testkit::TRANSCRIPT_FIXTURES {
        let path = testkit::copy_fixture_into(fixture, &work);
        match ingest::run(&data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{fixture}: {other:?}"),
        }
    }

    Bench {
        _dir: dir,
        data_dir,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn store(&self) -> Store {
        Store::open(&self.data_dir).unwrap()
    }

    /// Drop the four derived tables outright, as AC4 specifies.
    fn drop_derived(&self) {
        let conn = self.conn();
        for table in schema::DERIVED_TABLES {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))
                .unwrap();
        }
    }
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// AC4. Drop `turns`, `turns_fts`, `entities` and `paths`, rebuild, and the
/// fixed query set's JSON is byte-identical.
#[test]
fn the_fixed_query_set_is_byte_identical_across_a_rebuild() {
    let bench = bench();
    let before = testkit::query_set_json(&bench.conn());
    let archive_before = testkit::archive_digest(&bench.conn());

    // The premise: the query set is not empty, or this compares nothing.
    assert!(before.contains("\"hits\": ["));
    let hits: usize = before.matches(", ").count();
    assert!(hits > 10, "the query set returns too little to compare");

    bench.drop_derived();
    let mut store = bench.store();
    let rebuilt = reindex::reindex(&mut store).unwrap();
    drop(store);

    assert_eq!(rebuilt.sessions, testkit::TRANSCRIPT_FIXTURES.len());
    assert!(rebuilt.turns > 0);
    assert_eq!(testkit::query_set_json(&bench.conn()), before, "AC4");
    assert_eq!(
        testkit::archive_digest(&bench.conn()),
        archive_before,
        "the rebuild touched the archive"
    );
}

/// A rebuild over an intact index changes nothing either: the seam is
/// idempotent, so `reindex` is safe to run whenever (D-10).
#[test]
fn rebuilding_an_intact_index_changes_nothing() {
    let bench = bench();
    let before = testkit::query_set_json(&bench.conn());
    let mut store = bench.store();
    reindex::reindex(&mut store).unwrap();
    drop(store);
    assert_eq!(testkit::query_set_json(&bench.conn()), before);
}

/// STOR-04's actual claim: the blobs are the only input. If the derived tables
/// were consulted, dropping them would change what comes back.
#[test]
fn the_rebuild_reads_nothing_but_the_blobs() {
    let bench = bench();
    let reference = testkit::query_set_json(&bench.conn());

    // Corrupt the derived tables rather than emptying them: a rebuild that read
    // any of this would carry it forward.
    {
        let conn = bench.conn();
        conn.execute(
            "UPDATE turns SET record_type = 'wrong', ts = 'nonsense'",
            [],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM turns_fts WHERE rowid IN (SELECT id FROM turns)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO paths (turn_id, path) SELECT id, '/invented' FROM turns",
            [],
        )
        .unwrap();
    }

    let mut store = bench.store();
    reindex::reindex(&mut store).unwrap();
    drop(store);

    assert_eq!(testkit::query_set_json(&bench.conn()), reference);
    assert_eq!(count(&bench.conn(), "paths"), 0, "an invented row survived");
}

/// STOR-05's older-format branch, in the caller rather than in `Store::open`:
/// opening a store older than this build rebuilds its derived tables, leaves
/// the archive alone and brings `meta` forward.
///
/// `Store::open` itself stays side-effect-free and only reports the outcome -
/// `crates/verbatim-core/tests/store_open.rs` asserts exactly that, and a read
/// that silently rewrites four tables is not a read.
#[test]
fn an_older_store_is_rebuilt_when_it_is_opened_for_work() {
    let bench = bench();
    let reference = testkit::query_set_json(&bench.conn());
    let archive_before = testkit::archive_digest(&bench.conn());

    bench.drop_derived();
    {
        let conn = bench.conn();
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            rusqlite::params![(DERIVED_SCHEMA - 1).to_string(), META_DERIVED_SCHEMA],
        )
        .unwrap();
    }

    // The plain open reports the rebuild and performs none of it.
    {
        let store = bench.store();
        assert!(store.rebuild_required().is_some());
        assert_eq!(
            store.meta_int(META_DERIVED_SCHEMA).unwrap(),
            Some(DERIVED_SCHEMA - 1)
        );
    }

    let store = reindex::open_up_to_date(&bench.data_dir).unwrap();
    assert!(
        store.rebuild_required().is_none(),
        "the reopened handle still reports a rebuild"
    );
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA)
    );
    drop(store);

    assert_eq!(testkit::query_set_json(&bench.conn()), reference);
    assert_eq!(testkit::archive_digest(&bench.conn()), archive_before);
}

/// An ingest into an older store brings it forward before it appends, so turn
/// rows in two shapes never sit side by side.
#[test]
fn an_ingest_brings_an_older_store_forward_first() {
    let bench = bench();
    {
        let conn = bench.conn();
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            rusqlite::params![(DERIVED_SCHEMA - 1).to_string(), META_DERIVED_SCHEMA],
        )
        .unwrap();
        conn.execute("DELETE FROM turns", []).unwrap();
    }

    let work = bench.data_dir.parent().unwrap().join("late");
    std::fs::create_dir_all(&work).unwrap();
    let path = testkit::copy_fixture_into("session-basic.jsonl", &work);
    match ingest::run(&bench.data_dir, &path).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("{other:?}"),
    }

    let conn = bench.conn();
    assert_eq!(
        conn.query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_DERIVED_SCHEMA],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        DERIVED_SCHEMA.to_string()
    );
    // The rebuild restored the turns the setup deleted, and the new file added
    // its own session on top.
    assert_eq!(
        count(&conn, "sessions") as usize,
        testkit::TRANSCRIPT_FIXTURES.len() + 1
    );
    assert!(count(&conn, "turns") > 8);
}

/// `meta` moves with the rebuild and inside its transaction: a store stamped up
/// to date whose rebuild rolled back is the state this must not produce.
#[test]
fn the_version_integers_move_with_the_rebuild() {
    let bench = bench();
    {
        let conn = bench.conn();
        conn.execute(
            "UPDATE meta SET value = '0' WHERE key IN (?1, ?2)",
            rusqlite::params![META_ARCHIVE_FORMAT, META_DERIVED_SCHEMA],
        )
        .unwrap();
    }
    let mut store = bench.store();
    reindex::reindex(&mut store).unwrap();
    assert_eq!(
        store.meta_int(META_ARCHIVE_FORMAT).unwrap(),
        Some(verbatim_core::ARCHIVE_FORMAT)
    );
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA)
    );
}

/// One damaged blob must not take every undamaged session down with it.
///
/// A `?` on the decompression aborted the whole rebuild on the first bad blob
/// and named no session. Worse, `open_up_to_date` runs on the ingest path, so
/// once `DERIVED_SCHEMA` was bumped a single corrupt session stopped every
/// future ingest of every other transcript, with no repair path in phase 1.
/// `DESIGN-BRIEF.md:98`: "this session is damaged", never "the store is gone".
#[test]
fn a_corrupt_blob_is_skipped_and_named_rather_than_stopping_the_rebuild() {
    let bench = bench();
    let db = bench.data_dir.join(DB_FILE_NAME);

    let (damaged, undamaged): (String, i64) = {
        let conn = Connection::open(&db).unwrap();
        let key: String = conn
            .query_row(
                "SELECT session_key FROM sessions ORDER BY session_no",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let others: i64 = conn
            .query_row(
                "SELECT count(*) FROM sessions WHERE session_key <> ?1",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        assert!(others > 0, "this test needs more than one ingested session");

        let mut blob: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        // Inside the payload, so the header still parses and the failure lands
        // where a real bit-rot would: in the decompressor.
        for byte in blob.iter_mut().skip(60).take(150) {
            *byte = 0;
        }
        conn.execute(
            "UPDATE sessions SET blob = ?1 WHERE session_key = ?2",
            rusqlite::params![blob, &key],
        )
        .unwrap();
        (key, others)
    };

    let mut store = Store::open(&bench.data_dir).unwrap();
    let rebuilt =
        reindex::reindex(&mut store).expect("one damaged session must not fail the rebuild");

    assert_eq!(
        rebuilt.failed.len(),
        1,
        "expected exactly one skipped session"
    );
    assert_eq!(
        rebuilt.failed[0].0, damaged,
        "the skipped session must be named"
    );
    assert!(rebuilt.turns > 0, "the undamaged sessions were not rebuilt");

    let sessions_with_turns: i64 = store
        .conn()
        .query_row("SELECT count(DISTINCT session_key) FROM turns", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        sessions_with_turns, undamaged,
        "every session but the damaged one must have its turns back"
    );
}
