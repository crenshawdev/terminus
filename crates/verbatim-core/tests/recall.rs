//! The query layer both terminal recall and the MCP server sit on.
//!
//! Every test runs against a store holding the whole fixture corpus, built by
//! the ordinary ingest path, so what is asserted is what a real pass wrote and
//! not what a helper decided to insert.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::config::Config;
use verbatim_core::recall::{
    search, Hit, Query, Reason, Request, Response, Scope, MAX_QUERY_TOKENS, MAX_RESULTS,
};
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{ingest, testkit};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    /// The root the rooted fixtures' `cwd` values were substituted with, and so
    /// the parent of every project directory a scoped search can stand in.
    root: PathBuf,
}

/// A store with every transcript fixture ingested.
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

    /// Archive one more session, written by the test and ingested through the
    /// ordinary path.
    ///
    /// The fixture corpus is 47 turns and no single term reaches 17 of them, so
    /// a claim about a result limit in the tens has nothing to be true about
    /// until a session exists that exceeds it. The records are as thin as the
    /// parser accepts: what is being measured is how many rows come back, not
    /// what is in them.
    fn flood(&self, name: &str, token: &str, records: usize) -> PathBuf {
        let session = "77777777-7777-4777-8777-777777777777";
        let mut body = String::new();
        for n in 0..records {
            let line = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                // A directory nothing created, so project identity degrades to
                // the path itself rather than spawning git for an answer.
                "cwd": self.work.join("flooded").to_string_lossy(),
                "sessionId": session,
                "type": "user",
                "uuid": format!("dddddddd-0000-4000-8000-{n:012}"),
                "timestamp": format!("2026-08-12T21:{:02}:{:02}.000Z", n / 60, n % 60),
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": format!("{token} line {n}")}],
                },
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }

        let path = self.work.join(name);
        std::fs::write(&path, body).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{name}: {other:?}"),
        }
        path
    }
}

/// The turn ids one FTS5 expression returns, or the error it raised.
///
/// Deliberately fallible where the rest of the suite unwraps: the whole point of
/// D-09 is which strings raise and which do not, so the error has to be a value
/// the test can assert on rather than a panic.
fn matching(conn: &Connection, expression: &str) -> rusqlite::Result<Vec<i64>> {
    conn.prepare("SELECT rowid FROM turns_fts WHERE turns_fts MATCH ?1 ORDER BY rowid")?
        .query_map([expression], |r| r.get::<_, i64>(0))?
        .collect()
}

/// A user's raw string, run the way a read command will run it.
fn search_ids(conn: &Connection, raw: &str) -> rusqlite::Result<Vec<i64>> {
    match Query::parse(raw).match_expression() {
        Some(expression) => matching(conn, &expression),
        // `MATCH ''` is itself an fts5 error, so a query that reduces to no
        // tokens is answered without asking SQLite anything.
        None => Ok(Vec::new()),
    }
}

/// One turn's record bytes, read back out of its session blob.
fn record(conn: &Connection, turn_id: i64) -> String {
    let (bytes, _) = testkit::read_turn(conn, turn_id);
    String::from_utf8(bytes).unwrap()
}

