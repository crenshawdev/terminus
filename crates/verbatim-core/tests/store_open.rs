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

fn runs_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap()
}
