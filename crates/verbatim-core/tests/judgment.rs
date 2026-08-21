//! OBS-03: one call per session, and every claim anchored to a turn that is
//! really there.
//!
//! The endpoint is a real `std::net::TcpListener` from `testkit` (D-20), the
//! store is a real SQLite file with real fixtures ingested into it, and the
//! request count is read off `observe::net`'s attempt log rather than argued
//! from the shape of the code (D-21).

#![cfg(feature = "testkit")]

use std::collections::BTreeSet;
use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::config::{Config, CONFIG_FILE_NAME};
use verbatim_core::observe::judgment::{self, Verdict};
use verbatim_core::observe::net;
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::testkit::{self, HttpStub};
use verbatim_core::{ingest, observe};

/// Held by every test that resets or reads the attempt log: it is process
/// global, so two tests counting in parallel would each see the other's calls.
static NET: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The session every anchoring assertion here is about.
const JUDGED: &str = "session-errors-a.jsonl";

/// A second session, so "a real turn id, but of another session" is a state a
/// test can actually build.
const OTHER: &str = "session-edits.jsonl";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    config_dir: PathBuf,
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let config_dir = dir.path().join("config");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        config_dir,
        root,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// Archive one rooted fixture and answer with its session key.
    fn ingest(&self, fixture: &str) -> String {
        let path = testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root);
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{fixture}: {other:?}"),
        }
        path.canonicalize().unwrap().to_string_lossy().into_owned()
    }

    /// Archive both fixtures, close them, and give each the mechanical row the
    /// judgment half fills in. The pass is what normally does this; here the
    /// idle rule is short-circuited so the fixtures' own old timestamps do not
    /// have to be rewritten.
    fn observed(&self) -> (String, String) {
        let judged = self.ingest(JUDGED);
        let other = self.ingest(OTHER);
        let conn = self.conn();
        conn.execute("UPDATE session_meta SET is_final = 1", [])
            .unwrap();
        let observed = observe::observe_new(&conn, &Config::default());
        assert_eq!(observed.written, 2, "{observed:?}");
        (judged, other)
    }

    /// A config pointing judgment at `stub`, with the destination declared
    /// local so the egress filter is not what this file is measuring.
    fn config(&self, stub: &HttpStub) -> Config {
        std::fs::write(
            self.config_dir.join(CONFIG_FILE_NAME),
            format!(
                "[provider]\nenabled = true\nbase_url = \"{}\"\n\
                 model = \"{MODEL}\"\nlocal = true\n",
                stub.base_url()
            ),
        )
        .unwrap();
        Config::load_from(&self.config_dir).unwrap()
    }

    /// Every `turns.id` of one session, which is the whole of what a claim of
    /// that session may anchor to.
    fn turn_ids(&self, session_key: &str) -> Vec<i64> {
        self.conn()
            .prepare("SELECT id FROM turns WHERE session_key = ?1 ORDER BY turn_seq")
            .unwrap()
            .query_map([session_key], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// One row's judgment columns.
    fn row(&self, session_key: &str) -> Row {
        self.conn()
            .query_row(
                "SELECT o.status, o.model, o.prompt_version, o.topic, o.outcome,
                        o.decisions, o.learned, o.unresolved, o.raw, o.tokens, o.mechanical
                   FROM observations o WHERE o.session_key = ?1",
                [session_key],
                |r| {
                    Ok(Row {
                        status: r.get(0)?,
                        model: r.get(1)?,
                        prompt_version: r.get(2)?,
                        topic: r.get(3)?,
                        outcome: r.get(4)?,
                        decisions: r.get(5)?,
                        learned: r.get(6)?,
                        unresolved: r.get(7)?,
                        raw: r.get(8)?,
                        tokens: r.get(9)?,
                        mechanical: r.get(10)?,
                    })
                },
            )
            .unwrap_or_else(|e| panic!("no observation for {session_key}: {e}"))
    }
}

const MODEL: &str = "qwen3:8b";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    status: Option<String>,
    model: Option<String>,
    prompt_version: Option<String>,
    topic: Option<String>,
    outcome: Option<String>,
    decisions: Option<String>,
    learned: Option<String>,
    unresolved: Option<String>,
    raw: Option<String>,
    tokens: Option<i64>,
    mechanical: Option<String>,
}

