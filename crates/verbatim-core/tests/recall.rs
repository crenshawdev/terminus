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
    context, search, EntityMatch, Filters, Hit, Query, Reason, Request, Response, Scope, Window,
    MAX_CONTEXT_SIDE, MAX_QUERY_TOKENS, MAX_RESULTS,
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

        self.archive(name, &body)
    }

    /// A session of `Read` tool calls, one per named path.
    ///
    /// A path reaches the `entities` table only from a tool call's own input
    /// (RCL-02), so a document frequency to weight by is something only a real
    /// tool record can produce.
    fn reads(&self, name: &str, paths: &[&str]) -> PathBuf {
        let mut body = String::new();
        for (n, file) in paths.iter().enumerate() {
            let line = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": self.work.join("entities").to_string_lossy(),
                "sessionId": "88888888-8888-4888-8888-888888888888",
                "type": "assistant",
                "uuid": format!("eeeeeeee-0000-4000-8000-{n:012}"),
                "timestamp": format!("2026-08-12T22:{:02}:{:02}.000Z", n / 60, n % 60),
                "message": {
                    "role": "assistant",
                    "model": "claude-opus-5",
                    "content": [{
                        "type": "tool_use",
                        "id": format!("toolu_e{n}"),
                        "name": "Read",
                        "input": {"file_path": file},
                    }],
                },
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }
        self.archive(name, &body)
    }

    /// A session whose records carry the given timestamps, in the given order.
    ///
    /// Written so a test can hand it a clock that goes backwards: 31 of 62 real
    /// transcripts carry a record whose timestamp does, which is the whole of
    /// why D-22 orders a context window by `turn_seq`.
    fn timeline(&self, name: &str, stamps: &[&str]) -> PathBuf {
        let mut body = String::new();
        for (n, stamp) in stamps.iter().enumerate() {
            let line = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": self.work.join("timeline").to_string_lossy(),
                "sessionId": "99999999-9999-4999-8999-999999999999",
                "type": "user",
                "uuid": format!("ffffffff-0000-4000-8000-{n:012}"),
                "timestamp": stamp,
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": format!("timeline step {n}")}],
                },
            });
            body.push_str(&line.to_string());
            body.push('\n');
        }
        self.archive(name, &body)
    }

    fn archive(&self, name: &str, body: &str) -> PathBuf {
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

/// One filtered search over every project.
fn narrowed(conn: &Connection, raw: &str, filters: Filters) -> verbatim_core::Result<Response> {
    search::run(
        conn,
        &config(&[]),
        &Request::new(Query::parse(raw), Scope::Everything)
            .filters(filters)
            .limit(MAX_RESULTS),
    )
}

/// The timestamps a response answered with, in the order it answered.
fn stamps(response: &Response) -> Vec<String> {
    response
        .hits
        .iter()
        .map(|hit| hit.ts.clone().expect("every fixture turn carries a ts"))
        .collect()
}

/// One turn's `tool_name`, straight off the row.
fn tool_of(conn: &Connection, turn_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT tool_name FROM turns WHERE id = ?1",
        [turn_id],
        |r| r.get(0),
    )
    .unwrap()
}

/// D-16: `turns.tool_name` is the whole tool filter.
#[test]
fn a_tool_filter_returns_only_that_tools_turns() {
    let bench = bench();
    let conn = bench.conn();

    // The control: this query reaches turns that are not tool calls at all, so
    // a filtered result being all-Bash is the filter and not the corpus.
    let unfiltered = ranked(&conn, "cargo", MAX_RESULTS);
    assert!(unfiltered
        .iter()
        .any(|hit| tool_of(&conn, hit.turn_id).is_none()));

    let bash = narrowed(
        &conn,
        "cargo",
        Filters {
            tool: Some("Bash".into()),
            ..Filters::default()
        },
    )
    .unwrap();
    assert!(!bash.hits.is_empty());
    assert!(bash.hits.len() < unfiltered.len());
    for hit in &bash.hits {
        assert_eq!(tool_of(&conn, hit.turn_id).as_deref(), Some("Bash"));
    }

    // A tool nothing in the corpus ran is an empty result, not an error.
    let none = narrowed(
        &conn,
        "cargo",
        Filters {
            tool: Some("WebFetch".into()),
            ..Filters::default()
        },
    )
    .unwrap();
    assert!(none.hits.is_empty());
    assert_eq!(none.reason, None);
}

