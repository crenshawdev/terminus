//! What a fresh store contains, and the two properties the rebuild path turns
//! on: a deterministic turn id and an FTS5 table that can delete.

use std::collections::BTreeSet;

use rusqlite::Connection;
use verbatim_core::store::schema::{
    self, split_turn_id, turn_id, MAX_TURNS_PER_SESSION, TURN_SEQ_BITS,
};
use verbatim_core::Store;

fn fresh() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("fresh store");
    (dir, store)
}

fn names(conn: &Connection, kind: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1 ORDER BY name")
        .unwrap();
    let rows = stmt
        .query_map([kind], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap);
    rows.collect()
}

#[test]
fn a_fresh_store_carries_exactly_the_phase_two_tables() {
    let (_dir, store) = fresh();
    let tables = names(store.conn(), "table");

    // FTS5 keeps its own storage in shadow tables; they are the virtual table's
    // implementation, not tables of ours.
    let shadow: BTreeSet<String> = tables
        .iter()
        .filter(|n| n.starts_with("turns_fts_"))
        .cloned()
        .collect();
    let ours: BTreeSet<String> = tables.difference(&shadow).cloned().collect();

    let expected: BTreeSet<String> = schema::TABLES.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(ours, expected, "unexpected table set");

    for required in ["turns_fts_data", "turns_fts_idx", "turns_fts_docsize"] {
        assert!(
            shadow.contains(required),
            "missing FTS5 shadow table {required}: {shadow:?}"
        );
    }

    // Every index is either one we named or one SQLite made for a UNIQUE or
    // PRIMARY KEY constraint. Anything else means a table crept in.
    for index in names(store.conn(), "index") {
        assert!(
            index.starts_with("idx_") || index.starts_with("sqlite_autoindex_"),
            "unexpected index {index}"
        );
    }
}

/// D-10: `turns_fts` must be contentless *and* deletable, or a rebuild cannot
/// be idempotent.
#[test]
fn turns_fts_is_contentless_and_deletable() {
    let (_dir, store) = fresh();
    let sql: String = store
        .conn()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'turns_fts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("content=''"), "{sql}");
    assert!(sql.contains("contentless_delete=1"), "{sql}");
}

/// D-11 keeps two lineage signals apart, and each lives on its own table.
#[test]
fn the_two_lineage_columns_exist_on_their_own_tables() {
    let (_dir, store) = fresh();
    assert!(columns(store.conn(), "turns").contains("parent_uuid"));
    assert!(columns(store.conn(), "session_meta").contains("continues_from"));

    // The point of D-11 is that neither replaces the other.
    assert!(!columns(store.conn(), "turns").contains("continues_from"));
    assert!(!columns(store.conn(), "session_meta").contains("parent_uuid"));
}

/// D-04: turn coordinates address the uncompressed stream, so no column here
/// may name a compressed position.
#[test]
fn turn_coordinates_are_uncompressed_stream_coordinates() {
    let (_dir, store) = fresh();
    let cols = columns(store.conn(), "turns");
    assert!(cols.contains("stream_offset"));
    assert!(cols.contains("byte_len"));
}

/// The archive keeps a stable integer surrogate, because `turns.id` is derived
/// from it and must survive a drop-and-rebuild.
#[test]
fn sessions_carries_a_stable_integer_surrogate() {
    let (_dir, store) = fresh();
    let cols = columns(store.conn(), "sessions");
    assert!(cols.contains("session_key"));
    assert!(cols.contains("session_no"));
    assert!(cols.contains("blob"));
}

#[test]
fn turn_ids_round_trip_and_never_collide_across_sessions() {
    assert_eq!(split_turn_id(turn_id(0, 0)), (0, 0));
    assert_eq!(split_turn_id(turn_id(1, 0)), (1, 0));
    assert_eq!(
        split_turn_id(turn_id(7, MAX_TURNS_PER_SESSION - 1)),
        (7, MAX_TURNS_PER_SESSION - 1)
    );

    // The last turn of one session and the first of the next are adjacent
    // integers and still distinct.
    assert_eq!(
        turn_id(3, MAX_TURNS_PER_SESSION - 1) + 1,
        turn_id(4, 0),
        "the ordinal space must be exactly {MAX_TURNS_PER_SESSION} wide"
    );
    assert_eq!(MAX_TURNS_PER_SESSION, 1 << TURN_SEQ_BITS);

    // Deterministic: same inputs, same id, no counter and no clock involved.
    assert_eq!(turn_id(42, 17), turn_id(42, 17));
}

