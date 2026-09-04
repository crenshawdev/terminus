//! PRIV-03's opt-in knob, over the derived projections `recall_search` returns.
//!
//! Every store here is built by the ordinary `ingest::run` path over the whole
//! fixture corpus, and the knob is turned on the only way anything can turn it
//! on: by writing a `verbatim.toml` and loading it through `Config::load_from`.
//! That is deliberate rather than incidental. There is no in-memory setter, so
//! a test that could flip the boolean on a `Config` it built would be exercising
//! a path the binary does not have - and the reason the setter is absent is that
//! an environment variable or an in-process toggle is inherited by the hook and
//! by the MCP server, which is the one thing the escape hatch must never be able
//! to do.
//!
//! The sentinels are `tests/fixtures/session-secrets.jsonl`'s, planted for the
//! phase 2 egress work: unrealistic short values, because realistic ones in a
//! public repository trip GitHub push protection.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::config::{Config, REDACTED};
use verbatim_core::recall::{search, Query, Request, Response, Scope, EXCERPT_CHARS};
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{ingest, testkit};

/// The `Authorization: Bearer` value planted in `session-secrets.jsonl`.
const AUTHZ_SENTINEL: &str = "sk-VBEGRESS-authz-9f2";

/// The fragment every planted sentinel in that fixture shares.
///
/// Asserted on rather than the individual values because a search returns
/// several turns of that session at once: one surviving sentinel of any shape
/// is a leak, and naming only the one under test would miss it.
const SENTINEL_MARK: &str = "VBEGRESS";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    root: PathBuf,
}

