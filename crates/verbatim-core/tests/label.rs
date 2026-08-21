//! FEED-02: what a logged decision turned out to be worth.
//!
//! Two steps run at the far end of every ingest pass, after the walk. The idle
//! rule decides which sessions are over (D-06) - there is no event that says so,
//! since `SessionEnd` does not fire on a crash - and the label join then asks,
//! entirely in SQL over `decisions`, `entities` and `turns`, whether the turns
//! that followed a decision ever used what it injected (D-07).
//!
//! Every test here drives `pass::run_with` over a real transcript tree it wrote
//! itself, because the ordering is half of what is under test: a labelling step
//! that ran before the walk would judge each decision against the archive as it
//! stood before the turns that answer it arrived.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::config::Config;
use verbatim_core::ingest::pass::{self, PassOutcome, Summary};
use verbatim_core::store::DB_FILE_NAME;

/// The session that went quiet before the pass, and the one still being typed.
const IDLE: &str = "11111111-1111-4111-8111-111111111111";
const LIVE: &str = "22222222-2222-4222-8222-222222222222";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    /// The Claude config directory whose `projects` tree a pass walks.
    claude: PathBuf,
    /// Where the transcripts' `cwd` values point. Somewhere the test owns, so a
    /// project key is whatever this built and nothing a real repository decided.
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude = dir.path().join("claude");
    let root = dir.path().join("root");
    std::fs::create_dir_all(claude.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        claude,
        root,
    }
}

/// A UTC timestamp `offset` from now, in the shape a transcript writes.
///
/// SQLite computes it because SQLite is what the idle rule compares against: a
/// formatter written here could differ from the one under test in exactly the
/// way that would make the comparison pass for the wrong reason.
fn ts(offset: &str) -> String {
    Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
            [offset],
            |r| r.get(0),
        )
        .unwrap()
}

/// A turn that said something and touched nothing.
fn text(said: &str) -> serde_json::Value {
    serde_json::json!({"type": "text", "text": said})
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn config(&self) -> Config {
        Config::from_parts(vec![self.claude.clone()], Vec::new())
    }

    /// One whole pass over the tree: drain, walk, finalize, label.
    fn pass(&self) -> Summary {
        match pass::run_with(&self.data_dir, &self.config()).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    /// Write one transcript into the tree a pass walks.
    ///
    /// Whole-file, so a caller that appends turns just calls it again with a
    /// longer list: ingest tails from its watermark, which is what makes a
    /// second pass see only the new records.
    fn transcript(&self, session: &str, project: &str, turns: &[(String, serde_json::Value)]) {
        let cwd = self.root.join(project);
        std::fs::create_dir_all(&cwd).unwrap();
        let dir = self.claude.join("projects").join(project);
        std::fs::create_dir_all(&dir).unwrap();

        let mut body = String::new();
        for (n, (at, content)) in turns.iter().enumerate() {
            let record = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": cwd.to_string_lossy(),
                "sessionId": session,
                "type": "assistant",
                "uuid": format!("{session}-{n}"),
                "timestamp": at,
                "requestId": format!("req_{n}"),
                "message": {
                    "role": "assistant",
                    "model": "claude-opus-5",
                    "content": [content],
                },
            });
            body.push_str(&record.to_string());
            body.push('\n');
        }
        std::fs::write(dir.join(format!("{session}.jsonl")), body).unwrap();
    }

    /// The `is_final` flag of one session, as stored: `Some(1)` or `None`.
    fn is_final(&self, session: &str) -> Option<i64> {
        self.conn()
            .query_row(
                "SELECT is_final FROM session_meta WHERE session_id = ?1",
                [session],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("no session_meta row for {session}: {e}"))
    }
}

/// D-06: a pass closes the sessions that have gone quiet, leaves the live one
/// open, and does not touch a session it already closed.
///
/// This is the only thing in the product that writes `is_final`, so the live
/// session's `NULL` is as load-bearing as the idle session's `1`: a rule that
/// closed everything would label every decision of every open session against a
/// transcript that has not finished arriving.
#[test]
fn the_idle_rule_closes_a_quiet_session_and_leaves_a_live_one_open() {
    let bench = bench();
    bench.transcript(
        IDLE,
        "project-alpha",
        &[(ts("-7 hours"), text("the retry budget moved to three"))],
    );
    bench.transcript(
        LIVE,
        "project-alpha",
        &[(ts("-5 minutes"), text("still working on the retry budget"))],
    );

    let summary = bench.pass();
    assert_eq!(summary.files_committed, 2, "{summary:?}");
    assert_eq!(summary.outcomes.finalized, 1, "{summary:?}");
    assert!(summary.outcomes.notes.is_empty(), "{summary:?}");
    assert_eq!(bench.is_final(IDLE), Some(1));
    assert_eq!(bench.is_final(LIVE), None);

    // A second pass has nothing new to walk and nothing new to close: the
    // statement is incremental, so an already-closed session is not rewritten
    // and a live one does not drift shut.
    let again = bench.pass();
    assert_eq!(again.files_committed, 0, "{again:?}");
    assert_eq!(again.outcomes.finalized, 0, "{again:?}");
    assert_eq!(bench.is_final(IDLE), Some(1));
    assert_eq!(bench.is_final(LIVE), None);
}
