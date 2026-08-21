//! The decision log: the table that survives a rebuild, and the drain that
//! fills it (FEED-01).
//!
//! `decisions` is the one derived-looking table that is not derived. A prompt's
//! decision is state that existed for a few milliseconds inside a hook - which
//! spellings it extracted, what the index answered, which thresholds were in
//! force - and no amount of replaying the session blobs can reconstruct it, so
//! `reindex` must give the rows back untouched (D-03).

#![cfg(feature = "testkit")]

use rusqlite::types::Value;
use rusqlite::Connection;
use verbatim_core::store::{Store, DB_FILE_NAME};
use verbatim_core::{ingest, reindex, testkit};

/// Every column of every `decisions` row, in id order: the whole of what a
/// rebuild has to hand back.
fn decision_rows(conn: &Connection) -> Vec<Vec<Value>> {
    let mut statement = conn.prepare("SELECT * FROM decisions ORDER BY id").unwrap();
    let columns = statement.column_count();
    let rows = statement
        .query_map([], |row| {
            (0..columns).map(|i| row.get::<_, Value>(i)).collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<Vec<Value>>>>()
        .unwrap();
    rows
}

/// One row with every column set, so a rebuild that dropped one column would be
/// as visible as a rebuild that dropped the table.
fn insert_decision(conn: &Connection) {
    conn.execute(
        "INSERT INTO decisions (
            session_id, ts, cwd, prompt, watermark_session_no, chars_injected,
            spellings, candidates, injected, suppressed, thresholds
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55",
            "2026-08-20T09:14:21.000Z",
            "/code/verbatim",
            "where did we settle the retry budget in docs/RETRY.md",
            7_i64,
            412_i64,
            r#"["/code/verbatim/docs/RETRY.md","docs/RETRY.md"]"#,
            r#"[{"turn_id":117440514,"relevance":17.36,"entity_score":7.86,"entity_count":1,"matched_on":[{"kind":"path","value":"docs/RETRY.md"}]}]"#,
            r#"[{"turn_id":117440514,"chars":412}]"#,
            r#"[{"turn_id":117440515,"reason":"already_injected"}]"#,
            r#"{"ranked":10,"entity_rank":3,"co_occurring":2,"max_turns":3}"#,
        ],
    )
    .unwrap();
}

/// D-03: a rebuild returns the decision log exactly as it was.
///
/// The comparison is every column of the row rather than a count of rows: a
/// `reindex` that recreated the table and lost the JSON payload would keep the
/// count and destroy the only record of what the injector decided.
#[test]
fn reindex_gives_back_every_logged_decision() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    // A real archived session, so the rebuild has blobs to work from and the
    // assertion is about a store that was rebuilt rather than one that was
    // empty.
    let path = testkit::copy_fixture_into("session-basic.jsonl", &work);
    match ingest::run(&data_dir, &path).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("the fixture must archive: {other:?}"),
    }

    let mut store = Store::open(&data_dir).unwrap();
    insert_decision(store.conn());
    let before = decision_rows(store.conn());
    assert_eq!(before.len(), 1, "the premise: one logged decision");

    let rebuilt = reindex::reindex(&mut store).expect("the rebuild runs");
    assert!(
        rebuilt.turns > 0,
        "the rebuild rebuilt nothing: {rebuilt:?}"
    );
    drop(store);

    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    assert_eq!(
        decision_rows(&conn),
        before,
        "the rebuild changed the decision log"
    );
}
