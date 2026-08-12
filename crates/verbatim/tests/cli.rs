//! Process-level assertions: exit codes, stdout, stderr.
//!
//! These live here and not in `verbatim-core`'s tests because
//! `CARGO_BIN_EXE_verbatim` is defined only for integration tests of the
//! package that declares the bin, and `verbatim-core` does not depend on
//! `verbatim`. Hardcoding `target/debug/verbatim` would be wrong twice over: it
//! ignores `CARGO_TARGET_DIR` and it names the wrong profile under `--release`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;
use verbatim_core::store::{
    Store, ARCHIVE_FORMAT, DB_FILE_NAME, DERIVED_SCHEMA, META_ARCHIVE_FORMAT, META_DERIVED_SCHEMA,
};
use verbatim_core::testkit;

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        verbatim(&self.data_dir, args)
    }

    /// Ingest a fixture through the binary, returning its session key.
    fn ingest(&self, fixture: &str) -> String {
        let path = testkit::copy_fixture_into(fixture, &self.work);
        let out = self.run(&["ingest", path.to_str().unwrap()]);
        assert!(
            out.status.success(),
            "ingest {fixture}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        path.canonicalize().unwrap().to_str().unwrap().to_owned()
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

fn verbatim(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verbatim"))
        .args(args)
        .env("VERBATIM_DATA_DIR", data_dir)
        .output()
        .expect("spawn verbatim")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// AC3, at the process boundary: exit 0 and an empty stdout on a clean store.
#[test]
fn verify_exits_zero_and_says_nothing_on_a_clean_store() {
    let bench = bench();
    bench.ingest("session-basic.jsonl");
    bench.ingest("subagents/agent-alpha.jsonl");

    let out = bench.run(&["verify"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "a clean store names no session");
}

/// AC3. One byte flipped inside one session's blob: exit non-zero, that session
/// named on stdout, no other session named.
///
/// The two fixtures are D-01's colliding pair - the sidecar reports its
/// parent's `sessionId` - so "and no other" is a clause that can fail.
#[test]
fn verify_names_the_corrupt_session_and_no_other() {
    let bench = bench();
    let parent = bench.ingest("session-basic.jsonl");
    let sidecar = bench.ingest("subagents/agent-alpha.jsonl");

    {
        let conn = bench.conn();
        let mut bytes: Vec<u8> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [&sidecar],
                |r| r.get(0),
            )
            .unwrap();
        let at = bytes.len() / 2;
        bytes[at] ^= 0xff;
        conn.execute(
            "UPDATE sessions SET blob = ?1 WHERE session_key = ?2",
            rusqlite::params![bytes, &sidecar],
        )
        .unwrap();
    }

    let out = bench.run(&["verify"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));

    let text = stdout(&out);
    assert!(
        text.contains(&sidecar),
        "stdout must name the corrupt session: {text}"
    );
    assert!(
        !text.contains(&parent),
        "stdout named a session that verified: {text}"
    );
    assert!(
        !text.contains("integrity_check"),
        "D-16 keeps integrity_check in doctor: {text}"
    );
}

/// AC5's CLI half. A store one archive format ahead of this binary is refused
/// by a command that opens it: non-zero exit, a message naming both versions,
/// and the store file, its WAL and the `runs` table unchanged.
///
/// The store under test carries a hot, non-empty WAL with no live connection -
/// the state a store is in after a crash, and the only state in which a refused
/// open can quietly rewrite anything. Without that setup the byte comparison
/// passes even when the gate opens the database read-write.
#[test]
fn a_store_one_archive_format_ahead_is_refused_with_both_versions_named() {
    let source = bench();
    std::fs::create_dir_all(&source.data_dir).unwrap();
    let db = source.data_dir.join(DB_FILE_NAME);
    drop(Store::open(&source.data_dir).unwrap());

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

    let wal_name = format!("{DB_FILE_NAME}-wal");
    let source_wal = source.data_dir.join(&wal_name);
    assert!(
        std::fs::metadata(&source_wal).map(|m| m.len()).unwrap_or(0) > 0,
        "this test needs a non-empty WAL to mean anything"
    );

    let target = bench();
    std::fs::create_dir_all(&target.data_dir).unwrap();
    let target_db = target.data_dir.join(DB_FILE_NAME);
    let target_wal = target.data_dir.join(&wal_name);
    std::fs::copy(&db, &target_db).unwrap();
    std::fs::copy(&source_wal, &target_wal).unwrap();
    drop(keeper);

    let before_db = std::fs::read(&target_db).unwrap();
    let before_wal = std::fs::read(&target_wal).unwrap();

    // Both commands that open a store, because AC5 is a property of the gate
    // and not of one subcommand.
    let fixture = testkit::fixture_path("session-basic.jsonl");
    let fixture = fixture.to_str().unwrap().to_owned();
    for args in [vec!["verify"], vec!["ingest", fixture.as_str()]] {
        let out = verbatim(&target.data_dir, &args);
        assert_ne!(
            out.status.code(),
            Some(0),
            "`{}` opened a store from the future",
            args.join(" ")
        );
        let message = stderr(&out);
        assert!(
            message.contains(&(ARCHIVE_FORMAT + 1).to_string()),
            "message must name the store's format: {message}"
        );
        assert!(
            message.contains(&ARCHIVE_FORMAT.to_string()),
            "message must name the binary's format: {message}"
        );

        // Byte equality, not a digest of it: this is the whole of AC5.
        assert!(
            std::fs::read(&target_db).unwrap() == before_db,
            "`{}` changed verbatim.db",
            args.join(" ")
        );
        assert!(
            std::fs::read(&target_wal).unwrap() == before_wal,
            "`{}` changed the WAL",
            args.join(" ")
        );
    }

    // Read `runs` only now: any connection opened here may checkpoint on close
    // and would spoil the byte comparison above.
    let after = Connection::open(&target_db).unwrap();
    let runs: i64 = after
        .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(runs, 1, "runs changed on a refused open");
    let format: String = after
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_ARCHIVE_FORMAT],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(format, (ARCHIVE_FORMAT + 1).to_string());
    let derived: String = after
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_DERIVED_SCHEMA],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        derived,
        DERIVED_SCHEMA.to_string(),
        "the refused open rewrote meta"
    );
}

