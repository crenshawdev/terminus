//! `verbatim mcp`: the archive, reachable from inside Claude Code (RCL-07 ..
//! RCL-11).
//!
//! A subcommand of the same binary (D-26), dispatched from `main.rs` like every
//! other command, so install has one artifact to place and no version skew to
//! keep in step.
//!
//! **Bounded by its stdin (D-27).** It reads newline-delimited JSON-RPC
//! messages until EOF and returns 0. No socket, no listener, no port, no
//! background thread, and no ingest lock - reads run beside an ingest pass
//! under WAL, exactly as `cmd::status` already does. Claude Code's stdio
//! transport closes stdin first and only escalates to SIGTERM two seconds
//! later, measured in the 2.1.229 bundle, so exiting at EOF is the whole
//! lifecycle and no signal handling is needed on any platform. A server that
//! outlived its client would be the orphaned-process failure class this project
//! exists partly to retire.
//!
//! **stdout is the transport.** Nothing but JSON-RPC messages is ever written
//! there; every diagnostic goes to stderr. That includes the one warning the
//! shared read path prints for a store older than this build.
//!
//! **The store is opened per call, read-only.** Not once at startup: a session
//! that is ingested while this process is alive should be searchable by the next
//! call, and an open that happened before the client's first question would
//! answer with an archive that is already out of date. `cmd::read::open` is the
//! same entry point `search`, `show` and `sessions` use, so the read-only rule
//! (D-10) and the missing-store answer are one implementation rather than two.

pub mod rpc;
pub mod tools;

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use super::Failure;

/// The server name a client shows the user.
const SERVER_NAME: &str = "verbatim";

/// The protocol revisions Claude Code 2.1.229 accepts, newest first.
///
/// Read out of that bundle rather than guessed. The client sends its own in
/// `initialize` and the server answers with one it supports; a client that
/// cannot use the answer disconnects, and one that can proceeds.
const PROTOCOL_VERSIONS: [&str; 6] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

/// What the server answers when the client asked for a revision it does not
/// know.
///
/// The oldest revision every current client still accepts would be the safe
/// choice for reach and the wrong one for meaning, since the tool-annotation
/// fields this server relies on were not in it. `2025-06-18` is the revision
/// this server's descriptors are written against.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

/// The parsed command line for `mcp`, which takes nothing.
///
/// A unit rather than nothing at all so the dispatch table reads the same as
/// every other command's, and so a future flag has somewhere to land.
#[derive(Debug)]
pub struct Args;

/// `mcp` takes no arguments: it is spawned by a client, not typed.
///
/// Rejecting rather than ignoring, the rule every subcommand here keeps: a
/// command that silently ignored an argument is how `verify --json` came to look
/// supported before it was.
pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    if let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        return Err(Failure::Misuse(crate::unexpected(arg)));
    }
    Ok(Args)
}

pub fn run(_args: Args) -> Result<(), Failure> {
    serve(std::io::stdin().lock(), std::io::stdout().lock())
}

/// The loop, over any reader and writer so a test can drive it without a spawn.
///
/// Lines are read as **bytes** and converted lossily rather than read as UTF-8:
/// `read_line` on a non-UTF-8 byte is an `InvalidData` error, and a transport
/// glitch must be one malformed message answered with a parse error, not the end
/// of the session.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> Result<(), Failure> {
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        let read = input
            .read_until(b'\n', &mut line)
            .map_err(|e| Failure::Operational(format!("reading stdin: {e}")))?;
        // EOF. The client closed the pipe, which is how this process is meant to
        // end: exit 0, having held nothing.
        if read == 0 {
            return Ok(());
        }

        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        // A blank line is framing, not a message. Answering one with a parse
        // error would make a transport that writes `\r\n` look broken.
        if text.is_empty() {
            continue;
        }

        let Some(response) = answer(text) else {
            continue;
        };
        // Flushed per message: the client is blocked on this line, and a
        // response sitting in a buffer is a hung tool call.
        if writeln!(output, "{response}").is_err() || output.flush().is_err() {
            // The client went away mid-write. That is the same shutdown EOF is,
            // arriving through the other pipe, and it is not a failure of the
            // archive.
            return Ok(());
        }
    }
}

