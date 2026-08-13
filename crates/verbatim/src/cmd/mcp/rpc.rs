//! JSON-RPC 2.0, hand-written over `serde_json` (D-11).
//!
//! No SDK and no async runtime. `.planning/PROJECT.md` bars a runtime outright -
//! cold start is the product and the hook path must not pay for one - and the
//! official SDK would link tokio into the same binary the hooks call. What is
//! left is small: one message per line, one response line per request, and the
//! four standard error codes.
//!
//! **Framing is newline-delimited.** One JSON value per line, which is what
//! Claude Code's stdio transport writes and reads. A blank line is skipped
//! rather than answered, because a transport that writes `\r\n` should not
//! produce a parse error for the empty half.
//!
//! **A notification is never answered.** A message with no `id` is a
//! notification - `notifications/initialized` is the one every client sends
//! immediately after `initialize` - and JSON-RPC forbids a response to one.
//! Replying is a protocol violation, and a client that reads the reply as the
//! answer to its *next* request is desynchronized for the rest of the session.
//!
//! **Errors here are protocol-level only.** A request the server understood and
//! could not satisfy is a tool *result* carrying a reason
//! ([`super::tools`]); a message the server could not understand as a request is
//! one of these.

use serde_json::{json, Map, Value};

/// The only `jsonrpc` value this server accepts or emits.
pub const VERSION: &str = "2.0";

/// The standard JSON-RPC 2.0 error codes, and the only ones this server sends.
pub mod code {
    /// The line was not JSON at all.
    pub const PARSE_ERROR: i64 = -32700;
    /// The line was JSON but not a request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// A method this server does not implement.
    ///
    /// Deliberately NOT what `ping` gets: the bundled client implements `ping`
    /// on both sides, and a liveness probe answered `-32601` reads as an
    /// unhealthy server and gets the connection torn down mid-session.
    pub const METHOD_NOT_FOUND: i64 = -32601;
}

/// One request that has an id and therefore expects exactly one response.
#[derive(Debug)]
pub struct Request {
    /// Echoed back verbatim. A string and a number are different ids to a
    /// client that sent both, so this is the value and not a rendering of it.
    pub id: Value,
    pub method: String,
    /// `Value::Null` when the message carried none, so a handler reads
    /// `params.get(..)` without a branch for the absent case.
    pub params: Value,
}

/// What one line off stdin turned out to be.
#[derive(Debug)]
pub enum Incoming {
    Request(Request),
    /// No `id`: answered with nothing, ever.
    Notification,
    /// Not a request this server can route, and the error owed for it.
    Invalid {
        /// `None` when the message carried no usable id, which is the case
        /// JSON-RPC answers with a null id.
        id: Option<Value>,
        code: i64,
        message: String,
    },
}

/// Classify one line.
///
/// Never fails and never panics: every input is one of the three variants. The
/// transport is a pipe from a process the user did not write, so a line that
/// makes this function panic would put a Rust backtrace on the wire and take the
/// session's tool with it.
pub fn parse(line: &str) -> Incoming {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Incoming::Invalid {
            id: None,
            code: code::PARSE_ERROR,
            message: "the line is not JSON".to_owned(),
        };
    };

    let Some(object) = value.as_object() else {
        return Incoming::Invalid {
            id: None,
            code: code::INVALID_REQUEST,
            // A batch arrives here too. The MCP revisions this server speaks do
            // not use JSON-RPC batching, and answering half a batch would be
            // worse than saying so.
            message: "a JSON-RPC message is an object".to_owned(),
        };
    };

    // Absence, not nullity: a message with no `id` member is a notification and
    // gets no response whatever else is wrong with it.
    let Some(id) = object.get("id") else {
        return Incoming::Notification;
    };
    if !(id.is_string() || id.is_number()) {
        return Incoming::Invalid {
            id: None,
            code: code::INVALID_REQUEST,
            message: "a request id is a string or a number".to_owned(),
        };
    }
    let id = id.clone();

    if object.get("jsonrpc").and_then(Value::as_str) != Some(VERSION) {
        return Incoming::Invalid {
            id: Some(id),
            code: code::INVALID_REQUEST,
            message: format!("a request carries \"jsonrpc\": {VERSION:?}"),
        };
    }

    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Incoming::Invalid {
            id: Some(id),
            code: code::INVALID_REQUEST,
            message: "a request carries a method name".to_owned(),
        };
    };

    Incoming::Request(Request {
        id,
        method: method.to_owned(),
        params: object.get("params").cloned().unwrap_or(Value::Null),
    })
}

