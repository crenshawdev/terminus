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
        self.call_flagged(dir, tool, arguments).0
    }

    /// The same, with the result's `isError` flag.
    ///
    /// Kept separate because most assertions are about the document and only a
    /// few are about the flag - but the flag is what a client shows the model as
    /// an error, so "an empty result with a reason" and "a failed call" have to
    /// be tellable apart somewhere.
    fn call_flagged(&self, dir: &Path, tool: &str, arguments: Value) -> (Value, bool) {
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
        let response = &conversation.responses[1];
        (content(response), response["result"]["isError"] == true)
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

// ---------------------------------------------------------------------------
// recall_get (RCL-09, AC6)
// ---------------------------------------------------------------------------

/// The `records` of a `recall_get` document.
fn records(document: &Value) -> Vec<Value> {
    document["records"]
        .as_array()
        .unwrap_or_else(|| panic!("a get result carries a records array: {document}"))
        .clone()
}

/// One fixture's line `index`, with the root token substituted the way ingest
/// saw it.
fn fixture_line(bench: &Bench, fixture: &str, index: usize) -> String {
    let text = String::from_utf8(testkit::fixture_bytes(fixture)).unwrap();
    text.replace(
        testkit::FIXTURE_ROOT_TOKEN,
        &bench.root.to_string_lossy().replace('\\', "\\\\"),
    )
    .lines()
    .nth(index)
    .unwrap_or_else(|| panic!("{fixture} has no line {index}"))
    .to_owned()
}

/// Ids from two sessions come back as the archive holds them: byte for byte the
/// fixtures' own lines, in the order the ids were asked for.
///
/// Compared against the transcript files rather than against anything the store
/// derived. A projection would be prose about the right turn and would pass any
/// test that only checked the turn was found - and this is the one tool whose
/// whole job is that it does not do that.
#[test]
fn recall_get_returns_the_archived_lines_of_two_sessions() {
    let bench = bench();
    bench.ingest_fixtures();
    let alpha = bench.project("session-recall.jsonl");

    let id_of = |fixture: &str, seq: i64| -> i64 {
        bench
            .conn()
            .query_row(
                &format!(
                    "SELECT id FROM turns WHERE session_key LIKE '%{fixture}'
                      ORDER BY turn_seq LIMIT 1 OFFSET {seq}"
                ),
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("{fixture} turn {seq}: {e}"))
    };

    // Both sessions belong to project-alpha, so one auto-scoped call reaches
    // them both and the blob grouping has two sessions to group.
    let recall = id_of("session-recall.jsonl", 1);
    let echo = id_of("agent-echo.jsonl", 0);

    let document = bench.call(&alpha, "recall_get", json!({"turn_ids": [recall, echo]}));
    let got = records(&document);
    assert_eq!(got.len(), 2, "{document}");
    assert_eq!(
        got.iter()
            .map(|record| record["turn_id"].as_i64().unwrap_or_default())
            .collect::<Vec<i64>>(),
        vec![recall, echo],
        "records must come back in the order the ids were given: {document}"
    );

    assert_eq!(
        got[0]["body"].as_str().unwrap_or_default(),
        fixture_line(&bench, "session-recall.jsonl", 1)
    );
    assert_eq!(
        got[1]["body"].as_str().unwrap_or_default(),
        fixture_line(&bench, "subagents/agent-echo.jsonl", 0)
    );

    for record in &got {
        for field in [
            "turn_id",
            "session_key",
            "turn_seq",
            "record_type",
            "tool_name",
            "ts",
            "project",
            "body",
            "body_evicted",
        ] {
            assert!(
                record.get(field).is_some(),
                "a record is missing {field}: {record}"
            );
        }
        assert_eq!(record["body_evicted"], false, "{record}");
    }
}

/// D-08 through the server: a turn whose session is evicted comes back flagged,
/// with no body, and the call still succeeds.
///
/// Retention is phase 8 and nothing writes that column before then, so the test
/// sets it directly - which is also the point. The flag is read off the column
/// and never off a failed blob read, so this stays distinguishable from the
/// archive damage `verbatim verify` reports.
#[test]
fn an_evicted_body_is_a_flag_and_not_a_failed_call() {
    let bench = bench();
    bench.ingest_fixtures();
    let alpha = bench.project("session-recall.jsonl");

    let id: i64 = bench
        .conn()
        .query_row(
            "SELECT id FROM turns WHERE session_key LIKE '%session-recall.jsonl'
              ORDER BY turn_seq LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    bench
        .conn()
        .execute(
            "UPDATE session_meta SET is_evicted = 1
              WHERE session_key = (SELECT session_key FROM turns WHERE id = ?1)",
            [id],
        )
        .unwrap();

    let (document, is_error) = bench.call_flagged(&alpha, "recall_get", json!({"turn_ids": [id]}));
    assert!(
        !is_error,
        "an evicted body reported as a failed call: {document}"
    );

    let got = records(&document);
    assert_eq!(got.len(), 1, "{document}");
    assert_eq!(got[0]["body_evicted"], true, "{document}");
    assert_eq!(got[0]["body"], Value::Null, "{document}");
    // Everything else about the turn is still known: only its bytes are gone.
    assert_eq!(got[0]["turn_id"], id);
    assert!(got[0]["ts"].is_string(), "{document}");
}

/// An id that names no turn this caller may see is a row in `absent` with a
/// reason, never a throw - and a known id beside it still answers.
#[test]
fn an_unknown_id_is_a_reason_beside_the_records_that_did_answer() {
    let bench = bench();
    bench.ingest_fixtures();
    let alpha = bench.project("session-recall.jsonl");

    let known: i64 = bench
        .conn()
        .query_row(
            "SELECT id FROM turns WHERE session_key LIKE '%session-recall.jsonl'
              ORDER BY turn_seq LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let unknown: i64 = bench
        .conn()
        .query_row("SELECT max(id) + 1000 FROM turns", [], |r| r.get(0))
        .unwrap();

    let document = bench.call(&alpha, "recall_get", json!({"turn_ids": [known, unknown]}));
    assert_eq!(records(&document).len(), 1, "{document}");
    let absent = document["absent"].as_array().cloned().unwrap_or_default();
    assert_eq!(absent.len(), 1, "{document}");
    assert_eq!(absent[0]["turn_id"], unknown);
    assert!(
        absent[0]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no turn")),
        "{document}"
    );

    // Asked for the unknown id alone, the reason is the one a caller reads
    // first, not a key buried in a list.
    let (alone, is_error) =
        bench.call_flagged(&alpha, "recall_get", json!({"turn_ids": [unknown]}));
    assert!(
        !is_error,
        "an unknown id reported as a failed call: {alone}"
    );
    assert!(records(&alone).is_empty(), "{alone}");
    assert!(
        alone["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no turn")),
        "{alone}"
    );
}

/// The bound this tool needs and the terminal path does not: a client-supplied
/// list longer than the cap is truncated and reported, never bound one SQL
/// placeholder at a time until the query fails.
#[test]
fn a_list_longer_than_the_cap_is_truncated_and_said_so() {
    let bench = bench();
    let project = bench.long_session("project-window", 60);

    let ids: Vec<i64> = bench
        .conn()
        .prepare(
            "SELECT id FROM turns WHERE session_key LIKE '%project-window.jsonl'
              ORDER BY turn_seq LIMIT 40",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids.len(), 40, "this test needs more ids than the cap");

    let (document, is_error) =
        bench.call_flagged(&project, "recall_get", json!({"turn_ids": ids.clone()}));
    assert!(
        !is_error,
        "a bounded answer is not a failed call: {document}"
    );
    assert_eq!(records(&document).len(), 25, "{document}");
    assert_eq!(
        records(&document)
            .iter()
            .map(|record| record["turn_id"].as_i64().unwrap_or_default())
            .collect::<Vec<i64>>(),
        ids[..25].to_vec(),
        "the first ids asked for are the ones served: {document}"
    );
    assert!(
        document["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("caps one request")),
        "the truncation was not reported: {document}"
    );
}

// ---------------------------------------------------------------------------
// Bounds and malformed requests (RCL-10, AC6)
// ---------------------------------------------------------------------------

/// This tool's empty document, whatever went wrong: the same keys a successful
/// answer carries, so a client parses one shape either way.
fn assert_empty_shape(tool: &str, document: &Value) {
    match tool {
        "recall_search" => assert_eq!(document["hits"], json!([]), "{document}"),
        "recall_context" => {
            assert_eq!(document["turns"], json!([]), "{document}");
            assert_eq!(document["at_session_start"], false, "{document}");
            assert_eq!(document["continues_from"], Value::Null, "{document}");
        }
        _ => {
            assert_eq!(document["records"], json!([]), "{document}");
            assert_eq!(document["absent"], json!([]), "{document}");
        }
    }
}

/// Every way a caller can malform a `tools/call` for a tool that exists: a
/// successful JSON-RPC response whose content is an empty result carrying a
/// reason, marked `isError`, and never a panic.
///
/// RCL-10 wants the empty-result-with-reason contract because a throw is what
/// the client surfaces to the user as a broken server: the model can read a
/// reason and call again, and it cannot read a JSON-RPC error the same way. The
/// `isError` flag is what still tells the two apart from an ordinary empty
/// answer.
#[test]
fn a_malformed_call_is_an_empty_result_with_a_reason_and_never_a_throw() {
    let bench = bench();
    bench.ingest_fixtures();
    let alpha = bench.project("session-recall.jsonl");

    let cases: &[(&str, Value)] = &[
        // A required argument, absent.
        ("recall_search", json!({})),
        ("recall_context", json!({})),
        ("recall_get", json!({})),
        // `arguments` itself of the wrong JSON type.
        ("recall_search", json!("not an object")),
        ("recall_get", json!([1, 2, 3])),
        // An argument of the wrong JSON type.
        ("recall_search", json!({"query": 12})),
        ("recall_search", json!({"query": "cargo", "tool": true})),
        (
            "recall_search",
            json!({"query": "cargo", "paths": "src/a.rs"}),
        ),
        ("recall_search", json!({"query": "cargo", "paths": [7]})),
        ("recall_context", json!({"turn_id": "12"})),
        ("recall_get", json!({"turn_ids": 12})),
        ("recall_get", json!({"turn_ids": ["12"]})),
        ("recall_get", json!({"turn_ids": []})),
        // A limit that is not a number, is negative, or is not whole.
        ("recall_search", json!({"query": "cargo", "limit": "ten"})),
        ("recall_search", json!({"query": "cargo", "limit": -1})),
        ("recall_search", json!({"query": "cargo", "limit": 2.5})),
        ("recall_search", json!({"query": "cargo", "limit": 0})),
        ("recall_context", json!({"turn_id": 1, "before": -1})),
        // A date nothing can parse. `2026-13-99` is deliberately NOT in this
        // list: D-23 pins a bound to a shape rather than to a calendar - every
        // stored timestamp is `NNNN-NN-NNTNN:NN:NN.NNNZ` and the comparison is
        // lexicographic with no date parsing on the stored side - so a shaped
        // impossibility is a bound that compares cleanly, and only an unshaped
        // string is a caller error.
        (
            "recall_search",
            json!({"query": "cargo", "since": "last tuesday"}),
        ),
    ];

    for (tool, arguments) in cases {
        let (document, is_error) = bench.call_flagged(&alpha, tool, arguments.clone());
        assert!(
            is_error,
            "`{tool}` with {arguments} was not marked as a caller error: {document}"
        );
        assert!(
            document["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()),
            "`{tool}` with {arguments} gave no reason: {document}"
        );
        assert!(
            !document.to_string().contains("panicked"),
            "`{tool}` with {arguments} returned a panic message: {document}"
        );
        assert_empty_shape(tool, &document);
    }
}

/// The line between the two failure kinds: a tool this server does not have is a
/// JSON-RPC error, a bad argument to a tool it does have is not.
///
/// A request the server understood and could not satisfy is a result; a message
/// it could not understand as a request is an error. Collapsing the two either
/// way is what makes a client report a broken server for a typo, or swallow a
/// call it never routed.
#[test]
fn an_unknown_tool_is_a_protocol_error_and_a_bad_argument_is_not() {
    let bench = bench();

    let conversation = bench.talk(
        &bench.work,
        &[
            initialize(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                   "params": {"name": "recall_everything", "arguments": {}}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                   "params": {"name": "recall_search", "arguments": {}}}),
        ],
    );
    conversation.expect_ok();
    assert_eq!(conversation.responses.len(), 4);

    for index in [1, 2] {
        assert_eq!(
            conversation.responses[index]["error"]["code"], -32602,
            "params that name no call must be a protocol error: {:?}",
            conversation.responses[index]
        );
    }
    assert!(
        conversation.responses[3].get("error").is_none(),
        "a missing argument must not be a protocol error: {:?}",
        conversation.responses[3]
    );
}

/// A malformed call does not end the session: the server answers the next
/// request over the same pipes.
///
/// This is the property a per-call `unwrap` would break invisibly. The bad call
/// would still "fail", and the tool would be gone for the rest of the session
/// with nothing in the transcript saying why.
#[test]
fn the_server_answers_the_next_request_after_a_malformed_one() {
    let bench = bench();
    bench.ingest_fixtures();
    let alpha = bench.project("session-recall.jsonl");

    let conversation = bench.talk(
        &alpha,
        &[
            initialize(),
            initialized(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                   "params": {"name": "recall_get", "arguments": {"turn_ids": "nope"}}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                   "params": {"name": "recall_search",
                              "arguments": {"query": "src/worker/S.ts"}}}),
            json!({"jsonrpc": "2.0", "id": 4, "method": "ping"}),
        ],
    );
    conversation.expect_ok();
    // The handshake plus the three requests. The notification is not in the
    // count, because it is never answered.
    assert_eq!(
        conversation.responses.len(),
        4,
        "{:?}",
        conversation.responses
    );

    assert_eq!(conversation.responses[1]["result"]["isError"], true);
    assert!(
        !hits(&content(&conversation.responses[2])).is_empty(),
        "the search after the malformed call answered nothing: {:?}",
        conversation.responses[2]
    );
    assert_eq!(conversation.responses[3]["result"], json!({}));
}

/// An argument above a budget is lowered to it, not refused: the answer comes
/// back bounded and says the budget was applied.
///
/// Deliberately not an empty result. An over-large `limit` is a client being
/// optimistic rather than a client being wrong, and RCL-10 wants an answer -
/// what the budget exists to stop is the archive arriving in the model's context
/// one call at a time, and a bounded answer stops that.
#[test]
fn an_over_large_argument_is_lowered_to_the_budget_rather_than_refused() {
    let bench = bench();
    bench.ingest_fixtures();
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
    let ids: Vec<i64> = bench
        .conn()
        .prepare(
            "SELECT id FROM turns WHERE session_key LIKE '%project-window.jsonl'
              ORDER BY turn_seq LIMIT 60",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    let cases: &[(&str, Value, &str, usize)] = &[
        (
            "recall_search",
            json!({"query": "longwindowmarker", "limit": 100_000}),
            "hits",
            50,
        ),
        (
            "recall_context",
            json!({"turn_id": anchor, "before": 100_000, "after": 100_000}),
            "turns",
            51,
        ),
        (
            "recall_get",
            json!({"turn_ids": ids, "project": "*"}),
            "records",
            25,
        ),
    ];

    for (tool, arguments, key, most) in cases {
        let (document, is_error) = bench.call_flagged(&project, tool, arguments.clone());
        assert!(
            !is_error,
            "`{tool}` refused an over-large argument: {document}"
        );

        let returned = document[key].as_array().map(Vec::len).unwrap_or_default();
        assert!(
            returned > 0 && returned <= *most,
            "`{tool}` returned {returned}, outside 1..={most}: {document}"
        );
        assert!(
            document["reason"].as_str().is_some_and(|reason| {
                reason.contains("budget") || reason.contains("caps one request")
            }),
            "`{tool}` did not say the budget was applied: {document}"
        );
    }
}

// ---------------------------------------------------------------------------
// The process itself (RCL-11, AC7)
// ---------------------------------------------------------------------------

/// The socket-inspection tool, if this machine has one.
///
/// Probed rather than assumed, and its absence is announced rather than
/// swallowed: a suite that skipped this silently would go on reporting green
/// while the one assertion about the process holding no port had stopped
/// running.
fn socket_tool() -> Option<&'static str> {
    ["/usr/bin/ss", "/usr/sbin/ss", "/sbin/ss", "/bin/ss"]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
}

/// RCL-11 at the process level: while it is alive the server holds no listening
/// socket, and when its stdin closes it exits 0 without any signal being sent.
///
/// The second half is what makes Claude Code's shutdown clean rather than a
/// kill. That client closes stdin, waits two seconds, sends SIGTERM, waits two
/// more and sends SIGKILL - so a server that only stopped on a signal would look
/// like it worked while leaving a process alive past every session, which is the
/// orphaned-process failure class this project exists partly to retire. Nothing
/// in this test sends a signal on the passing path.
///
/// The server is driven through a real call first, on purpose: a process
/// inspected before it had done any work would hold nothing for the trivial
/// reason that it had not started.
#[test]
fn the_server_holds_no_listening_socket_and_exits_zero_when_stdin_closes() {
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    let bench = bench();
    bench.ingest_fixtures();
    let alpha = bench.project("session-recall.jsonl");

    let mut child = bench.spawn(&alpha);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    {
        let stdin = child.stdin.as_mut().expect("stdin is piped");
        writeln!(stdin, "{}", initialize()).unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                   "params": {"name": "recall_search",
                              "arguments": {"query": "src/worker/S.ts"}}})
        )
        .unwrap();
        stdin.flush().unwrap();
    }
    for expected in [1, 2] {
        let mut line = String::new();
        stdout.read_line(&mut line).expect("a response");
        let response: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("stdout is not JSON-RPC ({e}): {line}"));
        assert_eq!(response["id"], expected, "{response}");
    }

    let pid = child.id();
    match socket_tool() {
        None => eprintln!(
            "SKIP: no `ss` on this machine, so the listening-socket assertion for \
             pid {pid} did not run"
        ),
        Some(tool) => {
            let listening = Command::new(tool)
                .args(["-l", "-n", "-p"])
                .output()
                .expect("run the socket tool");
            assert!(
                listening.status.success(),
                "the socket tool failed: {}",
                String::from_utf8_lossy(&listening.stderr)
            );
            let table = String::from_utf8_lossy(&listening.stdout);
            assert!(
                !table.contains(&format!("pid={pid}")),
                "the server holds a listening socket:\n{table}"
            );
        }
    }

    // Stdin closes and nothing else happens: no signal, no kill.
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("wait on the server") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("the server was still alive 10s after its stdin closed");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    assert_eq!(
        status.code(),
        Some(0),
        "the server did not exit 0 at EOF: {status:?}"
    );
}