#[test]
fn a_kind_filter_returns_only_that_record_type() {
    let bench = bench();
    let conn = bench.conn();

    let unfiltered = ranked(&conn, "the", MAX_RESULTS);
    assert!(unfiltered.iter().any(|hit| hit.record_type != "user"));

    let users = narrowed(
        &conn,
        "the",
        Filters {
            kind: Some("user".into()),
            ..Filters::default()
        },
    )
    .unwrap();
    assert!(!users.hits.is_empty());
    assert!(users.hits.iter().all(|hit| hit.record_type == "user"));
}

/// The path filter is structural: it reads the `paths` table, so a turn that
/// merely names the path in prose is not a hit.
///
/// Both turns match the query as text - one is the `Read` that opened the file,
/// the other is the sentence about what was in it - which is what makes this a
/// test of where the filter reads rather than of what the query matched.
#[test]
fn a_path_filter_is_structural_and_not_textual() {
    let bench = bench();
    let conn = bench.conn();

    let textual = ranked(&conn, "docs/RETRY.md", MAX_RESULTS);
    assert_eq!(textual.len(), 2, "{textual:?}");

    let structural = narrowed(
        &conn,
        "docs/RETRY.md",
        Filters {
            paths: vec!["docs/RETRY.md".into()],
            ..Filters::default()
        },
    )
    .unwrap();
    assert_eq!(structural.hits.len(), 1);
    assert_eq!(
        tool_of(&conn, structural.hits[0].turn_id).as_deref(),
        Some("Read"),
        "the surviving hit should be the tool call that opened the file"
    );

    // Several paths are a union, and a turn carrying two of them is still one
    // hit rather than one per row.
    let union = narrowed(
        &conn,
        "the",
        Filters {
            paths: vec!["docs/RETRY.md".into(), "src".into()],
            ..Filters::default()
        },
    )
    .unwrap();
    let mut ids: Vec<i64> = union.hits.iter().map(|hit| hit.turn_id).collect();
    let unique = {
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    };
    assert_eq!(unique, union.hits.len(), "a turn came back twice");
}

/// D-23: the window is a lexicographic comparison against `turns.ts`, and a
/// bare date covers its own whole day.
#[test]
fn a_time_window_includes_and_excludes_by_timestamp() {
    let bench = bench();
    let conn = bench.conn();

    let everything = stamps(&narrowed(&conn, "cargo", Filters::default()).unwrap());
    assert!(everything.iter().any(|ts| ts.starts_with("2026-08-12")));
    assert!(
        everything.iter().any(|ts| ts.starts_with("2026-08-19")),
        "the corpus needs turns on two days for a window to exclude anything"
    );

    // The bare-date case that discriminates: `until` on a day whose turns
    // happen in the afternoon. Compared as written, `2026-08-12` sorts BEFORE
    // every timestamp on 2026-08-12, so an unextended bound would return
    // nothing at all from that day.
    let until_day = stamps(
        &narrowed(
            &conn,
            "cargo",
            Filters {
                until: Some("2026-08-12".into()),
                ..Filters::default()
            },
        )
        .unwrap(),
    );
    assert!(!until_day.is_empty());
    assert!(until_day.iter().all(|ts| ts.starts_with("2026-08-12")));
    assert!(until_day.iter().any(|ts| ts.as_str() > "2026-08-12T12:00"));

    let since_day = stamps(
        &narrowed(
            &conn,
            "cargo",
            Filters {
                since: Some("2026-08-19".into()),
                ..Filters::default()
            },
        )
        .unwrap(),
    );
    assert!(!since_day.is_empty());
    assert!(since_day.iter().all(|ts| ts.starts_with("2026-08-19")));

    // A full timestamp is used as written.
    let afternoon = stamps(
        &narrowed(
            &conn,
            "cargo",
            Filters {
                since: Some("2026-08-12T14:00:00.000Z".into()),
                until: Some("2026-08-12T23:59:59.999Z".into()),
                ..Filters::default()
            },
        )
        .unwrap(),
    );
    assert!(!afternoon.is_empty());
    assert!(afternoon
        .iter()
        .all(|ts| ts.as_str() >= "2026-08-12T14:00:00.000Z" && ts.starts_with("2026-08-12")));
    assert!(
        afternoon.len() < until_day.len(),
        "the morning turns should have dropped out: {afternoon:?}"
    );

    // Both ends, one day.
    let one_day = stamps(
        &narrowed(
            &conn,
            "cargo",
            Filters {
                since: Some("2026-08-12".into()),
                until: Some("2026-08-12".into()),
                ..Filters::default()
            },
        )
        .unwrap(),
    );
    assert_eq!(one_day, until_day);
}

