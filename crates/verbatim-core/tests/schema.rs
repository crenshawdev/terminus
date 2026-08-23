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

/// D-01: `observations` reaches a store written before it existed through
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

/// D-02: the table is archival. A `DERIVED_TABLES` entry would make `reindex`
/// drop it, and the judgment half of a row is a paid model call.
#[test]
fn observations_is_not_a_derived_table() {
    assert!(
        !schema::DERIVED_TABLES.contains(&"observations"),
        "reindex would delete every purchased summary in the archive"
    );
    assert!(schema::TABLES.contains(&"observations"));
}

/// D-03: no column of `observations` may reference `turns`, which `reindex`
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

// --- Phase 8: session_meta.capture_mode (ING-07, D-13, D-19) -----------------

/// Every column of a table, in declaration order, as `(cid, name, type)`.
///
/// The ORDER is the assertion. `ALTER TABLE ADD COLUMN` appends, so a column
/// declared mid-table in `CREATE_SQL` gives a fresh store one order and an
/// upgraded store another, and any positional `r.get(n)` then reads a different
/// column depending on how old the store is.
#[cfg(feature = "testkit")]
fn column_layout(conn: &Connection, table: &str) -> Vec<(i64, String, String)> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// D-19's whole point, stated as an assertion over both stores at once: a store
/// written by an earlier binary and brought forward carries `session_meta` in
/// the same column order a fresh one does.
#[cfg(feature = "testkit")]
#[test]
fn a_brought_forward_session_meta_has_the_same_column_order_as_a_fresh_one() {
    let aged = aged_store();
    let brought = Store::open(&aged.data_dir).expect("an aged store opens");
    let (_dir, fresh_store) = fresh();

    let brought_layout = column_layout(brought.conn(), "session_meta");
    let fresh_layout = column_layout(fresh_store.conn(), "session_meta");
    assert_eq!(
        brought_layout, fresh_layout,
        "a brought-forward session_meta must be laid out exactly like a fresh one"
    );

    // The premise: the column this phase added is really there, and really last.
    let last = brought_layout.last().expect("session_meta has columns");
    assert_eq!(
        last.1, "capture_mode",
        "capture_mode must be the last column"
    );
    assert_eq!(last.2, "TEXT");

    // And the same for `runs`, which is the other table with a bring-forward
    // list - a regression here would be silent everywhere else.
    assert_eq!(
        column_layout(brought.conn(), "runs"),
        column_layout(fresh_store.conn(), "runs")
    );
}

/// The upgrade a phase 8 binary actually performs on a phase 7 store: ONE
/// column missing, `derived_schema` already current. It is added, the store is
/// not rebuilt, and nothing asks for a rebuild.
///
/// `aged_store` cannot answer this - it lowers `derived_schema` to 1, so a
/// rebuild is required there for a reason that has nothing to do with the
/// column. D-19's claim is that an added column costs no `DERIVED_SCHEMA` bump,
/// and this is the only shape that can falsify it.
#[cfg(feature = "testkit")]
#[test]
fn the_capture_mode_column_reaches_an_older_store_without_a_rebuild() {
    use verbatim_core::store::{DB_FILE_NAME, DERIVED_SCHEMA, META_DERIVED_SCHEMA};

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
    let turns_before: i64 = conn
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    let blob_before: Vec<u8> = conn
        .query_row("SELECT blob FROM sessions", [], |r| r.get(0))
        .unwrap();
    conn.execute_batch("ALTER TABLE session_meta DROP COLUMN capture_mode")
        .unwrap();
    assert!(
        !columns(&conn, "session_meta").contains("capture_mode"),
        "the premise: the column is gone"
    );
    drop(conn);

    let store = Store::open(&data_dir).expect("a store one column behind still opens");
    assert!(
        columns(store.conn(), "session_meta").contains("capture_mode"),
        "the column did not come back"
    );
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA),
        "an added column must not move derived_schema"
    );
    assert!(
        store.rebuild_required().is_none(),
        "an added column must not force a rebuild of the derived tables"
    );

    let turns_after: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    let blob_after: Vec<u8> = store
        .conn()
        .query_row("SELECT blob FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(turns_after, turns_before, "the derived tables were rebuilt");
    assert!(
        blob_after == blob_before,
        "the bring-forward touched the blob"
    );
}

/// A config directory holding one `verbatim.toml` naming a capture mode.
#[cfg(feature = "testkit")]
fn mode_config(mode: &str) -> (tempfile::TempDir, verbatim_core::Config) {
    use verbatim_core::config::CONFIG_FILE_NAME;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(CONFIG_FILE_NAME),
        format!("[capture]\nmode = \"{mode}\"\n"),
    )
    .unwrap();
    let config = verbatim_core::Config::load_from(dir.path()).expect("a capture table parses");
    (dir, config)
}

/// D-13's rule, both directions: `full` only while every append was full.
///
/// The asymmetry is the point. A `full` pass over a session already captured
/// lean must NOT move the column back, because `blob::append` copies completed
/// blocks across untouched - the elided bytes are gone from that blob and no
/// later pass revisits them.
#[cfg(feature = "testkit")]
#[test]
fn capture_mode_records_the_reduced_mode_whichever_pass_came_first() {
    use verbatim_core::store::DB_FILE_NAME;

    /// Ingest `fixture`, then append a second fixture's lines to the same file
    /// and ingest again, under the two named modes in order.
    fn stored_mode(first: &str, second: &str) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let path = verbatim_core::testkit::copy_fixture_into("session-basic.jsonl", &work);

        let (_c1, config) = mode_config(first);
        match verbatim_core::ingest::run_with(&data_dir, &path, &config).unwrap() {
            verbatim_core::ingest::Outcome::Committed(_) => {}
            other => panic!("the first pass must commit: {other:?}"),
        }

        // Real new bytes, so the second pass is an APPEND and not a no-op: a
        // session with nothing new never reaches the upsert at all.
        let mut line = verbatim_core::testkit::boundary_line();
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, &line).unwrap();
        drop(file);

        let (_c2, config) = mode_config(second);
        match verbatim_core::ingest::run_with(&data_dir, &path, &config).unwrap() {
            verbatim_core::ingest::Outcome::Committed(_) => {}
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
