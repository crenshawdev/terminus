//! What a fresh store contains, and the two properties the rebuild path turns
//! on: a deterministic turn id and an FTS5 table that can delete.

use std::collections::BTreeSet;

use rusqlite::Connection;
use terminus_core::store::schema::{
    self, split_turn_id, turn_id, MAX_TURNS_PER_SESSION, TURN_SEQ_BITS,
};
use terminus_core::Store;

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

/// `turns_fts` must be contentless *and* deletable, or a rebuild cannot be
/// idempotent.
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

/// Two lineage signals are kept apart, and each lives on its own table.
#[test]
fn the_two_lineage_columns_exist_on_their_own_tables() {
    let (_dir, store) = fresh();
    assert!(columns(store.conn(), "turns").contains("parent_uuid"));
    assert!(columns(store.conn(), "session_meta").contains("continues_from"));

    // The point of keeping them apart is that neither replaces the other.
    assert!(!columns(store.conn(), "turns").contains("continues_from"));
    assert!(!columns(store.conn(), "session_meta").contains("parent_uuid"));
}

/// Turn coordinates address the uncompressed stream, so no column here may
/// name a compressed position.
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
    let expected: BTreeSet<String> = ["session_key", "session_no"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(cols, expected, "sessions keeps exactly its two columns");
    assert!(
        names(store.conn(), "table").contains("blocks"),
        "the block table exists"
    );
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

/// The FTS5 round trip a rebuild depends on, against the real table rather
/// than a scratch one: insert at an explicit rowid, match, delete, match again.
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

/// Derived tables start empty: opening a store creates them, and only ingest
/// or a rebuild fills them.
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

/// A store one `derived_schema` behind: one fixture archived, then
/// `derived_schema` put back below this build's.
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
    use terminus_core::store::{DB_FILE_NAME, META_DERIVED_SCHEMA};

    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let path = terminus_core::testkit::copy_fixture_into("session-basic.jsonl", &work);
    match terminus_core::ingest::run(&data_dir, &path).unwrap() {
        terminus_core::ingest::Outcome::Committed(_) => {}
        other => panic!("the fixture must archive: {other:?}"),
    }

    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    let session_key: String = conn
        .query_row("SELECT session_key FROM sessions", [], |r| r.get(0))
        .unwrap();
    let blob = terminus_core::testkit::stored(&conn, &session_key);
    let checksum: Vec<u8> = conn
        .query_row("SELECT checksum FROM session_meta", [], |r| r.get(0))
        .unwrap();

    conn.execute(
        "UPDATE meta SET value = '1' WHERE key = ?1",
        [META_DERIVED_SCHEMA],
    )
    .unwrap();
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
    /// The archive is byte-identical to what it held before the store was aged.
    fn assert_archive_untouched(&self, conn: &Connection) {
        let key: String = conn
            .query_row("SELECT session_key FROM sessions", [], |r| r.get(0))
            .unwrap();
        let blob = terminus_core::testkit::stored(conn, &key);
        let checksum: Vec<u8> = conn
            .query_row("SELECT checksum FROM session_meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(key, self.session_key, "the archive lost its session key");
        assert!(blob == self.blob, "the open rewrote the blob");
        assert_eq!(checksum, self.checksum, "the open moved the checksum");
    }
}

/// The ingest path's opener rebuilds the derived tables of an older store and
/// stamps it forward, and the archive comes through untouched.
#[cfg(feature = "testkit")]
#[test]
fn a_reindex_open_stamps_an_older_store_forward_and_keeps_its_archive() {
    use terminus_core::store::{DERIVED_SCHEMA, META_DERIVED_SCHEMA};

    let aged = aged_store();
    let store = terminus_core::reindex::open_up_to_date(&aged.data_dir)
        .expect("an aged store still opens for work");

    aged.assert_archive_untouched(store.conn());
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA),
        "the rebuild did not stamp the store forward"
    );
}

