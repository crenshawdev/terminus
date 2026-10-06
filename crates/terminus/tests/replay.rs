//! AC4 at the process boundary: `terminus replay` answers, and the store is
//! byte for byte what it was (FEED-03).
//!
//! Spawns rather than library calls. The byte-identity claim is about a
//! command, not about a function: `terminus-core`'s `tests/feedback.rs` asserts
//! what a replay COMPUTES, and what is here is that running it leaves nothing
//! behind - which is a statement about how the binary opens the file (D-15) and
//! is unfalsifiable inside a call that was handed an open store.

use std::path::PathBuf;
use std::process::{Command, Output};

use rusqlite::Connection;
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

/// The fixture whose `Edit` stores an absolute path, and the session id its own
/// records carry - a decision is joined to a session by `session_id` alone
/// (D-04), so a logged decision has to name this one to be labelled at all.
const FIXTURE: &str = "session-edits.jsonl";
const FIXTURE_SESSION: &str = "44444444-4444-4444-8444-444444444444";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        config_dir,
        claude_dir,
        root,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_terminus"))
            .args(args)
            .current_dir(&self.work)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn terminus")
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// Archive the rooted fixture through the binary's own ingest path.
    fn ingest_fixture(&self) -> String {
        let path = testkit::copy_rooted_fixture_into(FIXTURE, &self.work, &self.root);
        let out = self.run(&["ingest", path.to_str().unwrap()]);
        assert!(out.status.success(), "ingest: {}", stderr(&out));
        testkit::fixture_project(FIXTURE, &self.root)
            .to_string_lossy()
            .into_owned()
    }

    /// One logged decision naming a turn the archive holds.
    ///
    /// Written straight into the table the drain fills: what is under test here
    /// is the command, and driving a real hook would make the row's contents
    /// depend on a wall clock this test does not own.
    fn decide(&self, cwd: &str, prompt: &str, turn_id: i64) {
        let conn = self.conn();
        let watermark: i64 = conn
            .query_row("SELECT max(session_no) FROM sessions", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO decisions (
                session_id, ts, cwd, prompt, watermark_session_no, chars_injected,
                spellings, candidates, injected, suppressed, thresholds
             ) VALUES (?1, ?2, ?3, ?4, ?5, 137, '[]', '[]', ?6, '[]', '{}')",
            rusqlite::params![
                FIXTURE_SESSION,
                "2026-08-14T11:00:00.000Z",
                cwd,
                prompt,
                watermark,
                format!(r#"[{{"turn_id":{turn_id},"chars":120}}]"#),
            ],
        )
        .unwrap();
    }

    /// The id of the turn that stored the fixture's path entity.
    fn stored_turn(&self) -> i64 {
        self.conn()
            .query_row(
                "SELECT turn_id FROM entities WHERE kind = 'path' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// Every byte of the store file.
    fn store_bytes(&self) -> Vec<u8> {
        std::fs::read(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// The sizes of the two WAL sidecars.
    ///
    /// They are asserted on beside the database because a write that landed in
    /// the WAL and was not checkpointed would leave `terminus.db` untouched and
    /// still be a write - so hashing the database alone would pass on exactly
    /// the failure this is about.
    ///
    /// **A first read-only open of a WAL database materializes
    /// `terminus.db-shm` (32 KiB) and an empty `terminus.db-wal`.** That is
    /// SQLite's shared-memory index, which every reader needs and which a
    /// read-only connection cannot unlink on close; it carries no page of the
    /// database. So the claim asserted here is the one that means something: the
    /// database is byte for byte what it was, the WAL holds no frame, and a
    /// second run grows neither.
    fn sidecars(&self) -> (u64, u64) {
        (
            size(&self.data_dir.join(format!("{DB_FILE_NAME}-wal"))),
            size(&self.data_dir.join(format!("{DB_FILE_NAME}-shm"))),
        )
    }
}

fn size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one JSON document a `--json` run wrote to stdout, and nothing else.
fn document(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_else(|| panic!("stdout was empty"));
    assert_eq!(lines.next(), None, "stdout carried more than the document");
    serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {text:?}"))
}

/// AC4: an override flag, exit 0, a per-label diff, and a store nothing
/// touched.
#[test]
fn replay_reports_a_per_label_diff_and_leaves_the_store_alone() {
    let bench = bench();
    let cwd = bench.ingest_fixture();
    let turn = bench.stored_turn();
    bench.decide(&cwd, "what changed in crates/gizmo/lantern.rs", turn);

    // One ingest to close the idle session and label the decision, so the diff
    // has something on its `old` side.
    let out = bench.run(&["ingest"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let labels: i64 = bench
        .conn()
        .query_row("SELECT count(*) FROM labels", [], |r| r.get(0))
        .unwrap();
    assert!(labels > 0, "nothing was labelled, so a diff proves nothing");

    let before = bench.store_bytes();

    let out = bench.run(&["replay", "--entity-rank", "5", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let value = document(&out);
    assert_eq!(value["command"], "replay");
    assert_eq!(value["ok"], true, "{value}");
    // The numbers in force, including the one the flag moved.
    assert_eq!(value["data"]["thresholds"]["entity_rank"], 5, "{value}");
    assert_eq!(value["data"]["thresholds"]["max_turns"], 3, "{value}");
    assert_eq!(value["data"]["decisions"], 1, "{value}");

    let diff = value["data"]["labels"].as_array().unwrap();
    let names: Vec<&str> = diff
        .iter()
        .map(|entry| entry["label"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["hit", "false positive", "miss", "wasted budget"]);
    for entry in diff {
        assert!(entry["old"].is_u64() && entry["new"].is_u64(), "{entry}");
    }
    // The decision's turn was never referred to again, so it is a false
    // positive under any threshold - the flag widens what would be offered, not
    // what a turn turned out to be worth.
    let false_positive = &diff[1];
    assert_eq!(false_positive["old"], 1, "{value}");
    assert!(value["data"]["changed"].is_array(), "{value}");

    // The claim AC4 is really about.
    assert_eq!(bench.store_bytes(), before, "replay changed the store");
    let sidecars = bench.sidecars();
    assert_eq!(sidecars.0, 0, "replay left a frame in the WAL");

    // And again with no flags at all: the shipped numbers reproduce the labels
    // the store already holds, so nothing changed.
    let out = bench.run(&["replay", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(
        value["data"]["changed"].as_array().unwrap().len(),
        0,
        "the build does not reproduce its own labels: {value}"
    );
    assert_eq!(bench.store_bytes(), before);
    assert_eq!(bench.sidecars(), sidecars, "a second replay grew a sidecar");
}

/// The human mode: the diff on stdout, exit 0, and still no write.
#[test]
fn replay_without_json_prints_the_diff_and_writes_nothing() {
    let bench = bench();
    let cwd = bench.ingest_fixture();
    let turn = bench.stored_turn();
    bench.decide(&cwd, "what changed in crates/gizmo/lantern.rs", turn);
    assert!(bench.run(&["ingest"]).status.success());

    let before = bench.store_bytes();
    let out = bench.run(&["replay"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let text = stdout(&out);
    assert!(text.contains("decisions"), "{text}");
    assert!(text.contains("false positive"), "{text}");
    assert!(text.contains("->"), "{text}");
    assert_eq!(bench.store_bytes(), before);
    assert_eq!(bench.sidecars().0, 0, "replay left a frame in the WAL");
}

/// A machine that has never ingested: an empty answer with a reason and exit 0,
/// and no store created by having been asked (RCL-06, D-10).
#[test]
fn replay_against_no_store_is_a_reason_and_creates_nothing() {
    let bench = bench();

    let out = bench.run(&["replay", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert!(
        value["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no terminus store")),
        "{value}"
    );
    assert_eq!(value["data"]["decisions"], 0, "{value}");

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory it was asked about"
    );
}

/// A threshold flag with no number, and one that is not a number: misuse, and
/// nothing on stdout either way.
#[test]
fn a_malformed_threshold_is_misuse() {
    let bench = bench();
    bench.ingest_fixture();

    for argv in [
        vec!["replay", "--entity-rank", "three"],
        vec!["replay", "--entity-rank"],
        vec!["replay", "--not-a-threshold", "5"],
    ] {
        let out = bench.run(&argv);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{}` should be misuse: {}",
            argv.join(" "),
            stderr(&out)
        );
        assert_eq!(stdout(&out), "", "`{}` wrote to stdout", argv.join(" "));
    }
}

/// A store written before phase 6: both reads that live on the decision log
/// answer, and neither leaks a sqlite line where an envelope was promised.
///
/// The tables are dropped rather than a phase-5 binary being kept around,
/// because the condition under test is exactly "these two tables are absent" -
/// `decisions` and `labels` went into `schema::TABLES` with no `DERIVED_SCHEMA`
/// bump (D-02), so an older store is a current store minus these two and
/// nothing else. That is also why this could ever regress: `rebuild_required`
/// is `None` and `missing_columns` is empty on such a store, so it reads as
/// perfectly up to date right up until a query names a table that is not there.
///
/// The `--json` half is the half with teeth. The failure this replaces was an
/// operational error raised from inside the query and printed by `main`, which
/// never sees the flag - so `--json` exited 1 with an empty stdout and no
/// document at all, and `docs/json-shapes.md` allows that only for a failure
/// raised *before* the command reached its own answer.
#[test]
fn stats_and_replay_on_a_store_without_the_decision_log_still_answer() {
    let bench = bench();
    bench.ingest_fixture();

    let conn = bench.conn();
    conn.execute_batch("DROP TABLE labels; DROP TABLE decisions;")
        .unwrap();
    drop(conn);

    for command in ["stats", "replay"] {
        let out = bench.run(&[command, "--json"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{command} --json`: {}",
            stderr(&out)
        );

        // One document, and the reason inside it rather than on stderr.
        let value = document(&out);
        assert_eq!(value["command"], command, "{value}");
        assert_eq!(value["ok"], true, "{value}");
        assert!(
            value["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("predates the decision log")),
            "`{command} --json` did not say why it was empty: {value}"
        );
        assert!(
            !stderr(&out).contains("no such table"),
            "`{command} --json` leaked the sqlite error: {}",
            stderr(&out)
        );
        assert_eq!(value["data"]["decisions"], 0, "{value}");

        // The human mode: the same reason, on stderr, and still exit 0.
        let out = bench.run(&[command]);
        assert_eq!(out.status.code(), Some(0), "`{command}`: {}", stderr(&out));
        assert!(
            stderr(&out).contains("predates the decision log"),
            "`{command}` did not say why: {}",
            stderr(&out)
        );
    }
}

/// Half a decision log is damage, not age, and it is reported as such.
///
/// The two tables arrived in one `CREATE_SQL` (D-02), so "missing both" is the
/// only shape age can produce; a store missing exactly one was written by a
/// build that had both. The failure this guards is the comfortable one: reading
/// it as "nothing recorded yet" would answer `precision` and a per-label diff
/// with zeroes and `ok:true`, and a caller cannot tell that from an archive
/// where injection genuinely never fired.
#[test]
fn stats_and_replay_call_half_a_decision_log_a_failure() {
    for dropped in ["labels", "decisions"] {
        let bench = bench();
        bench.ingest_fixture();

        let conn = bench.conn();
        conn.execute_batch(&format!("DROP TABLE {dropped};"))
            .unwrap();
        drop(conn);

        for command in ["stats", "replay"] {
            let out = bench.run(&[command, "--json"]);
            assert_eq!(
                out.status.code(),
                Some(1),
                "`{command} --json` on a store missing `{dropped}` should fail: {}",
                stderr(&out)
            );

            // Exit 1 AND the envelope: the contract is that a caller parsing the
            // document and a caller checking the code agree.
            let value = document(&out);
            assert_eq!(value["command"], command, "{value}");
            assert_eq!(value["ok"], false, "{value}");
            let reason = value["reason"].as_str().unwrap_or_default();
            assert!(
                reason.contains(dropped) && reason.contains("something removed it"),
                "`{command} --json` did not name the damage: {value}"
            );

            // The human mode: exit 1 and one line naming the table.
            let out = bench.run(&[command]);
            assert_eq!(out.status.code(), Some(1), "`{command}`: {}", stderr(&out));
            assert!(
                stderr(&out).contains(dropped),
                "`{command}` did not name the missing table: {}",
                stderr(&out)
            );
            assert!(
                !stderr(&out).contains("no such table"),
                "`{command}` leaked the sqlite error: {}",
                stderr(&out)
            );
        }
    }
}