#[test]
#[should_panic(expected = "outside the")]
fn a_turn_seq_past_the_reserved_space_panics_rather_than_folding() {
    turn_id(1, MAX_TURNS_PER_SESSION);
}

/// The FTS5 round trip D-10 depends on, against the real table rather than a
/// scratch one: insert at an explicit rowid, match, delete, match again.
#[test]
fn an_fts_row_at_an_explicit_rowid_matches_then_deletes() {
    let (_dir, store) = fresh();
    let conn = store.conn();
    let rowid = turn_id(9, 4);

    conn.execute(
        "INSERT INTO turns_fts (rowid, body) VALUES (?1, ?2)",
        rusqlite::params![rowid, "twas brillig and the slithy toves"],
    )
    .unwrap();

    let hits: Vec<i64> = matching(conn, "brillig");
    assert_eq!(hits, vec![rowid]);

    conn.execute("DELETE FROM turns_fts WHERE rowid = ?1", [rowid])
        .unwrap();
    assert!(matching(conn, "brillig").is_empty());
}

/// Derived tables start empty; phase 1 owns the tables and the rebuild path,
/// not what fills them.
#[test]
fn derived_tables_start_empty() {
    let (_dir, store) = fresh();
    for table in schema::DERIVED_TABLES {
        let count: i64 = store
            .conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table} should start empty");
    }
}

/// A store carrying phase 1's shape: one fixture archived, then every column
/// phase 2 added dropped back off and `derived_schema` put back where a phase 1
/// binary left it. What a user's store looks like the moment they upgrade.
#[cfg(feature = "testkit")]
struct Aged {
    _dir: tempfile::TempDir,
    data_dir: std::path::PathBuf,
    session_key: String,
    blob: Vec<u8>,
    checksum: Vec<u8>,
}

