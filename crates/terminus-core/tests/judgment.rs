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
use std::time::{Duration, Instant};

use rusqlite::Connection;
use terminus_core::config::{Config, Secret, CONFIG_FILE_NAME};
use terminus_core::observe::cost::Skip;
use terminus_core::observe::judgment::{self, Verdict};
use terminus_core::observe::net;
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit::{self, HttpStub};
use terminus_core::{ingest, observe};

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
        self.config_with(stub, "")
    }

    /// The same config with more `terminus.toml` after it.
    ///
    /// One writer for the provider table so the two helpers cannot drift: what
    /// PRIV-03's tests need is this exact config - `local = true` included -
    /// plus a `[privacy]` table, and a second copy of the provider block would
    /// eventually stop being the same config the rest of the file uses.
    fn config_with(&self, stub: &HttpStub, extra: &str) -> Config {
        std::fs::write(
            self.config_dir.join(CONFIG_FILE_NAME),
            format!(
                "[provider]\nenabled = true\nbase_url = \"{}\"\n\
                 model = \"{MODEL}\"\nlocal = true\n{extra}",
                stub.base_url()
            ),
        )
        .unwrap();
        Config::load_from(&self.config_dir).unwrap()
    }

    /// That config with PRIV-03's knob written either way.
    ///
    /// A `terminus.toml` is the only thing that can set it - there is no
    /// in-memory setter - and `on = false` spells the key out rather than
    /// omitting it, so the two configs differ in one token of one file.
    fn redacting(&self, stub: &HttpStub, on: bool) -> Config {
        self.config_with(stub, &format!("\n[privacy]\nredact_recall = {on}\n"))
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
    // A bench each, because a refused answer leaves a `parse_failed` status
    // behind and a pass never asks the same session twice (OBS-06's fourth
    // gate). Two benches is what asking the question twice actually costs.
    for case in ["another session", "no session at all"] {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let bench = bench();
        let (judged, other) = bench.observed();
        let strangers = bench.turn_ids(&other);
        assert!(!strangers.is_empty(), "the second fixture carries no turns");
        let mine = bench.turn_ids(&judged);
        let bad = match case {
            "another session" => strangers[0],
            _ => mine.iter().chain(&strangers).max().copied().unwrap() + 10_000,
        };

        let canned = serde_json::json!({
            "topic": "a plausible summary of somebody else's work",
            "outcome": "completed",
            "decisions": [{"turn_id": mine[0], "text": "this one is fine"}],
            "learned": [{"turn_id": bad, "text": "this one points somewhere else"}],
            "unresolved": [],
        })
        .to_string();
        let stub = HttpStub::serving(&[
            testkit::chat_completion(&canned, 10, 10),
            testkit::chat_completion(&canned, 10, 10),
        ]);
        let config = bench.config(&stub);

        net::attempts::reset();
        let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

        match &verdict {
            Verdict::ParseFailed { reason, .. } => assert!(
                reason.contains(&bad.to_string()),
                "the refusal does not name the anchor it refused: {reason}"
            ),
            other => panic!("turn {bad} ({case}) was accepted as an anchor: {other:?}"),
        }
        assert_eq!(
            net::attempts::count(),
            2,
            "an unanchored answer was not retried"
        );

        let row = bench.row(&judged);
        assert_eq!(
            row.status.as_deref(),
            Some(judgment::STATUS_PARSE_FAILED),
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
        let bench = bench();
        let (judged, _other) = bench.observed();
        let stub = HttpStub::serving(&[
            testkit::chat_completion(&content, 1, 1),
            testkit::chat_completion(&content, 1, 1),
        ]);
        let config = bench.config(&stub);

        let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

        assert!(
            matches!(verdict, Verdict::ParseFailed { .. }),
            "{content:?} was accepted: {verdict:?}"
        );
        assert_eq!(
            bench.row(&judged).status.as_deref(),
            Some(judgment::STATUS_PARSE_FAILED)
        );
    }

    // The falsifying half: the same fixtures, an answer that IS the schema - so
    // the refusals above are about the answers and not about a bench that could
    // never store anything.
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _other) = bench.observed();
    let ids = bench.turn_ids(&judged);
    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 1, 1)]);
    let config = bench.config(&stub);
    assert_eq!(
        judgment::judge(&bench.conn(), &config, None, &judged),
        Verdict::Stored { tokens: 2 }
    );
}

