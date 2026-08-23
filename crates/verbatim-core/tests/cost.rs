//! OBS-06: what a judgment run is allowed to cost, and what stops it.
//!
//! Every "no request was made" here is read off `observe::net`'s attempt log
//! (D-21) rather than argued from the shape of the code, and the endpoint is a
//! real `TcpListener` from `testkit` (D-20). The store is a real SQLite file
//! with real fixtures in it, so the daily-spend row is exercised the way a
//! second invocation would find it: on disk, with a date on it.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::config::{Config, CONFIG_FILE_NAME};
use verbatim_core::observe::cost;
use verbatim_core::observe::judgment::{self, Verdict};
use verbatim_core::observe::net;
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::testkit::{self, HttpStub};
use verbatim_core::{ingest, observe};

/// Held by every test that resets or reads the attempt log: it is process
/// global, so two tests counting in parallel would each see the other's calls.
static NET: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Six indexed turns - exactly [`cost::MIN_TURNS`], so the gate is exercised at
/// its boundary rather than somewhere comfortably past it.
const LONG_ENOUGH: &str = "session-errors-a.jsonl";

/// Three indexed turns, which is under the minimum.
const TOO_SHORT: &str = "session-edits.jsonl";

const MODEL: &str = "qwen3:8b";

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

    /// Archive both fixtures, close them, and give each the mechanical row the
    /// judgment half fills in - answering with the two session keys.
    fn observed(&self) -> (String, String) {
        let mut keys = Vec::new();
        for fixture in [LONG_ENOUGH, TOO_SHORT] {
            let path = testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root);
            match ingest::run(&self.data_dir, &path).unwrap() {
                ingest::Outcome::Committed(_) => {}
                other => panic!("{fixture}: {other:?}"),
            }
            keys.push(path.canonicalize().unwrap().to_string_lossy().into_owned());
        }
        let conn = self.conn();
        conn.execute("UPDATE session_meta SET is_final = 1", [])
            .unwrap();
        let observed = observe::observe_new(&conn, &Config::default());
        assert_eq!(observed.written, 2, "{observed:?}");
        (keys.remove(0), keys.remove(0))
    }

    /// A config pointing judgment at `stub`, optionally under a daily budget.
    fn config(&self, stub: &HttpStub, budget: Option<u64>) -> Config {
        let budget = budget
            .map(|b| format!("daily_token_budget = {b}\n"))
            .unwrap_or_default();
        std::fs::write(
            self.config_dir.join(CONFIG_FILE_NAME),
            format!(
                "[provider]\nenabled = true\nbase_url = \"{}\"\n\
                 model = \"{MODEL}\"\nlocal = true\n{budget}",
                stub.base_url()
            ),
        )
        .unwrap();
        Config::load_from(&self.config_dir).unwrap()
    }

    fn turn_ids(&self, session_key: &str) -> Vec<i64> {
        self.conn()
            .prepare("SELECT id FROM turns WHERE session_key = ?1 ORDER BY turn_seq")
            .unwrap()
            .query_map([session_key], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The raw `meta` value holding the day's spend, exactly as stored.
    fn spend_row(&self) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT value FROM meta WHERE key = 'observe.daily_tokens'",
                [],
                |r| r.get(0),
            )
            .ok()
    }

    /// Stamp the spend row with a date and a count, the way a previous
    /// invocation on another day would have left it.
    fn stamp_spend(&self, days_ago: i64, tokens: u64) {
        self.conn()
            .execute(
                "INSERT INTO meta (key, value)
                 VALUES ('observe.daily_tokens',
                         strftime('%Y-%m-%d', 'now', ?1) || ' ' || ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![format!("-{days_ago} days"), tokens.to_string()],
            )
            .unwrap();
    }
}

/// A well-formed answer anchoring one claim per list at the ids it is handed.
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

