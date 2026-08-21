//! Opening the store, and the D-09 version gate that guards every open.

use std::sync::Mutex;

use rusqlite::Connection;
use verbatim_core::store::{
    ARCHIVE_FORMAT, DB_FILE_NAME, DERIVED_SCHEMA, META_ARCHIVE_FORMAT, META_DERIVED_SCHEMA,
};
use verbatim_core::{Error, Store};

/// `std::env::set_var` is process-global and the test harness is threaded, so
/// the one test that touches the environment holds this.
static ENV: Mutex<()> = Mutex::new(());

fn journal_mode(store: &Store) -> String {
    store
        .conn()
        .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
        .unwrap()
}

#[test]
fn fresh_store_opens_in_wal_with_both_version_integers() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).expect("fresh store opens");

    assert_eq!(journal_mode(&store).to_lowercase(), "wal");
    assert_eq!(
        store.meta_int(META_ARCHIVE_FORMAT).unwrap(),
        Some(ARCHIVE_FORMAT)
    );
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA)
    );
    assert_eq!(store.path(), dir.path().join(DB_FILE_NAME));
    assert!(store.path().is_file());
    assert!(store.rebuild_required().is_none());
}

#[test]
fn reopening_an_up_to_date_store_reports_no_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    let store = Store::open(dir.path()).expect("reopen");
    assert!(store.rebuild_required().is_none());
    assert_eq!(
        store.meta_int(META_ARCHIVE_FORMAT).unwrap(),
        Some(ARCHIVE_FORMAT)
    );
}

#[test]
fn data_dir_env_var_decides_where_the_store_lands() {
    let _guard = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();

    let previous = std::env::var_os("VERBATIM_DATA_DIR");
    std::env::set_var("VERBATIM_DATA_DIR", dir.path());
    let resolved = verbatim_core::data_dir().expect("VERBATIM_DATA_DIR resolves");
    match previous {
        Some(v) => std::env::set_var("VERBATIM_DATA_DIR", v),
        None => std::env::remove_var("VERBATIM_DATA_DIR"),
    }

    assert_eq!(resolved, dir.path());
    let store = Store::open(&resolved).unwrap();
    assert_eq!(store.path(), dir.path().join(DB_FILE_NAME));
    assert!(dir.path().join(DB_FILE_NAME).is_file());
}

/// AC5. A store one archive format ahead of this build is refused, the message
/// names both versions, and nothing on disk moves.
///
/// The store under test is a *copy* taken while another connection held the
/// WAL open, so it carries a hot, non-empty WAL with no live connection - the
/// state a store is in after a crash, and the only state in which a refused
/// open can quietly rewrite anything. Without that setup the assertion is
/// vacuous: it passes even when the gate opens the database read-write, because
/// there is nothing for the close-time checkpoint to flush.
#[test]
fn a_newer_archive_format_is_refused_without_touching_the_store() {
    let source = tempfile::tempdir().unwrap();
    let db = source.path().join(DB_FILE_NAME);
    drop(Store::open(source.path()).unwrap());

    // Held for the whole setup so the WAL survives the writer's close.
    let keeper = Connection::open(&db).unwrap();
    keeper.pragma_update(None, "journal_mode", "wal").unwrap();

    {
        let writer = Connection::open(&db).unwrap();
        writer
            .execute(
                "INSERT INTO runs (started_at, files_seen) VALUES ('2026-08-12T00:00:00Z', 1)",
                [],
            )
            .unwrap();
        writer
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = ?2",
                rusqlite::params![(ARCHIVE_FORMAT + 1).to_string(), META_ARCHIVE_FORMAT],
            )
            .unwrap();
    }
    assert_eq!(runs_count(&keeper), 1);

    let wal_name = format!("{DB_FILE_NAME}-wal");
    let source_wal = source.path().join(&wal_name);
    assert!(
        std::fs::metadata(&source_wal).map(|m| m.len()).unwrap_or(0) > 0,
        "this test needs a non-empty WAL to mean anything"
    );

    // Copy database and WAL out from under the live connection; the -shm file
    // is deliberately not copied, SQLite rebuilds it.
    let target = tempfile::tempdir().unwrap();
    let target_db = target.path().join(DB_FILE_NAME);
    let target_wal = target.path().join(&wal_name);
    std::fs::copy(&db, &target_db).unwrap();
    std::fs::copy(&source_wal, &target_wal).unwrap();
    drop(keeper);

    let before_db = std::fs::read(&target_db).unwrap();
    let before_wal = std::fs::read(&target_wal).unwrap();

    let err = Store::open(target.path()).expect_err("a newer archive format must be refused");
    let message = err.to_string();
    assert!(
        message.contains(&(ARCHIVE_FORMAT + 1).to_string()),
        "message must name the store's format: {message}"
    );
    assert!(
        message.contains(&ARCHIVE_FORMAT.to_string()),
        "message must name the binary's format: {message}"
    );
    assert!(
        matches!(err, Error::ArchiveFormatTooNew { .. }),
        "wrong error variant: {err:?}"
    );

    // Byte equality, not a digest of it: this is the whole of AC5's "unchanged".
    assert!(
        std::fs::read(&target_db).unwrap() == before_db,
        "verbatim.db changed on a refused open"
    );
    assert!(
        std::fs::read(&target_wal).unwrap() == before_wal,
        "the WAL changed on a refused open"
    );

    // Read `runs` only now: any connection we open here may checkpoint on close
    // and would spoil the byte comparison above.
    let after = Connection::open(&target_db).unwrap();
    assert_eq!(runs_count(&after), 1, "runs changed on a refused open");
}

