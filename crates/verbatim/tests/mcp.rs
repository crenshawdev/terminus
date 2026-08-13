//! `verbatim mcp` at the process boundary (RCL-07 .. RCL-11).
//!
//! Every test here spawns the built binary and talks JSON-RPC to it over pipes.
//! That is not ceremony: the claims are about a *process* - that it answers a
//! handshake, that it scopes to the directory it was started in, that it holds
//! no socket, that it exits 0 when its stdin closes - and none of them is
//! observable from inside a library call. The default project scope is the
//! process's own working directory (D-12), and `std::env::set_current_dir` is
//! process-global in a test binary that runs its tests in parallel, so a child
//! with a chosen `current_dir` is the only way to assert on it at all.
//!
//! The loop's own message handling is unit-tested beside it in
//! `cmd::mcp::{mod,rpc}`; what is here is what a spawn adds.

#![cfg(feature = "testkit")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use rusqlite::Connection;
use serde_json::{json, Value};
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::testkit;

/// Every directory a spawned `verbatim` may touch, all of them temporary.
///
/// `claude_dir` is as load-bearing as the data directory: a spawn that set only
/// `VERBATIM_DATA_DIR` would resolve the developer's real config, and no test
/// process may reach a real transcript root.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    /// The root the rooted fixtures' `cwd` values were substituted with, and so
    /// the parent of every project directory a spawn can stand in.
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
    /// The command every spawn is built from, standing in `dir`.
    fn command(&self, dir: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
        command
            .args(args)
            .current_dir(dir)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// Archive every fixture through the binary's own ingest path, so what the
    /// server queries is what a real pass wrote.
    fn ingest_fixtures(&self) {
        for fixture in testkit::TRANSCRIPT_FIXTURES {
            let rooted = testkit::ROOTED_FIXTURES.iter().any(|(f, _)| f == fixture);
            let path = if rooted {
                testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root)
            } else {
                testkit::copy_fixture_into(fixture, &self.work)
            };
            let out = self
                .command(&self.work, &["ingest", path.to_str().unwrap()])
                .output()
                .expect("spawn verbatim");
            assert!(
                out.status.success(),
                "ingest {fixture}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
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

    /// Archive one synthetic session of `turns` turns, and answer with the
    /// project directory it belongs to.
    ///
    /// Built here rather than checked in: the repo fixtures are 2 to 19 records
    /// each, and a context budget of 25 turns a side cannot be shown to bind by
    /// a session smaller than the budget. A window that returned the whole
    /// session would satisfy every assertion an existing fixture could make.
    fn long_session(&self, name: &str, turns: usize) -> PathBuf {
        let project = self.root.join(name);
        std::fs::create_dir_all(&project).unwrap();

        let escaped = project.to_string_lossy().replace('\\', "\\\\");
        let mut text = String::new();
        for seq in 0..turns {
            let minute = seq / 60;
            let second = seq % 60;
            text.push_str(&format!(
                "{{\"parentUuid\":null,\"isSidechain\":false,\"userType\":\"external\",\
                  \"cwd\":\"{escaped}\",\"sessionId\":\"99999999-9999-4999-8999-999999999999\",\
                  \"gitBranch\":\"main\",\"version\":\"2.0.31\",\"type\":\"assistant\",\
                  \"uuid\":\"cccccccc-0000-4000-8000-{seq:012}\",\
                  \"timestamp\":\"2026-08-12T21:{minute:02}:{second:02}.000Z\",\
                  \"message\":{{\"role\":\"assistant\",\"content\":\
                  [{{\"type\":\"text\",\"text\":\"longwindowmarker turn {seq}\"}}]}}}}\n"
            ));
        }

        let file = self.work.join(format!("{name}.jsonl"));
        std::fs::write(&file, text).unwrap();
        let out = self
            .command(&self.work, &["ingest", file.to_str().unwrap()])
            .output()
            .expect("spawn verbatim");
        assert!(
            out.status.success(),
            "ingest {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        project
    }

    /// Spawn a live server standing in `dir`, with all three streams piped.
    fn spawn(&self, dir: &Path) -> Child {
        self.command(dir, &["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn verbatim mcp")
    }

    /// Write every message, close stdin, and read back what the server said.
    ///
    /// Closing stdin is the client's own shutdown (D-27), so every conversation
    /// here also exercises the lifecycle: a server that needed a signal to stop
    /// would hang this helper rather than fail an assertion elsewhere.
    fn talk(&self, dir: &Path, messages: &[Value]) -> Conversation {
        let mut child = self.spawn(dir);
        {
            let stdin = child.stdin.as_mut().expect("stdin is piped");
            for message in messages {
                writeln!(stdin, "{message}").expect("write a request");
            }
        }
        // Dropped, which is what closes the pipe and ends the server.
        drop(child.stdin.take());

        let out = child.wait_with_output().expect("the server exits");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let responses = stdout
            .lines()
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|e| {
                    panic!("stdout carried something that is not JSON-RPC ({e}): {line}")
                })
            })
            .collect();
        Conversation {
            responses,
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// One `tools/call`, through the handshake a real client performs first.
    ///
    /// The value returned is the tool's own document - the JSON its single text
    /// content block carries - so a test asserts on hits and reasons rather than
    /// on the envelope around them.
    fn call(&self, dir: &Path, tool: &str, arguments: Value) -> Value {
        let conversation = self.talk(
            dir,
            &[
                initialize(),
                initialized(),
                json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": tool, "arguments": arguments},
                }),
            ],
        );
        conversation.expect_ok();
        assert_eq!(
            conversation.responses.len(),
            2,
            "one response for initialize and one for the call: {:?}",
            conversation.responses
        );
        content(&conversation.responses[1])
    }
}

/// What one conversation with a spawned server produced.
struct Conversation {
    responses: Vec<Value>,
    code: Option<i32>,
    stderr: String,
}

impl Conversation {
    /// The server exited 0 and put no Rust panic anywhere near the wire.
    ///
    /// The panic clause is not decoration: a panic on this path writes a
    /// backtrace onto the transport stream and takes the session's tool with it,
    /// and the symptom a user sees is a tool that silently stops working.
    fn expect_ok(&self) {
        assert_eq!(
            self.code,
            Some(0),
            "the server did not exit 0. stderr: {}",
            self.stderr
        );
        assert!(
            !self.stderr.contains("panicked"),
            "the server panicked: {}",
            self.stderr
        );
        for response in &self.responses {
            assert!(
                !response.to_string().contains("panicked"),
                "a response carried a panic message: {response}"
            );
        }
    }
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"},
        },
    })
}

