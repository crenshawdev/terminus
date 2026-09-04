//! Terminal recall at the process boundary: `search`, `show` and `sessions`.
//!
//! These have to be spawns rather than library calls. The default project scope
//! is the process's own working directory (D-12), and a test binary cannot chdir
//! to assert on that - `std::env::set_current_dir` is process-global and these
//! tests run in parallel with every other test in the binary. A child process
//! with a chosen `current_dir` is the only way `Scope::current_directory()` is
//! testable at all, and it is the same reason exit codes and the stdout/stderr
//! split live here.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;
use serde_json::Value;
use verbatim_core::config::REDACTED;
use verbatim_core::store::{DB_FILE_NAME, DERIVED_SCHEMA, META_DERIVED_SCHEMA};
use verbatim_core::testkit;

/// Every directory a spawned `verbatim` may touch, all of them temporary.
///
/// `claude_dir` is as load-bearing as the data directory even though no test
/// here runs a tree pass: a spawn that set only `VERBATIM_DATA_DIR` would
/// resolve the developer's real config, and no test process may reach a real
/// transcript root.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    /// The root the rooted fixtures' `cwd` values were substituted with, and so
    /// the parent of every project directory a scoped search can stand in.
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        config_dir,
        claude_dir,
        root,
    }
}

impl Bench {
    /// Run the binary from the work directory, which no archived project covers.
    fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.work, args)
    }

    /// Run the binary standing in `dir`, which is what makes the default scope
    /// assertable.
    fn run_in(&self, dir: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_verbatim"))
            .args(args)
            .current_dir(dir)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn verbatim")
    }

    /// Archive every fixture through the binary's own ingest path.
    ///
    /// Through the binary and not through the library, so what the read commands
    /// query is what a real pass wrote.
    fn ingest_fixtures(&self) {
        for fixture in testkit::TRANSCRIPT_FIXTURES {
            let rooted = testkit::ROOTED_FIXTURES.iter().any(|(f, _)| f == fixture);
            let path = if rooted {
                testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root)
            } else {
                testkit::copy_fixture_into(fixture, &self.work)
            };
            let out = self.run(&["ingest", path.to_str().unwrap()]);
            assert!(out.status.success(), "ingest {fixture}: {}", stderr(&out));
        }
    }

    /// The directory one rooted fixture's project sits at, which a spawn can
    /// stand in.
    fn project(&self, fixture: &str) -> PathBuf {
        testkit::fixture_project(fixture, &self.root)
    }

    fn config(&self, text: &str) {
        std::fs::write(self.config_dir.join("verbatim.toml"), text).unwrap();
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one JSON document a `--json` run wrote, and the assertion that stdout
/// held nothing else.
fn document(output: &Output) -> Value {
    let text = stdout(output);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_else(|| panic!("stdout was empty"));
    assert_eq!(
        lines.next(),
        None,
        "stdout carried more than the document: {text:?}"
    );
    serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {text:?}"))
}

/// Every read command, as a command line that needs no store to be valid.
///
/// One list rather than one test per command: the two behaviours D-10 and D-18
/// fix are properties of the shared entry point, so a seventh read command added
/// later is one line here rather than a new test.
const READ_COMMANDS: &[&[&str]] = &[&["search", "anything"], &["show", "1"], &["sessions"]];

// ---------------------------------------------------------------------------
// The shared read entry point (D-10, D-18)
// ---------------------------------------------------------------------------

/// A machine that has never ingested: exit 0, an empty result with a reason, and
/// not one byte written where the store would go.
///
/// The last clause is the one `Store::open` would fail. It would create the data
/// directory, initialize a database and set `journal_mode=wal`, so asking a
/// question would be how a store came to exist - and every later "the store is
/// empty" report would be about a store the read itself made.
#[test]
fn a_read_against_an_empty_data_directory_answers_and_creates_nothing() {
    let bench = bench();

    for args in READ_COMMANDS {
        let out = bench.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{}` against an empty data dir: {}",
            args.join(" "),
            stderr(&out)
        );
        assert_eq!(
            stdout(&out),
            "",
            "`{}` printed data with no store to read",
            args.join(" ")
        );
        assert!(
            stderr(&out).contains("no verbatim store"),
            "`{}` gave no reason: {}",
            args.join(" "),
            stderr(&out)
        );

        // The same run in JSON mode is a document carrying the reason.
        let json: Vec<&str> = args.iter().copied().chain(["--json"]).collect();
        let out = bench.run(&json);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let document = document(&out);
        assert_eq!(document["ok"], true, "{document}");
        assert!(
            document["reason"]
                .as_str()
                .is_some_and(|r| r.contains("no verbatim store")),
            "{document}"
        );
    }

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory it was asked about"
    );
    assert!(
        !bench.data_dir.join(DB_FILE_NAME).exists(),
        "a read created a store"
    );
}

/// D-18: a store whose derived tables predate this build says so on stderr,
/// answers from the old-shape index anyway, and is not repaired.
///
/// All three clauses together are the decision. Saying nothing would make a
/// pending rebuild look like a search bug; repairing would make a read rewrite
/// four tables; refusing would make an upgraded binary useless until the next
/// hook fired.
#[test]
fn a_read_against_a_store_older_than_this_build_says_so_and_repairs_nothing() {
    let bench = bench();
    bench.ingest_fixtures();

    // Age it by one, which is exactly what an upgrade looks like from the
    // store's side.
    bench
        .conn()
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            rusqlite::params![(DERIVED_SCHEMA - 1).to_string(), META_DERIVED_SCHEMA],
        )
        .unwrap();

    for args in READ_COMMANDS {
        let out = bench.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{}` refused an aged store: {}",
            args.join(" "),
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("predate this build"),
            "`{}` said nothing about the aged store: {}",
            args.join(" "),
            stderr(&out)
        );
    }

    let after: String = bench
        .conn()
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_DERIVED_SCHEMA],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        after,
        (DERIVED_SCHEMA - 1).to_string(),
        "a read repaired the store it was only asked to query"
    );
}