/// STOR-05's other half: an older store opens, and says so, without rebuilding
/// anything here. The rebuild itself is PLAN-2's.
#[test]
fn an_older_derived_schema_opens_with_a_rebuild_outcome() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());

    {
        let conn = Connection::open(dir.path().join(DB_FILE_NAME)).unwrap();
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            rusqlite::params![(DERIVED_SCHEMA - 1).to_string(), META_DERIVED_SCHEMA],
        )
        .unwrap();
    }

    let store = Store::open(dir.path()).expect("an older store still opens");
    let rebuild = store
        .rebuild_required()
        .expect("an older derived_schema must report a rebuild");
    assert_eq!(rebuild.store_derived_schema, DERIVED_SCHEMA - 1);
    assert_eq!(rebuild.binary_derived_schema, DERIVED_SCHEMA);
    assert_eq!(rebuild.store_archive_format, ARCHIVE_FORMAT);

    // Opening did not silently perform the rebuild, and did not bump meta.
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA - 1)
    );
}

#[test]
fn an_older_archive_format_also_reports_a_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    {
        let conn = Connection::open(dir.path().join(DB_FILE_NAME)).unwrap();
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            rusqlite::params![(ARCHIVE_FORMAT - 1).to_string(), META_ARCHIVE_FORMAT],
        )
        .unwrap();
    }
    let store = Store::open(dir.path()).expect("an older store still opens");
    let rebuild = store.rebuild_required().expect("rebuild outcome");
    assert_eq!(rebuild.store_archive_format, ARCHIVE_FORMAT - 1);
    assert_eq!(rebuild.binary_archive_format, ARCHIVE_FORMAT);
}

#[test]
fn a_file_that_is_not_a_store_is_named_as_such() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(DB_FILE_NAME), b"not a database at all").unwrap();
    let err = Store::open(dir.path()).expect_err("a non-store must not be opened");
    assert!(matches!(err, Error::NotAStore { .. }), "{err:?}");
}

/// D-10's first half: a read never creates the store it did not find.
///
/// Asserted on the filesystem rather than on the error, because the failure
/// this prevents - `verbatim search` on a machine that has never ingested
/// leaving a data directory behind - is a file that exists, not a message.
#[test]
fn a_read_only_open_of_a_missing_store_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("never-ingested");

    let err = Store::open_read_only(&absent).expect_err("there is no store to open");
    assert!(matches!(err, Error::StoreNotFound { .. }), "{err:?}");
    assert!(!absent.exists(), "the read created the data directory");

    // The other half: the directory is there and the database is not.
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let err = Store::open_read_only(&empty).expect_err("an empty data directory holds no store");
    assert!(matches!(err, Error::StoreNotFound { .. }), "{err:?}");
    assert_eq!(
        std::fs::read_dir(&empty).unwrap().count(),
        0,
        "the read wrote something into an empty data directory"
    );
}