fn initialized() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// The document inside a `tools/call` response's single text content block.
fn content(response: &Value) -> Value {
    assert!(
        response.get("error").is_none(),
        "a tool call was answered with a JSON-RPC error rather than a result: {response}"
    );
    let blocks = response["result"]["content"]
        .as_array()
        .unwrap_or_else(|| panic!("a tool result carries a content array: {response}"));
    assert_eq!(blocks.len(), 1, "one content block: {response}");
    assert_eq!(blocks[0]["type"], "text", "{response}");
    let text = blocks[0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("a text block carries text: {response}"));
    serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("the tool result is not one JSON document ({e}): {text}"))
}

/// The `hits` of a `recall_search` document, which is always an array.
fn hits(document: &Value) -> Vec<Value> {
    document["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("a search result carries a hits array: {document}"))
        .clone()
}

/// The distinct `project` values a set of hits came from.
fn projects(hits: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = hits
        .iter()
        .map(|hit| hit["project"].as_str().unwrap_or_default().to_owned())
        .collect();
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------
// The handshake (RCL-11, AC7)
// ---------------------------------------------------------------------------

/// AC7's first half: `initialize` and `tools/list` are answered, exactly three
/// tools come back, each marked `readOnlyHint`, and the process exits 0 when
/// stdin closes.
///
/// The exit is the load-bearing part. Claude Code closes stdin, waits two
/// seconds, then sends SIGTERM and two seconds later SIGKILL, so a server that
/// only stopped on a signal would still "work" - and would leave a process alive
/// for two seconds past every session, which is the orphan class this project
/// exists partly to retire.
#[test]
fn the_handshake_reports_three_read_only_tools_and_ends_at_eof() {
    let bench = bench();

    let conversation = bench.talk(
        &bench.work,
        &[
            initialize(),
            initialized(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        ],
    );
    conversation.expect_ok();

    // Two requests, two responses, in order - and nothing at all for the
    // notification, which is a protocol violation to answer.
    let ids: Vec<&Value> = conversation
        .responses
        .iter()
        .map(|response| &response["id"])
        .collect();
    assert_eq!(
        ids,
        vec![&Value::from(1), &Value::from(2)],
        "one response per request and none for the notification: {:?}",
        conversation.responses
    );

    let handshake = &conversation.responses[0]["result"];
    assert_eq!(handshake["protocolVersion"], "2025-06-18");
    assert_eq!(handshake["serverInfo"]["name"], "verbatim");
    assert!(handshake["capabilities"]["tools"].is_object());

    let tools = conversation.responses[1]["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list returns an array: {:?}", conversation.responses));
    assert_eq!(tools.len(), 3, "exactly three tools: {tools:?}");

    let mut names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["recall_context", "recall_get", "recall_search"]);

    for tool in tools {
        assert_eq!(
            tool["annotations"]["readOnlyHint"], true,
            "a tool is not marked read-only: {tool}"
        );
        assert!(
            tool["inputSchema"]["properties"].is_object(),
            "a tool has no input schema: {tool}"
        );
    }
}

/// `ping` is an empty result and an unknown method is `-32601`, with the loop
/// still answering afterwards.
///
/// Both halves are about not being torn down. The bundled client implements
/// `ping` on both sides, so a probe answered "no such method" reads as an
/// unhealthy server; and a session that ended at the first unrecognized message
/// would end at the first client the server did not anticipate.
#[test]
fn ping_is_answered_and_an_unknown_method_does_not_end_the_session() {
    let bench = bench();

    let conversation = bench.talk(
        &bench.work,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}),
        ],
    );
    conversation.expect_ok();

    assert_eq!(conversation.responses.len(), 3);
    assert_eq!(conversation.responses[0]["result"], json!({}));
    assert_eq!(conversation.responses[1]["error"]["code"], -32601);
    assert!(
        conversation.responses[2]["result"].is_object(),
        "the loop stopped at an unknown method: {:?}",
        conversation.responses
    );
}

