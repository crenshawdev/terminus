//! `terminus retention --dry-run` at the process boundary (RET-02, RET-03).
//!
//! The report's whole claim is that it names what the NEXT pass will do, so
//! every case here runs the pass afterwards and compares. Asserting the
//! document against a second reading of the policy would prove only that the
//! test and the command agree.
//!
//! The corpus is aged in one statement with one bound value, deliberately far
//! from the age boundary on either side: `'now'` re-evaluates on every call, so
//! a fixture sitting a minute from the cutoff would make the comparison depend
//! on how long the two commands took.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

const PROJECT: &str = "-data-projects-cadence";

/// Evict everything a month past its last turn.
const EVICT_30: &str = "[retention]\naction = \"evict\"\nage_days = 30\n";
/// The same window, the other verb. `delete` still waits for the transcript to
/// be gone (D-02), so the age alone never selects anything.
const DELETE_30: &str = "[retention]\naction = \"delete\"\nage_days = 30\n";

/// Comfortably outside a 30-day window: ten days of clearance, so neither
/// command can move a session across the boundary by taking a moment to run.
const AGED: &str = "-40 days";
/// Comfortably inside it, with twenty-nine days to spare.
const FRESH: &str = "-1 days";

/// The same isolation `cli.rs` and `status.rs` use, and for the same reason: a
/// spawn that set only `TERMINUS_DATA_DIR` would resolve the developer's real
/// config and walk the live `~/.claude` tree.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        config_dir,
        claude_dir,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_terminus"))
            .args(args)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn terminus")
    }

    /// Write `terminus.toml` into the config directory this bench points the
    /// binary at. Retention is configured by hand-edited TOML and by nothing
    /// else (D-01), so this is exactly what a user does.
    fn config(&self, text: &str) {
        std::fs::write(self.config_dir.join("terminus.toml"), text).unwrap();
    }

    fn place(&self, n: u8, fixture: &str) -> PathBuf {
        let name = format!("{n:08x}-1111-4111-8111-111111111111.jsonl");
        let dest = self.claude_dir.join("projects").join(PROJECT).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    fn ingest(&self) {
        let out = self.run(&["ingest"]);
        assert!(out.status.success(), "ingest: {}", stderr(&out));
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// Close a session and move its last turn to `offset` before now.
    ///
    /// One clock reading for the whole call and a bound value rather than a SQL
    /// expression per row, so two sessions aged the same way sort by their key
    /// and not by which millisecond SQLite got to them.
    fn age(&self, keys: &[&PathBuf], offset: &str) {
        let conn = self.conn();
        let at: String = conn
            .query_row(
                "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
                [offset],
                |r| r.get(0),
            )
            .unwrap();
        for key in keys {
            let changed = conn
                .execute(
                    "UPDATE session_meta SET is_final = 1, last_turn_at = ?2
                     WHERE session_key = ?1",
                    rusqlite::params![key.to_string_lossy(), at],
                )
                .unwrap();
            assert_eq!(changed, 1, "no session_meta row for {}", key.display());
        }
    }

    fn is_evicted(&self, key: &Path) -> Option<i64> {
        self.conn()
            .query_row(
                "SELECT is_evicted FROM session_meta WHERE session_key = ?1",
                [key.to_string_lossy()],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn archived(&self, key: &Path) -> bool {
        self.conn()
            .query_row(
                "SELECT count(*) FROM sessions WHERE session_key = ?1",
                [key.to_string_lossy()],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            > 0
    }

    /// The turn ids one search returns that belong to one session.
    ///
    /// Through the binary rather than through a direct `turns_fts MATCH`,
    /// because what AC2 claims is about the command a person runs: exclusion,
    /// scoping and the excerpt read all sit between the index and the answer,
    /// and the excerpt read is the one that touches the emptied blob.
    fn search_hits_in(&self, key: &Path, token: &str) -> Vec<i64> {
        let out = self.run(&["search", "--project", "*", token, "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let value = document(&out);
        let wanted = key.to_string_lossy().into_owned();
        value["data"]["hits"]
            .as_array()
            .unwrap_or_else(|| panic!("hits is an array: {value}"))
            .iter()
            .filter(|hit| hit["session_key"] == *wanted.as_str())
            .map(|hit| hit["turn_id"].as_i64().expect("a turn id"))
            .collect()
    }

    /// Every row of every table the dry run must not touch, as comparable text.
    ///
    /// The archive digest and the fixed query set cover `sessions`,
    /// `session_meta`, `turns`, `turns_fts`, `entities` and `paths`; the
    /// watermarks are the one thing neither of them reads, and they are what a
    /// command that accidentally ingested would move.
    fn snapshot(&self) -> String {
        let conn = self.conn();
        let watermarks: Vec<String> = conn
            .prepare("SELECT transcript_path, byte_offset FROM watermarks ORDER BY 1")
            .unwrap()
            .query_map([], |r| {
                Ok(format!("{}={}", r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let meta: Vec<String> = conn
            .prepare(
                "SELECT session_key, coalesce(last_turn_at, ''), coalesce(is_final, -1),
                        coalesce(is_evicted, -1)
                   FROM session_meta ORDER BY 1",
            )
            .unwrap()
            .query_map([], |r| {
                Ok(format!(
                    "{}|{}|{}|{}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        format!(
            "{}\n{}\n{}\n{}",
            testkit::archive_digest(&conn),
            testkit::query_set_json(&conn),
            watermarks.join("\n"),
            meta.join("\n")
        )
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one JSON document a `--json` run wrote to stdout.
fn document(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_else(|| panic!("stdout was empty"));
    assert_eq!(lines.next(), None, "stdout carried more than one line");
    serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {text:?}"))
}

fn strings(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("not an array: {value}"))
        .iter()
        .map(|v| v.as_str().expect("a session key is a string").to_owned())
        .collect()
}

/// AC3. The report names exactly the sessions the immediately following pass
/// evicts - and it names them before anything has happened.
#[test]
fn the_dry_run_names_exactly_what_the_next_ingest_pass_evicts() {
    let bench = bench();
    bench.config(EVICT_30);
    let old_a = bench.place(1, "session-basic.jsonl");
    let old_b = bench.place(2, "session-recall.jsonl");
    let fresh = bench.place(3, "session-continuation.jsonl");
    bench.ingest();
    bench.age(&[&old_a, &old_b], AGED);
    bench.age(&[&fresh], FRESH);

    let out = bench.run(&["retention", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");

    let mut named = strings(&value["data"]["evict"]);
    named.sort();
    let mut expected = vec![
        old_a.to_string_lossy().into_owned(),
        old_b.to_string_lossy().into_owned(),
    ];
    expected.sort();
    assert_eq!(named, expected, "{value}");
    assert_eq!(strings(&value["data"]["delete"]), Vec::<String>::new());
    assert_eq!(value["data"]["over"], 0, "{value}");

    // The instant it judged against, in the shape every stored timestamp has.
    let cutoff = value["data"]["cutoff"]
        .as_str()
        .unwrap_or_else(|| panic!("no cutoff: {value}"));
    assert_eq!(cutoff.len(), "NNNN-NN-NNTNN:NN:NN.NNNZ".len(), "{cutoff}");
    assert!(cutoff.ends_with('Z'), "{cutoff}");

    // Nothing has happened yet, which is the other half of "dry".
    assert_eq!(bench.is_evicted(&old_a), None);
    assert_eq!(bench.is_evicted(&old_b), None);

    bench.ingest();

    assert_eq!(bench.is_evicted(&old_a), Some(1));
    assert_eq!(bench.is_evicted(&old_b), Some(1));
    assert_eq!(
        bench.is_evicted(&fresh),
        None,
        "a session inside the window was evicted"
    );
}

/// The dry run applies nothing, ever - however many times it is asked. Row for
/// row across every table an eviction or a deletion would touch, plus the
/// watermarks a command that accidentally ingested would move.
#[test]
fn running_the_dry_run_twice_changes_no_row_in_any_table() {
    let bench = bench();
    bench.config(EVICT_30);
    let aged = bench.place(1, "session-basic.jsonl");
    bench.place(2, "session-recall.jsonl");
    bench.ingest();
    bench.age(&[&aged], AGED);

    let before = bench.snapshot();

    for _ in 0..2 {
        let out = bench.run(&["retention", "--dry-run", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        // The premise: it really did have something to report, or this compares
        // a store nothing was ever going to touch.
        let value = document(&out);
        assert_eq!(strings(&value["data"]["evict"]).len(), 1, "{value}");
    }

    assert_eq!(bench.snapshot(), before, "the dry run wrote something");
}

/// AC2 end to end. An evicted session stays listed and stays searchable with
/// its body flagged; a deleted one is gone from both.
///
/// Two passes and two policies, because one global `action` is one verb: the
/// first evicts the session whose transcript is still there, the second deletes
/// the one Claude Code's own cleanup has already removed (D-02). The evicted
/// session survives the second pass untouched, which is the arm on the ingest
/// path being exercised rather than described.
#[test]
fn an_evicted_session_stays_listed_and_searchable_while_a_deleted_one_is_gone() {
    let bench = bench();
    bench.config(EVICT_30);
    let evicted = bench.place(1, "session-basic.jsonl");
    let doomed = bench.place(2, "session-recall.jsonl");
    bench.ingest();
    // The turns this token matched BEFORE anything was evicted, so "still
    // returns that turn" is a comparison and not a re-reading of the index.
    let matched = bench.search_hits_in(&evicted, testkit::UNIQUE_TOKEN);
    assert!(
        !matched.is_empty(),
        "the fixture has to be searchable to start with"
    );
    bench.age(&[&evicted], AGED);
    bench.age(&[&doomed], FRESH);

    bench.ingest();
    assert_eq!(bench.is_evicted(&evicted), Some(1));
    assert_eq!(bench.is_evicted(&doomed), None);

    // Now the other verb, over the session whose transcript is gone.
    bench.config(DELETE_30);
    bench.age(&[&doomed], AGED);
    std::fs::remove_file(&doomed).unwrap();

    let out = bench.run(&["retention", "--dry-run", "--json"]);
    let value = document(&out);
    assert_eq!(
        strings(&value["data"]["delete"]),
        vec![doomed.to_string_lossy().into_owned()],
        "{value}"
    );
    assert_eq!(
        strings(&value["data"]["evict"]),
        Vec::<String>::new(),
        "a delete policy evicts nothing: {value}"
    );

    bench.ingest();
    assert!(!bench.archived(&doomed), "the deleted session is still there");
    assert!(
        bench.archived(&evicted),
        "the deletion pass took the evicted session with it"
    );
    assert_eq!(
        bench.is_evicted(&evicted),
        Some(1),
        "a later pass un-evicted the session"
    );

    // Still listed.
    let out = bench.run(&["sessions", "--project", "*", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let listed = document(&out).to_string();
    assert!(
        listed.contains(&evicted.to_string_lossy().into_owned()),
        "the evicted session is no longer listed: {listed}"
    );
    assert!(
        !listed.contains(&doomed.to_string_lossy().into_owned()),
        "the deleted session is still listed: {listed}"
    );

    // Still searchable, by the same token, returning the same turns.
    assert_eq!(
        bench.search_hits_in(&evicted, testkit::UNIQUE_TOKEN),
        matched,
        "the evicted session's turns stopped matching"
    );

    // And the body says why it is not there.
    let turn = matched[0].to_string();
    let out = bench.run(&["show", "--project", "*", &turn, "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    let record = &value["data"]["records"][0];
    assert_eq!(record["body_evicted"], true, "{value}");
    assert_eq!(record["body"], serde_json::Value::Null, "{value}");
}

/// A store the policy has nothing to say about is an empty result with a
/// reason, and the reason distinguishes "off" from "configured and not due".
#[test]
fn an_off_policy_and_a_policy_with_nothing_due_say_different_things() {
    let bench = bench();
    bench.place(1, "session-basic.jsonl");
    bench.ingest();

    let out = bench.run(&["retention", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert!(
        value["reason"]
            .as_str()
            .is_some_and(|r| r.contains("retention is off")),
        "{value}"
    );

    bench.config(EVICT_30);
    let out = bench.run(&["retention", "--dry-run", "--json"]);
    let value = document(&out);
    assert!(
        value["reason"]
            .as_str()
            .is_some_and(|r| r.contains("names no session yet")),
        "{value}"
    );
    assert_eq!(strings(&value["data"]["evict"]), Vec::<String>::new());
}

/// A machine that has never ingested is the ordinary starting state: a reason
/// and exit 0, and not one byte created where the store would go.
#[test]
fn against_no_store_it_exits_zero_and_creates_no_data_directory() {
    let bench = bench();

    let out = bench.run(&["retention", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "the reason belongs on stderr");
    assert!(
        stderr(&out).contains("no terminus store"),
        "{}",
        stderr(&out)
    );

    let out = bench.run(&["retention", "--dry-run", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["data"]["cutoff"], serde_json::Value::Null, "{value}");
    assert_eq!(strings(&value["data"]["evict"]), Vec::<String>::new());

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory"
    );
}

/// Without `--json` the list is on stdout, one parseable line per session, and
/// every count is on stderr.
#[test]
fn the_human_output_keeps_the_stream_split() {
    let bench = bench();
    bench.config(EVICT_30);
    let aged = bench.place(1, "session-basic.jsonl");
    bench.ingest();
    bench.age(&[&aged], AGED);

    let out = bench.run(&["retention", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let text = stdout(&out);
    let key = aged.to_string_lossy().into_owned();
    assert!(text.contains(&format!("evict   {key}")), "{text:?}");
    assert!(text.starts_with("cutoff  "), "{text:?}");
    assert!(
        stderr(&out).contains("1 session(s) would be evicted"),
        "{}",
        stderr(&out)
    );
    // The counts are commentary and must not be on the stream a script reads.
    assert!(!text.contains("would be evicted"), "{text:?}");
}