/// D-10's second half: the connection cannot write, whatever the caller asks of
/// it. `readOnlyHint` is a claim about this flag and nothing else.
#[test]
fn a_read_only_connection_refuses_every_write() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());

    let store = Store::open_read_only(dir.path()).expect("an existing store opens for reading");
    assert!(store.rebuild_required().is_none());
    assert!(store.missing_columns().is_empty());
    assert!(!store.predates_this_build());

    for statement in [
        "INSERT INTO runs (started_at) VALUES ('2026-08-13T00:00:00Z')",
        "CREATE TABLE scratch (x INTEGER)",
        "ALTER TABLE session_meta ADD COLUMN invented TEXT",
        "DELETE FROM meta",
    ] {
        let err = store
            .conn()
            .execute_batch(statement)
            .expect_err("a read-only connection accepted a write");
        assert!(
            err.to_string().contains("readonly"),
            "unexpected refusal for `{statement}`: {err}"
        );
    }
}

/// D-18: a store written before this build opens, reports itself, and is left
/// exactly as it was - no bring-forward, no pragma, no byte of the store.
///
/// **What "untouched" can mean for a WAL database.** Measured here: a read-only
/// connection to a WAL database materializes `verbatim.db-shm` (32 KB) and an
/// empty `verbatim.db-wal` whenever the directory allows it, because the
/// shared-memory index is how WAL readers find the snapshot at all. That is
/// SQLite's, not ours - the same mechanism this constructor's
/// [`Error::StoreUnreadable`] exists to report when the medium will *not* allow
/// it. So the assertion is the one that carries the meaning: `verbatim.db` is
/// byte-identical, every file that already existed is byte-identical, and the
/// only additions are those two, with the WAL empty because nothing was
/// committed through it.
#[cfg(feature = "testkit")]
#[test]
fn a_read_only_open_of_an_older_store_reports_it_and_writes_nothing() {
    use verbatim_core::store::schema::BRING_FORWARD_COLUMNS;

    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let path = verbatim_core::testkit::copy_fixture_into("session-basic.jsonl", &work);
    match verbatim_core::ingest::run(&data_dir, &path).unwrap() {
        verbatim_core::ingest::Outcome::Committed(_) => {}
        other => panic!("the fixture must archive: {other:?}"),
    }

    // Age it the way `tests/schema.rs` does: phase 2's columns back off, the
    // derived-schema integer back where a phase 1 binary left it.
    {
        let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
        for (table, added) in BRING_FORWARD_COLUMNS {
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
    }

    let before = directory_bytes(&data_dir);
    assert!(
        before.iter().any(|(name, _)| name == DB_FILE_NAME),
        "the aged store must exist to be compared"
    );

    let store = Store::open_read_only(&data_dir).expect("an aged store still opens for reading");
    let rebuild = store
        .rebuild_required()
        .expect("an older derived_schema must be reported");
    assert_eq!(rebuild.store_derived_schema, 1);
    assert_eq!(rebuild.binary_derived_schema, DERIVED_SCHEMA);

    // The columns a read query would have named. Reported, not added.
    let missing: Vec<(&str, &str)> = store
        .missing_columns()
        .iter()
        .map(|c| (c.table, c.column))
        .collect();
    for (table, added) in BRING_FORWARD_COLUMNS {
        for (name, _) in *added {
            assert!(
                missing.contains(&(*table, *name)),
                "{table}.{name} is gone and was not reported"
            );
        }
    }
    assert!(store.predates_this_build());

    // A query naming one of them still fails - the point is that the caller was
    // told, not that SQLite was made tolerant.
    assert!(store
        .conn()
        .query_row("SELECT project_pre_worktree FROM session_meta", [], |_| Ok(
            ()
        ))
        .is_err());

    drop(store);
    let after = directory_bytes(&data_dir);

    for (name, bytes) in &before {
        let found = after
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("the read-only open removed {name}"));
        assert!(found.1 == *bytes, "the read-only open rewrote {name}");
    }
    for (name, bytes) in &after {
        if before.iter().any(|(n, _)| n == name) {
            continue;
        }
        assert!(
            name == "verbatim.db-shm" || name == "verbatim.db-wal",
            "the read-only open created {name}"
        );
        assert!(
            name != "verbatim.db-wal" || bytes.is_empty(),
            "the read-only open committed {} bytes through the WAL",
            bytes.len()
        );
    }
}