/// A successful response to `id`.
pub fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": VERSION, "id": id, "result": result})
}

/// An error response. A message with no usable id is answered with a null one,
/// which is what JSON-RPC specifies for a parse error.
pub fn failure(id: Option<&Value>, code: i64, message: &str) -> Value {
    let mut envelope = Map::new();
    envelope.insert("jsonrpc".into(), Value::from(VERSION));
    envelope.insert("id".into(), id.cloned().unwrap_or(Value::Null));
    envelope.insert("error".into(), json!({"code": code, "message": message}));
    Value::Object(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three classifications, over the messages a real client sends.
    #[test]
    fn a_message_without_an_id_is_a_notification_and_one_with_an_id_is_a_request() {
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        assert!(matches!(parse(notification), Incoming::Notification));

        match parse(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#) {
            Incoming::Request(request) => {
                assert_eq!(request.id, Value::from(7));
                assert_eq!(request.method, "tools/list");
                assert_eq!(request.params, Value::Null);
            }
            other => panic!("expected a request, got {other:?}"),
        }

        // A string id round-trips as a string: a client that sent `"1"` and a
        // client that sent `1` must not both be answered with the same value.
        match parse(r#"{"jsonrpc":"2.0","id":"1","method":"ping"}"#) {
            Incoming::Request(request) => assert_eq!(request.id, Value::from("1")),
            other => panic!("expected a request, got {other:?}"),
        }
    }

    /// Every malformed envelope is a code, never a panic.
    #[test]
    fn a_malformed_line_is_an_error_code_and_never_a_panic() {
        let cases: &[(&str, i64)] = &[
            ("not json at all", code::PARSE_ERROR),
            ("", code::PARSE_ERROR),
            ("[1,2,3]", code::INVALID_REQUEST),
            (r#""a bare string""#, code::INVALID_REQUEST),
            // A wrong protocol version, a missing one, an id of the wrong type
            // and a missing method: four ways to be an object and not a request.
            (
                r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
                code::INVALID_REQUEST,
            ),
            (r#"{"id":1,"method":"ping"}"#, code::INVALID_REQUEST),
            (
                r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
                code::INVALID_REQUEST,
            ),
            (r#"{"jsonrpc":"2.0","id":1}"#, code::INVALID_REQUEST),
        ];
        for (line, expected) in cases {
            match parse(line) {
                Incoming::Invalid { code, .. } => assert_eq!(code, *expected, "{line}"),
                other => panic!("{line} classified as {other:?}"),
            }
        }
    }

    /// Both envelopes carry `jsonrpc` and an `id`, and neither carries both a
    /// result and an error.
    #[test]
    fn the_response_envelopes_are_the_shapes_a_client_reads() {
        let ok = result(&Value::from(3), json!({"tools": []}));
        assert_eq!(ok["jsonrpc"], Value::from(VERSION));
        assert_eq!(ok["id"], Value::from(3));
        assert!(ok.get("error").is_none());

        let bad = failure(None, code::PARSE_ERROR, "the line is not JSON");
        assert_eq!(bad["id"], Value::Null);
        assert_eq!(bad["error"]["code"], Value::from(code::PARSE_ERROR));
        assert!(bad.get("result").is_none());
    }
}