/// AC4, end to end through the command. Drop `turns`, `turns_fts`, `entities`
/// and `paths`, run `verbatim reindex`, and the fixed query set's JSON is
/// byte-identical to what it was before the drop.
#[test]
fn reindex_rebuilds_the_derived_tables_to_byte_identical_query_output() {
    let bench = bench();
    for fixture in testkit::TRANSCRIPT_FIXTURES {
        bench.ingest(fixture);
    }

    let before = testkit::query_set_json(&bench.conn());
    let archive_before = testkit::archive_digest(&bench.conn());
    assert!(before.contains("\"hits\": ["));

    {
        let conn = bench.conn();
        // Children first: the bundled SQLite enforces foreign keys.
        for table in verbatim_core::store::DERIVED_TABLES.iter().rev() {
            conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        }
    }

    let out = bench.run(&["reindex"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "reindex produces no data on stdout");

    assert_eq!(testkit::query_set_json(&bench.conn()), before, "AC4");
    assert_eq!(
        testkit::archive_digest(&bench.conn()),
        archive_before,
        "the rebuild touched the archive"
    );
}

/// The exit-code split phase 1 commits to: 2 is misuse, and it is not the same
/// as an operational failure.
#[test]
fn misuse_exits_two_with_an_empty_stdout() {
    let bench = bench();
    for args in [
        vec!["ingest"],
        vec!["verify", "--json"],
        vec!["no-such-command"],
    ] {
        let out = bench.run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{}` should be misuse: {}",
            args.join(" "),
            stderr(&out)
        );
        assert_eq!(stdout(&out), "", "misuse must print nothing to stdout");
    }
}

/// `reindex` drops and recreates all four derived tables, which is the most
/// destructive thing phase 1 does, and it was running without the ingest lock.
/// That made false the invariant `ingest` rests on - that the LOCK guard makes
/// it the only writer, so nothing changes the store between its read and its
/// write. The race runs both ways: a reindex could drop the tables under a
/// hook-spawned ingest, or lose the SQLite write lock to one and die on the
/// busy timeout with a raw "database is locked", which is the wait D-15 exists
/// to avoid.
#[test]
fn reindex_refuses_while_another_process_holds_the_ingest_lock() {
    let bench = bench();
    bench.ingest("session-basic.jsonl");

    let turns_before = {
        let conn = Connection::open(bench.data_dir.join(DB_FILE_NAME)).unwrap();
        conn.query_row("SELECT count(*) FROM turns", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    assert!(turns_before > 0, "this test needs turn rows to protect");

    let guard = match verbatim_core::ingest::lock::try_acquire(&bench.data_dir).unwrap() {
        verbatim_core::ingest::Attempt::Acquired(guard) => guard,
        other => panic!("the lock should have been free: {other:?}"),
    };

    let out = bench.run(&["reindex"]);
    assert!(
        !out.status.success(),
        "reindex ran while the ingest lock was held: {}",
        stderr(&out)
    );
    let message = stderr(&out);
    assert!(
        message.contains("LOCK"),
        "the refusal must name the lock: {message}"
    );
    assert!(
        !message.contains("database is locked"),
        "reindex waited on the SQLite busy handler instead of the lock: {message}"
    );

    // Nothing was dropped.
    let conn = Connection::open(bench.data_dir.join(DB_FILE_NAME)).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM turns", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        turns_before,
        "the refused reindex still dropped the derived tables"
    );

    // And it works again once the lock is free.
    drop(guard);
    let out = bench.run(&["reindex"]);
    assert!(
        out.status.success(),
        "reindex failed with the lock free: {}",
        stderr(&out)
    );
}
