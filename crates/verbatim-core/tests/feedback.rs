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
use verbatim_core::inject::decision::{self, Decision};
use verbatim_core::inject::state::Reason;
use verbatim_core::inject::{prompt, Payload};
use verbatim_core::store::{Store, DB_FILE_NAME};
use verbatim_core::{ingest, reindex, testkit};

/// The fixture that stores an absolute path, and the relative spelling of that
/// same file beneath its project - the same pair `inject_prompt.rs` uses, so a
/// prompt asserted about there is the prompt logged about here.
const FIXTURE: &str = "session-edits.jsonl";
const RELATIVE: &str = "crates/gizmo/lantern.rs";

/// The `session_id` a payload carries unless the test names another.
const SESSION: &str = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";

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
