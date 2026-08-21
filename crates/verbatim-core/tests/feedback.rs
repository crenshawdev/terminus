//! The decision log: what a prompt writes down, the table that survives a
//! rebuild, and the drain that fills it (FEED-01).
//!
//! `decisions` is the one derived-looking table that is not derived. A prompt's
//! decision is state that existed for a few milliseconds inside a hook - which
//! spellings it extracted, what the index answered, which thresholds were in
//! force - and no amount of replaying the session blobs can reconstruct it, so
//! `reindex` must give the rows back untouched (D-03).
//!
//! The record-writing tests live here rather than in `inject_prompt.rs` beside
//! the rest of the prompt arm: they are about the log, they run against the same
//! fixture through the same public entry point, and PLAN-2's labelling and
//! PLAN-3's replay extend exactly this file.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::types::Value;
use rusqlite::Connection;
use verbatim_core::config::Config;
use verbatim_core::feedback::replay::{self, Replayed};
use verbatim_core::ingest::pass::{self, PassOutcome, Summary};
use verbatim_core::inject::decision::{self, Decision};
use verbatim_core::inject::prompt::Thresholds;
use verbatim_core::inject::state::Reason;
use verbatim_core::inject::{prompt, Payload};
use verbatim_core::recall::{search, Query, Request, Scope};
use verbatim_core::store::{schema, Store, DB_FILE_NAME};
use verbatim_core::{ingest, reindex, testkit};

/// The fixture that stores an absolute path, and the relative spelling of that
/// same file beneath its project - the same pair `inject_prompt.rs` uses, so a
/// prompt asserted about there is the prompt logged about here.
const FIXTURE: &str = "session-edits.jsonl";
const RELATIVE: &str = "crates/gizmo/lantern.rs";

/// The `session_id` a payload carries unless the test names another.
const SESSION: &str = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";

/// The session id the edits fixture's own records carry.
///
/// A decision is only ever labelled - and so only ever replayed - against a
/// session the archive holds and has closed (`label::ELIGIBLE`), and a decision
/// belongs to a session by `session_id` alone (D-04). So a test about labels
/// submits under this id and not under [`SESSION`].
const FIXTURE_SESSION: &str = "44444444-4444-4444-8444-444444444444";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    root: PathBuf,
}