/// OBS-04: two requests, then the raw answer is KEPT rather than dropped - and
/// kept scrubbed, because it is text this build did not author and `terminus
/// observations` prints the column (D-16).
#[test]
fn an_unusable_answer_is_asked_twice_and_then_stored_raw() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _other) = bench.observed();
    let before = bench.row(&judged);

    // A provider echoing the caller's own key back inside its answer is the
    // case that makes the scrub load-bearing: `raw` is durable and printed.
    const KEY: &str = "sk-observations-should-never-store-this";
    let leaky = format!("{} (api_key: {KEY})", testkit::UNPARSEABLE_CONTENT);
    let stub = HttpStub::serving(&[
        testkit::chat_completion(&leaky, 10, 5),
        testkit::chat_completion(&leaky, 10, 5),
    ]);
    let config = bench.config(&stub);
    let secret = Secret::new(KEY);

    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, Some(&secret), &judged);

    match &verdict {
        Verdict::ParseFailed { tokens, reason } => {
            assert_eq!(*tokens, 30, "both requests must be charged for");
            assert!(
                !reason.contains(KEY),
                "the reason carries the key: {reason}"
            );
        }
        other => panic!("an unparseable answer was not stored as a failure: {other:?}"),
    }
    assert_eq!(
        net::attempts::count(),
        2,
        "exactly one retry: {:?}",
        net::attempts::destinations()
    );
    assert_eq!(stub.requests().len(), 2);

    let row = bench.row(&judged);
    assert_eq!(
        row.status.as_deref(),
        Some(judgment::STATUS_PARSE_FAILED),
        "the failure was dropped instead of stored"
    );
    let raw = row.raw.as_deref().expect("the raw answer");
    assert!(
        raw.contains(testkit::UNPARSEABLE_CONTENT),
        "the stored raw answer is not what came back: {raw}"
    );
    assert!(
        !raw.contains(KEY),
        "the credential is durably stored: {raw}"
    );
    assert_eq!(row.topic, None, "a failed row must carry no claims");
    assert_eq!(row.decisions, None);
    assert_eq!(row.tokens, Some(30));
    assert_eq!(
        row.mechanical, before.mechanical,
        "the failure arm disturbed the mechanical half"
    );

    // And a later run of the same session makes no third request: the status is
    // what stops a pass asking again (OBS-06's fourth gate).
    let again = HttpStub::serving(&[testkit::chat_completion("{}", 1, 1)]);
    let config = bench.config(&again);
    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);
    assert!(
        matches!(verdict, Verdict::Skipped(_)),
        "a stored failure was asked again: {verdict:?}"
    );
    assert_eq!(net::attempts::count(), 0);
}

/// The transient case the retry exists for: the second answer parses, and the
/// row is a success carrying what both requests cost.
#[test]
fn a_retry_that_parses_is_stored_as_a_success() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _other) = bench.observed();
    let ids = bench.turn_ids(&judged);

    let stub = HttpStub::serving(&[
        testkit::chat_completion(testkit::UNPARSEABLE_CONTENT, 10, 5),
        testkit::chat_completion(&answer(&ids), 325, 69),
    ]);
    let config = bench.config(&stub);

    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

    assert_eq!(verdict, Verdict::Stored { tokens: 409 }, "{verdict:?}");
    assert_eq!(net::attempts::count(), 2);

    let row = bench.row(&judged);
    assert_eq!(row.status.as_deref(), Some(judgment::STATUS_OK));
    assert_eq!(row.raw, None, "a successful row kept the failed text");
    assert_eq!(
        row.tokens,
        Some(409),
        "the row must say what the session cost, both requests included"
    );
    assert_eq!(row.anchors().len(), 3);
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