/// A time of any other shape is a caller error, not an empty result.
///
/// Every string compares cleanly against every other, so a malformed bound
/// would return a plausible wrong answer and nothing would say so.
#[test]
fn a_malformed_time_is_a_caller_error() {
    let bench = bench();
    let conn = bench.conn();

    for raw in [
        "2026-8-1",
        "12/08/2026",
        "yesterday",
        "2026-08-12T14:00:00Z",
        "2026-08-12 14:00:00.000Z",
        "",
    ] {
        for (field, filters) in [
            (
                "since",
                Filters {
                    since: Some(raw.into()),
                    ..Filters::default()
                },
            ),
            (
                "until",
                Filters {
                    until: Some(raw.into()),
                    ..Filters::default()
                },
            ),
        ] {
            match narrowed(&conn, "cargo", filters) {
                Err(verbatim_core::Error::InvalidTimeFilter { field: got, value }) => {
                    assert_eq!(got, field);
                    assert_eq!(value, raw);
                }
                other => panic!("`{raw}` as {field} was not a caller error: {other:?}"),
            }
        }
    }
}

/// RCL-04 matches a whole stored value, never one query token against one.
///
/// `path` and `error` values are multi-token by construction, so a
/// token-equality rule could never fire for the two kinds this phase headlines.
#[test]
fn an_entity_match_is_against_the_whole_stored_value() {
    let path = "src/worker/S.ts";

    assert_eq!(
        Query::parse(path).matches_entity(path),
        Some(EntityMatch::Exact)
    );
    // Spelling that survives the tokenizer is the same value.
    assert_eq!(
        Query::parse("SRC worker s TS").matches_entity(path),
        Some(EntityMatch::Exact)
    );
    // The query asks for more than the value, so the value is covered.
    assert_eq!(
        Query::parse("who edited src/worker/S.ts today").matches_entity(path),
        Some(EntityMatch::Covered)
    );
    // One token of a multi-token value is not that value.
    assert_eq!(Query::parse("worker").matches_entity(path), None);
    assert_eq!(Query::parse("").matches_entity(path), None);
    // A value that tokenizes to nothing cannot be asked for.
    assert_eq!(Query::parse("anything").matches_entity("///"), None);
}