/// A store holding the edits fixture, ingested the ordinary way.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();

    let path = testkit::copy_rooted_fixture_into(FIXTURE, &work, &root);
    match ingest::run(&data_dir, &path).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("{FIXTURE}: {other:?}"),
    }

    Bench {
        _dir: dir,
        data_dir,
        work,
        root,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// The directory the fixture's `cwd` names: what a payload carries.
    fn project(&self) -> String {
        testkit::fixture_project(FIXTURE, &self.root)
            .to_string_lossy()
            .into_owned()
    }

    /// The one `path` entity value the fixture stored, read back out of the
    /// store rather than composed here: what the query has to match is what
    /// ingest actually wrote.
    fn stored_path(&self) -> String {
        let values: Vec<String> = self
            .conn()
            .prepare("SELECT DISTINCT value_norm FROM entities WHERE kind = 'path'")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(values.len(), 1, "{values:?}");
        values.into_iter().next().unwrap()
    }

    /// What a `UserPromptSubmit` in this project carries.
    fn payload<'a>(&self, cwd: &'a str, prompt: &'a str) -> Payload<'a> {
        Payload {
            session_id: Some(SESSION),
            transcript_path: None,
            cwd: Some(cwd),
            prompt: Some(prompt),
            source: None,
        }
    }

    /// One prompt through the arm the hook calls - which is the only arm that
    /// persists a record.
    fn submit(&self, cwd: &str, prompt: &str) -> Option<String> {
        prompt::user_prompt_submit(
            &self.data_dir,
            &Config::default(),
            &self.payload(cwd, prompt),
        )
    }

    /// The same, under a session id the archive actually holds.
    fn submit_as(&self, session_id: &str, cwd: &str, prompt: &str) -> Option<String> {
        prompt::user_prompt_submit(
            &self.data_dir,
            &Config::default(),
            &Payload {
                session_id: Some(session_id),
                transcript_path: None,
                cwd: Some(cwd),
                prompt: Some(prompt),
                source: None,
            },
        )
    }

    /// The decision records this data directory holds, oldest first.
    ///
    /// Every one of them must parse: a record this build wrote and cannot read
    /// back is the failure the format field exists to make visible, and a helper
    /// that skipped it would hide it.
    fn records(&self) -> Vec<Decision> {
        decision::read_all(&self.data_dir)
            .into_iter()
            .map(|found| {
                found
                    .decision
                    .unwrap_or_else(|| panic!("{} did not parse", found.path.display()))
            })
            .collect()
    }

    /// The one decision this prompt left behind.
    fn record_of(&self, prompt: &str) -> Decision {
        let mut matching: Vec<Decision> = self
            .records()
            .into_iter()
            .filter(|record| record.prompt == prompt)
            .collect();
        assert_eq!(matching.len(), 1, "{prompt:?}: {matching:?}");
        matching.pop().unwrap()
    }

    /// One tree pass over a transcript root that holds nothing.
    ///
    /// The fixtures are archived through `ingest::run` directly, so the walk has
    /// nothing to do and every row this pass writes came out of the decision
    /// log - which is what the assertions are about.
    fn pass(&self) -> Summary {
        let claude = self._dir.path().join("claude");
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        let config = Config::from_parts(vec![claude], Vec::new());
        match pass::run_with(&self.data_dir, &config).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    /// Every decision file currently on disk.
    fn files(&self) -> Vec<PathBuf> {
        decision::read_all(&self.data_dir)
            .into_iter()
            .map(|found| found.path)
            .collect()
    }

    /// The highest `sessions.session_no` this store holds: the watermark a
    /// decision records (D-10).
    fn watermark(&self) -> i64 {
        self.conn()
            .query_row("SELECT max(session_no) FROM sessions", [], |r| r.get(0))
            .unwrap()
    }

    /// Archive one session of hand-written records through the ordinary ingest
    /// path, under a project directory of its own.
    fn archive(&self, name: &str, project: &str, records: Vec<serde_json::Value>) {
        let cwd = self.root.join(project);
        let session = format!("{:0>8}-0000-4000-8000-000000000000", name.len());
        let mut body = String::new();
        for (n, content) in records.into_iter().enumerate() {
            let record = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": cwd.to_string_lossy(),
                "sessionId": session,
                "type": "assistant",
                "uuid": format!("{session}-{n}"),
                "timestamp": format!("2026-08-14T10:{:02}:{:02}.000Z", n / 60, n % 60),
                "requestId": format!("req_{n}"),
                "message": {"role": "assistant", "model": "claude-opus-5", "content": [content]},
            });
            body.push_str(&record.to_string());
            body.push('\n');
        }

        let path = self.work.join(name);
        std::fs::write(&path, body).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{name}: {other:?}"),
        }
    }
}

