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

use serde_json::{json, Value};

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
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        config_dir,
        claude_dir,
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