/// RCL-04's point: an exact structural match outranks a free-text one.
///
/// The assertion is the reordering itself. `cargo` reaches turns that ran it as
/// a command and turns that merely say the word, and among them is a pair the
/// text score ranks the wrong way round - which is what makes this a test of
/// the entity weight rather than of bm25 agreeing with it.
#[test]
fn an_exact_entity_match_outranks_a_free_text_match() {
    let bench = bench();
    let conn = bench.conn();

    let hits = ranked(&conn, "cargo", MAX_RESULTS);
    let structural: Vec<&Hit> = hits.iter().filter(|h| h.entity_score > 0.0).collect();
    let textual: Vec<&Hit> = hits.iter().filter(|h| h.entity_score == 0.0).collect();
    assert!(!structural.is_empty() && !textual.is_empty(), "{hits:?}");

    // The text score alone, which is what the relevance was before weighting.
    let text_only = |hit: &Hit| hit.relevance - hit.entity_score;

    let flipped = structural.iter().any(|entity_hit| {
        textual.iter().any(|text_hit| {
            text_only(text_hit) > text_only(entity_hit) && entity_hit.relevance > text_hit.relevance
        })
    });
    assert!(
        flipped,
        "no turn carrying the exact command climbed past a turn that only says it: {hits:?}"
    );
}

/// The same rule for a path: the tool call that opened the file outranks the
/// sentence about what was in it.
#[test]
fn a_path_in_a_tool_call_outranks_the_same_path_in_prose() {
    let bench = bench();
    let conn = bench.conn();

    let hits = ranked(&conn, "docs/RETRY.md", MAX_RESULTS);
    assert_eq!(hits.len(), 2, "{hits:?}");

    assert_eq!(
        tool_of(&conn, hits[0].turn_id).as_deref(),
        Some("Read"),
        "the tool call should rank first: {hits:?}"
    );
    assert!(hits[0].entity_score > 0.0);
    assert_eq!(
        hits[1].entity_score, 0.0,
        "a path named in prose emits no entity, so it earns no structural weight"
    );
}

/// A value half the corpus carries moves a hit far less than a rare one.
///
/// Nothing is rejected and there is no stop-list: the common value still
/// matches, and RCL-04 puts the judgement at query time precisely because "too
/// common" changes as the archive grows.
#[test]
fn a_rare_entity_value_moves_a_hit_more_than_a_common_one() {
    let bench = bench();
    let common = "src/common/everywhere.rs";
    let rare = "src/rare/once.rs";
    let mut paths: Vec<&str> = vec![common; 30];
    paths.push(rare);
    bench.reads("entities.jsonl", &paths);
    let conn = bench.conn();

    let common_hits = ranked(&conn, common, MAX_RESULTS);
    let rare_hits = ranked(&conn, rare, MAX_RESULTS);
    assert_eq!(rare_hits.len(), 1, "{rare_hits:?}");
    assert!(common_hits.len() > 10, "{}", common_hits.len());

    let common_score = common_hits[0].entity_score;
    let rare_score = rare_hits[0].entity_score;
    assert!(common_score > 0.0, "the common value still matches");
    assert!(
        rare_score > common_score,
        "rare {rare_score} should outweigh common {common_score}"
    );

    // Deterministic: the same store answers the same way twice.
    assert_eq!(ranked(&conn, rare, MAX_RESULTS), rare_hits);
    assert_eq!(ranked(&conn, common, MAX_RESULTS), common_hits);
}

/// D-05: every hit carries an excerpt cut from the archive, showing the text
/// that matched.
///
/// `snippet()` over the contentless FTS table returns an empty string with exit
/// 0, so an excerpt built from it would validate against the documented shape
/// and say nothing. The assertion is therefore on the content: the queried word
/// is in it, and the JSON the record is written in is not.
#[test]
fn every_hit_carries_an_excerpt_from_the_blob() {
    let bench = bench();
    let conn = bench.conn();

    for raw in [
        "cargo",
        "docs/RETRY.md",
        "the echo agent repeats this line exactly",
    ] {
        let hits = ranked(&conn, raw, MAX_RESULTS);
        assert!(!hits.is_empty(), "`{raw}` matched nothing");
        for hit in &hits {
            assert!(
                !hit.excerpt.is_empty(),
                "turn {} came back with no excerpt for `{raw}`",
                hit.turn_id
            );
            let folded = hit.excerpt.to_lowercase();
            assert!(
                Query::parse(raw)
                    .tokens()
                    .iter()
                    .any(|token| folded.contains(&token.to_lowercase())),
                "excerpt for turn {} shows none of `{raw}`: {:?}",
                hit.turn_id,
                hit.excerpt
            );
            for key in ["\"type\"", "\"message\"", "\"content\"", "\"tool_use\""] {
                assert!(
                    !hit.excerpt.contains(key),
                    "excerpt for turn {} is JSON, not prose: {:?}",
                    hit.turn_id,
                    hit.excerpt
                );
            }
        }
    }
}