// ---------------------------------------------------------------------------
// `verbatim search` (RCL-05)
// ---------------------------------------------------------------------------

/// The headline case, at the process boundary: the most natural command in the
/// product finds the turn that names that path, and exits 0.
///
/// A raw `MATCH 'src/worker/S.ts'` is `fts5: syntax error near "/"` and exit 1
/// (D-09), so without the tokenizer this exact command line is the one that
/// fails.
#[test]
fn searching_for_a_path_exits_zero_with_the_turn_that_names_it() {
    let bench = bench();
    bench.ingest_fixtures();

    let out = bench.run(&["search", "--project", "*", "src/worker/S.ts"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let text = stdout(&out);
    assert!(
        text.contains("src/worker/S.ts"),
        "the excerpt must show what matched: {text}"
    );

    // The id printed is the id `verbatim show` takes, so it has to be a real one.
    let id: i64 = text
        .split_whitespace()
        .next()
        .expect("a hit line starts with its turn id")
        .parse()
        .unwrap_or_else(|e| panic!("the first column is not a turn id ({e}): {text}"));
    let session: String = bench
        .conn()
        .query_row("SELECT session_key FROM turns WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap_or_else(|e| panic!("turn {id} is not in the store: {e}"));
    assert!(session.ends_with("session-recall.jsonl"), "{session}");
}

/// RCL-01 through the command: a camelCase identifier and its component find the
/// same turn.
///
/// Both halves matter. `SearchManager` stays one term, and `manager` reaches it
/// because the expansion put that component in the indexed body - which is the
/// half of ROADMAP criterion 1 that no path expansion would have delivered.
#[test]
fn a_camel_case_identifier_and_its_component_find_the_same_turn() {
    let bench = bench();
    bench.ingest_fixtures();

    let ids = |query: &str| -> Vec<i64> {
        let out = bench.run(&["search", "--project", "*", "--json", query]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        document(&out)["data"]["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hit| hit["turn_id"].as_i64().unwrap())
            .collect()
    };

    let exact = ids("SearchManager");
    let component = ids("manager");
    assert!(!exact.is_empty(), "SearchManager matched nothing");
    let shared: Vec<i64> = exact
        .iter()
        .copied()
        .filter(|id| component.contains(id))
        .collect();
    assert_eq!(
        shared, exact,
        "every SearchManager hit must also answer to `manager`: {exact:?} vs {component:?}"
    );
}

/// RCL-06: a query that matches nothing exits 0 with an empty result set, never
/// non-zero.
#[test]
fn a_query_matching_nothing_exits_zero_with_an_empty_result() {
    let bench = bench();
    bench.ingest_fixtures();

    let out = bench.run(&["search", "--project", "*", "zzzznotinanyfixture"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "an empty result prints no hits");
    assert!(stderr(&out).contains("0 hit(s)"), "{}", stderr(&out));

    let out = bench.run(&["search", "--project", "*", "--json", "zzzznotinanyfixture"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let document = document(&out);
    assert_eq!(document["ok"], true);
    assert_eq!(document["data"]["hits"], serde_json::json!([]));
}

/// D-12, and the one line of the query layer PLAN-2 could not reach: the default
/// scope is the project the process is standing in.
///
/// `cargo` is in project-beta's transcripts and in no project-alpha turn, so
/// standing in alpha and asking for it must return nothing while `*` returns
/// hits. Without the scoping that query answers from beta wherever it is run.
#[test]
fn search_scopes_to_the_directory_it_is_run_in() {
    let bench = bench();
    bench.ingest_fixtures();

    let alpha = bench.project("session-recall.jsonl");
    let beta = bench.project("session-errors-a.jsonl");
    assert_ne!(alpha, beta, "the fixtures must span two projects");

    let hits = |dir: &Path, args: &[&str]| -> Vec<Value> {
        let mut full = vec!["search", "--json"];
        full.extend_from_slice(args);
        let out = bench.run_in(dir, &full);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        document(&out)["data"]["hits"].as_array().unwrap().clone()
    };

    // Standing in alpha: its own turns yes, beta's no.
    assert!(!hits(&alpha, &["SearchManager"]).is_empty());
    assert!(
        hits(&alpha, &["cargo"]).is_empty(),
        "a search in one project returned another project's turns"
    );

    // Standing in beta: the mirror image.
    assert!(!hits(&beta, &["cargo"]).is_empty());
    assert!(hits(&beta, &["SearchManager"]).is_empty());

    // `*` opts out, from either directory.
    assert!(!hits(&alpha, &["--project", "*", "cargo"]).is_empty());

    // And every hit really does carry the project it was scoped to - compared
    // against the key ingest wrote rather than against the directory string,
    // because the resolver canonicalizes and the temp root may be a symlink.
    let stored: String = bench
        .conn()
        .query_row(
            "SELECT project FROM session_meta WHERE session_key LIKE '%session-recall.jsonl'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for hit in hits(&alpha, &["SearchManager"]) {
        assert_eq!(hit["project"], stored);
    }
}

/// AC5's exclusion half for the terminal path: with a project excluded, `search`
/// returns none of its turns, including under `*` and including turns archived
/// before the exclusion was configured.
#[test]
fn an_excluded_projects_turns_are_absent_from_search() {
    let bench = bench();
    bench.ingest_fixtures();

    let beta = bench.project("session-errors-a.jsonl");
    // The control: before the exclusion, those turns are reachable.
    let out = bench.run(&["search", "--project", "*", "--json", "cargo"]);
    let before = document(&out)["data"]["hits"].as_array().unwrap().len();
    assert!(before > 0, "this test needs beta turns to hide");

    bench.config(&format!(
        "exclude = [{:?}]\n",
        beta.to_string_lossy().into_owned()
    ));

    let out = bench.run(&["search", "--project", "*", "--json", "cargo"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let hits = document(&out)["data"]["hits"].as_array().unwrap().clone();
    for hit in &hits {
        assert_ne!(
            hit["project"],
            beta.to_string_lossy().into_owned(),
            "an excluded project's turn came back: {hit}"
        );
    }

    // The turns are still archived. Exclusion is a read-path predicate, not a
    // deletion (D-23), and a test that let them be deleted would pass for the
    // wrong reason.
    let archived: i64 = bench
        .conn()
        .query_row(
            "SELECT count(*) FROM turns t JOIN session_meta m USING (session_key)
              WHERE m.project = ?1",
            [beta.to_string_lossy().into_owned()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(archived > 0, "the excluded project's turns were deleted");
}

/// An unknown flag is misuse (exit 2) and prints nothing to stdout; a malformed
/// `--since` is misuse too, not an empty result.
///
/// The second is the one that could go the other way. Every string compares
/// cleanly against every other, so a bad bound would return a plausible wrong
/// answer with nothing to say it had.
#[test]
fn a_bad_command_line_is_misuse_and_not_an_empty_result() {
    let bench = bench();
    bench.ingest_fixtures();

    for args in [
        vec!["search", "--nope", "x"],
        // `--project *` and not the default scope: `scope::resolve` runs first
        // inside `search::run` and short-circuits a directory no archived
        // project covers, so the bound is never reached to be rejected there.
        // See the open item on PLAN-3.
        vec![
            "search",
            "--project",
            "*",
            "--since",
            "last tuesday",
            "cargo",
        ],
        vec!["search", "--limit", "many", "cargo"],
        vec!["search"],
    ] {
        let out = bench.run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{}` should be misuse: {}",
            args.join(" "),
            stderr(&out)
        );
        assert_eq!(stdout(&out), "", "misuse must print nothing to stdout");
    }
}

/// `--json` puts a document on stdout and nothing else, with every diagnostic on
/// stderr - which is what lets `verbatim search --json | jq` work.
#[test]
fn search_json_is_a_document_and_stdout_holds_nothing_else() {
    let bench = bench();
    bench.ingest_fixtures();

    let out = bench.run(&["search", "--project", "*", "--json", "cargo"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let document = document(&out);
    assert_eq!(document["command"], "search");
    assert_eq!(document["data"]["query"], "cargo");
    let hits = document["data"]["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "{document}");
    for hit in hits {
        for field in [
            "turn_id",
            "session_key",
            "project",
            "record_type",
            "ts",
            "sidechain",
            "relevance",
            "entity_score",
            "excerpt",
        ] {
            assert!(hit.get(field).is_some(), "a hit is missing {field}: {hit}");
        }
    }
}

/// The filters RCL-07 names, through the command line rather than through the
/// library: each one narrows, and `--limit` bounds.
#[test]
fn the_command_line_filters_reach_the_query_layer() {
    let bench = bench();
    bench.ingest_fixtures();

    let hits = |args: &[&str]| -> Vec<Value> {
        let mut full = vec!["search", "--project", "*", "--json"];
        full.extend_from_slice(args);
        let out = bench.run(&full);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{}`: {}",
            full.join(" "),
            stderr(&out)
        );
        document(&out)["data"]["hits"].as_array().unwrap().clone()
    };

    // A tool filter returns only that tool's turns. Every `cargo` in the corpus
    // is a Bash command, so `--tool Read` is the arm that has to come back
    // empty: a filter that reached the query layer and did nothing there would
    // still return the unfiltered set.
    let all = hits(&["cargo"]);
    assert!(!all.is_empty(), "this test needs cargo turns");
    assert!(
        !hits(&["--tool", "Bash", "cargo"]).is_empty(),
        "the Bash filter matched nothing"
    );
    assert!(
        hits(&["--tool", "Read", "cargo"]).is_empty(),
        "the tool filter narrowed nothing"
    );

    // A kind filter, likewise.
    let assistant = hits(&["--kind", "assistant", "cargo"]);
    assert!(!assistant.is_empty());
    for hit in &assistant {
        assert_eq!(hit["record_type"], "assistant");
    }

    // A time window that ends before the corpus begins returns nothing, and
    // exits 0 doing it.
    assert!(hits(&["--until", "2000-01-01", "cargo"]).is_empty());

    // `--limit` bounds the page.
    assert_eq!(hits(&["--limit", "1", "cargo"]).len(), 1);
}

// ---------------------------------------------------------------------------
// `verbatim show` (RCL-09 plus RCL-08's window)
// ---------------------------------------------------------------------------

/// The turn id `search` printed for a query, which is the only id a user ever
/// types into `show`.
fn one_id(bench: &Bench, query: &str) -> i64 {
    let out = bench.run(&["search", "--project", "*", "--json", query]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    document(&out)["data"]["hits"]
        .as_array()
        .unwrap()
        .first()
        .unwrap_or_else(|| panic!("{query:?} matched nothing"))["turn_id"]
        .as_i64()
        .unwrap()
}

/// `verbatim show <id>` prints the record's own archived line, byte for byte.
///
/// Compared against the transcript file rather than against anything the store
/// derived. A projection would be prose about the right turn and would pass any
/// test that only checked the turn was found - and this is the command whose
/// whole job is that it does not do that.
#[test]
fn show_prints_the_records_own_line() {
    let bench = bench();
    bench.ingest_fixtures();

    let id = one_id(&bench, "src/worker/S.ts");
    let out = bench.run(&["show", "--project", "*", &id.to_string()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let text = String::from_utf8(testkit::fixture_bytes("session-recall.jsonl")).unwrap();
    let expected = text
        .replace(
            testkit::FIXTURE_ROOT_TOKEN,
            &bench.root.to_string_lossy().replace('\\', "\\\\"),
        )
        .lines()
        .nth(1)
        .expect("the fixture has a second line")
        .to_owned();

    let printed = stdout(&out);
    assert!(
        printed.contains(&expected),
        "the archived line is not in the output.\nwanted: {expected}\ngot: {printed}"
    );
    // The header is there too, and leads with the id the user typed.
    assert!(
        printed.starts_with(&format!("{id}  ")),
        "the header must lead with the id: {printed}"
    );
}

/// RCL-08 through the command: `--before`/`--after` return the neighbouring
/// turns in `turn_seq` order, and the session's own ends are marked.
///
/// The boundary flag is the half that could be dropped silently. A window that
/// asked for five earlier turns and got two says "there is nothing earlier"
/// rather than leaving a caller to guess whether the archive is short.
#[test]
fn show_with_a_window_returns_the_neighbours_and_flags_the_boundary() {
    let bench = bench();
    bench.ingest_fixtures();

    // The first turn of a session: there is nothing before it, by construction.
    let first: i64 = bench
        .conn()
        .query_row(
            "SELECT id FROM turns WHERE session_key LIKE '%session-recall.jsonl'
              ORDER BY turn_seq LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let out = bench.run(&[
        "show",
        "--project",
        "*",
        "--json",
        "--before",
        "5",
        "--after",
        "2",
        &first.to_string(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let context = document(&out)["data"]["records"][0]["context"].clone();
    assert_eq!(
        context["at_session_start"], true,
        "the first turn of a session is a boundary: {context}"
    );
    assert_eq!(context["at_session_end"], false, "{context}");

    let turns = context["turns"].as_array().unwrap();
    // Asked for five before and got none, because there are none: the shorter
    // list is exactly what the flag exists to explain.
    assert_eq!(turns.len(), 3, "one anchor plus two after: {context}");
    let seqs: Vec<i64> = turns
        .iter()
        .map(|t| t["turn_seq"].as_i64().unwrap())
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(
        seqs, sorted,
        "the window is not in turn_seq order: {seqs:?}"
    );
    assert_eq!(
        turns.iter().filter(|t| t["is_anchor"] == true).count(),
        1,
        "exactly one turn is the anchor: {context}"
    );

    // And the human rendering says the same thing out loud.
    let out = bench.run(&[
        "show",
        "--project",
        "*",
        "--before",
        "5",
        "--after",
        "2",
        &first.to_string(),
    ]);
    assert!(
        stdout(&out).contains("--- start of session ---"),
        "{}",
        stdout(&out)
    );
}

/// D-08 through the command: a turn whose session is evicted prints as evicted
/// and the command still exits 0.
///
/// Retention is phase 8 and nothing writes that column before then, so the test
/// sets it directly - which is also the point. The flag comes off the column and
/// never off a failed blob read, so this stays distinguishable from the archive
/// damage `verbatim verify` reports.
#[test]
fn show_prints_an_evicted_turn_as_evicted_and_exits_zero() {
    let bench = bench();
    bench.ingest_fixtures();

    let id = one_id(&bench, "src/worker/S.ts");
    bench
        .conn()
        .execute(
            "UPDATE session_meta SET is_evicted = 1
              WHERE session_key = (SELECT session_key FROM turns WHERE id = ?1)",
            [id],
        )
        .unwrap();

    let out = bench.run(&["show", "--project", "*", &id.to_string()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("(body evicted)"), "{}", stdout(&out));

    let out = bench.run(&["show", "--project", "*", "--json", &id.to_string()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let record = document(&out)["data"]["records"][0].clone();
    assert_eq!(record["body_evicted"], true, "{record}");
    assert_eq!(record["body"], Value::Null, "{record}");
    // Everything else about the turn is still known: only its bytes are gone.
    assert_eq!(record["turn_id"], id);
    assert!(record["ts"].is_string(), "{record}");
}

/// An id that is not a number is misuse; an id that names no turn is an empty
/// result with a reason.
///
/// The split is the whole exit-code contract in one command. A word where an id
/// belongs is a mistake in the command line, and answering "no such turn" would
/// suggest the archive had been consulted about it.
#[test]
fn a_word_is_misuse_and_an_unknown_id_is_a_reason() {
    let bench = bench();
    bench.ingest_fixtures();

    for args in [
        vec!["show", "notanumber"],
        vec!["show"],
        vec!["show", "--before", "lots", "1"],
    ] {
        let out = bench.run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{}` should be misuse: {}",
            args.join(" "),
            stderr(&out)
        );
        assert_eq!(stdout(&out), "", "misuse must print nothing to stdout");
    }

    let unknown: i64 = bench
        .conn()
        .query_row("SELECT max(id) + 1000 FROM turns", [], |r| r.get(0))
        .unwrap();

    let out = bench.run(&["show", "--project", "*", &unknown.to_string()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "", "an unknown id prints no record");
    assert!(
        stderr(&out).contains(&format!("no turn {unknown}")),
        "{}",
        stderr(&out)
    );

    let out = bench.run(&["show", "--project", "*", "--json", &unknown.to_string()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let document = document(&out);
    assert_eq!(document["ok"], true, "{document}");
    assert_eq!(document["data"]["records"], serde_json::json!([]));
    assert_eq!(document["data"]["absent"][0]["turn_id"], unknown);
    assert!(
        document["reason"]
            .as_str()
            .is_some_and(|r| r.contains("no turn")),
        "{document}"
    );
}

/// Several ids at once come back in the order they were asked for, and a known
/// id beside an unknown one still answers.
#[test]
fn show_answers_several_ids_and_keeps_the_order_asked_for() {
    let bench = bench();
    bench.ingest_fixtures();

    let ids: Vec<i64> = bench
        .conn()
        .prepare(
            "SELECT id FROM turns WHERE session_key LIKE '%session-recall.jsonl'
              ORDER BY turn_seq DESC LIMIT 3",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids.len(), 3);

    let mut args = vec![
        "show".to_owned(),
        "--project".into(),
        "*".into(),
        "--json".into(),
    ];
    args.extend(ids.iter().map(|id| id.to_string()));
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();

    let out = bench.run(&borrowed);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let returned: Vec<i64> = document(&out)["data"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["turn_id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        returned, ids,
        "records must come back in the order the ids were given"
    );
}

// ---------------------------------------------------------------------------
// `verbatim sessions` (RCL-05)
// ---------------------------------------------------------------------------

/// The `--json` listing, as a vector of session objects.
fn listing(bench: &Bench, args: &[&str]) -> Vec<Value> {
    let mut full = vec!["sessions", "--json"];
    full.extend_from_slice(args);
    let out = bench.run(&full);
    assert_eq!(
        out.status.code(),
        Some(0),
        "`{}`: {}",
        full.join(" "),
        stderr(&out)
    );
    document(&out)["data"]["sessions"]
        .as_array()
        .unwrap()
        .clone()
}

/// Every ingested session is listed, with its project, and the sidecar fixtures
/// are flagged as sidecars.
///
/// The sidecar flag is `session_meta.parent_session_key` being non-null (D-07),
/// which is the column phase 2 wrote specifically so this phase could tell a
/// subagent transcript from the session that spawned it - two files that report
/// the same `sessionId`.
#[test]
fn sessions_lists_every_archived_session_and_flags_the_sidecars() {
    let bench = bench();
    bench.ingest_fixtures();

    let listed = listing(&bench, &["--project", "*"]);
    let archived: i64 = bench
        .conn()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(listed.len() as i64, archived, "every session is listed");

    for session in &listed {
        for field in [
            "session_key",
            "project",
            "branch",
            "first_turn_at",
            "last_turn_at",
            "turns",
            "sidecar",
            "evicted",
        ] {
            assert!(
                session.get(field).is_some(),
                "a listed session is missing {field}: {session}"
            );
        }
        assert!(session["turns"].as_i64().unwrap() > 0, "{session}");
    }

    // The sidecar fixtures, and only those, carry the flag.
    let flagged: Vec<String> = listed
        .iter()
        .filter(|s| s["sidecar"] == true)
        .map(|s| s["session_key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(flagged.len(), 3, "three sidecar fixtures: {flagged:?}");
    for key in &flagged {
        assert!(
            key.contains("agent-"),
            "a non-sidecar was flagged as one: {key}"
        );
    }

    // And the human listing says the same, one parseable line per session with
    // the count kept on stderr.
    let out = bench.run(&["sessions", "--project", "*"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out).lines().count() as i64,
        archived,
        "one line per session: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains(&format!("{archived} session(s)")),
        "{}",
        stderr(&out)
    );
    assert!(stdout(&out).contains("(subagent)"), "{}", stdout(&out));
}

/// AC5's exclusion half, through the listing: an excluded project's sessions are
/// absent from both outputs while its turns are still in the store.
///
/// Archived first, excluded second, which is the case a per-session flag written
/// at ingest could never answer (D-23). Exclusion is a read-path predicate
/// re-applied on every read, not a deletion and not a stamp.
#[test]
fn an_excluded_projects_sessions_are_absent_from_the_listing() {
    let bench = bench();
    bench.ingest_fixtures();

    let beta = bench.project("session-errors-a.jsonl");
    let beta_key = beta.to_string_lossy().into_owned();

    let before = listing(&bench, &["--project", "*"]);
    let hidden: Vec<&Value> = before.iter().filter(|s| s["project"] == beta_key).collect();
    assert_eq!(hidden.len(), 2, "two beta fixtures to hide: {before:?}");

    bench.config(&format!("exclude = [{beta_key:?}]\n"));

    let after = listing(&bench, &["--project", "*"]);
    assert!(
        after.iter().all(|s| s["project"] != beta_key),
        "an excluded project's session was listed: {after:?}"
    );
    assert_eq!(
        after.len(),
        before.len() - 2,
        "exactly the excluded sessions went away"
    );

    // The human output too, and by session key rather than by count, so a
    // renamed column cannot make this pass.
    let text = stdout(&bench.run(&["sessions", "--project", "*"]));
    assert!(!text.contains("session-errors-a"), "{text}");
    assert!(!text.contains("session-errors-b"), "{text}");

    // Still archived. Exclusion hides; it does not delete.
    let sessions: i64 = bench
        .conn()
        .query_row(
            "SELECT count(*) FROM session_meta WHERE project = ?1",
            [&beta_key],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sessions, 2, "the excluded project's sessions were deleted");
}

/// The listing is scoped like every other read path, and `--limit` bounds it.
#[test]
fn sessions_scopes_and_bounds_the_listing() {
    let bench = bench();
    bench.ingest_fixtures();

    let alpha = bench.project("session-recall.jsonl");
    let alpha_key = alpha.to_string_lossy().into_owned();

    let scoped = listing(&bench, &["--project", &alpha_key]);
    assert!(!scoped.is_empty(), "alpha has sessions");
    for session in &scoped {
        assert_eq!(session["project"], alpha_key, "{session}");
    }
    assert!(
        scoped.len() < listing(&bench, &["--project", "*"]).len(),
        "the scope narrowed nothing"
    );

    assert_eq!(
        listing(&bench, &["--project", "*", "--limit", "2"]).len(),
        2
    );

    // A time window that ends before the corpus begins lists nothing, at exit 0.
    let out = bench.run(&["sessions", "--project", "*", "--until", "2000-01-01"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("0 session(s)"), "{}", stderr(&out));
}

/// `verbatim search --kind observation` reaches the same branch `recall_search`
/// does, and renders a claim as an ordinary hit.
#[test]
fn the_terminal_search_reaches_observations_through_the_same_kind() {
    let bench = bench();
    bench.ingest_fixtures();

    let key: String = bench
        .conn()
        .query_row(
            "SELECT session_key FROM session_meta WHERE session_key LIKE '%session-recall.jsonl'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let anchor: i64 = bench
        .conn()
        .query_row(
            "SELECT min(id) FROM turns WHERE session_key = ?1",
            [&key],
            |r| r.get(0),
        )
        .unwrap();
    let claim = "the brillig helper was replaced by the steady one";
    bench
        .conn()
        .execute(
            "INSERT INTO observations (
                session_key, generated_at, status, model, prompt_version, topic, outcome,
                decisions, learned, unresolved, tokens
             ) VALUES (?1, '2026-08-21T10:00:00.000Z', 'ok', 'stub', 'obs-judgment-1',
                       'a topic', 'completed', ?2, '[]', '[]', 1)",
            rusqlite::params![
                key,
                serde_json::json!([{"turn_id": anchor, "text": claim}]).to_string()
            ],
        )
        .unwrap();

    let out = bench.run(&[
        "search",
        "brillig",
        "--project",
        "*",
        "--kind",
        "observation",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    let hits = value["data"]["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 1, "{value}");
    assert_eq!(hits[0]["record_type"], "observation", "{value}");
    assert_eq!(hits[0]["turn_id"], anchor, "{value}");
    assert_eq!(hits[0]["excerpt"], claim, "{value}");

    // And a person sees it too, with the same fields the human renderer gives
    // every other hit.
    let human = bench.run(&[
        "search",
        "brillig",
        "--project",
        "*",
        "--kind",
        "observation",
    ]);
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    let text = stdout(&human);
    assert!(text.contains(claim), "the claim never printed: {text}");
    assert!(
        text.contains(&anchor.to_string()),
        "the anchor never printed: {text}"
    );

    // A query no claim matches is zero hits and exit 0, not a failure.
    let empty = bench.run(&[
        "search",
        "zzzznotinanyclaim",
        "--project",
        "*",
        "--kind",
        "observation",
        "--json",
    ]);
    assert_eq!(empty.status.code(), Some(0), "{}", stderr(&empty));
    assert_eq!(
        document(&empty)["data"]["hits"],
        serde_json::json!([]),
        "{}",
        stdout(&empty)
    );
}

// ---------------------------------------------------------------------------
// PRIV-03's escape hatch: `--raw` on the two commands that print a projection
// ---------------------------------------------------------------------------

/// The flag reaches `search` and `show` and no other command (D-07).
///
/// The negative half is the load-bearing one. `sessions` prints no projection,
/// `export` stays unfiltered by D-12 and `observations` prints whatever the
/// column holds by D-13, so for each of those `--raw` is an argument the command
/// was not written for: misuse and exit 2, and never a flag quietly accepted and
/// ignored. That is the failure `verbatim verify --json` already had once.
#[test]
fn the_raw_flag_is_accepted_by_the_two_projecting_commands_and_by_no_other() {
    let bench = bench();
    bench.ingest_fixtures();

    let id = one_id(&bench, "src/worker/S.ts");
    let id_text = id.to_string();

    for args in [
        vec!["search", "--project", "*", "--raw", "cargo"],
        vec!["search", "--project", "*", "--raw", "--json", "cargo"],
        vec!["show", "--project", "*", "--raw", id_text.as_str()],
        vec![
            "show",
            "--project",
            "*",
            "--raw",
            "--json",
            id_text.as_str(),
        ],
    ] {
        let out = bench.run(&args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`{}` should be accepted: {}",
            args.join(" "),
            stderr(&out)
        );
    }

    let destination = bench.work.join("export-raw");
    for args in [
        vec!["sessions", "--raw"],
        vec!["export", "--raw", destination.to_str().unwrap()],
        vec!["observations", "--raw"],
    ] {
        let out = bench.run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{}` should be misuse: {}",
            args.join(" "),
            stderr(&out)
        );
        assert_eq!(stdout(&out), "", "misuse must print nothing to stdout");
    }
    assert!(
        !destination.exists(),
        "a rejected export must not have started writing"
    );
}

/// The `Authorization: Bearer` value planted in `session-secrets.jsonl`.
const AUTHZ_SENTINEL: &str = "sk-VBEGRESS-authz-9f2";

/// The fragment every planted sentinel in that fixture shares.
///
/// Asserted on rather than the one value under test because a search returns
/// several turns of that session at once: one surviving sentinel of any shape
/// is a leak, and naming only the `Authorization` one would miss it.
const SENTINEL_MARK: &str = "VBEGRESS";

/// PRIV-03's knob, written the only way anything can turn it on.
///
/// A file in the config directory the spawn points at, and no environment
/// variable: there is nothing a process could inherit that turns this on, which
/// is the same property that stops anything inherited turning it off.
const KNOB_ON: &str = "[privacy]\nredact_recall = true\n";

/// The turn of `session-secrets.jsonl` that pastes an `Authorization` header.
///
/// Found by the record's own `uuid` rather than by searching for it: a query
/// would be answered through the very projection under test, so with the knob
/// on the test would be picking its subject out of filtered text.
fn authz_turn(bench: &Bench) -> String {
    let id: i64 = bench
        .conn()
        .query_row(
            "SELECT id FROM turns WHERE uuid = 'eeeeeeee-0000-4000-8000-000000001003'",
            [],
            |r| r.get(0),
        )
        .expect("the fixture's Authorization turn is archived");
    id.to_string()
}

/// One JSON object's keys, sorted - the shape D-08 says neither setting moves.
fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {value}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// Every excerpt a `search --json` document carries, joined.
fn hit_excerpts(document: &Value) -> String {
    let hits = document["data"]["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "no hits, so no projection to filter");
    hits.iter()
        .map(|hit| hit["excerpt"].as_str().expect("an excerpt").to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// AC4's `show` half, at the process boundary rather than at a library call.
///
/// Three runs over ONE store: no `verbatim.toml` at all, the knob on, and the
/// knob on with `--raw`. The first is the default this phase must not move; the
/// third has to come back byte for byte identical to the first, which is what
/// "the archive is untouched and the filter is a projection" means from the
/// only place a user can see it.
#[test]
fn the_knob_filters_show_at_the_process_boundary_and_raw_reaches_past_it() {
    let bench = bench();
    bench.ingest_fixtures();
    let project = bench.project("session-secrets.jsonl");
    let id = authz_turn(&bench);

    let default = bench.run_in(&project, &["show", &id]);
    assert_eq!(default.status.code(), Some(0), "{}", stderr(&default));
    assert!(
        stdout(&default).contains(AUTHZ_SENTINEL),
        "with no config file the archive answers what it holds:\n{}",
        stdout(&default)
    );

    bench.config(KNOB_ON);

    let filtered = bench.run_in(&project, &["show", &id]);
    assert_eq!(filtered.status.code(), Some(0), "{}", stderr(&filtered));
    let text = stdout(&filtered);
    assert!(
        text.contains(REDACTED),
        "the marker names what went:\n{text}"
    );
    assert!(
        !text.contains(SENTINEL_MARK),
        "a sentinel survived into `show`:\n{text}"
    );

    let raw = bench.run_in(&project, &["show", "--raw", &id]);
    assert_eq!(raw.status.code(), Some(0), "{}", stderr(&raw));
    assert!(
        stdout(&raw).contains(AUTHZ_SENTINEL),
        "--raw must print the record's own bytes:\n{}",
        stdout(&raw)
    );
    assert_eq!(
        stdout(&raw),
        stdout(&default),
        "--raw under the knob must be what the store answers with the knob absent"
    );

    // The same pair under `--json`, where the flag changes a value and never a
    // key (D-08).
    let filtered_json = bench.run_in(&project, &["show", "--json", &id]);
    let raw_json = bench.run_in(&project, &["show", "--json", "--raw", &id]);
    let filtered_doc = document(&filtered_json);
    let raw_doc = document(&raw_json);

    let filtered_body = filtered_doc["data"]["records"][0]["body"]
        .as_str()
        .expect("a body")
        .to_owned();
    let raw_body = raw_doc["data"]["records"][0]["body"]
        .as_str()
        .expect("a body")
        .to_owned();
    assert!(filtered_body.contains(REDACTED), "{filtered_body}");
    assert!(
        !filtered_body.contains(SENTINEL_MARK),
        "a sentinel survived into `show --json`:\n{filtered_body}"
    );
    assert!(raw_body.contains(AUTHZ_SENTINEL), "{raw_body}");

    assert_eq!(keys(&filtered_doc), keys(&raw_doc));
    assert_eq!(
        keys(&filtered_doc["data"]["records"][0]),
        keys(&raw_doc["data"]["records"][0]),
        "neither setting adds a key to the record shape"
    );
}

/// AC4's `search` half, and the same three runs over one store.
#[test]
fn the_knob_filters_the_search_excerpt_and_raw_reaches_past_it() {
    let bench = bench();
    bench.ingest_fixtures();
    let project = bench.project("session-secrets.jsonl");

    let default = bench.run_in(&project, &["search", "gateway"]);
    assert_eq!(default.status.code(), Some(0), "{}", stderr(&default));
    assert!(
        stdout(&default).contains(AUTHZ_SENTINEL),
        "the premise: this query really does return credential-bearing text:\n{}",
        stdout(&default)
    );

    bench.config(KNOB_ON);

    let filtered = bench.run_in(&project, &["search", "gateway"]);
    assert_eq!(filtered.status.code(), Some(0), "{}", stderr(&filtered));
    let text = stdout(&filtered);
    assert!(
        text.contains(REDACTED),
        "the marker names what went:\n{text}"
    );
    assert!(
        !text.contains(SENTINEL_MARK),
        "a sentinel survived into the excerpt:\n{text}"
    );

    let raw = bench.run_in(&project, &["search", "--raw", "gateway"]);
    assert_eq!(raw.status.code(), Some(0), "{}", stderr(&raw));
    assert_eq!(
        stdout(&raw),
        stdout(&default),
        "--raw under the knob must be what the store answers with the knob absent"
    );

    let filtered_doc = document(&bench.run_in(&project, &["search", "--json", "gateway"]));
    let raw_doc = document(&bench.run_in(&project, &["search", "--json", "--raw", "gateway"]));

    let filtered_text = hit_excerpts(&filtered_doc);
    assert!(filtered_text.contains(REDACTED), "{filtered_text}");
    assert!(
        !filtered_text.contains(SENTINEL_MARK),
        "a sentinel survived into `search --json`:\n{filtered_text}"
    );
    assert!(hit_excerpts(&raw_doc).contains(AUTHZ_SENTINEL), "{raw_doc}");

    assert_eq!(keys(&filtered_doc), keys(&raw_doc));
    assert_eq!(
        keys(&filtered_doc["data"]["hits"][0]),
        keys(&raw_doc["data"]["hits"][0]),
        "neither setting adds a key to the hit shape"
    );
}