/// The race D-07 makes reachable: a provider call in flight does not stop a
/// second `terminus ingest`, so two runs really can be over the same unjudged
/// session at once. Between them they cost ONE request, and the loser skips.
///
/// The whole assertion is on the attempt log while the first call is still in
/// flight: reserving after the answer comes back would leave both requests
/// made, and the request is the charge.
#[test]
fn two_runs_over_one_unjudged_session_make_one_request_between_them() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _other) = bench.observed();
    assert_eq!(bench.row(&judged).status, None, "the premise: unjudged");

    // Held long enough that the second run happens provably inside the first
    // run's request, then answered - a stalled thread that never returns would
    // leak into the rest of the file.
    let stub = HttpStub::stalling(Duration::from_secs(3));
    let config = bench.config(&stub);

    net::attempts::reset();
    let db = bench.data_dir.join(DB_FILE_NAME);
    let running = config.clone();
    let asked = judged.clone();
    let first = std::thread::spawn(move || {
        let conn = Connection::open(&db).expect("a second handle on the same store");
        judgment::judge(&conn, &running, None, &asked)
    });

    // The request has been read and the stub is sitting on it, so the first run
    // is inside the HTTP call for the length of the hold.
    let deadline = Instant::now() + Duration::from_secs(20);
    while stub.arrived() == 0 {
        assert!(
            Instant::now() < deadline,
            "the first run never reached the provider"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let held = bench.row(&judged).status.expect("the row is reserved");
    assert!(
        judgment::is_reservation(&held),
        "the row was not claimed before the request went out: {held:?}"
    );

    let second = judgment::judge(&bench.conn(), &config, None, &judged);
    match &second {
        Verdict::Skipped(Skip::InFlight { since }) => assert!(
            held.contains(since.as_str()),
            "the skip names {since}, the reservation says {held}"
        ),
        other => panic!("the second run did not skip a session in flight: {other:?}"),
    }
    assert_eq!(
        net::attempts::count(),
        1,
        "one session was paid for twice: {:?}",
        net::attempts::destinations()
    );

    // The stall answers 503, which is not an answer this build could not read -
    // it is one that never arrived. The reservation is released rather than
    // left standing, so the session is not unjudgeable until the lease lapses.
    let first = first.join().expect("the first thread");
    assert!(
        matches!(first, Verdict::Failed { .. }),
        "a 503 stored something: {first:?}"
    );
    assert_eq!(
        bench.row(&judged).status,
        None,
        "a provider that was down left the session permanently unjudgeable"
    );
    assert_eq!(stub.requests().len(), 1);

    // The falsifier: the same bench, the same session, a provider that answers.
    // One request, and the row is judged - so the two zero-request assertions
    // above are about the reservation and not about a bench that could never
    // buy anything.
    let ids = bench.turn_ids(&judged);
    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 325, 69)]);
    let config = bench.config(&stub);
    net::attempts::reset();
    assert_eq!(
        judgment::judge(&bench.conn(), &config, None, &judged),
        Verdict::Stored { tokens: 394 }
    );
    assert_eq!(net::attempts::count(), 1);
    assert_eq!(
        bench.row(&judged).status.as_deref(),
        Some(judgment::STATUS_OK)
    );
}

/// A reservation whose run died is not a life sentence: past the lease the next
/// run takes the session, and inside it nobody does.
#[test]
fn an_abandoned_reservation_is_taken_only_once_the_lease_has_lapsed() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _other) = bench.observed();
    let ids = bench.turn_ids(&judged);

    // What a killed process leaves behind: a token nothing will ever release.
    // Written straight into the column, because the only way to produce one
    // honestly is to kill a run mid-request.
    let stamp = |seconds: i64| -> String {
        bench
            .conn()
            .query_row(
                "SELECT ?1 || ' ' || strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2)",
                rusqlite::params![judgment::STATUS_JUDGING, format!("{seconds} seconds")],
                |r| r.get(0),
            )
            .unwrap()
    };
    let abandon = |token: &str| {
        bench
            .conn()
            .execute(
                "UPDATE observations SET status = ?2 WHERE session_key = ?1",
                rusqlite::params![judged, token],
            )
            .unwrap();
    };

    // Inside the lease: somebody may still be asking, so nothing is bought.
    abandon(&stamp(-(judgment::RESERVATION_LEASE_SECONDS / 2)));
    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 1, 1)]);
    let config = bench.config(&stub);
    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);
    assert!(
        matches!(verdict, Verdict::Skipped(Skip::InFlight { .. })),
        "a live reservation was taken: {verdict:?}"
    );
    assert_eq!(net::attempts::count(), 0);
    assert!(
        observe::judge_new(&bench.data_dir, &config).judged == 0,
        "a pass spent its one slot on a session already in flight"
    );
    assert_eq!(net::attempts::count(), 0);

    // Past it: the run that held it is gone, and the session is judged rather
    // than left unjudgeable forever.
    abandon(&stamp(-(judgment::RESERVATION_LEASE_SECONDS + 60)));
    let judged_now = observe::judge_new(&bench.data_dir, &config);
    assert_eq!(judged_now.judged, 1, "{judged_now:?}");
    assert_eq!(net::attempts::count(), 1);
    assert_eq!(
        bench.row(&judged).status.as_deref(),
        Some(judgment::STATUS_OK)
    );
}