/// D-20: several hits from one session read that session's blob once.
///
/// Whole-blob materialization is what this bounds, not decompression: the
/// `blob` feature that would give the reader incremental I/O is deliberately
/// not enabled, and one archived session reaches 10.1 MB uncompressed.
#[test]
fn one_session_is_read_once_however_many_hits_it_answered() {
    let bench = bench();
    let conn = bench.conn();

    let response = answer(&conn, &[], Scope::Everything, "retry budget");
    let mut sessions: Vec<&str> = response
        .hits
        .iter()
        .map(|hit| hit.session_key.as_str())
        .collect();
    sessions.sort_unstable();
    sessions.dedup();

    assert!(
        response.hits.len() > sessions.len(),
        "this query needs several hits in one session to be about anything: {:?}",
        response.hits
    );
    assert_eq!(response.reads.blobs, sessions.len());
}

/// A long turn is cut down, with the cut marked and the match inside the
/// window.
#[test]
fn a_long_excerpt_is_a_window_around_the_match() {
    let bench = bench();
    let filler = "padding ".repeat(200);
    bench.flood("long.jsonl", &format!("{filler} needleword"), 1);
    let conn = bench.conn();

    let hits = ranked(&conn, "needleword", MAX_RESULTS);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let excerpt = &hits[0].excerpt;

    assert!(excerpt.contains("needleword"), "{excerpt:?}");
    assert!(excerpt.starts_with("..."), "the cut is marked: {excerpt:?}");
    assert!(
        excerpt.chars().count() <= verbatim_core::recall::EXCERPT_CHARS + 2 * "...".len(),
        "excerpt is {} chars",
        excerpt.chars().count()
    );
}

/// A blob that will not decompress costs that hit its excerpt and nothing else.
///
/// `verbatim verify` is the command that reports archive damage; a search that
/// failed outright would take every undamaged session's results down with it.
#[test]
fn a_damaged_blob_leaves_an_empty_excerpt_and_a_whole_hit() {
    let bench = bench();
    let conn = bench.conn();

    let before = ranked(&conn, "cargo", MAX_RESULTS);
    let damaged = before
        .iter()
        .find(|hit| hit.session_key.ends_with("session-errors-a.jsonl"))
        .expect("the errors fixture answers this query")
        .session_key
        .clone();
    conn.execute(
        "UPDATE sessions SET blob = ?1 WHERE session_key = ?2",
        rusqlite::params![vec![0u8; 64], &damaged],
    )
    .unwrap();

    let after = ranked(&conn, "cargo", MAX_RESULTS);
    assert_eq!(after.len(), before.len(), "the search still answered");
    for hit in &after {
        if hit.session_key == damaged {
            assert!(hit.excerpt.is_empty());
            // Everything else survived: the damage is one field wide.
            assert!(!hit.record_type.is_empty());
            assert!(hit.relevance > 0.0);
        } else {
            assert!(!hit.excerpt.is_empty(), "{hit:?}");
        }
    }
}

/// The turn at `seq` of the fixture whose file name ends with `fixture`.
fn turn_at(conn: &Connection, fixture: &str, seq: i64) -> i64 {
    conn.query_row(
        "SELECT id FROM turns WHERE session_key LIKE '%' || ?1 AND turn_seq = ?2",
        rusqlite::params![fixture, seq],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("{fixture} turn {seq}: {e}"))
}

