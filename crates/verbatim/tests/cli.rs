//! Process-level assertions: exit codes, stdout, stderr.
//!
//! These live here and not in `verbatim-core`'s tests because
//! `CARGO_BIN_EXE_verbatim` is defined only for integration tests of the
//! package that declares the bin, and `verbatim-core` does not depend on
//! `verbatim`. Hardcoding `target/debug/verbatim` would be wrong twice over: it
//! ignores `CARGO_TARGET_DIR` and it names the wrong profile under `--release`.

use std::path::PathBuf;
use std::process::{Command, Output};

use rusqlite::Connection;
use verbatim_core::store::{
    Store, ARCHIVE_FORMAT, DB_FILE_NAME, DERIVED_SCHEMA, META_ARCHIVE_FORMAT, META_DERIVED_SCHEMA,
};
use verbatim_core::testkit;

/// Every directory a spawned `verbatim` may touch, all of them temporary.
///
/// The config directory and the Claude directory are as load-bearing as the
/// data directory, and they are new here. Once bare `verbatim ingest` walks the
/// configured roots, a spawn that sets only `VERBATIM_DATA_DIR` resolves the
/// developer's real config and walks the live `~/.claude` tree - 2,000+ private
/// transcripts and ~988 MB ingested into a temp dir on every `cargo test`. No
/// test process may resolve a real transcript root.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    /// Holds no `verbatim.toml`, so the loader yields defaults.
    config_dir: PathBuf,
    /// `<claude_dir>/projects` is the only tree a spawn can reach.
    claude_dir: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        config_dir,
        claude_dir,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_verbatim"))
            .args(args)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn verbatim")
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

    /// Put a fixture into the temporary transcript tree under a
    /// transcript-shaped name, so the tree pass discovers it.
    fn place(&self, project: &str, name: &str, fixture: &str) -> PathBuf {
        let dest = self.claude_dir.join("projects").join(project).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one JSON document a `--json` run wrote to stdout.
///
/// Parsed rather than substring-matched, and it asserts stdout holds that
/// document and nothing else: "JSON and only JSON on stdout" is the half of the
/// contract a `contains` check cannot fail on, because a progress line printed
/// beside a valid document still contains it.
fn document(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_else(|| panic!("stdout was empty"));
    assert_eq!(
        lines.next(),
        None,
        "stdout carried more than the document: {text:?}"
    );
    serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {text:?}"))
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
        let out = target.run(&args);
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
///
/// `ingest` with no path is deliberately absent from this list. It exited 2 in
/// phase 1 and is the tree pass now, which is an intended change to a shipped
/// contract; `bare_ingest_walks_only_the_configured_temporary_root` is the
/// assertion that replaced it.
///
/// `verify --json` left it in phase 3 the same way and for the same reason:
/// D-24 makes it supported, so the case that stands for "a subcommand rejects
/// what it was not written for" is now a flag no command accepts.
#[test]
fn misuse_exits_two_with_an_empty_stdout() {
    let bench = bench();
    for args in [
        vec!["verify", "--nope"],
        vec!["reindex", "extra"],
        vec!["status", "-x"],
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

/// Bare `verbatim ingest` is the tree pass now: exit 0, and it walks the
/// configured root and nothing else.
///
/// The second half is the isolation assertion, and it is not decoration. If the
/// spawn resolved a real root this store would hold thousands of sessions from
/// `~/.claude` rather than the two placed here, so "every archived transcript
/// path sits inside this test's temporary tree" is the property that says no
/// test process reached the developer's own transcripts.
#[test]
fn bare_ingest_walks_only_the_configured_temporary_root() {
    let bench = bench();
    let project = "-data-projects-cadence";
    let top = bench.place(
        project,
        "11111111-1111-4111-8111-111111111111.jsonl",
        "session-basic.jsonl",
    );
    let sidecar = bench.place(
        project,
        "33333333-3333-4333-8333-333333333333/subagents/agent-a.jsonl",
        "subagents/agent-alpha.jsonl",
    );

    let out = bench.run(&["ingest"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "the tree pass produces no data on stdout");

    let conn = bench.conn();
    let keys: Vec<String> = conn
        .prepare("SELECT session_key FROM sessions ORDER BY session_key")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    let mut expected = vec![
        top.to_str().unwrap().to_owned(),
        sidecar.to_str().unwrap().to_owned(),
    ];
    expected.sort();
    assert_eq!(keys, expected, "the pass walked a tree it was not given");

    let root = bench.claude_dir.canonicalize().unwrap();
    for key in &keys {
        assert!(
            std::path::Path::new(key).starts_with(&root),
            "{key} is outside this test's temporary transcript root"
        );
    }

    // A rerun is the steady state: still exit 0, still the same two sessions.
    let again = bench.run(&["ingest"]);
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    let after: i64 = bench
        .conn()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, 2);
}

/// D-24, on the three commands phase 1 and 2 shipped: `--json` writes one
/// document to stdout, and the run without it writes exactly what it wrote
/// before the flag existed.
///
/// The second half is the half that can regress silently. Adding a mode to a
/// command is how its default output picks up a stray line, and `reindex`
/// producing nothing on stdout is a contract phase 1 already shipped.
#[test]
fn the_three_shipped_commands_take_json_without_changing_their_plain_output() {
    let bench = bench();
    bench.ingest("session-basic.jsonl");
    bench.ingest("subagents/agent-alpha.jsonl");

    // Plain: byte-for-byte what phase 1 and 2 print.
    assert_eq!(stdout(&bench.run(&["verify"])), "");
    assert_eq!(stdout(&bench.run(&["reindex"])), "");

    for command in ["verify", "reindex", "status"] {
        let out = bench.run(&[command, "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let document = document(&out);
        assert_eq!(document["command"], command);
        assert_eq!(document["ok"], true);
        assert_eq!(document["reason"], serde_json::Value::Null);
        assert!(document["data"].is_object(), "{command}: {document}");
    }

    // Plain mode keeps its commentary on stderr; JSON mode MOVES it into the
    // document rather than printing it twice. Two accounts of one walk that can
    // disagree is what one shape exists to prevent.
    for command in ["verify", "reindex"] {
        assert!(
            !stderr(&bench.run(&[command])).is_empty(),
            "{command} lost its plain-mode commentary"
        );
        assert_eq!(
            stderr(&bench.run(&[command, "--json"])),
            "",
            "{command} --json printed commentary the document already carries"
        );
    }

    // The numbers are the work, not a placeholder.
    let verify = document(&bench.run(&["verify", "--json"]));
    assert_eq!(verify["data"]["checked"], 2);
    assert_eq!(verify["data"]["failures"].as_array().unwrap().len(), 0);

    let reindex = document(&bench.run(&["reindex", "--json"]));
    assert_eq!(reindex["data"]["sessions"], 2);
    assert!(reindex["data"]["turns"].as_i64().unwrap() > 0, "{reindex}");
    assert_eq!(reindex["data"]["skipped"].as_array().unwrap().len(), 0);
}

/// A store with a flipped byte: `verify --json` exits 1 AND names the session,
/// in the one document.
///
/// Both halves together are the point. A caller that parses the document and a
/// caller that checks the exit code must not disagree about whether the store
/// is whole, so the failing id being present is not an alternative to the
/// non-zero exit.
#[test]
fn verify_json_names_the_corrupt_session_and_still_exits_one() {
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

    let out = bench.run(&["verify", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));

    let document = document(&out);
    assert_eq!(document["ok"], false);
    assert!(
        document["reason"].as_str().is_some_and(|r| r.contains('1')),
        "the reason must say how many failed: {document}"
    );
    let failures = document["data"]["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{document}");
    assert_eq!(failures[0]["session_key"], sidecar);
    assert!(
        failures[0]["detail"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "a failure says why: {document}"
    );
    assert!(
        !document.to_string().contains(&parent),
        "the document named a session that verified: {document}"
    );
}

/// `reindex --json` on a store whose lock another process holds: the refusal is
/// a document with `ok: false` and a reason, not an empty stdout.
///
/// A `--json` caller that got nothing on stdout could not tell a refusal from a
/// crash, which is the case this covers and the plain-mode path does not.
#[test]
fn reindex_json_reports_a_held_lock_as_a_document() {
    let bench = bench();
    bench.ingest("session-basic.jsonl");

    let guard = match verbatim_core::ingest::lock::try_acquire(&bench.data_dir).unwrap() {
        verbatim_core::ingest::Attempt::Acquired(guard) => guard,
        other => panic!("the lock should have been free: {other:?}"),
    };

    let out = bench.run(&["reindex", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let document = document(&out);
    assert_eq!(document["ok"], false);
    assert!(
        document["reason"]
            .as_str()
            .is_some_and(|r| r.contains("LOCK")),
        "the refusal must name the lock: {document}"
    );

    drop(guard);
}

/// AC5: `verbatim stats` reports the numbers a reader can add up by hand, in
/// both modes, off a placed outcome mix (FEED-04).
///
/// The mix is written straight into `decisions` and `labels` rather than
/// produced by an ingest pass: what is under test is the aggregation and the
/// command, and a mix the labeller derived would make the expected numbers a
/// second computation of the thing being checked.
#[test]
fn stats_reports_the_hand_computed_numbers_in_both_modes() {
    let bench = bench();
    bench.ingest("session-basic.jsonl");

    let conn = bench.conn();
    let place = |prompt: &str, chars: i64, injected: &str, labels: &[(&str, Option<i64>)]| {
        conn.execute(
            "INSERT INTO decisions (
                session_id, ts, cwd, prompt, watermark_session_no, chars_injected,
                spellings, candidates, injected, suppressed, thresholds
             ) VALUES ('44444444-4444-4444-8444-444444444444',
                       '2026-08-14T11:00:00.000Z', '/code', ?1, 1, ?2,
                       '[]', '[]', ?3, '[]', '{}')",
            rusqlite::params![prompt, chars, injected],
        )
        .unwrap();
        let decision = conn.last_insert_rowid();
        for (label, turn_id) in labels {
            conn.execute(
                "INSERT INTO labels (decision_id, turn_id, label, labeled_at)
                 VALUES (?1, ?2, ?3, '2026-08-14T12:00:00.000Z')",
                rusqlite::params![decision, turn_id, label],
            )
            .unwrap();
        }
    };
    place(
        "the mixed one",
        100,
        r#"[{"turn_id":10,"chars":60},{"turn_id":11,"chars":40}]"#,
        &[("hit", Some(10)), ("false positive", Some(11))],
    );
    place(
        "the wasteful one",
        50,
        r#"[{"turn_id":12,"chars":50}]"#,
        &[("false positive", Some(12)), ("wasted budget", None)],
    );
    place("the non-fire", 0, "[]", &[("miss", None)]);

    let out = bench.run(&["stats", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let data = &document(&out)["data"];
    assert_eq!(data["decisions"], 3, "{data}");
    assert_eq!(data["injected_turns"], 3, "{data}");
    assert_eq!(data["hits"], 1, "{data}");
    assert_eq!(data["false_positives"], 2, "{data}");
    assert_eq!(data["misses"], 1, "{data}");
    assert_eq!(data["wasted_budget"], 1, "{data}");
    assert_eq!(data["chars_injected"], 150, "{data}");
    assert_eq!(data["chars_referenced"], 60, "{data}");
    let precision = data["precision"].as_f64().expect("a precision");
    assert!((precision - 1.0 / 3.0).abs() < 1e-9, "{data}");

    // The same numbers in the human mode, and no others.
    let out = bench.run(&["stats"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    for line in [
        "decisions         3",
        "injected turns    3",
        "hits              1",
        "false positives   2",
        "precision         0.33",
        "misses            1",
        "wasted budget     1",
        "chars injected    150",
        "chars referenced  60",
    ] {
        assert!(text.contains(line), "no {line:?} in:\n{text}");
    }
}

/// A machine that has never ingested: exit 0 with a reason, and nothing created
/// by having been asked.
#[test]
fn stats_against_no_store_is_a_reason_and_exits_zero() {
    let bench = bench();

    let out = bench.run(&["stats"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "the reason belongs on stderr");
    assert!(
        stderr(&out).contains("no verbatim store"),
        "{}",
        stderr(&out)
    );

    let out = bench.run(&["stats", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert!(
        value["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no verbatim store")),
        "{value}"
    );
    assert_eq!(value["data"]["decisions"], 0, "{value}");
    assert_eq!(
        value["data"]["precision"],
        serde_json::Value::Null,
        "{value}"
    );

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory"
    );
}

// ---------------------------------------------------------------------------
// RCL-06: the shapes and the exit codes, swept across every data command
//
// One test per property rather than one test per command. A seventh data
// command added in a later phase joins `DATA_COMMANDS` on one line and is held
// to the whole contract by that alone.
// ---------------------------------------------------------------------------

/// Every data command, the arguments that make it answer, and the `data` fields
/// `docs/json-shapes.md` says it emits.
///
/// The field lists are the documented shape, transcribed. That transcription is
/// the point: a shape that drifts from the doc fails here, and a doc updated
/// without the code fails here too.
const DATA_COMMANDS: &[(&str, &[&str], &[&str])] = &[
    (
        "search",
        &["--project", "*", "cargo"],
        &["query", "truncated", "hits"],
    ),
    ("show", &["--project", "*"], &["records", "absent"]),
    ("sessions", &["--project", "*"], &["sessions"]),
    (
        "status",
        &[],
        &[
            "store",
            "size_bytes",
            "sessions",
            "turns",
            "watermarks",
            "watermark_bytes",
            "excluded",
            "last_run",
        ],
    ),
    ("verify", &[], &["checked", "failures"]),
    ("reindex", &[], &["sessions", "turns", "skipped"]),
    (
        "stats",
        &[],
        &[
            "decisions",
            "injected_turns",
            "hits",
            "false_positives",
            "precision",
            "misses",
            "wasted_budget",
            "chars_injected",
            "chars_referenced",
        ],
    ),
    // `replay` joins on the same terms and passes the same five properties: it
    // is a data command, it emits the envelope, and its `data` is the same shape
    // whether or not the store holds a decision to score.
    (
        "replay",
        &[],
        &["thresholds", "decisions", "labels", "changed"],
    ),
];

/// A bench with the whole fixture corpus in it and one known turn id, which is
/// the argument `show` needs to answer at all.
fn swept() -> (Bench, String) {
    let bench = bench();
    for fixture in testkit::TRANSCRIPT_FIXTURES {
        bench.ingest(fixture);
    }
    let id: i64 = bench
        .conn()
        .query_row("SELECT min(id) FROM turns", [], |r| r.get(0))
        .unwrap();
    (bench, id.to_string())
}

/// The command line for one swept command, with `show`'s id appended.
fn sweep_args<'a>(command: &'a str, args: &'a [&'a str], id: &'a str) -> Vec<&'a str> {
    let mut out = vec![command];
    out.extend_from_slice(args);
    if command == "show" {
        out.push(id);
    }
    out
}

/// Property 1: `--json` output parses and matches the documented shape field for
/// field, on every data command.
#[test]
fn every_data_command_emits_the_documented_shape() {
    let (bench, id) = swept();

    for (command, args, fields) in DATA_COMMANDS {
        let mut argv = sweep_args(command, args, &id);
        argv.push("--json");
        let out = bench.run(&argv);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{}`: {}",
            argv.join(" "),
            stderr(&out)
        );

        let value = document(&out);
        let envelope = value.as_object().expect("a document is an object");
        let mut keys: Vec<&str> = envelope.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["command", "data", "ok", "reason"],
            "`{}` emitted a different envelope",
            argv.join(" ")
        );
        assert_eq!(value["command"], *command);

        let data = value["data"].as_object().expect("data is an object");
        let mut present: Vec<&str> = data.keys().map(String::as_str).collect();
        present.sort_unstable();
        let mut documented: Vec<&str> = fields.to_vec();
        documented.sort_unstable();
        assert_eq!(
            present,
            documented,
            "`{}` does not match docs/json-shapes.md",
            argv.join(" ")
        );
    }
}

/// Property 2: with `--json`, stdout is the document and nothing else, and a
/// warning still lands on stderr.
///
/// The rule `--json` keeps is that the document IS the answer: routine
/// commentary moves into it, so a clean run writes nothing to stderr at all, and
/// stderr is left for warnings. The second half of this test is what keeps the
/// first from being vacuous - a store older than this build is a real warning,
/// and it has to reach stderr without putting one byte on stdout.
#[test]
fn json_mode_keeps_stdout_pure_and_warnings_on_stderr() {
    let (bench, id) = swept();

    for (command, args, _) in DATA_COMMANDS {
        let mut argv = sweep_args(command, args, &id);
        argv.push("--json");
        let out = bench.run(&argv);

        let text = stdout(&out);
        assert_eq!(
            text.lines().count(),
            1,
            "`{}` wrote more than the document to stdout: {text:?}",
            argv.join(" ")
        );
        serde_json::from_str::<serde_json::Value>(text.trim())
            .unwrap_or_else(|e| panic!("`{}` stdout is not JSON ({e}): {text:?}", argv.join(" ")));
        assert_eq!(
            stderr(&out),
            "",
            "`{}` printed commentary the document already carries",
            argv.join(" ")
        );
    }

    // Now give the commands something to warn about. `reindex` would repair it
    // and `verify` and `status` do not read through the shared entry point, so
    // the three read commands are the ones with a warning to place.
    bench
        .conn()
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            rusqlite::params![(DERIVED_SCHEMA - 1).to_string(), META_DERIVED_SCHEMA],
        )
        .unwrap();

    for (command, args, _) in DATA_COMMANDS.iter().take(3) {
        let mut argv = sweep_args(command, args, &id);
        argv.push("--json");
        let out = bench.run(&argv);

        assert!(
            stderr(&out).contains("predate this build"),
            "`{}` swallowed the warning: {:?}",
            argv.join(" "),
            stderr(&out)
        );
        // And it is still exactly one document on stdout.
        let text = stdout(&out);
        assert_eq!(text.lines().count(), 1, "{text:?}");
        serde_json::from_str::<serde_json::Value>(text.trim())
            .unwrap_or_else(|e| panic!("`{}` stdout is not JSON ({e})", argv.join(" ")));
    }
}

/// Property 3a: exit 0 on success, including an empty result set.
///
/// The empty case is the one RCL-06 exists for. Searching for a path is the
/// headline command in the product and a raw `MATCH` would exit 1 on it, so
/// "found nothing" and "could not ask" must never share a code.
#[test]
fn success_and_an_empty_result_both_exit_zero() {
    let (bench, id) = swept();

    for (command, args, _) in DATA_COMMANDS {
        let argv = sweep_args(command, args, &id);
        let out = bench.run(&argv);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{}`: {}",
            argv.join(" "),
            stderr(&out)
        );
    }

    // The empty results, one per read command, each of which could plausibly
    // have been an error instead.
    let unknown: String = bench
        .conn()
        .query_row("SELECT max(id) + 1000 FROM turns", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
        .to_string();
    for argv in [
        vec!["search", "--project", "*", "zzzznotinanyfixture"],
        vec!["search", "--project", "*", "src/worker/does-not-exist.ts"],
        vec!["show", "--project", "*", unknown.as_str()],
        vec!["sessions", "--project", "*", "--until", "2000-01-01"],
    ] {
        let out = bench.run(&argv);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{}` should be an empty result, not a failure: {}",
            argv.join(" "),
            stderr(&out)
        );
        assert_eq!(
            stdout(&out),
            "",
            "`{}` printed data for an empty result",
            argv.join(" ")
        );
    }
}

/// Property 3b: exit 2 for misuse, on every command, for an unknown flag - and
/// for an unknown subcommand.
#[test]
fn an_unknown_flag_and_an_unknown_subcommand_both_exit_two() {
    let (bench, id) = swept();

    for (command, args, _) in DATA_COMMANDS {
        let mut argv = sweep_args(command, args, &id);
        argv.push("--definitely-not-a-flag");
        let out = bench.run(&argv);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{}` should be misuse: {}",
            argv.join(" "),
            stderr(&out)
        );
        assert_eq!(
            stdout(&out),
            "",
            "`{}` printed to stdout on misuse",
            argv.join(" ")
        );
    }

    for argv in [vec!["no-such-command"], vec!["no-such-command", "--json"]] {
        let out = bench.run(&argv);
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert_eq!(stdout(&out), "");
    }
}

/// Property 3c: exit 1 for an operational failure, with the `--json` document
/// still written and `ok` false.
///
/// A caller that parses the document and a caller that checks the code must not
/// disagree about whether the store is whole, which is why this asserts both on
/// the same run rather than one on each.
#[test]
fn an_operational_failure_exits_one_with_ok_false() {
    let bench = bench();
    bench.ingest("session-basic.jsonl");
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

    let out = bench.run(&["verify", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], false, "{value}");
    assert!(value["reason"].is_string(), "{value}");
}