#[cfg(feature = "testkit")]
fn aged_store() -> Aged {
    use verbatim_core::store::{DB_FILE_NAME, META_DERIVED_SCHEMA};

    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let path = verbatim_core::testkit::copy_fixture_into("session-basic.jsonl", &work);
    match verbatim_core::ingest::run(&data_dir, &path).unwrap() {
        verbatim_core::ingest::Outcome::Committed(_) => {}
        other => panic!("the fixture must archive: {other:?}"),
    }

    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    let (session_key, blob): (String, Vec<u8>) = conn
        .query_row("SELECT session_key, blob FROM sessions", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let checksum: Vec<u8> = conn
        .query_row("SELECT checksum FROM session_meta", [], |r| r.get(0))
        .unwrap();

    for (table, added) in schema::BRING_FORWARD_COLUMNS {
        for (name, _) in *added {
            conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {name}"))
                .unwrap_or_else(|e| panic!("drop {table}.{name}: {e}"));
        }
    }
    conn.execute(
        "UPDATE meta SET value = '1' WHERE key = ?1",
        [META_DERIVED_SCHEMA],
    )
    .unwrap();

    // The premise. Without this the two tests below pass on a store that was
    // never aged at all.
    for (table, added) in schema::BRING_FORWARD_COLUMNS {
        let present = columns(&conn, table);
        for (name, _) in *added {
            assert!(!present.contains(*name), "{table}.{name} survived the drop");
        }
    }
    drop(conn);

    Aged {
        _dir: dir,
        data_dir,
        session_key,
        blob,
        checksum,
    }
}

#[cfg(feature = "testkit")]
impl Aged {
    /// Every column phase 2 added is back, and the archive is byte-identical to
    /// what it held before the columns were dropped.
    fn assert_brought_forward(&self, conn: &Connection) {
        for (table, added) in schema::BRING_FORWARD_COLUMNS {
            let present = columns(conn, table);
            for (name, _) in *added {
                assert!(present.contains(*name), "{table}.{name} did not come back");
            }
        }

        let (key, blob): (String, Vec<u8>) = conn
            .query_row("SELECT session_key, blob FROM sessions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        let checksum: Vec<u8> = conn
            .query_row("SELECT checksum FROM session_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(key, self.session_key, "the archive lost its session key");
        assert!(blob == self.blob, "the bring-forward rewrote the blob");
        assert_eq!(
            checksum, self.checksum,
            "the bring-forward moved the checksum"
        );
    }
}

/// The ingest path's opener brings the columns back and rebuilds the derived
/// tables, and the archive comes through untouched.
#[cfg(feature = "testkit")]
#[test]
fn a_reindex_open_brings_every_phase_two_column_back() {
    use verbatim_core::store::{DERIVED_SCHEMA, META_DERIVED_SCHEMA};

    let aged = aged_store();
    let store = verbatim_core::reindex::open_up_to_date(&aged.data_dir)
        .expect("an aged store still opens for work");

    aged.assert_brought_forward(store.conn());
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA),
        "the rebuild did not stamp the store forward"
    );
}

/// The one that matters for `status` and `verify`: a plain `Store::open` - no
/// `reindex`, no ingest lock - still finds every phase 2 column, and fires no
/// derived rebuild while doing it. Gating the bring-forward on
/// `rebuild_required` would make a user who upgraded the binary and ran
/// `status` before any ingest hit `no such column` on a healthy store.
#[cfg(feature = "testkit")]
#[test]
fn a_plain_open_brings_the_columns_back_and_fires_no_rebuild() {
    use verbatim_core::store::META_DERIVED_SCHEMA;

    let aged = aged_store();
    let store = Store::open(&aged.data_dir).expect("an aged store opens read-side too");

    aged.assert_brought_forward(store.conn());
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(1),
        "a plain open performed the derived rebuild"
    );
    assert!(
        store.rebuild_required().is_some(),
        "the aged store must still be reported as needing a rebuild"
    );

    // The failure this exists to catch, in the shape it actually takes: a
    // SELECT naming every phase 2 column.
    store
        .conn()
        .query_row(
            "SELECT agent_meta, transcript_diverged, project_pre_worktree
             FROM session_meta LIMIT 1",
            [],
            |_| Ok(()),
        )
        .expect("session_meta is missing a phase 2 column");
    store
        .conn()
        .query_row(
            "SELECT files_committed, files_failed FROM runs LIMIT 1",
            [],
            |_| Ok(()),
        )
        .expect("runs is missing a phase 2 column");
}

fn columns(conn: &Connection, table: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn matching(conn: &Connection, query: &str) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT rowid FROM turns_fts WHERE turns_fts MATCH ?1 ORDER BY rowid")
        .unwrap();
    stmt.query_map([query], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// How many indexes of that name the store carries: 0 or 1.
fn index_count(conn: &Connection, name: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [name],
        |r| r.get(0),
    )
    .unwrap()
}

/// D-19: an index added in a later phase reaches an existing store because
/// `reindex` runs the whole `CREATE_SQL` batch, and not because `Store::open`
/// creates it.
///
/// `Store::open` runs `CREATE_SQL` only when a whole table is missing and
/// `BRING_FORWARD_COLUMNS` covers columns rather than indexes, so an index that
/// arrived with project-scoped search would otherwise exist on fresh stores and
/// silently not on upgraded ones - two machines running one binary at different
/// speeds, with nothing saying why.
#[test]
fn the_project_index_reaches_a_store_that_predates_it() {
    let (_dir, mut store) = fresh();
    assert_eq!(index_count(store.conn(), "idx_session_meta_project"), 1);

    // A store written before the index existed.
    store
        .conn()
        .execute_batch("DROP INDEX idx_session_meta_project")
        .unwrap();
    assert_eq!(index_count(store.conn(), "idx_session_meta_project"), 0);

    verbatim_core::reindex::reindex(&mut store).unwrap();
    assert_eq!(index_count(store.conn(), "idx_session_meta_project"), 1);
}