/// FEED-01: a prompt that injected something says what it injected, what the
/// candidates matched on, what it spent and under which thresholds.
///
/// The threshold values are asserted as literals on purpose. They are
/// compile-time constants a build may change (D-08), and the record is the only
/// place a later analysis can learn which numbers produced a label - so a change
/// to one of them has to be a deliberate edit here rather than a silently
/// re-derived expectation.
#[test]
fn a_firing_prompt_records_what_it_injected() {
    let bench = bench();
    let cwd = bench.project();
    let stored = bench.stored_path();
    let raw = format!("what changed in {RELATIVE}");

    let text = bench
        .submit(&cwd, &raw)
        .expect("the prompt names a path the archive stored");

    let record = bench.record_of(&raw);
    assert_eq!(record.session_id.as_deref(), Some(SESSION));
    assert_eq!(record.cwd.as_deref(), Some(cwd.as_str()));
    assert!(record.at_ms > 0, "no wall clock on the anchor: {record:?}");

    // D-10: the store was opened, so the bound was taken here rather than left
    // for the drain.
    let sessions: i64 = bench
        .conn()
        .query_row("SELECT max(session_no) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(record.watermark_session_no, Some(sessions));

    // What the prompt asked the archive about: the resolved absolute spelling
    // is in there, which is what D-05 is for.
    assert!(
        record.spellings.contains(&stored),
        "the resolved spelling is not in the record: {:?}",
        record.spellings
    );

    let injected: Vec<i64> = record.injected.iter().map(|i| i.turn_id).collect();
    assert_eq!(injected.len(), 1, "{record:?}");
    assert!(record.injected[0].chars > 0, "{record:?}");
    assert_eq!(
        record.chars_injected,
        text.chars().count(),
        "the record's total is not what was emitted"
    );

    let candidate = record
        .candidates
        .iter()
        .find(|candidate| candidate.turn_id == injected[0])
        .expect("the injected turn is not among the scored candidates");
    assert_eq!(
        candidate.matched_on.len(),
        candidate.entity_count,
        "{candidate:?}"
    );
    assert!(
        candidate
            .matched_on
            .iter()
            .any(|matched| matched.kind == "path" && matched.value == stored),
        "the candidate does not say it matched the stored path: {candidate:?}"
    );
    assert!(candidate.relevance > 0.0 && candidate.entity_score > 0.0);

    assert_eq!(record.thresholds.ranked, 10);
    assert_eq!(record.thresholds.compacted_ranked, 50);
    assert_eq!(record.thresholds.entity_rank, 3);
    assert_eq!(record.thresholds.co_occurring, 2);
    assert_eq!(record.thresholds.max_turns, prompt::MAX_TURNS);
    assert_eq!(record.thresholds.max_candidates, 8);
    assert_eq!(record.thresholds.prompt_chars, 4_000);

    assert!(record.suppressed.is_empty(), "{record:?}");
    assert!(!record.compacted, "{record:?}");
}

/// D-11's two non-fires, which are different facts and must read differently.
///
/// One reached the index and was refused by the threshold; the other never
/// opened the store at all. A log that could not tell them apart would put every
/// prompt a person types into one bucket, and the miss data lives in exactly
/// that difference.
#[test]
fn a_non_fire_records_how_far_it_got() {
    let bench = bench();
    let cwd = bench.project();
    bench.archive(
        "prose.jsonl",
        "project-alpha",
        vec![serde_json::json!({
            "type": "text",
            "text": "the widgetFactory helper is still the one we settled on",
        })],
    );

    // Refused by the threshold: a candidate, a scored hit, and nothing injected.
    let refused = "is widgetFactory still the one we settled on";
    assert_eq!(bench.submit(&cwd, refused), None);
    let record = bench.record_of(refused);
    assert!(
        record.spellings.contains(&"widgetFactory".to_owned()),
        "{record:?}"
    );
    assert!(record.watermark_session_no.is_some(), "{record:?}");
    assert_eq!(record.candidates.len(), 1, "{record:?}");
    assert_eq!(
        record.candidates[0].entity_count, 0,
        "the hit is not free-text-only, so the threshold is not what refused \
         it: {record:?}"
    );
    assert!(record.candidates[0].matched_on.is_empty(), "{record:?}");
    assert!(record.injected.is_empty(), "{record:?}");
    assert_eq!(record.chars_injected, 0);

    // Nothing path-shaped and nothing identifier-shaped: the store was never
    // opened, so there is no watermark to stamp and no candidate to score.
    let bare = "what changed";
    assert_eq!(bench.submit(&cwd, bare), None);
    let record = bench.record_of(bare);
    assert!(record.spellings.is_empty(), "{record:?}");
    assert!(record.candidates.is_empty(), "{record:?}");
    assert_eq!(
        record.watermark_session_no, None,
        "a prompt that never opened the store stamped a watermark: {record:?}"
    );
    assert_eq!(record.chars_injected, 0);
    // Written all the same: this is the row a miss is counted against.
    assert_eq!(record.prompt, bare);
}

/// A prompt whose every candidate was refused by INJ-04 records the refusals
/// with their reasons.
///
/// The record's list is THIS prompt's, which is what the session state file's
/// cannot be: that one is cumulative and capped, so the second prompt below
/// would inherit the first prompt's refusals if the record were read back out of
/// it.
#[test]
fn a_suppressed_prompt_records_the_refusals_and_why() {
    let bench = bench();
    let cwd = bench.project();
    let raw = format!("what changed in {RELATIVE}");

    assert!(
        bench.submit(&cwd, &raw).is_some(),
        "the first prompt has to inject something"
    );
    // The same session and the same question: INJ-04 refuses what it was
    // already given.
    assert_eq!(
        bench.submit(&cwd, &raw),
        None,
        "the turn was injected twice"
    );

    let records: Vec<Decision> = bench
        .records()
        .into_iter()
        .filter(|record| record.prompt == raw)
        .collect();
    assert_eq!(records.len(), 2, "{records:?}");
    let injected = records[0].injected[0].turn_id;

    let refused = &records[1];
    assert!(refused.injected.is_empty(), "{refused:?}");
    assert_eq!(refused.chars_injected, 0);
    assert_eq!(refused.suppressed.len(), 1, "{refused:?}");
    assert_eq!(refused.suppressed[0].turn_id, injected);
    assert_eq!(refused.suppressed[0].reason, Reason::AlreadyInjected);
    // The candidate is still there and still scored: what changed is the
    // decision, not what the index answered.
    assert!(
        refused
            .candidates
            .iter()
            .any(|candidate| candidate.turn_id == injected),
        "{refused:?}"
    );
}

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

/// One `decisions` row, as the columns a test reads off it.
fn row_of(conn: &Connection, prompt: &str) -> (String, Option<i64>, i64, String, String, String) {
    conn.query_row(
        "SELECT ts, watermark_session_no, chars_injected, spellings, candidates, injected
         FROM decisions WHERE prompt = ?1",
        [prompt],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .unwrap_or_else(|e| panic!("no decisions row for {prompt:?}: {e}"))
}

/// D-01's other half: the files the prompt path wrote become rows on the next
/// pass, and stop being files.
///
/// The pass is the whole entry point under test. `ingest::run`'s single-file
/// path deliberately does not drain - the hook only ever spawns the pass - so a
/// test that called the library's drain directly would prove nothing about what
/// a real machine does.
#[test]
fn a_pass_drains_every_decision_file_into_the_table() {
    let bench = bench();
    let cwd = bench.project();
    let fired = format!("what changed in {RELATIVE}");
    let bare = "what changed";

    let text = bench.submit(&cwd, &fired).expect("the prompt injects");
    assert_eq!(bench.submit(&cwd, bare), None);
    assert_eq!(bench.files().len(), 2, "the premise: two records on disk");

    let summary = bench.pass();
    assert_eq!(summary.feedback.decisions, 2, "{summary:?}");
    assert!(summary.feedback.discarded.is_empty(), "{summary:?}");
    assert_eq!(summary.files_committed, 0, "the walk had nothing to do");
    assert!(
        bench.files().is_empty(),
        "the drained files are still there: {:?}",
        bench.files()
    );

    let conn = bench.conn();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM decisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 2);

    let (ts, watermark, chars, spellings, candidates, injected) = row_of(&conn, &fired);
    // The archive's own timestamp shape, produced by SQLite from the unix
    // milliseconds the prompt path recorded - so a decision sorts and compares
    // against `turns.ts` directly.
    assert_eq!(ts.len(), 24, "{ts}");
    assert!(ts.starts_with("20") && ts.ends_with('Z'), "{ts}");
    let sessions: i64 = conn
        .query_row("SELECT max(session_no) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(watermark, Some(sessions));
    assert_eq!(chars, text.chars().count() as i64);

    // The list-shaped columns are JSON documents and arrived whole.
    let spellings: Vec<String> = serde_json::from_str(&spellings).unwrap();
    assert!(
        spellings.iter().any(|s| s.ends_with(RELATIVE)),
        "{spellings:?}"
    );
    let candidates: serde_json::Value = serde_json::from_str(&candidates).unwrap();
    assert!(
        candidates[0]["matched_on"][0]["kind"] == "path",
        "the matched pairs did not survive the drain: {candidates}"
    );
    let injected: serde_json::Value = serde_json::from_str(&injected).unwrap();
    assert!(injected[0]["chars"].as_i64().unwrap() > 0, "{injected}");

    // D-10: the prompt that never opened the store had no bound of its own, so
    // the drain stamped the archive as it stood before this pass walked.
    let (_, stamped, chars, spellings, _, _) = row_of(&conn, bare);
    assert_eq!(stamped, Some(sessions));
    assert_eq!(chars, 0);
    assert_eq!(spellings, "[]");

    // A second pass has nothing left to move, and moves nothing.
    let again = bench.pass();
    assert_eq!(again.feedback.decisions, 0, "{again:?}");
    let rows: i64 = bench
        .conn()
        .query_row("SELECT count(*) FROM decisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 2, "the second pass inserted a decision twice");
}

/// A file that is not a record this build can read is removed and named, and
/// the pass commits everything else.
///
/// Named because there is no log file: the note has to reach `runs.error`, which
/// is what `verbatim status` surfaces. Removed because there is no migration -
/// leaving it would mean re-reading and re-reporting it on every pass forever.
#[test]
fn a_file_that_is_not_a_record_is_removed_and_named() {
    let bench = bench();
    let cwd = bench.project();
    let fired = format!("what changed in {RELATIVE}");
    assert!(bench.submit(&cwd, &fired).is_some());

    let garbage = bench
        .data_dir
        .join(decision::DIR_NAME)
        .join("99999999-9999-4999-8999-999999999999-0-0-0.json");
    std::fs::write(
        &garbage,
        b"{\"format\": 99, \"prompt\": \"from the future\"}",
    )
    .unwrap();

    let summary = bench.pass();
    assert_eq!(summary.feedback.decisions, 1, "{summary:?}");
    assert_eq!(summary.feedback.discarded.len(), 1, "{summary:?}");
    assert_eq!(summary.feedback.discarded[0].0, garbage);
    assert!(!garbage.exists(), "the unreadable file is still there");
    assert!(bench.files().is_empty());

    let conn = bench.conn();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM decisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "the good record did not commit");

    // The one textual channel this product has.
    let error: Option<String> = conn
        .query_row("SELECT error FROM runs ORDER BY id DESC LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let error = error.expect("the pass wrote no note about the discarded file");
    assert!(
        error.contains(&garbage.display().to_string()),
        "the note does not name the file: {error}"
    );
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

// ---------------------------------------------------------------------------
// FEED-03: the index as it stood
// ---------------------------------------------------------------------------

/// A `Read` tool call, which is what leaves a `path` entity behind.
fn read_call(path: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_use",
        "id": "toolu_read",
        "name": "Read",
        "input": {"file_path": path},
    })
}

/// D-10: a search bounded by a watermark answers about the archive as it stood,
/// and a session ingested afterwards is invisible to it.
///
/// Two sessions naming the same file, archived in order. The bound is
/// `turn_id(watermark + 1, 0)`, which is sound because a turn id is
/// `session_no << TURN_SEQ_BITS | turn_seq`: every turn of the first session
/// sits below it and every turn of the second sits above. The unbounded search
/// is the premise - without it "one hit" could mean the second session simply
/// did not match.
#[test]
fn a_watermark_bound_hides_a_later_ingested_session() {
    let bench = bench();
    let project = "project-beta";
    let file = "/code/beta/src/kettle.rs";

    bench.archive("first.jsonl", project, vec![read_call(file)]);
    let first = bench.watermark();
    bench.archive("second2.jsonl", project, vec![read_call(file)]);
    let second = bench.watermark();
    assert!(second > first, "the two sessions share a session_no");

    let conn = bench.conn();
    let cwd = bench.root.join(project);
    let scope = Scope::Directory(cwd.clone());
    let request = Request::new(Query::parse(file), scope)
        .candidates(vec![Query::parse(file)])
        .excerpts(false)
        .limit(10);

    let unbounded = search::run(&conn, &Config::default(), &request).unwrap();
    let sessions: Vec<i64> = unbounded
        .hits
        .iter()
        .map(|hit| schema::split_turn_id(hit.turn_id).0)
        .collect();
    // Both hits carry the same relevance and the same timestamp, so the total
    // order falls through to `t.id ASC` and the first-archived session leads.
    assert_eq!(sessions, vec![first, second], "{:?}", unbounded.hits);

    let bounded = search::run(
        &conn,
        &Config::default(),
        &request
            .clone()
            .before_turn_id(Some(schema::turn_id(first + 1, 0))),
    )
    .unwrap();
    let sessions: Vec<i64> = bounded
        .hits
        .iter()
        .map(|hit| schema::split_turn_id(hit.turn_id).0)
        .collect();
    assert_eq!(sessions, vec![first], "{:?}", bounded.hits);

    // The default is no bound at all, so every existing caller is untouched.
    assert_eq!(request.before_turn_id, None);
}

/// One session of hand-written records with the timestamps the test chose.
///
/// [`Bench::archive`] stamps its own clock a second apart, and every replay
/// assertion here is about which side of a decision's wall clock a turn falls
/// on - so the times have to be the test's to set.
fn archive_at(
    bench: &Bench,
    name: &str,
    project: &str,
    session: &str,
    turns: &[(&str, serde_json::Value)],
) {
    let cwd = bench.root.join(project);
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
            "message": {"role": "assistant", "model": "claude-opus-5", "content": [content]},
        });
        body.push_str(&record.to_string());
        body.push('\n');
    }
    let path = bench.work.join(name);
    std::fs::write(&path, body).unwrap();
    match ingest::run(&bench.data_dir, &path).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("{name}: {other:?}"),
    }
}