/// A store with every transcript fixture ingested, built the way
/// `tests/recall.rs` builds one.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();

    for fixture in testkit::TRANSCRIPT_FIXTURES {
        let rooted = testkit::ROOTED_FIXTURES.iter().any(|(f, _)| f == fixture);
        let path = if rooted {
            testkit::copy_rooted_fixture_into(fixture, &work, &root)
        } else {
            testkit::copy_fixture_into(fixture, &work)
        };
        match ingest::run(&data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{fixture}: {other:?}"),
        }
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

    /// A config that has been through a real `verbatim.toml`.
    ///
    /// `on = false` writes the table with the key spelled `false` rather than
    /// omitting it, so the two configs differ in exactly one token of one file
    /// and nothing else about how they were built.
    fn config(&self, on: bool) -> Config {
        let dir = self.work.join(format!("config-{on}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("verbatim.toml"),
            format!("[privacy]\nredact_recall = {on}\n"),
        )
        .unwrap();
        Config::load_from(&dir).unwrap()
    }

    /// The project one rooted fixture's `cwd` names.
    fn project(&self, fixture: &str) -> Scope {
        Scope::Named(
            testkit::fixture_project(fixture, &self.root)
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// Archive one session the test wrote itself, through the ordinary path.
    fn archive(&self, name: &str, body: &str) {
        let path = self.work.join(name);
        std::fs::write(&path, body).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{name}: {other:?}"),
        }
    }

    fn search(&self, config: &Config, scope: &Scope, raw: &str) -> Response {
        let request = Request::new(Query::parse(raw), scope.clone());
        search::run(&self.conn(), config, &request).unwrap()
    }
}

/// Every excerpt of a response, joined - which is the whole of what the caller
/// is handed as text.
fn excerpts(response: &Response) -> String {
    assert!(
        !response.hits.is_empty(),
        "the query matched nothing, so there is no projection to be filtered"
    );
    response
        .hits
        .iter()
        .map(|hit| hit.excerpt.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The default, stated as an assertion: with the knob off the archive answers
/// exactly what it holds.
#[test]
fn an_unfiltered_search_returns_the_planted_credential_whole() {
    let bench = bench();
    let scope = bench.project("session-secrets.jsonl");
    let text = excerpts(&bench.search(&bench.config(false), &scope, "gateway"));

    assert!(
        text.contains(AUTHZ_SENTINEL),
        "the default must not filter anything:\n{text}"
    );
}

/// AC1's search half. One token of one config file is the only difference.
#[test]
fn the_knob_takes_every_planted_credential_out_of_the_search_excerpt() {
    let bench = bench();
    let scope = bench.project("session-secrets.jsonl");

    let raw = excerpts(&bench.search(&bench.config(false), &scope, "gateway"));
    let filtered = excerpts(&bench.search(&bench.config(true), &scope, "gateway"));

    // The premise: this query really does return credential-bearing text, so
    // the assertions below are about the filter rather than about a query that
    // stopped matching.
    assert!(raw.contains(AUTHZ_SENTINEL), "{raw}");

    assert!(
        filtered.contains(REDACTED),
        "a filtered excerpt names what went:\n{filtered}"
    );
    for absent in [AUTHZ_SENTINEL, SENTINEL_MARK, "authz"] {
        assert!(
            !filtered.contains(absent),
            "{absent:?} survived into the excerpt:\n{filtered}"
        );
    }
    // The turns are still the same turns: filtering is not a filter on results.
    assert_eq!(
        bench
            .search(&bench.config(true), &scope, "gateway")
            .hits
            .iter()
            .map(|hit| hit.turn_id)
            .collect::<Vec<_>>(),
        bench
            .search(&bench.config(false), &scope, "gateway")
            .hits
            .iter()
            .map(|hit| hit.turn_id)
            .collect::<Vec<_>>()
    );
}

/// The control: a project with nothing credential-shaped in it is untouched in
/// both settings.
///
/// Here because the tolerated failure direction of the rule set is
/// over-matching (`observe::egress`'s module doc), and this phase points those
/// rules at free prose rather than at a request body. If a length floor is low
/// enough to fire on ordinary English, this is where it shows.
#[test]
fn the_knob_leaves_ordinary_prose_exactly_as_it_was() {
    let bench = bench();
    let scope = bench.project("session-recall.jsonl");

    assert_eq!(
        excerpts(&bench.search(&bench.config(true), &scope, "retry budget")),
        excerpts(&bench.search(&bench.config(false), &scope, "retry budget")),
    );
}

/// D-02, falsifiably: the filter runs over the FULL projection, before the
/// 240-character window is cut.
///
/// The turn is built so that the two orderings disagree. Its projection opens
/// with `Authorization: Bearer` and a single unbroken value long enough that the
/// window centred on the query token contains the END of that value and not the
/// `Authorization:` that names it. Filter first and the whole value is gone;
/// window first and the header rule is handed a fragment with no name in it,
/// matches nothing, and the tail of the credential is returned to the model.
///
/// Both premises are asserted before the claim, because either one failing
/// would make the claim true for a reason that has nothing to do with ordering.
#[test]
fn a_credential_whose_name_falls_outside_the_window_is_still_taken() {
    let bench = bench();
    let tail = "VBEGRESS-tail-4c2";
    let value = format!("{}{tail}", "q".repeat(400 - tail.len()));
    let text = format!(
        "Authorization: Bearer {value} zorbfilament is the token this turn is found by, \
         and the rest of this sentence is here only so that the projection runs past the \
         end of the window the excerpt would cut around that word."
    );
    assert!(
        text.chars().count() > EXCERPT_CHARS,
        "the projection has to exceed the window for the ordering to matter"
    );

    let record = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "cwd": bench.work.join("windowed").to_string_lossy(),
        "sessionId": "aaaaaaaa-1111-4111-8111-111111111111",
        "type": "user",
        "uuid": "aaaaaaaa-0000-4000-8000-000000000001",
        "timestamp": "2026-09-04T10:00:00.000Z",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    });
    bench.archive("session-windowed.jsonl", &format!("{record}\n"));

    let scope = Scope::Everything;
    let raw = excerpts(&bench.search(&bench.config(false), &scope, "zorbfilament"));
    let filtered = excerpts(&bench.search(&bench.config(true), &scope, "zorbfilament"));

    // Premise one: unfiltered, the window really does carry the tail of the
    // credential. Premise two: it really does not carry the name that labels
    // it, so a rule run after the cut has nothing to catch it by.
    assert!(raw.contains(tail), "{raw}");
    assert!(!raw.contains("Authorization"), "{raw}");

    assert!(
        !filtered.contains(SENTINEL_MARK),
        "a credential whose name was cut away survived the filter:\n{filtered}"
    );
}