impl Row {
    /// Every `turn_id` the three claim lists carry, in one set.
    fn anchors(&self) -> Vec<i64> {
        let mut out = Vec::new();
        for column in [&self.decisions, &self.learned, &self.unresolved] {
            let text = column.as_deref().expect("a claim list column");
            let list: serde_json::Value = serde_json::from_str(text).expect("a stored JSON array");
            for entry in list.as_array().expect("an array") {
                out.push(entry["turn_id"].as_i64().expect("an integer turn_id"));
            }
        }
        out
    }
}

/// The body of a well-formed answer, anchoring one claim per list at the ids
/// it is handed.
fn answer(ids: &[i64]) -> String {
    serde_json::json!({
        "topic": "the ingest lock and the errors it swallowed",
        "outcome": "partial",
        "decisions": [{"turn_id": ids[0], "text": "kept the per-file transaction"}],
        "learned": [{"turn_id": ids[1 % ids.len()], "text": "the walk skips a damaged file"}],
        "unresolved": [{"turn_id": ids[2 % ids.len()], "text": "the retry budget is unset"}],
    })
    .to_string()
}

/// AC2's anchoring half: one request, and every stored claim points at a turn
/// row of the session it is about.
#[test]
fn one_call_stores_claims_whose_turn_ids_are_real_turns_of_that_session() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _other) = bench.observed();
    let ids = bench.turn_ids(&judged);
    assert!(ids.len() >= 3, "the fixture must carry turns: {ids:?}");
    let before = bench.row(&judged);

    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 325, 69)]);
    let config = bench.config(&stub);

    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

    assert_eq!(verdict, Verdict::Stored { tokens: 394 }, "{verdict:?}");
    assert_eq!(
        net::attempts::count(),
        1,
        "one session cost more than one call: {:?}",
        net::attempts::destinations()
    );
    assert_eq!(stub.requests().len(), 1);

    let row = bench.row(&judged);
    assert_eq!(row.status.as_deref(), Some(judgment::STATUS_OK));
    assert_eq!(row.model.as_deref(), Some(MODEL));
    assert_eq!(
        row.prompt_version.as_deref(),
        Some(judgment::PROMPT_VERSION),
        "regenerate --prompt-version selects on this"
    );
    assert_eq!(
        row.topic.as_deref(),
        Some("the ingest lock and the errors it swallowed")
    );
    assert_eq!(row.outcome.as_deref(), Some("partial"));
    assert_eq!(row.tokens, Some(394));
    assert_eq!(row.raw, None);
    assert_eq!(
        row.mechanical, before.mechanical,
        "the judgment half disturbed the mechanical half"
    );

    // The whole of OBS-03: every stored anchor selects a real turn row of this
    // session, asked of the database rather than of the answer that was sent.
    let real: BTreeSet<i64> = ids.iter().copied().collect();
    let anchored = row.anchors();
    assert_eq!(anchored.len(), 3, "one claim per list: {anchored:?}");
    for turn_id in anchored {
        let of_session: String = bench
            .conn()
            .query_row(
                "SELECT session_key FROM turns WHERE id = ?1",
                [turn_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("turn {turn_id} is not in the archive: {e}"));
        assert_eq!(
            of_session, judged,
            "turn {turn_id} belongs to another session"
        );
        assert!(real.contains(&turn_id));
    }
}