/// AC7's second half: pointed at a data directory holding no store, the server
/// answers with an empty result and a reason, exits 0, and leaves not one byte
/// behind.
///
/// The last clause is what `Store::open` would fail. It would create the data
/// directory, initialize a database and set `journal_mode=wal`, so a server
/// advertising `readOnlyHint` would have created a database as the side effect
/// of being asked a question - the contradiction D-10 exists to prevent. Both
/// starting states are covered: a directory that exists and is empty, and a path
/// that is not there at all.
#[test]
fn a_server_with_no_store_answers_with_a_reason_and_creates_no_file() {
    let bench = bench();

    // A directory that exists and holds nothing. Read back afterwards, because
    // "the directory is still there" and "the directory is still empty" are
    // different facts and only the second one is the claim.
    std::fs::create_dir_all(&bench.data_dir).unwrap();
    let (document, is_error) =
        bench.call_flagged(&bench.work, "recall_search", json!({"query": "anything"}));
    assert!(
        !is_error,
        "an archive with nothing in it is an empty result, not a failed call: {document}"
    );
    assert!(hits(&document).is_empty(), "{document}");
    assert!(
        document["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no verbatim store")),
        "a missing store gave no reason: {document}"
    );
    let left = std::fs::read_dir(&bench.data_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert!(left.is_empty(), "a read created {left:?} in the data dir");

    // And a data directory that is not there at all is not brought into being.
    std::fs::remove_dir(&bench.data_dir).unwrap();
    for tool in ["recall_search", "recall_context", "recall_get"] {
        let arguments = match tool {
            "recall_search" => json!({"query": "anything"}),
            "recall_context" => json!({"turn_id": 1}),
            _ => json!({"turn_ids": [1]}),
        };
        let document = bench.call(&bench.work, tool, arguments);
        assert!(
            document["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("no verbatim store")),
            "`{tool}` gave no reason with no store: {document}"
        );
    }
    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory it was asked about"
    );
}