/// One logged decision, written straight into the table the drain fills.
///
/// Hand-built because these tests choose the wall clock, the watermark and the
/// refusals - three things the injector reads off a real machine.
#[allow(clippy::too_many_arguments)]
fn decide(
    bench: &Bench,
    session: &str,
    at: &str,
    cwd: &str,
    prompt: &str,
    watermark: i64,
    injected: &[(i64, usize)],
    chars: i64,
    suppressed: &[i64],
) -> i64 {
    let injected: Vec<serde_json::Value> = injected
        .iter()
        .map(|(turn_id, chars)| serde_json::json!({"turn_id": turn_id, "chars": chars}))
        .collect();
    let suppressed: Vec<serde_json::Value> = suppressed
        .iter()
        .map(|turn_id| serde_json::json!({"turn_id": turn_id, "reason": "already_injected"}))
        .collect();
    let conn = bench.conn();
    conn.execute(
        "INSERT INTO decisions (
            session_id, ts, cwd, prompt, watermark_session_no, chars_injected,
            spellings, candidates, injected, suppressed, thresholds
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', '[]', ?7, ?8, '{}')",
        rusqlite::params![
            session,
            at,
            cwd,
            prompt,
            watermark,
            chars,
            serde_json::to_string(&injected).unwrap(),
            serde_json::to_string(&suppressed).unwrap(),
        ],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// One label's `(old, new)` out of a replay.
fn movement(replayed: &Replayed, label: &str) -> (usize, usize) {
    let found = replayed
        .labels
        .iter()
        .find(|movement| movement.label == label)
        .unwrap_or_else(|| panic!("{label} is not in the diff: {replayed:?}"));
    (found.old, found.new)
}

/// Replay this store under `thresholds`, through the read-only open the command
/// uses (D-15).
fn replayed(bench: &Bench, thresholds: &Thresholds) -> Replayed {
    let store = Store::open_read_only(&bench.data_dir).unwrap();
    replay::replay(&store, &Config::default(), thresholds).unwrap()
}

/// FEED-03: replaying the shipped thresholds over freshly-written labels moves
/// nothing.
///
/// The zero diff is the whole assertion and it is not vacuous: the counts it
/// compares are non-zero, so a replay that re-extracted a different spelling,
/// searched a different pool or judged by a different rule would show up as
/// movement rather than as an empty answer. This is what makes any diff a
/// later run reports attributable to the change under test.
#[test]
fn replaying_the_shipped_thresholds_moves_nothing() {
    let bench = bench();
    let cwd = bench.project();
    let raw = format!("what changed in {RELATIVE}");

    assert!(
        bench.submit_as(FIXTURE_SESSION, &cwd, &raw).is_some(),
        "the prompt has to inject for there to be a label to reproduce"
    );

    let labelled = bench.pass();
    assert_eq!(labelled.feedback.decisions, 1, "{labelled:?}");
    assert_eq!(labelled.outcomes.count("false positive"), 1, "{labelled:?}");
    assert_eq!(labelled.outcomes.count("wasted budget"), 1, "{labelled:?}");

    let diff = replayed(&bench, &Thresholds::default());
    assert_eq!(diff.decisions, 1, "{diff:?}");
    assert_eq!(diff.changed, Vec::<i64>::new(), "{diff:?}");
    assert_eq!(movement(&diff, "false positive"), (1, 1), "{diff:?}");
    assert_eq!(movement(&diff, "wasted budget"), (1, 1), "{diff:?}");
    assert_eq!(movement(&diff, "hit"), (0, 0), "{diff:?}");
    assert_eq!(movement(&diff, "miss"), (0, 0), "{diff:?}");

    // Determinism: the same store and the same thresholds, twice.
    assert_eq!(replayed(&bench, &Thresholds::default()), diff);
}

/// FEED-03: a turn that sat one place below the rank threshold is injected under
/// a widened one, and the label that would have earned is what the diff reports.
///
/// The decision under test refused everything the default admits - the three
/// suppressions are read off the ranked order the store actually produces, not
/// guessed - so under the shipped numbers it injects nothing and carries no
/// label. Widening the rank by one admits exactly one more turn, and a later
/// turn of the same session names the same file, so that turn is a `hit`.
#[test]
fn a_widened_rank_threshold_moves_a_decision_from_silence_to_a_hit() {
    let bench = bench();
    let session = "55555555-5555-4555-8555-555555555555";
    let project = "project-beta";
    let file = "/code/beta/src/kettle.rs";
    let cwd = bench.root.join(project).to_string_lossy().into_owned();

    archive_at(
        &bench,
        "beta.jsonl",
        project,
        session,
        &[
            ("2026-08-14T10:00:00.000Z", read_call(file)),
            ("2026-08-14T10:01:00.000Z", read_call(file)),
            ("2026-08-14T10:02:00.000Z", read_call(file)),
            ("2026-08-14T10:03:00.000Z", read_call(file)),
            // After the decision below, and what makes an injected turn a hit:
            // the session went back to the same file.
            ("2026-08-14T12:00:00.000Z", read_call(file)),
        ],
    );
    let closed = bench.pass();
    assert_eq!(closed.outcomes.finalized, 2, "{closed:?}");

    // The ranked order this store actually produces, through the same
    // extraction and the same request shape the injector builds.
    let raw = format!("what happened in {file}");
    let (_, candidates) = prompt::candidates(&raw, Some(&cwd), prompt::MAX_CANDIDATES);
    let request = Request::new(
        prompt::query_of(&raw, Some(&cwd)),
        Scope::Directory(bench.root.join(project)),
    )
    .candidates(candidates)
    .excerpts(false)
    .limit(prompt::RANKED);
    let hits = search::run(&bench.conn(), &Config::default(), &request)
        .unwrap()
        .hits;
    assert!(hits.len() >= 4, "{hits:?}");
    for hit in &hits[..4] {
        assert!(
            hit.entity_match.is_some() && hit.entity_count < prompt::CO_OCCURRING,
            "the rank is what admits this hit, so it has to be the only thing \
             that does: {hit:?}"
        );
    }
    let suppressed: Vec<i64> = hits[..prompt::ENTITY_RANK]
        .iter()
        .map(|hit| hit.turn_id)
        .collect();

    let id = decide(
        &bench,
        session,
        "2026-08-14T11:00:00.000Z",
        &cwd,
        &raw,
        bench.watermark(),
        &[],
        0,
        &suppressed,
    );
    let labelled = bench.pass();
    assert!(
        labelled.outcomes.labels.is_empty(),
        "the decision injected nothing, so it carries no label: {labelled:?}"
    );

    // Under the shipped numbers: every eligible turn was refused, so nothing.
    let same = replayed(&bench, &Thresholds::default());
    assert_eq!(same.decisions, 1, "{same:?}");
    assert_eq!(same.changed, Vec::<i64>::new(), "{same:?}");
    assert_eq!(movement(&same, "hit"), (0, 0), "{same:?}");

    // One rank wider, and the turn below the cut is injected and earns a hit.
    let widened = Thresholds {
        entity_rank: prompt::ENTITY_RANK + 1,
        ..Thresholds::default()
    };
    let diff = replayed(&bench, &widened);
    assert_eq!(diff.changed, vec![id], "{diff:?}");
    assert_eq!(movement(&diff, "hit"), (0, 1), "{diff:?}");
    assert_eq!(movement(&diff, "false positive"), (0, 0), "{diff:?}");
    assert_eq!(movement(&diff, "wasted budget"), (0, 0), "{diff:?}");
}

/// D-10: a decision is scored against the archive as it stood, so a session
/// ingested after it cannot be offered to it however well it matches.
///
/// The two halves are one test on purpose. Raising the stored watermark and
/// replaying again is what makes the first half a statement about the bound
/// rather than about a prompt that simply matched nothing.
#[test]
fn a_decision_is_never_scored_against_a_session_archived_after_it() {
    let bench = bench();
    let project = "project-gamma";
    let cwd = bench.root.join(project).to_string_lossy().into_owned();
    let earlier = "66666666-6666-4666-8666-666666666666";
    let later = "77777777-7777-4777-8777-777777777777";
    let file = "/code/gamma/src/beacon.rs";

    // The session the decision belongs to, holding nothing that matches.
    archive_at(
        &bench,
        "gamma-one.jsonl",
        project,
        earlier,
        &[(
            "2026-08-14T10:00:00.000Z",
            read_call("/code/gamma/src/other.rs"),
        )],
    );
    let watermark = bench.watermark();
    // Archived afterwards, and the only thing in the store that names the file
    // the prompt asks about.
    archive_at(
        &bench,
        "gamma-two.jsonl",
        project,
        later,
        &[("2026-08-14T10:30:00.000Z", read_call(file))],
    );
    assert!(bench.watermark() > watermark, "one session, not two");
    bench.pass();

    let raw = format!("what happened in {file}");
    let id = decide(
        &bench,
        earlier,
        "2026-08-14T11:00:00.000Z",
        &cwd,
        &raw,
        watermark,
        &[],
        0,
        &[],
    );

    let bounded = replayed(&bench, &Thresholds::default());
    assert_eq!(bounded.decisions, 1, "{bounded:?}");
    assert_eq!(
        bounded.changed,
        Vec::<i64>::new(),
        "a turn archived after the decision was offered to it: {bounded:?}"
    );
    assert_eq!(movement(&bounded, "false positive"), (0, 0), "{bounded:?}");

    // The falsifying half: the same prompt, the same store, one integer moved.
    bench
        .conn()
        .execute(
            "UPDATE decisions SET watermark_session_no = ?1 WHERE id = ?2",
            rusqlite::params![bench.watermark(), id],
        )
        .unwrap();
    let unbounded = replayed(&bench, &Thresholds::default());
    assert_eq!(unbounded.changed, vec![id], "{unbounded:?}");
    assert_eq!(
        movement(&unbounded, "false positive"),
        (0, 1),
        "{unbounded:?}"
    );
    assert_eq!(
        movement(&unbounded, "wasted budget"),
        (0, 1),
        "{unbounded:?}"
    );
}