fn around(
    conn: &Connection,
    exclusions: &[&str],
    anchor: i64,
    before: usize,
    after: usize,
) -> Window {
    context::window(conn, &config(exclusions), anchor, before, after).unwrap()
}

/// RCL-08: the turns on either side of a hit, in `turn_seq` order.
#[test]
fn a_window_returns_the_requested_turns_in_turn_seq_order() {
    let bench = bench();
    let conn = bench.conn();

    let anchor = turn_at(&conn, "session-recall.jsonl", 2);
    let window = around(&conn, &[], anchor, 2, 2);

    assert_eq!(window.reason, None);
    let seqs: Vec<i64> = window.turns.iter().map(|turn| turn.turn_seq).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
    assert_eq!(window.turns.iter().filter(|turn| turn.is_anchor).count(), 1);
    assert_eq!(window.turns[2].turn_id, anchor);
    assert!(window.at_session_start, "seq 0 is the session's first turn");
    assert!(!window.at_session_end, "seq 5 is still ahead");

    // The text comes with them, projected the way the index projected it.
    assert!(window
        .turns
        .iter()
        .all(|turn| !turn.text.is_empty() && !turn.text.contains("\"type\"")));

    // Asking for more than the session holds is a short window, not an error.
    let whole = around(&conn, &[], anchor, MAX_CONTEXT_SIDE, MAX_CONTEXT_SIDE);
    assert!(whole.at_session_start && whole.at_session_end);
    assert_eq!(whole.turns.len(), 6);
}

/// D-22: `turn_seq` order, never timestamp order.
///
/// Half of all real sessions carry a record whose timestamp goes backwards, so
/// a timestamp sort would show a reply before the prompt it answers in half the
/// archive. This session's clock runs backwards on purpose.
#[test]
fn a_window_is_ordered_by_turn_seq_and_not_by_timestamp() {
    let bench = bench();
    bench.timeline(
        "backwards.jsonl",
        &[
            "2026-08-12T23:00:04.000Z",
            "2026-08-12T23:00:03.000Z",
            "2026-08-12T23:00:02.000Z",
            "2026-08-12T23:00:01.000Z",
            "2026-08-12T23:00:00.000Z",
        ],
    );
    let conn = bench.conn();

    let anchor = turn_at(&conn, "backwards.jsonl", 2);
    let window = around(&conn, &[], anchor, 2, 2);

    let seqs: Vec<i64> = window.turns.iter().map(|turn| turn.turn_seq).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4]);

    let stamps: Vec<&str> = window
        .turns
        .iter()
        .map(|turn| turn.ts.as_deref().unwrap())
        .collect();
    let mut sorted = stamps.clone();
    sorted.sort_unstable();
    assert_ne!(
        stamps, sorted,
        "the fixture's clock must go backwards or this asserts nothing"
    );
}