/// The `paths` filter binds one SQL placeholder each, so an unbounded
/// client-supplied array was an operational SQLite failure handed back as an
/// ordinary empty result carrying the whole generated statement.
///
/// Two wrong behaviours in the one answer: a model reads `isError:false` with
/// no hits as "no matches" rather than "your filter was rejected", and the
/// reason itself pushed megabytes into the context the budget exists to
/// protect.
#[test]
fn a_path_filter_longer_than_the_cap_is_bounded_and_said_so() {
    let bench = bench();
    bench.ingest_fixtures();
    let alpha = bench.project("session-recall.jsonl");

    let paths: Vec<String> = (0..40_000).map(|n| format!("/x/{n}")).collect();
    let (document, is_error) = bench.call_flagged(
        &alpha,
        "recall_search",
        serde_json::json!({ "query": "the", "paths": paths }),
    );

    let reason = document["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("paths were filtered on"),
        "the cap was not reported: {reason}"
    );
    assert!(
        !reason.contains("SELECT") && !reason.contains("sqlite:"),
        "the generated SQL reached the model's context: {reason}"
    );
    assert!(
        reason.len() < 400,
        "the reason is {} bytes, which is a context budget problem",
        reason.len()
    );
    assert!(!is_error, "a bounded filter is not a failed call");
}