/// The writable openers that never rebuild - `compact`, `observations
/// regenerate` and the snapshot taken after a pass - open through `Store::open`
/// alone. On an older store that open rebuilds nothing and still reports the
/// rebuild as required, which is left to `reindex::up_to_date`.
#[cfg(feature = "testkit")]
#[test]
fn a_plain_open_of_an_older_store_fires_no_rebuild() {
    use terminus_core::store::META_DERIVED_SCHEMA;

    let aged = aged_store();
    let store = Store::open(&aged.data_dir).expect("an aged store opens");

    aged.assert_archive_untouched(store.conn());
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(1),
        "a plain open performed the derived rebuild"
    );
    assert!(
        store.rebuild_required().is_some(),
        "the aged store must still be reported as needing a rebuild"
    );
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

/// An index added in a later build reaches an existing store because
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

    terminus_core::reindex::reindex(&mut store).unwrap();
    assert_eq!(index_count(store.conn(), "idx_session_meta_project"), 1);
}

/// `observations` reaches a store written before it existed through
/// `bring_forward`'s missing-table arm, and costs no rebuild to get there.
///
/// The alternative was a `DERIVED_SCHEMA` bump, which forces a full blob replay
/// of the whole archive inside the ingest lock on the first hook-spawned pass
/// after an upgrade - roughly 49 s over the real corpus - for a table nothing
/// rebuilds anyway. So the assertion is two-part on purpose: the table is back
/// AND nothing was scheduled to be rebuilt.
#[test]
fn the_observations_table_reaches_a_store_that_predates_it() {
    let (dir, store) = fresh();
    assert!(names(store.conn(), "table").contains("observations"));

    // A store written by a build that did not declare the table.
    store
        .conn()
        .execute_batch("DROP TABLE observations")
        .unwrap();
    assert!(!names(store.conn(), "table").contains("observations"));
    drop(store);

    let reopened = Store::open(dir.path()).expect("an older store still opens");
    assert!(
        names(reopened.conn(), "table").contains("observations"),
        "the missing-table arm did not create `observations`"
    );
    assert!(
        reopened.rebuild_required().is_none(),
        "adding `observations` must not force a derived rebuild"
    );
}

/// `observations` is archival. A `DERIVED_TABLES` entry would make `reindex`
/// drop it, and an evicted session's row cannot be recomputed from its emptied
/// blob.
#[test]
fn observations_is_not_a_derived_table() {
    assert!(
        !schema::DERIVED_TABLES.contains(&"observations"),
        "reindex would delete observations an evicted blob cannot recompute"
    );
    assert!(schema::TABLES.contains(&"observations"));
}

/// No column of `observations` may reference `turns`, which `reindex`
/// drops - the bundled SQLite enforces foreign keys, so the first `reindex`
/// after the first observation would fail outright.
#[test]
fn observations_declares_no_reference_to_turns() {
    let (_dir, store) = fresh();
    let sql: String = store
        .conn()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'observations'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !sql.contains("REFERENCES turns"),
        "a declared reference to a dropped table wedges every later ingest: {sql}"
    );
    assert!(sql.contains("REFERENCES sessions"), "{sql}");
}

const SUMMARY_TABLES: [&str; 2] = ["project_summary", "session_summary"];

/// The summary tables reach a store written before they existed through the
/// same missing-table arm, with no rebuild scheduled.
#[test]
fn the_summary_tables_reach_a_store_that_predates_them() {
    let (dir, store) = fresh();
    for table in SUMMARY_TABLES {
        assert!(names(store.conn(), "table").contains(table));
        store
            .conn()
            .execute_batch(&format!("DROP TABLE {table}"))
            .unwrap();
    }
    drop(store);

    let reopened = Store::open(dir.path()).expect("an older store still opens");
    for table in SUMMARY_TABLES {
        assert!(
            names(reopened.conn(), "table").contains(table),
            "the missing-table arm did not create `{table}`"
        );
    }
    assert!(
        reopened.rebuild_required().is_none(),
        "adding the summary tables must not force a derived rebuild"
    );
}

