//! D-19's two assertions, made in CI rather than at a user's first ingest.
//!
//! Both are facts about the SQLite that `rusqlite`'s `bundled` feature compiles
//! in, and both are load-bearing for the derived tables. A build that quietly
//! resolves a different SQLite fails here.

use rusqlite::Connection;

/// `contentless_delete=1` needs SQLite 3.43 or newer.
#[test]
fn bundled_sqlite_is_at_least_3_43() {
    let version = rusqlite::version();
    let number = rusqlite::version_number();
    assert!(
        number >= 3_043_000,
        "bundled SQLite is {version} ({number}); contentless_delete=1 needs 3.43"
    );
}

/// FTS5 is compiled in, and the exact FTS5 shape D-10 requires works.
///
/// D-19 asserted this needed an `fts5` cargo feature on `rusqlite`. No such
/// feature exists; `libsqlite3-sys` builds the bundled amalgamation with
/// `-DSQLITE_ENABLE_FTS5`. This test is what proves the claim either way, which
/// is why D-19 called for it.
#[test]
fn fts5_supports_a_contentless_deletable_table() {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    conn.execute_batch(
        "CREATE VIRTUAL TABLE scratch USING fts5(body, content='', contentless_delete=1);",
    )
    .expect("FTS5 with contentless_delete=1");

    conn.execute(
        "INSERT INTO scratch (rowid, body) VALUES (?1, ?2)",
        rusqlite::params![7_i64, "brillig and the slithy toves"],
    )
    .expect("insert at an explicit rowid");

    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM scratch WHERE scratch MATCH 'brillig'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);

    // The delete half is the point: a contentless table normally cannot delete,
    // and a rebuild that cannot delete cannot be idempotent (D-10).
    conn.execute("DELETE FROM scratch WHERE rowid = 7", [])
        .expect("contentless_delete=1 permits DELETE");

    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM scratch WHERE scratch MATCH 'brillig'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 0);
}
