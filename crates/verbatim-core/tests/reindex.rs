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
use verbatim_core::{ingest, reindex, testkit, verify};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    /// Where the transcripts were copied to. Removable, which is how a test
    /// makes "the rebuild read no transcript" a fact about the filesystem
    /// rather than an inference about the code.
    work: PathBuf,
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
        work,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn store(&self) -> Store {
        Store::open(&self.data_dir).unwrap()
    }

    /// Drop every derived table outright, as AC4 specifies.
    ///
    /// Children first, so in reverse: `DERIVED_TABLES` is in creation order and
    /// the bundled SQLite enforces foreign keys, so dropping `turns` while
    /// `compaction_boundaries` still holds a row for one of its ids fails with a
    /// constraint violation. `reindex` itself drops in this order for the same
    /// reason.
    fn drop_derived(&self) {
        let conn = self.conn();
        for table in schema::DERIVED_TABLES.iter().rev() {
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

    // And the same premise for the sections phase 3 added. Two empty sections
    // compare byte-identically across a rebuild that extracted nothing at all,
    // which is exactly the extractor failure AC3 is meant to catch.
    let section = |name: &str| {
        let body = before
            .split_once(&format!("\"{name}\": [\n"))
            .unwrap_or_else(|| panic!("no `{name}` section: {before}"))
            .1;
        body.split_once("\n  ]").expect("an unterminated section").0
    };
    for name in ["entities", "paths"] {
        let rows = section(name).lines().count();
        assert!(rows > 0, "the `{name}` section is empty");
    }
    // Every kind, so the comparison covers each extraction rule and not just
    // the one that fires most often.
    for kind in verbatim_core::index::KINDS {
        assert!(
            section("entities").contains(&format!("\"{kind}\"")),
            "no `{kind}` entity in the query set"
        );
    }

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

    // The invented rows are counted BEFORE the rebuild, so "they are gone" is a
    // claim about these rows and not about the table being empty - which it no
    // longer is, since phase 3 fills `paths` from the blobs like everything
    // else.
    let invented: i64 = bench
        .conn()
        .query_row(
            "SELECT count(*) FROM paths WHERE path = '/invented'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(invented > 0, "the corruption did not take");

    let mut store = bench.store();
    reindex::reindex(&mut store).unwrap();
    drop(store);

    assert_eq!(testkit::query_set_json(&bench.conn()), reference);
    assert_eq!(
        bench
            .conn()
            .query_row(
                "SELECT count(*) FROM paths WHERE path = '/invented'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0,
        "an invented row survived"
    );
    assert!(
        count(&bench.conn(), "paths") > 0,
        "the rebuild produced no path row at all, so the comparison above is empty"
    );
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
        // Every child row goes first: `compaction_boundaries`, `entities` and
        // `paths` all reference `turns(id)` and the bundled SQLite enforces it.
        for table in ["compaction_boundaries", "entities", "paths"] {
            conn.execute(&format!("DELETE FROM {table}"), []).unwrap();
        }
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
        rebuilt.preserved, 0,
        "a store with no evicted session must take the drop-and-recreate path"
    );
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

/// D-02, as the failure it prevents: a `reindex` must leave every observation
/// row exactly where it was.
///
/// The judgment half of one of these rows is a paid model call that nothing in
/// a blob reproduces, so a `reindex` that dropped the table would silently
/// delete summaries a user bought - and `open_up_to_date` runs on the ingest
/// path, so it would happen unattended on the first pass after an upgrade.
///
/// The comparison is the column VALUES and not a row count: a rebuild that
/// recreated the table empty and a rebuild that rewrote `mechanical` are both
/// failures, and a count cannot tell either of them from success.
#[test]
fn a_reindex_leaves_every_observation_row_untouched() {
    let bench = bench();
    let key: String = bench
        .conn()
        .query_row(
            "SELECT session_key FROM sessions ORDER BY session_no",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let row = |conn: &Connection| -> Vec<Option<String>> {
        conn.query_row(
            "SELECT o.session_key, o.session_id, o.generated_at, o.mechanical,
                    o.status, o.model, o.prompt_version, o.topic, o.outcome,
                    o.decisions, o.learned, o.unresolved, o.raw,
                    CAST(o.tokens AS TEXT)
               FROM observations o WHERE o.session_key = ?1",
            [&key],
            |r| (0..14).map(|i| r.get(i)).collect(),
        )
        .expect("the observation row must still be there")
    };

    bench
        .conn()
        .execute(
            "INSERT INTO observations (
                session_key, session_id, generated_at, mechanical, status, model,
                prompt_version, topic, outcome, decisions, learned, unresolved,
                raw, tokens
             ) VALUES (?1, 'sess-1', '2026-08-21T00:00:00.000Z', '{\"turns\":3}',
                       'ok', 'qwen3:8b', 'v1', 'the topic', 'the outcome',
                       '[{\"turn_id\":7,\"claim\":\"a\"}]', '[]', '[]', NULL, 394)",
            [&key],
        )
        .unwrap();
    let before = row(&bench.conn());

    let mut store = bench.store();
    reindex::reindex(&mut store).unwrap();
    drop(store);

    assert_eq!(
        row(&bench.conn()),
        before,
        "the rebuild moved an observation"
    );
    assert_eq!(count(&bench.conn(), "observations"), 1);
}

// ---------------------------------------------------------------------------
// Phase 8 D-03: the one case where the blob cannot answer.
//
// An evicted session has no blob and its `turns_fts` row cannot be
// reconstructed by any other means - the table is `content=''`, so the
// projected body is not readable back out of it, and the record bytes
// `index::project` built it from are gone. Dropping the table destroys them
// with nothing anywhere able to reproduce them, and `open_up_to_date` runs at
// the top of every pass, so it would happen unattended inside the ingest lock.
// ---------------------------------------------------------------------------

/// How many derived rows one session owns, per table.
fn derived_rows(conn: &Connection, key: &str) -> Vec<(&'static str, i64)> {
    let count = |sql: &str| -> i64 { conn.query_row(sql, [key], |r| r.get(0)).unwrap() };
    vec![
        (
            "turns",
            count("SELECT count(*) FROM turns WHERE session_key = ?1"),
        ),
        (
            "entities",
            count(
                "SELECT count(*) FROM entities WHERE turn_id IN
                 (SELECT id FROM turns WHERE session_key = ?1)",
            ),
        ),
        (
            "paths",
            count(
                "SELECT count(*) FROM paths WHERE turn_id IN
                 (SELECT id FROM turns WHERE session_key = ?1)",
            ),
        ),
        (
            "turns_fts",
            count(
                "SELECT count(*) FROM turns_fts WHERE rowid IN
                 (SELECT id FROM turns WHERE session_key = ?1)",
            ),
        ),
    ]
}

/// Every turn id one session owns, in order. Ids and not counts: a rebuild that
/// renumbered would return the same number of rows pointing at other records.
fn turn_ids(conn: &Connection, key: &str) -> Vec<i64> {
    conn.prepare("SELECT id FROM turns WHERE session_key = ?1 ORDER BY id")
        .unwrap()
        .query_map([key], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The turn ids one FTS query returns for one session.
fn fts_hits(conn: &Connection, key: &str, query: &str) -> Vec<i64> {
    conn.prepare(
        "SELECT f.rowid FROM turns_fts f JOIN turns t ON t.id = f.rowid
         WHERE turns_fts MATCH ?2 AND t.session_key = ?1 ORDER BY f.rowid",
    )
    .unwrap()
    .query_map(rusqlite::params![key, query], |r| r.get(0))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

impl Bench {
    /// The session key of an ingested fixture, by the file name it landed under.
    fn key_ending(&self, suffix: &str) -> String {
        self.conn()
            .query_row(
                "SELECT session_key FROM sessions WHERE session_key LIKE '%' || ?1",
                [suffix],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("no session ending in {suffix}: {e}"))
    }

    /// Empty a session's blob and mark it, exactly as `retention::apply` does.
    fn evict(&self, key: &str) {
        let conn = self.conn();
        conn.execute(
            "UPDATE sessions SET blob = x'' WHERE session_key = ?1",
            [key],
        )
        .unwrap();
        let changed = conn
            .execute(
                "UPDATE session_meta SET is_evicted = 1 WHERE session_key = ?1",
                [key],
            )
            .unwrap();
        assert_eq!(changed, 1, "no session_meta row for {key}");
    }
}

/// The evicted sessions' rows are carried across, every other session is
/// rebuilt from its blob exactly as before, and the whole fixed query set comes
/// back byte-identical.
///
/// Two sessions are evicted rather than one: the fixture carrying
/// `FIXED_QUERIES`' own token is what makes the FTS claim concrete, and the one
/// holding `paths` rows is what makes the claim cover all four tables.
#[test]
fn a_reindex_carries_an_evicted_session_across_rather_than_losing_its_index() {
    let bench = bench();
    let before = testkit::query_set_json(&bench.conn());

    let basic = bench.key_ending("session-basic.jsonl");
    let with_paths: String = bench
        .conn()
        .query_row(
            "SELECT t.session_key FROM paths p JOIN turns t ON t.id = p.turn_id
             GROUP BY t.session_key ORDER BY count(*) DESC, t.session_key LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("the corpus has to produce a path row somewhere");
    let mut evicted: Vec<String> = vec![basic.clone(), with_paths.clone()];
    evicted.sort();
    evicted.dedup();

    // The premise, stated per table: there is something to lose in each of them.
    let rows_before: Vec<(String, Vec<(&str, i64)>)> = evicted
        .iter()
        .map(|key| (key.clone(), derived_rows(&bench.conn(), key)))
        .collect();
    let ids_before: Vec<Vec<i64>> = evicted
        .iter()
        .map(|key| turn_ids(&bench.conn(), key))
        .collect();
    let brillig_before = fts_hits(&bench.conn(), &basic, testkit::UNIQUE_TOKEN);
    assert!(
        !brillig_before.is_empty(),
        "the evicted session has to be searchable to start with"
    );
    for (table, count) in derived_rows(&bench.conn(), &with_paths) {
        assert!(count > 0, "{table} has nothing to preserve");
    }

    for key in &evicted {
        bench.evict(key);
    }
    // Taken AFTER the eviction: emptying a blob is a change to the archive and
    // this comparison is about what the REBUILD does to it.
    let archive_before = testkit::archive_digest(&bench.conn());

    let mut store = bench.store();
    let rebuilt = reindex::reindex(&mut store).unwrap();
    drop(store);

    assert_eq!(rebuilt.preserved, evicted.len());
    assert_eq!(
        rebuilt.sessions,
        testkit::TRANSCRIPT_FIXTURES.len() - evicted.len(),
        "an evicted session must not be counted as one this build rebuilt"
    );
    assert!(
        rebuilt.failed.is_empty(),
        "an emptied blob is not a damaged one: {:?}",
        rebuilt.failed
    );

    // (a): the evicted sessions' own rows, table by table and id by id.
    for (index, key) in evicted.iter().enumerate() {
        assert_eq!(
            derived_rows(&bench.conn(), key),
            rows_before[index].1,
            "a derived row of the evicted session {key} was lost"
        );
        assert_eq!(turn_ids(&bench.conn(), key), ids_before[index]);
    }
    assert_eq!(
        fts_hits(&bench.conn(), &basic, testkit::UNIQUE_TOKEN),
        brillig_before,
        "a search that matched the evicted session's turn no longer does"
    );

    // (b): and every other session is exactly what it was, which the fixed
    // query set says across the whole store at once.
    assert_eq!(testkit::query_set_json(&bench.conn()), before);
    assert_eq!(
        testkit::archive_digest(&bench.conn()),
        archive_before,
        "the rebuild touched the archive"
    );
}

/// The preserving path is still a rebuild: it drops nothing it must not, and it
/// still reads every other blob. A corrupt session beside an evicted one lands
/// on `failed` and the evicted one does not, which is the distinction the two
/// counts exist for.
#[test]
fn the_preserving_path_still_names_a_corrupt_blob_and_does_not_confuse_the_two() {
    let bench = bench();
    let evicted = bench.key_ending("session-basic.jsonl");
    let corrupt = bench.key_ending("session-recall.jsonl");
    let preserved_ids = turn_ids(&bench.conn(), &evicted);
    assert!(!preserved_ids.is_empty());

    bench.evict(&evicted);
    {
        let conn = bench.conn();
        let mut blob: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [&corrupt],
                |r| r.get(0),
            )
            .unwrap();
        for byte in blob.iter_mut().skip(60).take(150) {
            *byte = 0;
        }
        conn.execute(
            "UPDATE sessions SET blob = ?1 WHERE session_key = ?2",
            rusqlite::params![blob, &corrupt],
        )
        .unwrap();
    }

    let mut store = bench.store();
    let rebuilt = reindex::reindex(&mut store).unwrap();
    drop(store);

    assert_eq!(rebuilt.preserved, 1);
    assert_eq!(rebuilt.failed.len(), 1, "{:?}", rebuilt.failed);
    assert_eq!(rebuilt.failed[0].0, corrupt);
    assert_eq!(
        turn_ids(&bench.conn(), &evicted),
        preserved_ids,
        "the evicted session lost its index on the path that preserves it"
    );
    assert_eq!(
        turn_ids(&bench.conn(), &corrupt),
        Vec::<i64>::new(),
        "a blob that will not decompress cannot have rebuilt anything"
    );
    // And every session that was neither is back.
    assert!(count(&bench.conn(), "turns") > 0);
    assert_eq!(
        count(&bench.conn(), "sessions") as usize,
        testkit::TRANSCRIPT_FIXTURES.len(),
        "the rebuild removed a session row"
    );
}

/// The preserving path is idempotent too: running it twice changes nothing, so
/// a store that acquires an evicted session does not drift a little on every
/// upgrade.
#[test]
fn rebuilding_twice_with_an_evicted_session_changes_nothing() {
    let bench = bench();
    bench.evict(&bench.key_ending("session-basic.jsonl"));

    let mut store = bench.store();
    reindex::reindex(&mut store).unwrap();
    drop(store);
    let after_one = testkit::query_set_json(&bench.conn());

    let mut store = bench.store();
    let twice = reindex::reindex(&mut store).unwrap();
    drop(store);

    assert_eq!(twice.preserved, 1);
    assert_eq!(testkit::query_set_json(&bench.conn()), after_one);
}

/// Does this table carry this column?
fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .any(|name| name == column)
}

fn count_where(conn: &Connection, predicate: &str) -> i64 {
    conn.query_row(
        &format!("SELECT count(*) FROM turns WHERE {predicate}"),
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// AC5, end to end: a store the PREVIOUS build wrote - no `turns.is_typed`, an
/// older `derived_schema` - opens, acquires the column, is filled from the
/// blobs with no transcript on disk to read, and then keeps working.
///
/// The evicted session is what makes this more than a schema test. Its rows
/// send `reindex` down the preserving path, which skips the DROP loop and runs
/// only `CREATE TABLE IF NOT EXISTS` - a no-op on a table that already exists -
/// so nothing there would create the column and `derive_turn`'s explicit INSERT
/// list would fail with `no such column` on the rebuild itself and on every
/// pass afterwards, permanently, since `open_up_to_date` runs at the top of
/// each one. The `turns` entry in `BRING_FORWARD_COLUMNS` is the only thing
/// standing between this store and that (v0.1.1 phase 1 D-08); delete it and
/// this test fails with exactly that message.
#[test]
fn a_previous_builds_store_gains_is_typed_and_is_filled_from_the_blobs() {
    let bench = bench();
    let evicted = bench.key_ending("session-basic.jsonl");
    bench.evict(&evicted);

    // The premise for the untouched claim below: there is something to lose.
    let rows_before = derived_rows(&bench.conn(), &evicted);
    let ids_before = turn_ids(&bench.conn(), &evicted);
    // `turns` and its FTS row, which is what this test is about; the
    // all-four-tables claim is
    // `a_reindex_carries_an_evicted_session_across_rather_than_losing_its_index`'s,
    // and it picks a different fixture for it because `session-basic.jsonl`
    // produces no `paths` row.
    for table in ["turns", "turns_fts"] {
        let count = rows_before
            .iter()
            .find(|(name, _)| *name == table)
            .map(|(_, count)| *count)
            .unwrap_or_else(|| panic!("no {table} count"));
        assert!(count > 0, "{table} has nothing to preserve");
    }
    let evicted_users = count_where(
        &bench.conn(),
        &format!("record_type = 'user' AND session_key = '{evicted}'"),
    );
    assert!(
        evicted_users > 0,
        "the evicted session must hold a user turn for its nulls to mean anything"
    );

    // Age it to what the previous build wrote.
    {
        let conn = bench.conn();
        conn.execute_batch("ALTER TABLE turns DROP COLUMN is_typed")
            .unwrap();
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            rusqlite::params![(DERIVED_SCHEMA - 1).to_string(), META_DERIVED_SCHEMA],
        )
        .unwrap();
        assert!(
            !has_column(&conn, "turns", "is_typed"),
            "the premise: the column has to be gone"
        );
    }

    // AC5's "with no transcript read", as a fact about the filesystem: every
    // file the store was built from is deleted, so anything that comes back
    // came out of `sessions.blob`.
    std::fs::remove_dir_all(&bench.work).unwrap();

    let store = reindex::open_up_to_date(&bench.data_dir)
        .expect("a store written by the previous build still opens for work");
    assert_eq!(
        store.meta_int(META_DERIVED_SCHEMA).unwrap(),
        Some(DERIVED_SCHEMA),
        "the store was not stamped forward"
    );
    drop(store);

    let conn = bench.conn();
    assert!(has_column(&conn, "turns", "is_typed"), "the column is missing");

    // Every rebuilt session's user rows are classified, and nothing else is.
    assert_eq!(
        count_where(
            &conn,
            &format!("record_type = 'user' AND session_key <> '{evicted}' AND is_typed IS NULL"),
        ),
        0,
        "a rebuilt user turn carries no classification"
    );
    assert!(
        count_where(
            &conn,
            &format!("record_type = 'user' AND session_key <> '{evicted}'"),
        ) > 0,
        "the rebuild classified nothing, so the count above is empty"
    );
    assert_eq!(
        count_where(&conn, "record_type <> 'user' AND is_typed IS NOT NULL"),
        0
    );

    // The evicted session: rows carried across untouched, and null is what its
    // classification stays, because nothing re-derived it (D-04).
    assert_eq!(derived_rows(&conn, &evicted), rows_before);
    assert_eq!(turn_ids(&conn, &evicted), ids_before);
    assert_eq!(
        count_where(&conn, &format!("session_key = '{evicted}' AND is_typed IS NOT NULL")),
        0,
        "a preserved evicted row was given a classification nothing derived"
    );
    drop(conn);

    // And the store still ingests: this is where `no such column` would land
    // if the column had not reached `turns`.
    let more = bench.data_dir.parent().unwrap().join("more");
    std::fs::create_dir_all(&more).unwrap();
    let path = testkit::copy_fixture_into("session-recall.jsonl", &more);
    match ingest::run(&bench.data_dir, &path).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("a further ingest must commit: {other:?}"),
    }
    assert_eq!(
        count_where(
            &bench.conn(),
            &format!("record_type = 'user' AND session_key <> '{evicted}' AND is_typed IS NULL"),
        ),
        0,
        "the newly ingested session left a user turn unclassified"
    );
    // The evicted session is the exception and stays one: its rows were never
    // re-derived, so they are still the only NULLs in the table (D-04).
    assert_eq!(
        count_where(&bench.conn(), "record_type = 'user' AND is_typed IS NULL"),
        evicted_users
    );

    // A regression guard rather than new work: `verify` reads `sessions` and
    // `session_meta` and never touches `turns` (D-09), so it must be as clean
    // over this store as over any other.
    let store = bench.store();
    let report = verify::verify(&store).unwrap();
    assert!(report.is_ok(), "{}", report.render());
    assert!(report.checked > 0);
}