/// The summary is kept, so `reindex` must not drop it. `session_summary`
/// references `sessions` and the store holds it to that, and `project_summary`
/// references nothing, because a pair is not a session.
#[test]
fn the_summary_tables_are_kept_and_session_summary_references_sessions() {
    let (_dir, store) = fresh();
    let sql = |table: &str| -> String {
        store
            .conn()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap()
    };
    for table in SUMMARY_TABLES {
        assert!(!schema::DERIVED_TABLES.contains(&table), "{table}");
        assert!(schema::TABLES.contains(&table), "{table}");
    }
    assert_eq!(terminus_core::DERIVED_SCHEMA, 6);
    let session = sql("session_summary");
    assert!(session.contains("REFERENCES sessions(session_key)"), "{session}");
    assert!(!session.contains("REFERENCES turns"), "{session}");
    let project = sql("project_summary");
    assert!(!project.contains("REFERENCES"), "{project}");

    let conn = store.conn();
    conn.execute("INSERT INTO sessions (session_key, session_no) VALUES ('k', 1)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO session_summary (session_key, has_typed_prompt, turn_count, prompts, edited_files)
         VALUES ('k', 0, 0, '[]', '[]')",
        [],
    )
    .unwrap();
    let refused = conn.execute("DELETE FROM sessions WHERE session_key = 'k'", []);
    assert!(refused.is_err(), "a session was deleted under its summary row");
    conn.execute("DELETE FROM session_summary WHERE session_key = 'k'", [])
        .unwrap();
    conn.execute("DELETE FROM sessions WHERE session_key = 'k'", [])
        .expect("a session with no summary row deletes");
}

// --- session_meta.capture_mode -----------------------------------------------

/// A config directory holding one `terminus.toml` naming a capture mode.
#[cfg(feature = "testkit")]
fn mode_config(mode: &str) -> (tempfile::TempDir, terminus_core::Config) {
    use terminus_core::config::CONFIG_FILE_NAME;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(CONFIG_FILE_NAME),
        format!("[capture]\nmode = \"{mode}\"\n"),
    )
    .unwrap();
    let config = terminus_core::Config::load_from(dir.path()).expect("a capture table parses");
    (dir, config)
}

/// The capture-mode rule, both directions: `full` only while every append was
/// full.
///
/// The asymmetry is the point. A `full` pass over a session already captured
/// lean must NOT move the column back, because `blob::append` copies completed
/// blocks across untouched - the elided bytes are gone from that blob and no
/// later pass revisits them.
#[cfg(feature = "testkit")]
#[test]
fn capture_mode_records_the_reduced_mode_whichever_pass_came_first() {
    use terminus_core::store::DB_FILE_NAME;

    /// Ingest `fixture`, then append a second fixture's lines to the same file
    /// and ingest again, under the two named modes in order.
    fn stored_mode(first: &str, second: &str) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let path = terminus_core::testkit::copy_fixture_into("session-basic.jsonl", &work);

        let (_c1, config) = mode_config(first);
        match terminus_core::ingest::run_with(&data_dir, &path, &config).unwrap() {
            terminus_core::ingest::Outcome::Committed(_) => {}
            other => panic!("the first pass must commit: {other:?}"),
        }

        // Real new bytes, so the second pass is an APPEND and not a no-op: a
        // session with nothing new never reaches the upsert at all.
        let mut line = terminus_core::testkit::boundary_line();
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, &line).unwrap();
        drop(file);

        let (_c2, config) = mode_config(second);
        match terminus_core::ingest::run_with(&data_dir, &path, &config).unwrap() {
            terminus_core::ingest::Outcome::Committed(_) => {}
            other => panic!("the second pass must commit: {other:?}"),
        }

        let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
        conn.query_row("SELECT capture_mode FROM session_meta", [], |r| r.get(0))
            .unwrap()
    }

    assert_eq!(stored_mode("full", "full").as_deref(), Some("full"));
    assert_eq!(
        stored_mode("full", "lean").as_deref(),
        Some("lean"),
        "a lean append to a full session makes the session lean"
    );
    assert_eq!(
        stored_mode("lean", "full").as_deref(),
        Some("lean"),
        "a full append must not un-lean a session whose earlier blocks are elided"
    );
    assert_eq!(
        stored_mode("minimal", "full").as_deref(),
        Some("minimal"),
        "the same, for the mode that elides unconditionally"
    );
}