/// A claim naming a turn of a DIFFERENT session, and a claim naming no turn at
/// all, are both refused - and refused whole, rather than stored with the bad
/// anchors dropped. An unverifiable claim reads as evidence, which is worse
/// than no claim.
#[test]
fn a_claim_anchored_outside_the_session_stores_no_successful_row() {
    let bench = bench();
    let (judged, other) = bench.observed();
    let strangers = bench.turn_ids(&other);
    assert!(!strangers.is_empty(), "the second fixture carries no turns");
    let mine = bench.turn_ids(&judged);

    // A real turn id of another session first, then an id belonging to nothing.
    let nowhere = mine.iter().chain(&strangers).max().copied().unwrap() + 10_000;
    for bad in [strangers[0], nowhere] {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let canned = serde_json::json!({
            "topic": "a plausible summary of somebody else's work",
            "outcome": "completed",
            "decisions": [{"turn_id": mine[0], "text": "this one is fine"}],
            "learned": [{"turn_id": bad, "text": "this one points somewhere else"}],
            "unresolved": [],
        })
        .to_string();
        let stub = HttpStub::serving(&[testkit::chat_completion(&canned, 10, 10)]);
        let config = bench.config(&stub);

        net::attempts::reset();
        let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

        match &verdict {
            Verdict::Failed { reason } => assert!(
                reason.contains(&bad.to_string()),
                "the refusal does not name the anchor it refused: {reason}"
            ),
            other => panic!("turn {bad} was accepted as an anchor: {other:?}"),
        }
        assert_eq!(net::attempts::count(), 1);

        let row = bench.row(&judged);
        assert_eq!(
            row.status, None,
            "a row was written with a success status for an unanchored answer"
        );
        assert_eq!(row.topic, None);
        assert_eq!(row.decisions, None);
    }
}

/// An answer that is not the document that was asked for stores nothing under
/// a success status either.
#[test]
fn an_answer_that_is_not_the_schema_stores_no_successful_row() {
    let bench = bench();
    let (judged, _other) = bench.observed();
    let ids = bench.turn_ids(&judged);

    let unusable = [
        testkit::UNPARSEABLE_CONTENT.to_owned(),
        serde_json::json!({
            "topic": "no outcome key here",
            "decisions": [], "learned": [], "unresolved": [],
        })
        .to_string(),
        serde_json::json!({
            "topic": "an outcome that is not one of the four",
            "outcome": "went fine",
            "decisions": [], "learned": [], "unresolved": [],
        })
        .to_string(),
        serde_json::json!({
            "topic": "a claim with no anchor at all",
            "outcome": "completed",
            "decisions": [{"text": "no turn_id on this entry"}],
            "learned": [], "unresolved": [],
        })
        .to_string(),
    ];

    for content in unusable {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion(&content, 1, 1)]);
        let config = bench.config(&stub);

        let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

        assert!(
            matches!(verdict, Verdict::Failed { .. }),
            "{content:?} was accepted: {verdict:?}"
        );
        assert_eq!(bench.row(&judged).status, None);
    }

    // The falsifying half: the same store, the same session, an answer that IS
    // the schema - so the refusals above are about the answers and not about a
    // bench that could never store anything.
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 1, 1)]);
    let config = bench.config(&stub);
    assert_eq!(
        judgment::judge(&bench.conn(), &config, None, &judged),
        Verdict::Stored { tokens: 2 }
    );
}

/// The model cannot anchor a claim to an id it was never shown, so the ids go
/// out with the turns.
#[test]
fn the_request_shows_the_model_each_turn_beside_its_real_turn_id() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, other) = bench.observed();
    let ids = bench.turn_ids(&judged);

    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 1, 1)]);
    let config = bench.config(&stub);
    judgment::judge(&bench.conn(), &config, None, &judged);

    let sent = stub.requests().remove(0);
    let body: serde_json::Value =
        serde_json::from_str(sent.split("\r\n\r\n").nth(1).expect("a body")).unwrap();
    let shown = body["messages"][1]["content"]
        .as_str()
        .expect("a user turn");
    for id in &ids {
        assert!(
            shown.contains(&format!("turn_id={id}")),
            "turn {id} was not shown to the model: {shown}"
        );
    }
    // And no other session's turns travelled with it.
    for id in bench.turn_ids(&other) {
        assert!(
            !shown.contains(&format!("turn_id={id}")),
            "another session's turn {id} was in the prompt"
        );
    }
    let system = body["messages"][0]["content"]
        .as_str()
        .expect("a system turn");
    assert!(
        system.contains("turn_id"),
        "the instructions never mention the anchor: {system}"
    );
}
