//! `terminus status` at the process boundary (ING-09).
//!
//! Everything asserted here has to come out of the store: there is no log file
//! by design, so a pass's failures are only ever visible because `runs` kept
//! them and this command prints them.

use std::path::PathBuf;
use std::process::{Command, Output};

use rusqlite::Connection;
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

const PROJECT: &str = "-data-projects-cadence";

/// The same isolation `cli.rs` uses, and for the same reason: once bare
/// `terminus ingest` walks the configured roots, a spawn that sets only
/// `TERMINUS_DATA_DIR` resolves the developer's real config and walks the live
/// `~/.claude` tree. No test process may resolve a real transcript root.
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

    fn place(&self, name: &str, fixture: &str) -> PathBuf {
        let dest = self.claude_dir.join("projects").join(PROJECT).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    /// A one-record transcript whose `cwd` is the caller's, in the project
    /// directory that `cwd` encodes to.
    fn write(&self, n: u8, cwd: &str) -> PathBuf {
        let dir = self
            .claude_dir
            .join("projects")
            .join(terminus_core::config::encode(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", uuid(n)));
        std::fs::write(
            &path,
            format!(
                "{{\"type\":\"user\",\"uuid\":\"cccccccc-0000-4000-8000-0000000000{n:02x}\",\
                  \"timestamp\":\"2026-08-12T10:00:{n:02}.000Z\",\
                  \"sessionId\":\"{n:08x}-2222-4222-8222-222222222222\",\"cwd\":\"{cwd}\",\
                  \"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hi\"}}]}}}}\n"
            ),
        )
        .unwrap();
        path.canonicalize().unwrap()
    }

    /// Write `terminus.toml` into the config directory this bench points the
    /// binary at.
    fn config(&self, text: &str) {
        std::fs::write(self.config_dir.join("terminus.toml"), text).unwrap();
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

fn uuid(n: u8) -> String {
    format!("{n:08x}-1111-4111-8111-111111111111")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// An empty store is exit 0 with zeroes, not an error: it is what a machine
/// looks like before the first hook has ever fired.
#[test]
fn status_on_an_empty_data_directory_exits_zero_with_zeroes() {
    let bench = bench();
    let out = bench.run(&["status"]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("sessions       0"), "{text}");
    assert!(text.contains("turns          0"), "{text}");
    assert!(text.contains("last run       none"), "{text}");
    assert!(
        text.contains(DB_FILE_NAME),
        "the store path must be named: {text}"
    );
}

/// After a tree pass: the counts and the last-run line are the store's own
/// numbers, matched against the `runs` row they were read from.
#[test]
fn status_reports_the_counts_and_the_last_runs_numbers() {
    let bench = bench();
    for (n, fixture) in [
        "session-basic.jsonl",
        "session-large-record.jsonl",
        "session-continuation.jsonl",
        "subagents/agent-alpha.jsonl",
        "subagents/workflows/wf_demo/agent-deep.jsonl",
    ]
    .into_iter()
    .enumerate()
    {
        bench.place(&format!("{}.jsonl", uuid(n as u8 + 1)), fixture);
    }

    let ingest = bench.run(&["ingest"]);
    assert_eq!(ingest.status.code(), Some(0), "{}", stderr(&ingest));

    let conn = bench.conn();
    let sessions: i64 = conn
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    let turns: i64 = conn
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    let (seen, committed, failed, bytes, added): (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT files_seen, files_committed, files_failed, bytes_read, turns_added
             FROM runs ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(sessions, 5, "the tree pass archived five transcripts");

    let out = bench.run(&["status"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);

    assert!(
        text.contains(&format!("sessions       {sessions}")),
        "{text}"
    );
    assert!(text.contains(&format!("turns          {turns}")), "{text}");
    assert!(
        text.contains(&format!(
            "files        {seen} walked, {committed} committed, {failed} failed"
        )),
        "{text}"
    );
    assert!(text.contains(&format!("bytes read   {bytes}")), "{text}");
    assert!(text.contains(&format!("turns added  {added}")), "{text}");
    assert!(
        text.contains(&format!("watermarks     {sessions} covering")),
        "every archived transcript carries a watermark: {text}"
    );

    // Size comes off the files on disk, so it cannot be zero here.
    assert!(!text.contains("size           0 byte(s)"), "{text}");
}

/// AC7's reporting half. A tree with one damaged transcript: the pass still
/// ingests the rest, and `status` names that file and its error.
#[test]
fn status_prints_the_damaged_files_path_and_error_from_the_last_run() {
    let bench = bench();
    let shrunk = bench.place(&format!("{}.jsonl", uuid(1)), "session-basic.jsonl");

    // Archive it whole, then leave it shorter than its watermark - a transcript
    // that was replaced under an archive that already holds more of it.
    assert_eq!(bench.run(&["ingest"]).status.code(), Some(0));
    std::fs::write(&shrunk, b"{}\n").unwrap();

    // A healthy transcript that appears only now, so the second pass has one
    // file to commit beside the one it must skip.
    bench.place(&format!("{}.jsonl", uuid(2)), "session-continuation.jsonl");

    let ingest = bench.run(&["ingest"]);
    assert_eq!(
        ingest.status.code(),
        Some(0),
        "a skipped file is not a failed pass: {}",
        stderr(&ingest)
    );
    assert!(
        stderr(&ingest).contains(shrunk.to_str().unwrap()),
        "the skipped file must be named on stderr: {}",
        stderr(&ingest)
    );

    let out = bench.run(&["status"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);

    assert!(
        text.contains("  error"),
        "the last run carried no error: {text}"
    );
    assert!(
        text.contains(shrunk.to_str().unwrap()),
        "status must name the damaged transcript: {text}"
    );
    assert!(
        text.contains("watermark"),
        "status must print the error in full: {text}"
    );
    assert!(
        text.contains("files        2 walked, 1 committed, 1 failed"),
        "{text}"
    );

    // The other transcript is still archived: one damaged file did not take
    // the tree down with it.
    let sessions: i64 = bench
        .conn()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 2);
}

/// D-24: `status --json` emits the same numbers the human output prints, as one
/// document, and any OTHER argument is still misuse.
///
/// The numbers are read out of the store independently and compared against the
/// document rather than against the prose, because two accounts of one store
/// that can disagree is the whole reason the retrofit landed in the phase that
/// owns the contract rather than beside install in phase 4.
#[test]
fn status_json_carries_the_same_numbers_as_the_human_output() {
    let bench = bench();
    bench.place(&format!("{}.jsonl", uuid(1)), "session-basic.jsonl");
    bench.place(&format!("{}.jsonl", uuid(2)), "session-large-record.jsonl");
    bench.config("exclude = [\"/data/projects/nowhere\"]\n");
    let ingest = bench.run(&["ingest"]);
    assert_eq!(ingest.status.code(), Some(0), "{}", stderr(&ingest));

    let conn = bench.conn();
    let sessions: i64 = conn
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    let turns: i64 = conn
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    let started: String = conn
        .query_row(
            "SELECT started_at FROM runs ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let out = bench.run(&["status", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let text = stdout(&out);
    let mut lines = text.lines();
    let document: serde_json::Value = serde_json::from_str(lines.next().expect("a document"))
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {text:?}"));
    assert_eq!(lines.next(), None, "stdout carried prose too: {text:?}");

    assert_eq!(document["command"], "status");
    assert_eq!(document["ok"], true);
    let data = &document["data"];
    assert_eq!(data["sessions"], sessions);
    assert_eq!(data["turns"], turns);
    assert_eq!(data["watermarks"], sessions);
    assert!(data["watermark_bytes"].as_i64().unwrap() > 0, "{document}");
    assert!(data["size_bytes"].as_u64().unwrap() > 0, "{document}");
    assert!(
        data["store"].as_str().unwrap().ends_with(DB_FILE_NAME),
        "{document}"
    );
    assert_eq!(
        data["excluded"],
        serde_json::json!(["/data/projects/nowhere"])
    );
    assert_eq!(data["last_run"]["started_at"], started);
    assert_eq!(data["last_run"]["error"], serde_json::Value::Null);

    // The human run is unchanged, and says the same things.
    let human = stdout(&bench.run(&["status"]));
    assert!(
        human.contains(&format!("sessions       {sessions}")),
        "{human}"
    );
    assert!(
        human.contains(&format!("turns          {turns}")),
        "{human}"
    );
    assert!(
        !human.starts_with('{'),
        "the plain run emitted JSON: {human}"
    );

    // Any other argument is still misuse.
    let out = bench.run(&["status", "extra"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
}

/// An empty store's document is zeroes and a null last run, not an error.
#[test]
fn status_json_on_an_empty_data_directory_is_zeroes_and_a_null_run() {
    let bench = bench();
    let out = bench.run(&["status", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let document: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(document["ok"], true);
    assert_eq!(document["data"]["sessions"], 0);
    assert_eq!(document["data"]["turns"], 0);
    assert_eq!(document["data"]["last_run"], serde_json::Value::Null);
    assert_eq!(document["data"]["excluded"], serde_json::json!([]));
}

/// `status` takes no ingest lock: reading while a pass runs must work, which is
/// what WAL is on for.
#[test]
fn status_reads_while_the_ingest_lock_is_held() {
    let bench = bench();
    bench.place(&format!("{}.jsonl", uuid(1)), "session-basic.jsonl");
    assert_eq!(bench.run(&["ingest"]).status.code(), Some(0));

    let guard = match terminus_core::ingest::lock::try_acquire(&bench.data_dir).unwrap() {
        terminus_core::ingest::Attempt::Acquired(guard) => guard,
        other => panic!("the lock should have been free: {other:?}"),
    };
    let out = bench.run(&["status"]);
    drop(guard);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("sessions       1"),
        "{}",
        stdout(&out)
    );
}

/// ING-08's read half at the process boundary. Two projects are archived with
/// no exclusions configured; one is excluded afterwards and `status` stops
/// counting it, without a re-ingest and without the rows leaving the store.
///
/// This is the case a flag written at ingest could never cover, and the
/// behaviour `.planning/PROJECT.md` names the incumbent for: exclusion honored
/// on write and ignored on read.
#[test]
fn status_stops_counting_a_project_excluded_after_it_was_archived() {
    let bench = bench();
    let hidden = bench.write(1, "/data/projects/hidden");
    bench.write(2, "/data/projects/kept");
    // Its encoded name extends the excluded one, so it must stay visible.
    bench.write(3, "/data/projects/hidden-research");

    assert_eq!(bench.run(&["ingest"]).status.code(), Some(0));
    let text = stdout(&bench.run(&["status"]));
    assert!(text.contains("sessions       3"), "{text}");
    assert!(!text.contains("excluded"), "{text}");

    let turns_before: i64 = bench
        .conn()
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();

    bench.config("exclude = [\"/data/projects/hidden\"]\n");

    let out = bench.run(&["status"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("sessions       2"), "{text}");
    assert!(
        text.contains(&format!("turns          {}", turns_before - 1)),
        "the excluded project's turns went with it: {text}"
    );
    assert!(
        text.contains("excluded       1 project(s): /data/projects/hidden"),
        "a count that drops without explanation is a bug report: {text}"
    );

    // Hidden on read, still archived: nothing was deleted and no re-ingest ran.
    let rows: i64 = bench
        .conn()
        .query_row(
            "SELECT count(*) FROM session_meta WHERE session_key = ?1",
            [hidden.to_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "exclusion hides a session, it does not delete it");
}