/// Gate one: a session too short to be worth a paid summary is never sent.
#[test]
fn a_session_under_the_minimum_turn_count_costs_nothing() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (long_enough, too_short) = bench.observed();
    assert_eq!(bench.turn_ids(&too_short).len(), 3);
    assert_eq!(bench.turn_ids(&long_enough).len(), cost::MIN_TURNS);

    // Two canned answers, because an unusable one is asked for again (OBS-04):
    // the falsifying half below spends both.
    let stub = HttpStub::serving(&[
        testkit::chat_completion("{}", 1, 1),
        testkit::chat_completion("{}", 1, 1),
    ]);
    let config = bench.config(&stub, None);

    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, None, &too_short);

    assert_eq!(verdict, Verdict::Skipped(cost::Skip::TooShort { turns: 3 }));
    assert_eq!(
        net::attempts::count(),
        0,
        "a session under the minimum reached for the network: {:?}",
        net::attempts::destinations()
    );
    assert_eq!(bench.spend_row(), None, "nothing was charged");

    // The falsifying half, on the same bench and the same stub: the session at
    // exactly the minimum IS sent, so the refusal above is about the turn count
    // and not about a bench that could never make a call.
    let ids = bench.turn_ids(&long_enough);
    let verdict = judgment::judge(&bench.conn(), &config, None, &long_enough);
    assert!(
        matches!(verdict, Verdict::ParseFailed { .. }),
        "the stub's `{{}}` is not the schema: {verdict:?}"
    );
    assert_eq!(net::attempts::count(), 2);
    assert!(!ids.is_empty());
}

/// Gate three: a day whose budget is spent buys nothing more (D-11, D-12).
#[test]
fn a_spent_daily_budget_stops_the_call_before_it_is_made() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _short) = bench.observed();
    let ids = bench.turn_ids(&judged);

    // Today's row already at the cap, the way a previous invocation would have
    // left it - which is the whole reason the counter is in the store (D-12).
    bench.stamp_spend(0, 500);
    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 325, 69)]);
    let config = bench.config(&stub, Some(100));

    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

    assert_eq!(
        verdict,
        Verdict::Skipped(cost::Skip::BudgetSpent {
            spent: 500,
            budget: 100
        })
    );
    assert_eq!(
        net::attempts::count(),
        0,
        "the budget was checked after the request: {:?}",
        net::attempts::destinations()
    );
    assert_eq!(
        bench.spend_row().unwrap().split_once(' ').unwrap().1,
        "500",
        "a refused call moved the counter"
    );
}

/// The counter moves by exactly what the provider said it charged.
#[test]
fn a_successful_call_moves_the_spend_row_by_the_reported_total() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _short) = bench.observed();
    let ids = bench.turn_ids(&judged);

    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 325, 69)]);
    let config = bench.config(&stub, Some(10_000));

    net::attempts::reset();
    assert_eq!(
        judgment::judge(&bench.conn(), &config, None, &judged),
        Verdict::Stored { tokens: 394 }
    );
    assert_eq!(net::attempts::count(), 1);

    let today: String = bench
        .conn()
        .query_row("SELECT strftime('%Y-%m-%d', 'now')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bench.spend_row(), Some(format!("{today} 394")));
    assert_eq!(cost::spent_today(&bench.conn()).unwrap(), 394);
}

/// Yesterday's spend is not today's: a row stamped with another date reads as
/// zero, which is the whole of the reset (D-12).
#[test]
fn a_spend_row_from_yesterday_is_reset_rather_than_carried_forward() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _short) = bench.observed();
    let ids = bench.turn_ids(&judged);

    // Far over the budget, but stamped with yesterday.
    bench.stamp_spend(1, 999_999);
    assert_eq!(cost::spent_today(&bench.conn()).unwrap(), 0);

    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 325, 69)]);
    let config = bench.config(&stub, Some(1_000));

    net::attempts::reset();
    assert_eq!(
        judgment::judge(&bench.conn(), &config, None, &judged),
        Verdict::Stored { tokens: 394 },
        "yesterday's spend blocked today's call"
    );
    assert_eq!(net::attempts::count(), 1);

    let today: String = bench
        .conn()
        .query_row("SELECT strftime('%Y-%m-%d', 'now')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        bench.spend_row(),
        Some(format!("{today} 394")),
        "yesterday's 999,999 was added to today's spend"
    );
}