// ---------------------------------------------------------------------------
// PRIV-03: the judgment columns, filtered before they reach SQLite
// ---------------------------------------------------------------------------

/// One distinct sentinel per column the filter has to reach, so a survivor
/// names which column let it through.
///
/// Each is planted in a shape the rule set owns - a header line - because the
/// only exact rule is fed the request's own credential and this bench sends
/// none, exactly like the common machine with no provider key.
const TOPIC_SENTINEL: &str = "sk-VBJUDGE-topic-7a1";
const DECISION_SENTINEL: &str = "sk-VBJUDGE-decide-3c5";
const LEARNED_SENTINEL: &str = "sid-VBJUDGE-learn-8e2";
const UNRESOLVED_SENTINEL: &str = "sk-VBJUDGE-open-4b9";

/// The fragment all four share: one survivor of any of them is a leak.
const SENTINEL_MARK: &str = "VBJUDGE";

/// A well-formed answer that quotes a credential in `topic` and in every one of
/// the three claim lists.
///
/// `outcome` is one of the four and carries no sentinel, and it cannot: `read`
/// refuses every other value, so an answer that put a credential there never
/// becomes a judgment at all. That path has a test of its own below.
fn leaky_answer(ids: &[i64]) -> String {
    serde_json::json!({
        "topic": format!("the sync run it kept retrying with Authorization: Bearer {TOPIC_SENTINEL}"),
        "outcome": "partial",
        "decisions": [{
            "turn_id": ids[0],
            "text": format!("kept the header Authorization: Bearer {DECISION_SENTINEL} on the retry"),
        }],
        "learned": [{
            "turn_id": ids[1 % ids.len()],
            "text": format!("the gateway echoes Cookie: {LEARNED_SENTINEL} back on a 403"),
        }],
        "unresolved": [{
            "turn_id": ids[2 % ids.len()],
            "text": format!("nobody rotated Authorization: Bearer {UNRESOLVED_SENTINEL} yet"),
        }],
    })
    .to_string()
}

/// Store one leaky answer under a config the test chooses, and hand back the
/// row and the session's real turn ids.
fn judged_under(bench: &Bench, on: bool) -> (Row, Vec<i64>) {
    let (judged, _other) = bench.observed();
    let ids = bench.turn_ids(&judged);
    assert!(ids.len() >= 3, "the fixture must carry turns: {ids:?}");

    let stub = HttpStub::serving(&[testkit::chat_completion(&leaky_answer(&ids), 1, 1)]);
    let config = bench.redacting(&stub, on);

    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);
    assert_eq!(verdict, Verdict::Stored { tokens: 2 }, "{verdict:?}");

    let row = bench.row(&judged);
    assert_eq!(row.status.as_deref(), Some(judgment::STATUS_OK));
    (row, ids)
}

/// Every text column the judgment half writes, as one string.
fn columns(row: &Row) -> String {
    [
        row.topic.as_deref(),
        row.outcome.as_deref(),
        row.decisions.as_deref(),
        row.learned.as_deref(),
        row.unresolved.as_deref(),
    ]
    .map(|column| column.expect("a judgment column").to_owned())
    .join("\n")
}