/// A line that is not JSON is a parse error with a null id, and the session
/// carries on.
#[test]
fn an_unparseable_line_is_a_parse_error_and_the_session_carries_on() {
    let bench = bench();
    let mut child = bench.spawn(&bench.work);
    {
        let stdin = child.stdin.as_mut().expect("stdin is piped");
        writeln!(stdin, "this is not json").unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": 9, "method": "ping"})
        )
        .unwrap();
    }
    drop(child.stdin.take());

    let out = child.wait_with_output().expect("the server exits");
    assert_eq!(out.status.code(), Some(0));
    let responses: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout is JSON-RPC"))
        .collect();

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert_eq!(responses[0]["id"], Value::Null);
    assert_eq!(responses[1]["id"], 9);
}

/// `mcp` takes no arguments: a flag is misuse (exit 2), not a server that
/// silently ignored it.
#[test]
fn mcp_rejects_an_argument_rather_than_ignoring_it() {
    let bench = bench();
    let out = bench
        .command(&bench.work, &["mcp", "--json"])
        .output()
        .expect("spawn verbatim");
    assert_eq!(out.status.code(), Some(2), "{:?}", out);
    assert!(out.stdout.is_empty(), "misuse must print nothing to stdout");
}

// ---------------------------------------------------------------------------
// recall_search (RCL-07, AC5)
// ---------------------------------------------------------------------------

/// The headline call, auto-scoped: a search for a path, made by a server started
/// inside one project, returns that project's turns and nobody else's.
///
/// The scope is not an argument here - it is the process's working directory,
/// which is what Claude Code sets to the session's own project. A raw
/// `MATCH 'src/worker/S.ts'` is `fts5: syntax error near "/"` (D-09), so the
/// path form is also the query shape that would fail without the tokenizer.
#[test]
fn recall_search_is_scoped_to_the_directory_the_server_was_started_in() {
    let bench = bench();
    bench.ingest_fixtures();

    let alpha = bench.project("session-recall.jsonl");
    let beta = bench.project("session-errors-a.jsonl");
    assert_ne!(alpha, beta, "the fixtures must span two projects");

    let found = hits(&bench.call(&alpha, "recall_search", json!({"query": "src/worker/S.ts"})));
    assert!(!found.is_empty(), "the path query matched nothing");
    assert_eq!(
        projects(&found),
        vec![alpha.to_string_lossy().into_owned()],
        "a scoped search returned another project's turns"
    );
    for hit in &found {
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
        assert!(
            hit["excerpt"]
                .as_str()
                .is_some_and(|e| e.contains("src/worker/S.ts")),
            "the excerpt must show what matched (D-05): {hit}"
        );
    }

    // A term that lives only in the other project is absent from this one.
    assert!(
        hits(&bench.call(&alpha, "recall_search", json!({"query": "cargo"}))).is_empty(),
        "a search in one project returned another project's turns"
    );
}