/// Gate four: one call per session, and a pass never asks twice.
#[test]
fn a_session_that_already_has_a_status_is_not_asked_again() {
    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let bench = bench();
    let (judged, _short) = bench.observed();
    let ids = bench.turn_ids(&judged);

    let stub = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 325, 69)]);
    let config = bench.config(&stub, None);
    net::attempts::reset();
    assert_eq!(
        judgment::judge(&bench.conn(), &config, None, &judged),
        Verdict::Stored { tokens: 394 }
    );

    // A second stub, so an accidental second request would be answered rather
    // than merely refused - the count is what fails, not a dead socket.
    let again = HttpStub::serving(&[testkit::chat_completion(&answer(&ids), 1, 1)]);
    let config = bench.config(&again, None);
    net::attempts::reset();
    let verdict = judgment::judge(&bench.conn(), &config, None, &judged);

    assert_eq!(
        verdict,
        Verdict::Skipped(cost::Skip::AlreadyJudged {
            status: judgment::STATUS_OK.to_owned()
        })
    );
    assert_eq!(
        net::attempts::count(),
        0,
        "one session was bought twice: {:?}",
        net::attempts::destinations()
    );
    assert_eq!(cost::spent_today(&bench.conn()).unwrap(), 394);

    // And `regenerate`'s waiver is the one thing that gets through it.
    net::attempts::reset();
    assert_eq!(
        judgment::judge_again(&bench.conn(), &config, None, &judged),
        Verdict::Stored { tokens: 2 }
    );
    assert_eq!(net::attempts::count(), 1);
    assert_eq!(cost::spent_today(&bench.conn()).unwrap(), 396);
}

/// Gate two: what is cut out is named, so the model is never shown a session
/// that looks whole and is not.
#[test]
fn a_truncated_session_tells_the_model_what_it_is_not_seeing() {
    // A session far over the budget, in lines small enough that no single one
    // trips the per-turn clip: this is the middle-elision arm.
    let line = "x".repeat(500);
    let many: Vec<String> = (0..1_000)
        .map(|i| format!("turn_id={i} user: {line}"))
        .collect();
    let cut = cost::truncate(&many);

    assert!(
        cut.len() <= cost::TRUNCATION_BUDGET + 200,
        "{} chars",
        cut.len()
    );
    assert!(
        cut.contains(cost::ELIDED),
        "the cut carries no elision marker: {}",
        &cut[..200.min(cut.len())]
    );
    assert!(cut.contains("turn_id=0 "), "the opening was dropped");
    assert!(cut.contains("turn_id=999 "), "the close was dropped");

    // One enormous turn is clipped rather than allowed to eat the whole budget,
    // and it says so too.
    let huge = vec![format!(
        "turn_id=1 user: {}",
        "y".repeat(cost::TRUNCATION_BUDGET * 2)
    )];
    let cut = cost::truncate(&huge);
    assert!(cut.len() < cost::TRUNCATION_BUDGET);
    assert!(
        cut.contains(cost::ELIDED),
        "the clipped turn is silent about it"
    );

    // And a session inside the budget is handed over whole, with no marker at
    // all - so the assertions above are about truncation and not about a
    // function that always says something.
    let short = vec![
        "turn_id=1 user: hello".to_owned(),
        "turn_id=2 assistant: hi".to_owned(),
    ];
    let whole = cost::truncate(&short);
    assert_eq!(whole, "turn_id=1 user: hello\nturn_id=2 assistant: hi");
    assert!(!whole.contains(cost::ELIDED));
}