/// The default, stated as an assertion: with the knob absent the answer is
/// stored exactly as the provider sent it.
#[test]
fn an_unfiltered_judgment_stores_the_planted_credentials_whole() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (row, _ids) = judged_under(&bench, false);

    for sentinel in [
        TOPIC_SENTINEL,
        DECISION_SENTINEL,
        LEARNED_SENTINEL,
        UNRESOLVED_SENTINEL,
    ] {
        assert!(
            columns(&row).contains(sentinel),
            "the default must not filter the judgment: {sentinel:?} is gone"
        );
    }
}

/// AC5: with the knob on, a direct SQL read of the judgment columns finds the
/// marker and none of the planted credentials.
///
/// The config this runs under declares `provider.local = true`, which is what
/// makes this a test of D-03 as well: `local` is a declaration about where the
/// REQUEST went, and it says nothing about a column `recall_search --kind
/// observation` hands the model. A filter routed through
/// `egress::for_destination` returns the text untouched here and fails.
#[test]
fn the_knob_filters_a_judgment_before_it_reaches_sqlite() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (row, ids) = judged_under(&bench, true);

    for column in [&row.topic, &row.decisions, &row.learned, &row.unresolved] {
        let column = column.as_deref().expect("a judgment column");
        assert!(
            column.contains(terminus_core::config::REDACTED),
            "a filtered column names what went: {column}"
        );
    }
    for absent in [
        TOPIC_SENTINEL,
        DECISION_SENTINEL,
        LEARNED_SENTINEL,
        UNRESOLVED_SENTINEL,
        SENTINEL_MARK,
    ] {
        assert!(
            !columns(&row).contains(absent),
            "{absent:?} survived into the observations row:\n{}",
            columns(&row)
        );
    }

    // OBS-03: the anchors are untouched. A claim is only auditable because it
    // points at a real turn, and a filter that moved one would be worse than
    // the leak it was fixing.
    let real: BTreeSet<i64> = ids.iter().copied().collect();
    let anchored = row.anchors();
    assert_eq!(anchored.len(), 3, "one claim per list: {anchored:?}");
    for anchor in anchored {
        assert!(
            real.contains(&anchor),
            "{anchor} is not a turn of this session"
        );
    }

    // `outcome` is one of the four, filtered or not: the filter runs over it and
    // has nothing to take.
    assert_eq!(row.outcome.as_deref(), Some("partial"));
    assert_eq!(row.raw, None);
}

/// The column the knob cannot reach, and does not need to.
///
/// `read` refuses every `outcome` but the four in `OUTCOMES`, so an answer that
/// puts a credential there never becomes a judgment and never reaches `store`.
/// It lands on the `parse_failed` path instead, whose `raw` is scrubbed
/// unconditionally under D-16 - which is why this is asserted with the knob
/// ABSENT: that scrub is not the knob's to gate, and a refactor that routed it
/// through the knob would fail here.
#[test]
fn an_outcome_carrying_a_credential_never_becomes_a_stored_judgment() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _other) = bench.observed();
    let ids = bench.turn_ids(&judged);

    let content = serde_json::json!({
        "topic": "an outcome that is not one of the four",
        "outcome": format!("Authorization: Bearer {TOPIC_SENTINEL}"),
        "decisions": [{"turn_id": ids[0], "text": "a claim that never gets stored"}],
        "learned": [], "unresolved": [],
    })
    .to_string();
    let stub = HttpStub::serving(&[
        testkit::chat_completion(&content, 1, 1),
        testkit::chat_completion(&content, 1, 1),
    ]);
    let config = bench.redacting(&stub, false);

    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);
    assert!(
        matches!(verdict, Verdict::ParseFailed { .. }),
        "an outcome outside the four was accepted: {verdict:?}"
    );

    let row = bench.row(&judged);
    assert_eq!(
        row.status.as_deref(),
        Some(judgment::STATUS_PARSE_FAILED),
        "{row:?}"
    );
    // No judgment columns at all, so there is nothing there to have leaked.
    assert_eq!(row.topic, None);
    assert_eq!(row.outcome, None);
    assert_eq!(row.decisions, None);

    // And the text that DID come back is scrubbed with the knob absent, because
    // that half was never the knob's.
    let raw = row.raw.as_deref().expect("the failed answer is kept");
    assert!(raw.contains(terminus_core::config::REDACTED), "{raw}");
    assert!(!raw.contains(SENTINEL_MARK), "{raw}");
}