/// D-06: the window stops at the session and hands back the link it did not
/// follow.
#[test]
fn a_window_stops_at_the_session_boundary() {
    let bench = bench();
    let conn = bench.conn();

    let first = turn_at(&conn, "session-continuation.jsonl", 0);
    let window = around(&conn, &[], first, 5, 1);

    assert!(window.at_session_start);
    assert_eq!(window.turns[0].turn_id, first);
    assert!(
        window.turns[0].is_anchor,
        "nothing before the first turn of the file, however much was asked for"
    );
    assert!(
        window.turns.iter().all(|turn| turn.turn_seq >= 0),
        "{:?}",
        window.turns
    );
    assert!(
        window.continues_from.is_some(),
        "this fixture continues another session, and the window returns the link \
         rather than walking it"
    );
    // Every returned turn belongs to the one session.
    let ids: Vec<i64> = window.turns.iter().map(|turn| turn.turn_id).collect();
    for id in ids {
        let key: String = conn
            .query_row("SELECT session_key FROM turns WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(key.ends_with("session-continuation.jsonl"), "{key}");
    }
}

/// A turn of an excluded project has no window, and says why.
#[test]
fn a_window_into_an_excluded_project_is_empty_with_a_reason() {
    let bench = bench();
    let conn = bench.conn();

    let alpha = testkit::fixture_project("session-recall.jsonl", &bench.root);
    let alpha_key = alpha.to_string_lossy().into_owned();
    let anchor = turn_at(&conn, "session-recall.jsonl", 2);

    assert!(!around(&conn, &[], anchor, 1, 1).turns.is_empty());

    let hidden = around(&conn, &[&alpha_key], anchor, 1, 1);
    assert!(hidden.turns.is_empty());
    assert_eq!(
        hidden.reason,
        Some(Reason::ProjectExcluded {
            project: alpha_key.clone()
        })
    );

    // And a turn that is not archived at all is a reason too, not a panic.
    let missing = around(&conn, &[], -1, 1, 1);
    assert!(missing.turns.is_empty());
    assert_eq!(missing.reason, Some(Reason::NoSuchTurn { turn_id: -1 }));
}

/// One directory is routinely both one row's `project` and another row's
/// `project_pre_worktree`, and the caller standing in it must be scoped to the
/// row that names it directly.
///
/// This is the ordinary state of a machine that ingested from a worktree before
/// phase 2 folded the two keys together and again after, not a contrived store.
/// Both hits cover the directory at the same depth, so before the folded key
/// outranked the pre-folding one the tie fell to the projection's `ORDER BY`:
/// `/home/u/main` sorts before `/home/u/proj`, so a user standing in
/// `/home/u/proj` was scoped to `/home/u/main` - handed another project's turns
/// while their own stayed invisible.
#[test]
fn a_directory_that_is_also_another_rows_pre_worktree_key_scopes_to_itself() {
    use verbatim_core::recall::scope::{self, Scope};

    let dir = tempfile::tempdir().unwrap();
    let store = verbatim_core::Store::open(dir.path()).unwrap();
    let conn = store.conn();

    let mut insert = |key: &str, no: i64, project: &str, pre: Option<&str>| {
        conn.execute(
            "INSERT INTO sessions (session_key, session_no, blob) VALUES (?1, ?2, x'00')",
            rusqlite::params![key, no],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_meta
                 (session_key, transcript_path, checksum, uncompressed_len,
                  project, project_pre_worktree)
             VALUES (?1, ?1, x'00', 0, ?2, ?3)",
            rusqlite::params![key, project, pre],
        )
        .unwrap();
    };
    // Written by a pre-folding binary run from the worktree itself.
    insert("a", 0, "/home/u/proj", None);
    // Written after folding, which maps that same worktree onto the repository.
    insert("b", 1, "/home/u/main", Some("/home/u/proj"));

    let scoped = scope::resolve(
        conn,
        &config(&[]),
        &Scope::Directory(std::path::PathBuf::from("/home/u/proj")),
    )
    .unwrap();
    assert_eq!(
        scoped.project(),
        Some("/home/u/proj"),
        "scoped to another project's key"
    );

    // A subdirectory resolves the same way, and so does the trailing-slash
    // spelling a shell completion produces.
    for spelling in ["/home/u/proj/src", "/home/u/proj/"] {
        let scoped = scope::resolve(
            conn,
            &config(&[]),
            &Scope::Directory(std::path::PathBuf::from(spelling)),
        )
        .unwrap();
        assert_eq!(scoped.project(), Some("/home/u/proj"), "{spelling}");
    }

    // The control: the folded key still resolves to itself, so ranking the
    // direct hit up did not break the case worktree folding exists for.
    let scoped = scope::resolve(
        conn,
        &config(&[]),
        &Scope::Directory(std::path::PathBuf::from("/home/u/main")),
    )
    .unwrap();
    assert_eq!(scoped.project(), Some("/home/u/main"));
}