/// How many archived turns carry `needle` anywhere in their raw record.
fn turns_whose_record_contains(conn: &Connection, needle: &str) -> Vec<i64> {
    conn.prepare("SELECT id FROM turns ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .filter(|id| record(conn, *id).contains(needle))
        .collect()
}

/// The strings D-09 measured, plus the two shapes RCL-06 has to survive: a query
/// that is nothing at all and a query that is nothing but punctuation.
const HOSTILE: &[&str] = &[
    "src/worker/S.ts",
    "foo AND (bar",
    "\"unbalanced",
    "*",
    "",
    "!@#$%^&*()",
];

/// D-09: no raw query reaches `MATCH`, so none of these is an error.
///
/// The control is the first half of the loop. "The query layer returned Ok" says
/// nothing unless the raw string really would have failed, and every one of
/// these six does - `fts5: syntax error near "/"`, `near ""`, `unterminated
/// string`, `unknown special query`. RCL-06 forbids a non-zero exit for a query
/// that simply found nothing, and searching for a path is the phase's headline
/// case, so the most natural command in the product is exactly the one that
/// would have failed.
#[test]
fn no_raw_query_reaches_fts5() {
    let bench = bench();
    let conn = bench.conn();

    for raw in HOSTILE {
        assert!(
            matching(&conn, raw).is_err(),
            "`{raw}` is not an fts5 syntax error, so routing it through the \
             query layer proves nothing"
        );

        let result = search_ids(&conn, raw);
        assert!(
            result.is_ok(),
            "`{raw}` reached fts5 as written: {:?}",
            result.unwrap_err()
        );
    }
}

/// The headline case: `verbatim search src/worker/S.ts` finds the turn that
/// names that path.
#[test]
fn a_path_query_returns_the_turn_that_contains_it() {
    let bench = bench();
    let conn = bench.conn();

    let carriers = turns_whose_record_contains(&conn, "src/worker/S.ts");
    assert_eq!(
        carriers.len(),
        1,
        "the fixture corpus should name that path in exactly one turn"
    );

    let hits = search_ids(&conn, "src/worker/S.ts").unwrap();
    assert!(
        hits.contains(&carriers[0]),
        "turn {} carries `src/worker/S.ts` and the search missed it: {hits:?}",
        carriers[0]
    );
}

/// A query that found nothing is an empty result, never an error and never a
/// wrong one.
#[test]
fn a_query_matching_nothing_is_an_empty_result() {
    let bench = bench();
    let conn = bench.conn();

    assert_eq!(
        search_ids(&conn, "zzzzunfindable").unwrap(),
        Vec::<i64>::new()
    );
    // A real token beside one nothing carries: the conjunction is what makes
    // this empty, not the absence of any indexable word at all.
    assert!(!search_ids(&conn, testkit::UNIQUE_TOKEN).unwrap().is_empty());
    assert_eq!(
        search_ids(&conn, &format!("{} zzzzunfindable", testkit::UNIQUE_TOKEN)).unwrap(),
        Vec::<i64>::new()
    );
}

/// A query that reduces to no tokens asks SQLite nothing.
#[test]
fn a_query_of_no_tokens_makes_no_match_call() {
    for raw in ["", "   ", "!@#$%^&*()", "-- ... --"] {
        let query = Query::parse(raw);
        assert!(query.is_empty(), "`{raw}` produced {:?}", query.tokens());
        assert_eq!(query.match_expression(), None, "`{raw}`");
    }
}

/// Every token is a quoted fts5 string and the tokens are conjoined, so nothing
/// the user typed can be read as an operator.
#[test]
fn tokens_are_quoted_and_conjoined() {
    assert_eq!(
        Query::parse("src/worker/S.ts")
            .match_expression()
            .as_deref(),
        Some(r#""src" AND "worker" AND "S" AND "ts""#)
    );
    // `AND` survives as a term rather than as the operator it looks like: it is
    // inside quotes, where fts5 reads a string and hands it to the tokenizer.
    assert_eq!(
        Query::parse("foo AND (bar").match_expression().as_deref(),
        Some(r#""foo" AND "AND" AND "bar""#)
    );
}

/// Query tokens are not expanded, because the expansion already happened at
/// index time.
///
/// Both halves matter. `SearchManager` stays one term - expanding it into
/// `Search` and `Manager` would widen a specific query into every turn that
/// says `manager` - and a query for `manager` still finds the turn, because
/// `index::expand` put that component in the stored body (D-13).
#[test]
fn query_tokens_are_not_expanded() {
    let bench = bench();
    let conn = bench.conn();

    assert_eq!(Query::parse("SearchManager").tokens(), ["SearchManager"]);

    let carriers = turns_whose_record_contains(&conn, "SearchManager");
    assert_eq!(carriers.len(), 1, "one fixture turn names SearchManager");

    for raw in ["SearchManager", "manager"] {
        assert!(
            search_ids(&conn, raw).unwrap().contains(&carriers[0]),
            "`{raw}` missed turn {}",
            carriers[0]
        );
    }
}

/// The same token twice is one term, whatever case it was typed in.
#[test]
fn repeated_tokens_collapse() {
    let query = Query::parse("cargo Cargo cargo/cargo");
    assert_eq!(query.tokens(), ["cargo"]);
    assert!(!query.truncated());
}

/// A pasted stack trace cannot build a thousand-term expression.
///
/// Truncation is safe in one direction only, and this is the direction: the
/// tokens are conjoined, so dropping some makes the query broader than the one
/// asked for rather than wrong in a way the user cannot see.
#[test]
fn the_token_count_is_bounded() {
    let bench = bench();
    let conn = bench.conn();

    let pasted: String = (0..MAX_QUERY_TOKENS * 4)
        .map(|n| format!("frame{n} "))
        .collect();

    let query = Query::parse(&pasted);
    assert_eq!(query.tokens().len(), MAX_QUERY_TOKENS);
    assert!(query.truncated());
    assert_eq!(
        query.match_expression().unwrap().matches(" AND ").count(),
        MAX_QUERY_TOKENS - 1
    );

    // And it still runs: a bounded expression is one fts5 accepts.
    assert_eq!(search_ids(&conn, &pasted).unwrap(), Vec::<i64>::new());

    let short = Query::parse("one two");
    assert!(!short.truncated());
}

/// A config with no roots and the given exclusions: the read path never opens a
/// transcript, so the roots are not what it is about.
fn config(exclusions: &[&str]) -> Config {
    Config::from_parts(
        Vec::new(),
        exclusions.iter().map(|e| (*e).to_owned()).collect(),
    )
}

/// One search, as a read command runs it.
fn answer(conn: &Connection, exclusions: &[&str], scope: Scope, raw: &str) -> Response {
    search::run(
        conn,
        &config(exclusions),
        &Request::new(Query::parse(raw), scope).limit(MAX_RESULTS),
    )
    .unwrap()
}

/// The ranked search over every project, which is what the ranking tests are
/// about.
fn ranked(conn: &Connection, raw: &str, limit: usize) -> Vec<Hit> {
    search::run(
        conn,
        &config(&[]),
        &Request::new(Query::parse(raw), Scope::Everything).limit(limit),
    )
    .unwrap()
    .hits
}

/// The last component of a session key, which is the fixture's own file name.
fn fixture_of(hit: &Hit) -> &str {
    hit.session_key
        .rsplit(['/', '\\'])
        .next()
        .expect("a session key is a path")
}

/// Hits come back best-first, across every session that matched.
#[test]
fn hits_come_back_in_descending_relevance() {
    let bench = bench();
    let conn = bench.conn();

    let found = ranked(&conn, "cargo", MAX_RESULTS);
    assert!(found.len() > 2, "{found:?}");

    let mut sessions: Vec<&str> = found.iter().map(fixture_of).collect();
    sessions.sort_unstable();
    sessions.dedup();
    assert!(
        sessions.len() > 1,
        "one query should reach several fixtures: {sessions:?}"
    );

    for pair in found.windows(2) {
        assert!(
            pair[0].relevance >= pair[1].relevance,
            "out of order: {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }

    // The sign, which is the thing a "hits came back" assertion cannot see.
    // bm25 is negative and more negative is better, so a relevance that came
    // back unnegated would be ordered exactly backwards and still be sorted.
    assert!(
        found.iter().all(|hit| hit.relevance > 0.0),
        "relevance is not the negated bm25: {found:?}"
    );
}

/// D-07: a subagent turn is searchable, and at equal score it sorts below the
/// turn the user actually watched.
///
/// The two fixture turns carry the same sentence and nothing else, so their
/// projected bodies are byte-identical and bm25 scores them identically - which
/// is what makes this a test of the tie-break rather than of bm25.
#[test]
fn a_sidechain_turn_sorts_below_an_identical_top_level_turn() {
    let bench = bench();
    let conn = bench.conn();

    let found = ranked(
        &conn,
        "the echo agent repeats this line exactly",
        MAX_RESULTS,
    );
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        found[0].relevance, found[1].relevance,
        "the two turns should score identically, so the order is the tie-break"
    );

    assert_eq!(fixture_of(&found[0]), "session-recall.jsonl");
    assert!(!found[0].sidechain);
    assert_eq!(fixture_of(&found[1]), "agent-echo.jsonl");
    assert!(found[1].sidechain);
}

/// The result limit is a budget the caller may lower and may not raise.
#[test]
fn the_result_limit_cannot_be_raised() {
    let bench = bench();
    bench.flood("flooded.jsonl", "floodtoken", MAX_RESULTS + 10);
    let conn = bench.conn();

    assert_eq!(
        Request::new(Query::parse("floodtoken"), Scope::Everything)
            .limit(usize::MAX)
            .effective_limit(),
        MAX_RESULTS
    );

    assert_eq!(ranked(&conn, "floodtoken", usize::MAX).len(), MAX_RESULTS);
    assert_eq!(ranked(&conn, "floodtoken", 3).len(), 3);
}

/// The distinct projects a set of hits came from.
fn projects_of(response: &Response) -> Vec<String> {
    let mut out: Vec<String> = response
        .hits
        .iter()
        .map(|hit| hit.project.clone().unwrap_or_default())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// D-12: the default scope is the project the caller is standing in, found by
/// matching the directory against the keys ingest already wrote.
#[test]
fn a_search_is_scoped_to_the_project_the_caller_stands_in() {
    let bench = bench();
    let conn = bench.conn();

    let alpha = testkit::fixture_project("session-recall.jsonl", &bench.root);
    let alpha_key = alpha.to_string_lossy().into_owned();

    // The control: this word is in more than one project, so a single-project
    // result is the scope working and not the corpus being thin.
    let everywhere = answer(&conn, &[], Scope::Everything, "the");
    assert!(
        projects_of(&everywhere).len() > 2,
        "`the` should reach several projects: {:?}",
        projects_of(&everywhere)
    );

    let here = answer(&conn, &[], Scope::Directory(alpha.clone()), "the");
    assert!(!here.hits.is_empty());
    assert_eq!(here.reason, None);
    assert_eq!(projects_of(&here), vec![alpha_key.clone()]);

    // A subdirectory of the project resolves to the same key: the match is on
    // path components, so anywhere inside the project is inside the project.
    let deeper = answer(
        &conn,
        &[],
        Scope::Directory(alpha.join("crates").join("core")),
        "the",
    );
    assert_eq!(projects_of(&deeper), vec![alpha_key.clone()]);

    // And naming it explicitly is the same rule over a caller-supplied string.
    let named = answer(&conn, &[], Scope::parse(&alpha_key), "the");
    assert_eq!(projects_of(&named), vec![alpha_key]);
    assert_eq!(Scope::parse("*"), Scope::Everything);
}

/// A directory no archived project covers matches nothing, and says so.
///
/// The alternative is silently widening to every project, which reads as a
/// scoping bug to anyone who notices and as an answer to everyone who does not.
#[test]
fn an_unknown_directory_matches_nothing_and_says_why() {
    let bench = bench();
    let conn = bench.conn();

    let elsewhere = bench.root.join("no-project-here");
    let response = answer(&conn, &[], Scope::Directory(elsewhere.clone()), "the");

    assert!(response.hits.is_empty());
    assert_eq!(
        response.reason,
        Some(Reason::UnknownProject {
            named: elsewhere.to_string_lossy().into_owned()
        })
    );
    // The reason renders: RCL-10 hands this to a caller to print.
    assert!(response
        .reason
        .unwrap()
        .to_string()
        .contains("no-project-here"));
}

/// An exclusion covering the filesystem root short-circuits before any query.
#[test]
fn excluding_everything_short_circuits() {
    let bench = bench();
    let conn = bench.conn();

    let response = answer(&conn, &["/"], Scope::Everything, "the");
    assert!(response.hits.is_empty());
    assert_eq!(response.reason, Some(Reason::EverythingExcluded));
}

/// The scope a caller stands in, when the config excludes it.
#[test]
fn standing_in_an_excluded_project_says_so() {
    let bench = bench();
    let conn = bench.conn();

    let alpha = testkit::fixture_project("session-recall.jsonl", &bench.root);
    let alpha_key = alpha.to_string_lossy().into_owned();

    let response = answer(&conn, &[&alpha_key], Scope::Directory(alpha), "the");
    assert!(response.hits.is_empty());
    assert_eq!(
        response.reason,
        Some(Reason::ProjectExcluded {
            project: alpha_key.clone()
        })
    );

    // And `*` still hides it, where there is no scope to report a reason for.
    let everywhere = answer(&conn, &[&alpha_key], Scope::Everything, "the");
    assert_eq!(everywhere.reason, None);
    assert!(!everywhere.hits.is_empty());
    assert!(!projects_of(&everywhere).contains(&alpha_key));
}