/// An excluded project's name must not reach a client that never asked for it.
///
/// Both `recall_get` and `recall_context` refuse an id they will not serve, but
/// only one of the two refusals names a path. The exclusion arm ran before the
/// scope arm, so at the DEFAULT scope - no `project` argument, the only shape a
/// model normally sends - an id inside an excluded project answered
/// "<absolute path> is excluded by config", confirming both that the id exists
/// and what the excluded project is called. Ids are `session_no << 24 |
/// turn_seq`, so sweeping them mapped every excluded project's name and live id
/// ranges with no argument a client had to pass.
#[test]
fn an_excluded_project_is_never_named_to_a_client_at_the_default_scope() {
    let bench = bench();
    bench.ingest_fixtures();

    let alpha = bench.project("session-recall.jsonl");
    let excluded = bench.project("session-errors-a.jsonl");
    assert_ne!(alpha, excluded, "the fixtures must span two projects");
    let excluded_name = excluded.to_string_lossy().into_owned();

    // Ids that really do live in the excluded project, which is what makes the
    // silence meaningful rather than incidental. Read before the exclusion,
    // since afterwards nothing will admit they exist.
    let ids: Vec<i64> = {
        let conn = bench.conn();
        let mut statement = conn
            .prepare(
                "SELECT t.id FROM turns t JOIN session_meta m USING (session_key)
                  WHERE m.project = ?1 ORDER BY t.id LIMIT 4",
            )
            .unwrap();
        let rows = statement
            .query_map([&excluded_name], |r| r.get::<_, i64>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows
    };
    assert!(!ids.is_empty(), "the excluded project must hold turns");

    bench.config(&format!("exclude = [{excluded_name:?}]\n"));

    let got = bench.call(&alpha, "recall_get", serde_json::json!({ "turn_ids": ids }));
    let rendered = got.to_string();
    assert!(
        !rendered.contains(&excluded_name),
        "recall_get named the excluded project: {rendered}"
    );
    assert!(
        !rendered.contains("excluded by config"),
        "recall_get confirmed the id belongs to an excluded project: {rendered}"
    );

    let window = bench.call(
        &alpha,
        "recall_context",
        serde_json::json!({ "turn_id": ids[0] }),
    );
    let rendered = window.to_string();
    assert!(
        !rendered.contains(&excluded_name),
        "recall_context named the excluded project: {rendered}"
    );
    assert!(
        !rendered.contains("excluded by config"),
        "recall_context confirmed the id belongs to an excluded project: {rendered}"
    );

    // The control: an id that is simply not archived answers the same way, so
    // "excluded" and "absent" are indistinguishable from outside.
    let absent = bench.call(&alpha, "recall_context", serde_json::json!({"turn_id": 1}));
    assert_eq!(
        absent["reason"].as_str().is_some(),
        window["reason"].as_str().is_some(),
        "an excluded id and an unarchived id answer differently"
    );
}

/// OBS-08 at the tool boundary: the model reaches observations through the
/// search tool it already has, and there is still no fourth tool.
#[test]
fn recall_search_reaches_observations_through_the_kind_it_already_has() {
    let bench = bench();
    bench.ingest_fixtures();

    // A claim naming a word the corpus really carries, so the two kinds have
    // something to be told apart by.
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
                json!([{"turn_id": anchor, "text": "the brillig helper was replaced"}]).to_string()
            ],
        )
        .unwrap();

    let document = bench.call(
        &bench.work,
        "recall_search",
        json!({"query": "brillig", "project": "*", "kind": "observation"}),
    );
    let hits = document["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 1, "{document}");
    assert_eq!(hits[0]["record_type"], "observation", "{document}");
    assert_eq!(hits[0]["turn_id"], anchor, "{document}");
    assert!(
        hits[0]["excerpt"]
            .as_str()
            .is_some_and(|text| text.contains("brillig")),
        "the claim text is not the excerpt: {document}"
    );

    // The same query under the default kind is a turn search, and no
    // observation is in it.
    let turns = bench.call(
        &bench.work,
        "recall_search",
        json!({"query": "brillig", "project": "*"}),
    );
    let hits = turns["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "{turns}");
    assert!(
        hits.iter().all(|hit| hit["record_type"] != "observation"),
        "{turns}"
    );

    // And the schema told the model the value exists at all - a filter it is
    // never shown is a filter it never sends.
    let listed = bench.talk(
        &bench.work,
        &[
            initialize(),
            initialized(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        ],
    );
    listed.expect_ok();
    let tools = listed.responses[1]["result"]["tools"]
        .as_array()
        .expect("a tool list");
    assert_eq!(tools.len(), 3, "OBS-08 must not add a fourth tool");
    let kind = tools
        .iter()
        .find(|tool| tool["name"] == "recall_search")
        .expect("recall_search")["inputSchema"]["properties"]["kind"]["description"]
        .as_str()
        .expect("a kind description");
    assert!(
        kind.contains("observation"),
        "the model is never told the value exists: {kind}"
    );
}
