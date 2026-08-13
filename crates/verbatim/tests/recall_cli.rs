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
const READ_COMMANDS: &[&[&str]] = &[&["search", "anything"]];

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