/// `project: "*"` opts out of the auto-scoping and reaches every project.
///
/// Asserted by the project set widening rather than by the hit count: more hits
/// could mean one project's second turn, and what the argument promises is
/// turns from projects the caller is not standing in.
#[test]
fn the_star_project_reaches_more_than_one_project() {
    let bench = bench();
    bench.ingest_fixtures();

    let beta = bench.project("session-errors-a.jsonl");
    let beta_key = beta.to_string_lossy().into_owned();

    let scoped = hits(&bench.call(&beta, "recall_search", json!({"query": "cargo"})));
    assert!(!scoped.is_empty(), "this test needs beta cargo turns");
    assert_eq!(projects(&scoped), vec![beta_key.clone()]);

    let everywhere = hits(&bench.call(
        &beta,
        "recall_search",
        json!({"query": "cargo", "project": "*"}),
    ));
    assert!(
        projects(&everywhere).len() > 1,
        "`*` reached only one project: {:?}",
        projects(&everywhere)
    );
    assert!(
        projects(&everywhere).contains(&beta_key),
        "`*` lost the project the server was standing in"
    );
}

/// AC5's exclusion half through the server: with a project excluded, neither the
/// scoped call nor the `*` call returns any of its turns.
///
/// Archived first and excluded second, which is the case a flag written at
/// ingest could never answer: exclusion is a read-path predicate re-applied on
/// every read, not a stamp and not a deletion.
#[test]
fn an_excluded_projects_turns_are_absent_from_recall_search() {
    let bench = bench();
    bench.ingest_fixtures();

    let beta = bench.project("session-errors-a.jsonl");
    let beta_key = beta.to_string_lossy().into_owned();

    // The control: before the exclusion those turns answer.
    assert!(!hits(&bench.call(&beta, "recall_search", json!({"query": "cargo"}))).is_empty());

    bench.config(&format!("exclude = [{beta_key:?}]\n"));

    // Standing inside the excluded project: an empty result that says why,
    // rather than a silent nothing that reads as an empty archive.
    let document = bench.call(&beta, "recall_search", json!({"query": "cargo"}));
    assert!(hits(&document).is_empty(), "{document}");
    assert!(
        document["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("excluded")),
        "an excluded scope gave no reason: {document}"
    );

    // And under `*`, where exclusion stays in force.
    let everywhere = hits(&bench.call(
        &beta,
        "recall_search",
        json!({"query": "cargo", "project": "*"}),
    ));
    assert!(
        !projects(&everywhere).contains(&beta_key),
        "an excluded project's turn came back under `*`: {everywhere:?}"
    );
}

/// The filters RCL-07 names reach the query layer, and the limit is the
/// server's to lower.
#[test]
fn the_search_filters_and_the_result_budget_reach_the_query_layer() {
    let bench = bench();
    bench.ingest_fixtures();

    let beta = bench.project("session-errors-a.jsonl");
    let found = |arguments: Value| hits(&bench.call(&beta, "recall_search", arguments));

    assert!(!found(json!({"query": "cargo", "tool": "Bash"})).is_empty());
    assert!(
        found(json!({"query": "cargo", "tool": "Read"})).is_empty(),
        "the tool filter narrowed nothing"
    );
    assert!(found(json!({"query": "cargo", "until": "2000-01-01"})).is_empty());
    assert_eq!(found(json!({"query": "cargo", "limit": 1})).len(), 1);

    // A limit above the budget is lowered rather than refused, and the answer
    // says so instead of quietly returning fewer than asked.
    let document = bench.call(
        &beta,
        "recall_search",
        json!({"query": "cargo", "limit": 5000}),
    );
    assert!(!hits(&document).is_empty(), "{document}");
    assert!(hits(&document).len() <= 50, "{document}");
    assert!(
        document["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("budget")),
        "the lowered limit was not reported: {document}"
    );
}

// ---------------------------------------------------------------------------
// recall_context (RCL-08, AC6)
// ---------------------------------------------------------------------------

/// The turns of a window, as `(turn_seq, is_anchor)` pairs.
fn window_seqs(document: &Value) -> Vec<(i64, bool)> {
    document["turns"]
        .as_array()
        .unwrap_or_else(|| panic!("a context result carries a turns array: {document}"))
        .iter()
        .map(|turn| {
            (
                turn["turn_seq"].as_i64().unwrap_or_default(),
                turn["is_anchor"] == true,
            )
        })
        .collect()
}

/// A window around a middle turn returns the turns asked for, in `turn_seq`
/// order, with exactly one of them marked as the anchor.
///
/// The order is the assertion that could pass by accident on a small fixture and
/// be wrong on a real transcript, which is why it is checked against a sort
/// rather than against a literal list: 31 of 62 sampled real transcripts carry a
/// timestamp that goes backwards, so a timestamp sort would look right here and
/// scramble half of the corpus.
#[test]
fn recall_context_returns_the_turns_around_a_hit_in_turn_seq_order() {
    let bench = bench();
    bench.ingest_fixtures();
    let project = bench.long_session("project-window", 60);

    // A turn in the middle of the session, so both sides have something in them.
    let anchor: i64 = bench
        .conn()
        .query_row(
            "SELECT id FROM turns WHERE session_key LIKE '%project-window.jsonl'
              ORDER BY turn_seq LIMIT 1 OFFSET 30",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let document = bench.call(
        &project,
        "recall_context",
        json!({"turn_id": anchor, "before": 3, "after": 2}),
    );
    let turns = window_seqs(&document);
    assert_eq!(
        turns.len(),
        6,
        "three before, the anchor, two after: {document}"
    );

    let seqs: Vec<i64> = turns.iter().map(|(seq, _)| *seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(
        seqs, sorted,
        "the window is not in turn_seq order: {seqs:?}"
    );
    assert_eq!(
        turns.iter().filter(|(_, anchor)| *anchor).count(),
        1,
        "exactly one turn is the anchor: {document}"
    );

    // Neither end of a 60-turn session is 30 turns away, so no boundary is
    // claimed and the text really is the turn's own.
    assert_eq!(document["at_session_start"], false, "{document}");
    assert_eq!(document["at_session_end"], false, "{document}");
    assert!(
        document["turns"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("longwindowmarker")),
        "{document}"
    );
}

/// The first turn of a session that continues from another: nothing before it,
/// the start boundary reported, and the link handed back unfollowed (D-06).
///
/// All three together are the decision. A window that silently returned the
/// previous file's turns would be a lineage walk, and continuation is a fan-out
/// with zero, one or many successors, so the safe shape is to say where the
/// session began and let the caller ask a second question.
#[test]
fn a_window_stops_at_the_session_start_and_names_what_it_continues_from() {
    let bench = bench();
    bench.ingest_fixtures();

    let (first, seq): (i64, i64) = bench
        .conn()
        .query_row(
            "SELECT id, turn_seq FROM turns WHERE session_key LIKE '%session-continuation.jsonl'
              ORDER BY turn_seq LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    // `*`: this fixture hardcodes its `cwd`, so its project key is whatever this
    // machine resolves for that directory and no spawn can reliably stand in it.
    let document = bench.call(
        &bench.work,
        "recall_context",
        json!({"turn_id": first, "before": 5, "after": 1, "project": "*"}),
    );

    assert_eq!(
        document["at_session_start"], true,
        "the first turn of a session is a boundary: {document}"
    );
    assert!(
        window_seqs(&document).iter().all(|(s, _)| *s >= seq),
        "the window returned a turn from before the session began: {document}"
    );
    assert!(
        document["continues_from"].is_string(),
        "the session this one continues from must be named, unfollowed: {document}"
    );
}

/// A request for more turns than the budget returns the budget's worth, and says
/// it was lowered.
///
/// The session is 60 turns and the budget is 25 a side, so a window that ignored
/// the budget would return all 60 and this is the only assertion in the file
/// that can tell the two apart.
#[test]
fn a_window_larger_than_the_budget_returns_the_budgets_worth() {
    let bench = bench();
    let project = bench.long_session("project-window", 60);

    let anchor: i64 = bench
        .conn()
        .query_row(
            "SELECT id FROM turns WHERE session_key LIKE '%project-window.jsonl'
              ORDER BY turn_seq LIMIT 1 OFFSET 30",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let document = bench.call(
        &project,
        "recall_context",
        json!({"turn_id": anchor, "before": 1000, "after": 1000}),
    );
    assert_eq!(
        window_seqs(&document).len(),
        51,
        "twenty-five a side plus the anchor: {document}"
    );
    assert!(
        document["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("budget")),
        "the lowered window was not reported: {document}"
    );
    // The session is longer than the window on both sides, so neither end is a
    // boundary - which is what distinguishes "the budget bound this" from "the
    // session ran out".
    assert_eq!(document["at_session_start"], false, "{document}");
    assert_eq!(document["at_session_end"], false, "{document}");
}