/// Every file in `dir`, by name, with its bytes. Sorted, so two snapshots
/// compare directly.
#[cfg(feature = "testkit")]
fn directory_bytes(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap_or_default(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// D-02: phase 6's two tables reach a store a phase-5 binary initialized, on the
/// next ordinary `Store::open`, and nothing is rebuilt to get them there.
///
/// The store under test is aged by dropping exactly the two tables, which is
/// what a store written before they existed looks like. The assertion that
/// carries the phase is the one about the rest of it: `bring_forward` runs
/// `CREATE_SQL`, every statement in it is `IF NOT EXISTS`, and the turn rows the
/// old binary derived are still there afterwards - so an upgraded machine gains
/// a decision log without paying the measured ~50 s in-lock rebuild that a
/// `DERIVED_SCHEMA` bump would have forced on its first hook-spawned pass.
#[cfg(feature = "testkit")]
#[test]
fn a_store_written_before_phase_six_gains_decisions_without_a_reindex() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let path = verbatim_core::testkit::copy_fixture_into("session-basic.jsonl", &work);
    match verbatim_core::ingest::run(&data_dir, &path).unwrap() {
        verbatim_core::ingest::Outcome::Committed(_) => {}
        other => panic!("the fixture must archive: {other:?}"),
    }

    let turns_before = {
        let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
        let turns = table_count(&conn, "turns");
        assert!(turns > 0, "the fixture archived no turns");
        conn.execute_batch("DROP TABLE labels; DROP TABLE decisions;")
            .expect("the tables this test ages away must exist to be dropped");
        turns
    };

    let store = Store::open(&data_dir).expect("an aged store still opens");
    assert!(
        store.rebuild_required().is_none(),
        "a missing table is not a version mismatch, and must not ask for a rebuild"
    );
    assert_eq!(table_count(store.conn(), "decisions"), 0);
    assert_eq!(table_count(store.conn(), "labels"), 0);
    assert_eq!(
        table_count(store.conn(), "turns"),
        turns_before,
        "the open rebuilt the derived tables instead of adding the missing ones"
    );
}

/// `SELECT count(*)`, which is also the assertion that the table is there at
/// all: a missing one is an `Err` rather than a zero.
#[cfg(feature = "testkit")]
fn table_count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("counting {table}: {e}"))
}

fn runs_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap()
}

/// `open` sets `journal_mode=wal` on the writable connection before
/// `initialize` runs, and that pragma alone stamps the database header and
/// grows a brand-new file to 4096 bytes. So an interruption anywhere inside
/// `initialize` - a crash, a failed commit, a full disk - leaves a non-empty
/// file holding no tables. Deciding "is this an existing store" by file length
/// calls that a store, sends it to the gate, and returns `NotAStore` from then
/// on: the data directory is bricked with no repair path, for a database that
/// contains nothing. Presence of tables is the test instead, so this reopens
/// and initializes.
#[test]
fn an_initialize_that_never_committed_reopens_instead_of_bricking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DB_FILE_NAME);

    // Exactly what `open` does up to the point of no return, then rolled back.
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "journal_mode", "wal").unwrap();
        conn.pragma_update(None, "synchronous", "normal").unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute_batch("CREATE TABLE scratch (x INTEGER)")
            .unwrap();
        tx.rollback().unwrap();
    }

    // The premise: the file is not empty, and it holds nothing.
    assert!(
        std::fs::metadata(&path).unwrap().len() > 0,
        "the WAL pragma did not grow the file, so this test proves nothing"
    );

    let store = Store::open(dir.path()).expect("a database with no tables must re-initialize");
    assert_eq!(
        store.meta_int(META_ARCHIVE_FORMAT).unwrap(),
        Some(ARCHIVE_FORMAT)
    );
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA)
    );
}

/// The other side of the same decision: a SQLite database that holds tables but
/// none of ours is somebody else's file. Initializing over it would destroy
/// data, so it is refused. `initialize` is only ever reached with nothing to
/// lose.
#[test]
fn a_foreign_sqlite_database_is_refused_rather_than_initialized_over() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DB_FILE_NAME);
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE somebody_elses (x INTEGER); INSERT INTO somebody_elses VALUES (1)",
        )
        .unwrap();
    }

    let err = Store::open(dir.path()).expect_err("a foreign database must not be opened");
    assert!(matches!(err, Error::NotAStore { .. }), "{err:?}");

    let conn = Connection::open(&path).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM somebody_elses", [], |r| r.get(0))
        .expect("the refused open destroyed the foreign table");
    assert_eq!(rows, 1, "the refused open wrote over somebody else's data");
}