/// One line in, one response line out - or `None` for a notification, which is
/// answered with nothing, ever.
fn answer(line: &str) -> Option<Value> {
    match rpc::parse(line) {
        rpc::Incoming::Notification => None,
        rpc::Incoming::Invalid { id, code, message } => {
            Some(rpc::failure(id.as_ref(), code, &message))
        }
        rpc::Incoming::Request(request) => Some(match request.method.as_str() {
            "initialize" => rpc::result(&request.id, initialize(&request.params)),
            "tools/list" => rpc::result(&request.id, json!({"tools": tools::descriptors()})),
            // The only method that reads the archive. Its `Err` is a protocol
            // failure - params that do not name a call this server has - and
            // never a tool that could not answer, which is a result carrying a
            // reason (RCL-10).
            "tools/call" => match tools::call(&request.params) {
                Ok(result) => rpc::result(&request.id, result),
                Err((code, message)) => rpc::failure(Some(&request.id), code, &message),
            },
            // Answered with an empty result, not `-32601`: the bundled client
            // implements `ping` on both sides, and a liveness probe answered
            // "no such method" reads as an unhealthy server.
            "ping" => rpc::result(&request.id, json!({})),
            other => rpc::failure(
                Some(&request.id),
                rpc::code::METHOD_NOT_FOUND,
                &format!("this server has no method {other:?}"),
            ),
        }),
    }
}

/// The handshake answer: the negotiated revision, the one capability this
/// server has, and who it is.
fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = match requested {
        Some(asked) if PROTOCOL_VERSIONS.contains(&asked) => asked,
        _ => DEFAULT_PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": version,
        // Tools and nothing else. No resources, no prompts, no sampling, and
        // no `listChanged`: the tool list is fixed at compile time, so a server
        // advertising that it might change would be promising a notification it
        // can never have cause to send.
        "capabilities": {"tools": {}},
        "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the loop over a byte buffer and read back one response per line.
    fn exchange(input: &str) -> Vec<Value> {
        let mut output: Vec<u8> = Vec::new();
        serve(input.as_bytes(), &mut output).expect("the loop ends at EOF");
        String::from_utf8_lossy(&output)
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("stdout is not JSON-RPC ({e}): {line}"))
            })
            .collect()
    }

    /// The client's own revision is echoed when the server knows it, and an
    /// unknown one is answered with the server's pinned default rather than
    /// with an error.
    #[test]
    fn initialize_negotiates_a_version_both_sides_know() {
        for asked in PROTOCOL_VERSIONS {
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{asked}"}}}}"#
            );
            let responses = exchange(&line);
            assert_eq!(responses.len(), 1);
            assert_eq!(
                responses[0]["result"]["protocolVersion"],
                Value::from(asked)
            );
        }

        let responses = exchange(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        );
        assert_eq!(
            responses[0]["result"]["protocolVersion"],
            Value::from(DEFAULT_PROTOCOL_VERSION),
            "an unknown revision must be answered, not refused"
        );
        assert_eq!(
            responses[0]["result"]["serverInfo"]["name"],
            Value::from(SERVER_NAME)
        );
        assert!(responses[0]["result"]["capabilities"]["tools"].is_object());
    }

    /// One response per request, in order, and none at all for a notification.
    #[test]
    fn a_notification_is_never_answered_and_every_request_is() {
        let responses = exchange(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n\
             {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
             \n\
             {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n\
             {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/list\"}\n",
        );

        let ids: Vec<&Value> = responses.iter().map(|r| &r["id"]).collect();
        assert_eq!(
            ids,
            vec![&Value::from(1), &Value::from(2), &Value::from(3)],
            "one response per request, in order, and none for the notification"
        );
        assert_eq!(responses[1]["result"], json!({}), "ping is an empty result");
        assert_eq!(
            responses[2]["result"]["tools"].as_array().map(Vec::len),
            Some(3)
        );
    }

    /// An unknown method is `-32601`, and the loop answers the next request
    /// afterwards rather than ending.
    #[test]
    fn an_unknown_method_is_an_error_and_the_loop_carries_on() {
        let responses = exchange(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"resources/list\"}\n\
             garbage that is not json\n\
             {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n",
        );
        assert_eq!(responses.len(), 3);
        assert_eq!(
            responses[0]["error"]["code"],
            Value::from(rpc::code::METHOD_NOT_FOUND)
        );
        assert_eq!(
            responses[1]["error"]["code"],
            Value::from(rpc::code::PARSE_ERROR)
        );
        assert_eq!(responses[1]["id"], Value::Null);
        assert!(responses[2]["result"].is_object(), "the loop carried on");
    }
}
