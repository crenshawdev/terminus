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

/// A turn that read a file, which is what leaves a `path` entity behind.
fn read(path: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_use",
        "id": "toolu_read",
        "name": "Read",
        "input": {"file_path": path},
    })
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

    /// The id of one archived turn, by the session it is in and its ordinal.
    fn turn(&self, session: &str, seq: i64) -> i64 {
        self.conn()
            .query_row(
                "SELECT t.id FROM turns t
                   JOIN session_meta m ON m.session_key = t.session_key
                  WHERE m.session_id = ?1 AND t.turn_seq = ?2",
                rusqlite::params![session, seq],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("no turn {seq} of {session}: {e}"))
    }

    /// One logged decision, written straight into the table the drain fills.
    ///
    /// Hand-built rather than driven through the prompt path: a decision has to
    /// name turn ids that already exist and a wall clock the test chooses, and
    /// the injector reads a real clock and picks its own turns. What the drain
    /// produces is `feedback.rs`'s subject; what labelling makes of it is this
    /// file's.
    fn decide(
        &self,
        session: &str,
        at: &str,
        injected: &[(i64, usize)],
        chars: i64,
        spellings: &[&str],
    ) -> i64 {
        let injected: Vec<serde_json::Value> = injected
            .iter()
            .map(|(turn_id, chars)| serde_json::json!({"turn_id": turn_id, "chars": chars}))
            .collect();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO decisions (
                session_id, ts, cwd, prompt, watermark_session_no, chars_injected,
                spellings, candidates, injected, suppressed, thresholds
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, '[]', ?7, '[]', '{}')",
            rusqlite::params![
                session,
                at,
                self.root.join("project-alpha").to_string_lossy(),
                "what changed",
                chars,
                serde_json::to_string(spellings).unwrap(),
                serde_json::to_string(&injected).unwrap(),
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// Every label one decision carries, in the order they were written.
    fn labels_of(&self, decision: i64) -> Vec<(String, Option<i64>, Option<String>)> {
        self.conn()
            .prepare(
                "SELECT label, turn_id, detail FROM labels
                  WHERE decision_id = ?1 ORDER BY id",
            )
            .unwrap()
            .query_map([decision], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
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

/// D-07: the four labels, off a session whose turns the test wrote and whose
/// decisions the test placed against a clock it chose.
///
/// The two injected turns of the first decision differ in exactly one thing -
/// whether a later turn of the same session names the file they named - so the
/// `hit`/`false positive` split is attributable to the rule and to nothing else.
#[test]
fn a_finalized_decision_is_labelled_by_what_the_session_did_next() {
    let bench = bench();
    bench.transcript(
        IDLE,
        "project-alpha",
        &[
            // Before the decision: the turn it will inject and be right about,
            // and the turn it will inject and be wrong about.
            (ts("-480 minutes"), read("crates/gizmo/lantern.rs")),
            (ts("-480 minutes"), read("docs/ORPHAN.md")),
            // After it: the same file again, and never the other one.
            (ts("-450 minutes"), read("crates/gizmo/lantern.rs")),
        ],
    );
    bench.transcript(
        LIVE,
        "project-alpha",
        &[
            (ts("-10 minutes"), read("crates/gizmo/lantern.rs")),
            (ts("-5 minutes"), read("crates/gizmo/lantern.rs")),
        ],
    );

    // The turns have to be in the store before a decision can name their ids.
    let archived = bench.pass();
    assert_eq!(archived.files_committed, 2, "{archived:?}");
    assert_eq!(archived.outcomes.finalized, 1, "{archived:?}");
    assert!(
        archived.outcomes.labels.is_empty(),
        "nothing is logged yet: {archived:?}"
    );

    let used = bench.turn(IDLE, 0);
    let ignored = bench.turn(IDLE, 1);
    let at = ts("-465 minutes");
    let mixed = bench.decide(IDLE, &at, &[(used, 100), (ignored, 50)], 150, &[]);
    let wasteful = bench.decide(IDLE, &at, &[(ignored, 50)], 50, &[]);
    // Same shape, on a session that is still being typed into.
    let open = bench.decide(LIVE, &ts("-8 minutes"), &[(bench.turn(LIVE, 0), 40)], 40, &[]);

    let summary = bench.pass();
    assert!(summary.outcomes.notes.is_empty(), "{summary:?}");
    assert_eq!(summary.outcomes.count("hit"), 1, "{summary:?}");
    assert_eq!(summary.outcomes.count("false positive"), 2, "{summary:?}");
    assert_eq!(summary.outcomes.count("wasted budget"), 1, "{summary:?}");
    assert_eq!(summary.outcomes.count("miss"), 0, "{summary:?}");

    assert_eq!(
        bench.labels_of(mixed),
        vec![
            ("hit".to_owned(), Some(used), None),
            ("false positive".to_owned(), Some(ignored), None),
        ]
    );
    // Every injected turn unreferenced and characters spent all the same: the
    // decision cost something and bought nothing, and the detail is what it
    // cost.
    assert_eq!(
        bench.labels_of(wasteful),
        vec![
            ("false positive".to_owned(), Some(ignored), None),
            ("wasted budget".to_owned(), None, Some("50".to_owned())),
        ]
    );
    // The live session's decision is not judged: its downstream turns have not
    // finished arriving, so every label it could be given now is a guess.
    assert_eq!(bench.labels_of(open), Vec::new());

    // Incremental: the second pass finds every eligible decision already
    // labelled and writes nothing.
    let again = bench.pass();
    assert!(again.outcomes.labels.is_empty(), "{again:?}");
    assert_eq!(bench.labels_of(mixed).len(), 2);
    assert_eq!(bench.labels_of(wasteful).len(), 2);
    assert_eq!(bench.labels_of(open), Vec::new());
}
