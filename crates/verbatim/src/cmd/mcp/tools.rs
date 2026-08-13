//! The three tools, and nothing else ever (RCL-07 .. RCL-09).
//!
//! **Three, by decision.** Every tool description sits in every session's
//! context forever (`.planning/PROJECT.md`), so `stats`, `usage`, `verify`,
//! `reindex` and `export` stay CLI-only: they are things a person runs, not
//! things a model needs a permanent line of context to know about.
//!
//! **Every description states what the tool returns.** None of them tells the
//! model when to call it or how to behave, because a description is not a place
//! to put a prompt: it is read by a client that never asked for one and is
//! charged to a user who cannot edit it.
//!
//! **`readOnlyHint` lives inside `annotations`**, beside `destructiveHint`,
//! `idempotentHint` and `openWorldHint` - measured against the Claude Code
//! 2.1.229 bundle rather than assumed, because a hint written at the top level
//! is a field nothing reads and an assertion on it tests nothing.
//!
//! **A tool result is a JSON document in one text block.** Not prose framing
//! around the answer, and not a second envelope: the model reads the same field
//! names `verbatim search --json` prints, so a turn described in a terminal and
//! a turn described here are the same object. The document is serialized
//! compactly, because a tool result sits in the model's context and indentation
//! is tokens spent on whitespace.
//!
//! **Every failure the server understood is a result, never a throw (RCL-10).**
//! A missing argument, an argument of the wrong JSON type, an unparseable date,
//! an unknown turn id, a store that does not exist yet: all of them come back as
//! this tool's own empty document carrying a `reason`. A JSON-RPC error is
//! reserved for a message that could not be read as a request at all, because a
//! throw is what the client surfaces to the user as a broken server.

use serde_json::{json, Map, Value};

use verbatim_core::recall::{self, search, Filters, Query, Request, Scope};
use verbatim_core::Error;

use crate::cmd::read::{self, Opened, Reader};
use crate::cmd::Failure;

use super::rpc;

pub const RECALL_SEARCH: &str = "recall_search";
pub const RECALL_CONTEXT: &str = "recall_context";
pub const RECALL_GET: &str = "recall_get";

/// The most hits `recall_search` returns, whatever the caller asks for.
///
/// The query layer's own ceiling, not a second one: a budget the server invented
/// would be a second definition of "too many" to keep in step with the library's
/// (`recall::MAX_RESULTS`).
pub const MAX_HITS: usize = recall::MAX_RESULTS;

/// The most turns `recall_context` returns on either side of the anchor.
pub const MAX_CONTEXT_SIDE: usize = recall::MAX_CONTEXT_SIDE;

/// The most ids one `recall_get` call may name.
///
/// This is the budget the terminal path does not need. `verbatim show` is
/// bounded by what a person types; `recall_get` is where a *client-supplied*
/// list arrives, and `recall::get::records` binds one SQL placeholder per id, so
/// an unbounded list is an operational failure rather than a bounded answer.
/// Each id can also cost a whole decompressed session (D-20), which is what puts
/// the number in the tens rather than the hundreds.
pub const MAX_IDS: usize = 25;

/// How many hits `recall_search` returns when the caller does not say.
///
/// Below [`MAX_HITS`] on purpose: this answer goes into a model's context, and
/// the caller that wants the rest asks for it.
pub const DEFAULT_HITS: usize = 10;

/// The `annotations` object every tool here carries.
///
/// All four hints, not only the one the acceptance criterion names: a client
/// that reads `destructiveHint` and finds it absent has to assume the default,
/// and the default for a tool that does not say is the cautious one.
fn read_only() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        // The archive is this machine's own store and nothing else: no network,
        // no provider, no open world.
        "openWorldHint": false,
    })
}

/// The three tool descriptors, exactly.
pub fn descriptors() -> Vec<Value> {
    vec![
        json!({
            "name": RECALL_SEARCH,
            "description": "Search archived Claude Code sessions for past turns \
                            matching a query, ranked, each with its turn id and an excerpt.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Words, an identifier, a path, an error line or a command."
                    },
                    "project": {
                        "type": "string",
                        "description": "A path inside one project, or \"*\" for every project. \
                                        Defaults to the project this session is in."
                    },
                    "paths": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Only turns that structurally touched one of these files."
                    },
                    "tool": {
                        "type": "string",
                        "description": "Only turns using this tool, such as Bash or Read."
                    },
                    "kind": {
                        "type": "string",
                        "description": "Only turns of this record type: user, assistant, system or attachment."
                    },
                    "since": {
                        "type": "string",
                        "description": "Inclusive lower bound, YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS.mmmZ."
                    },
                    "until": {
                        "type": "string",
                        "description": "Inclusive upper bound, in the same two forms as since."
                    },
                    "limit": {
                        "type": "integer",
                        "description": format!("How many hits to return, at most {MAX_HITS}."),
                        "minimum": 1,
                        "maximum": MAX_HITS,
                    },
                },
                "required": ["query"],
                "additionalProperties": false,
            },
            "annotations": read_only(),
        }),
        json!({
            "name": RECALL_CONTEXT,
            "description": "Return the turns immediately before and after one archived turn, \
                            in the order they happened, stopping at the session boundary.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "turn_id": {
                        "type": "integer",
                        "description": "A turn id from a recall_search hit."
                    },
                    "before": {
                        "type": "integer",
                        "description": format!("How many earlier turns to return, at most {MAX_CONTEXT_SIDE}."),
                        "minimum": 0,
                        "maximum": MAX_CONTEXT_SIDE,
                    },
                    "after": {
                        "type": "integer",
                        "description": format!("How many later turns to return, at most {MAX_CONTEXT_SIDE}."),
                        "minimum": 0,
                        "maximum": MAX_CONTEXT_SIDE,
                    },
                    "project": {
                        "type": "string",
                        "description": "A path inside one project, or \"*\" for every project. \
                                        Defaults to the project this session is in."
                    },
                },
                "required": ["turn_id"],
                "additionalProperties": false,
            },
            "annotations": read_only(),
        }),
        json!({
            "name": RECALL_GET,
            "description": "Return the full archived text of named turns, exactly as it was recorded.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "turn_ids": {
                        "type": "array",
                        "items": {"type": "integer"},
                        "description": format!("Turn ids from recall_search hits, at most {MAX_IDS} per call."),
                        "minItems": 1,
                        "maxItems": MAX_IDS,
                    },
                    "project": {
                        "type": "string",
                        "description": "A path inside one project, or \"*\" for every project. \
                                        Defaults to the project this session is in."
                    },
                },
                "required": ["turn_ids"],
                "additionalProperties": false,
            },
            "annotations": read_only(),
        }),
    ]
}

// ---------------------------------------------------------------------------
// Calling one
// ---------------------------------------------------------------------------

/// Why a tool answered with nothing.
///
/// Two arms and not one, because a client behaves differently on each.
/// [`Refused::Caller`] is a request that could not be used as asked, and it
/// comes back with `isError` so the model retries with different arguments.
/// [`Refused::Archive`] is a question the archive simply had no answer to - no
/// store yet, a directory no archived project covers - and that is an ordinary
/// empty result, not something to retry.
enum Refused {
    Caller(String),
    Archive(String),
}

impl Refused {
    fn caller(message: impl Into<String>) -> Refused {
        Refused::Caller(message.into())
    }

    fn archive(message: impl Into<String>) -> Refused {
        Refused::Archive(message.into())
    }
}

/// Run one `tools/call`.
///
/// `Err` here is a **protocol** failure and nothing else: params that are not a
/// call, or a tool this server does not have. Everything a tool itself could not
/// do is inside the `Ok` as a result carrying a reason.
pub fn call(params: &Value) -> Result<Value, (i64, String)> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err((
            rpc::code::INVALID_PARAMS,
            "tools/call names the tool to call".to_owned(),
        ));
    };

    let outcome = match name {
        RECALL_SEARCH => arguments(params).and_then(|args| run_search(&args)),
        // Tasks 3 and 4 of this plan.
        RECALL_CONTEXT | RECALL_GET => Err(Refused::archive(format!(
            "{name} is listed by this build and not yet served"
        ))),
        other => {
            return Err((
                rpc::code::INVALID_PARAMS,
                format!("this server has no tool named {other:?}"),
            ))
        }
    };

    Ok(answer(name, outcome))
}

/// One tool result, as the client reads it.
///
/// `isError` is the tool-level failure signal MCP defines; a JSON-RPC error at
/// this level would be a protocol failure and the client would treat the server
/// as broken rather than the call as wrong.
fn answer(name: &str, outcome: Result<Value, Refused>) -> Value {
    let (document, is_error) = match outcome {
        Ok(document) => (document, false),
        Err(Refused::Archive(reason)) => (nothing(name, &reason), false),
        Err(Refused::Caller(reason)) => (nothing(name, &reason), true),
    };
    json!({
        "content": [{"type": "text", "text": document.to_string()}],
        "isError": is_error,
    })
}

/// This tool's own empty document, carrying why it is empty.
///
/// The same keys a successful answer has, so a client parses one shape either
/// way and never has to branch on whether the call worked before it can read the
/// result.
fn nothing(name: &str, reason: &str) -> Value {
    let mut document = match name {
        RECALL_CONTEXT => json!({
            "turns": [],
            "at_session_start": false,
            "at_session_end": false,
            "continues_from": Value::Null,
        }),
        RECALL_GET => json!({"records": [], "absent": []}),
        _ => json!({"hits": []}),
    };
    if let Some(object) = document.as_object_mut() {
        object.insert("reason".to_owned(), Value::from(reason));
    }
    document
}

// ---------------------------------------------------------------------------
// recall_search (RCL-07)
// ---------------------------------------------------------------------------

fn run_search(args: &Map<String, Value>) -> Result<Value, Refused> {
    let raw = required_string(args, "query")?;
    let project = optional_string(args, "project")?;
    let filters = Filters {
        tool: optional_string(args, "tool")?,
        kind: optional_string(args, "kind")?,
        paths: optional_strings(args, "paths")?,
        since: optional_time(args, "since")?,
        until: optional_time(args, "until")?,
    };
    let asked = optional_count(args, "limit")?.unwrap_or(DEFAULT_HITS);
    if asked == 0 {
        return Err(Refused::caller("limit 0 asks for no results"));
    }
    // Lowered, never raised: RCL-10's budget is the server's, and a caller's own
    // limit may only narrow it.
    let limit = asked.min(MAX_HITS);

    let query = Query::parse(&raw);
    let truncated = query.truncated();
    let tokens = query.tokens().len();

    let reader = reader()?;
    let request = Request::new(query, scope(project.as_deref())?)
        .filters(filters)
        .limit(limit);

    let response = match search::run(reader.store().conn(), reader.config(), &request) {
        Ok(response) => response,
        // A malformed bound is the caller's, not the archive's. Every string
        // orders against every other, so the alternative is a plausible wrong
        // answer nothing reports (D-23).
        Err(Error::InvalidTimeFilter { field, value }) => {
            return Err(Refused::caller(format!(
                "{field} {value:?} is not a time; expected YYYY-MM-DD or \
                 YYYY-MM-DDTHH:MM:SS.mmmZ"
            )))
        }
        Err(other) => return Err(Refused::archive(other.to_string())),
    };

    // The scope's reason first: it explains an empty answer, and a caller told
    // only that its limit was lowered would read the emptiness as "no matches".
    let mut reason = response.reason.as_ref().map(ToString::to_string);
    if reason.is_none() && asked > limit {
        reason = Some(format!(
            "the limit was lowered from {asked} to this server's budget of {MAX_HITS}"
        ));
    }
    if reason.is_none() && truncated {
        reason = Some(format!(
            "only the first {tokens} tokens of the query were used"
        ));
    }

    let hits: Vec<Value> = response
        .hits
        .iter()
        .map(|hit| {
            json!({
                "turn_id": hit.turn_id,
                "session_key": hit.session_key,
                "project": hit.project,
                "record_type": hit.record_type,
                "ts": hit.ts,
                // D-07: a subagent turn is in the result and ranked below a
                // top-level turn of equal score, and said out loud so a reader
                // knows it is looking at work it never watched.
                "sidechain": hit.sidechain,
                "relevance": hit.relevance,
                "entity_score": hit.entity_score,
                "excerpt": hit.excerpt,
            })
        })
        .collect();

    Ok(json!({"hits": hits, "reason": reason}))
}

// ---------------------------------------------------------------------------
// The store, the scope, and the arguments
// ---------------------------------------------------------------------------

/// Open the store for one call, read-only (D-10).
///
/// The same entry point `search`, `show` and `sessions` use, so there is one
/// implementation of "a read never creates a store" rather than two - and a
/// server advertising `readOnlyHint` that performed DDL on connect would be
/// exactly the contradiction that decision exists to prevent.
///
/// Per call rather than once at startup: a session ingested while this process
/// is alive is searchable by the next question, and a store that appears after
/// the client connected is found rather than reported missing for the rest of
/// the session.
fn reader() -> Result<Box<Reader>, Refused> {
    match read::open() {
        Ok(Opened::Ready(reader)) => Ok(reader),
        // Not an error. A machine that has installed verbatim and not yet
        // ingested is the ordinary starting state, and a model told "error"
        // would report a broken tool instead of an empty archive.
        Ok(Opened::Nothing(reason)) => Err(Refused::archive(reason)),
        Err(Failure::Operational(message) | Failure::Misuse(message)) => {
            Err(Refused::archive(message))
        }
        Err(Failure::Silent) => Err(Refused::archive("the store could not be read")),
    }
}

/// The project a call works in: the one named, or the one this process is
/// standing in (D-12).
///
/// Claude Code spawns the server with the session's own working directory, so
/// the default scope is the project the user is in - resolved by a longest-prefix
/// match against the keys `session_meta` already holds, never by a `git`
/// subprocess that costs 10-30 ms and can disagree with what ingest wrote.
fn scope(named: Option<&str>) -> Result<Scope, Refused> {
    read::scope(named).map_err(|_| {
        Refused::archive(
            "this server's working directory could not be read, so it cannot tell \
             which project it is in; pass project explicitly",
        )
    })
}

/// The `arguments` member of a `tools/call`.
///
/// Absent is an empty object rather than a refusal: a tool called with no
/// arguments should be told which one it needed, by name, and not that
/// "arguments" was missing.
fn arguments(params: &Value) -> Result<Map<String, Value>, Refused> {
    match params.get("arguments") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(other) => Err(Refused::caller(format!(
            "arguments must be an object, not {}",
            kind(other)
        ))),
    }
}

/// What a JSON value is, for a message a model reads.
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn required_string(args: &Map<String, Value>, name: &str) -> Result<String, Refused> {
    match args.get(name) {
        None | Some(Value::Null) => Err(Refused::caller(format!("{name} is required"))),
        Some(Value::String(text)) if text.trim().is_empty() => {
            Err(Refused::caller(format!("{name} is empty")))
        }
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) => Err(Refused::caller(format!(
            "{name} must be a string, not {}",
            kind(other)
        ))),
    }
}

fn optional_string(args: &Map<String, Value>, name: &str) -> Result<Option<String>, Refused> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(Refused::caller(format!(
            "{name} must be a string, not {}",
            kind(other)
        ))),
    }
}

fn optional_strings(args: &Map<String, Value>, name: &str) -> Result<Vec<String>, Refused> {
    let Some(value) = args.get(name).filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(Refused::caller(format!(
            "{name} must be an array of strings, not {}",
            kind(value)
        )));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item.as_str() {
            Some(text) => out.push(text.to_owned()),
            None => {
                return Err(Refused::caller(format!(
                    "{name} holds strings; {} is not one",
                    kind(item)
                )))
            }
        }
    }
    Ok(out)
}

/// A count argument: a whole number that is not negative, or nothing.
///
/// `as_u64` and not a cast: `3.5`, `-1` and `1e30` all fail it, which is the
/// difference between refusing a wrong-typed argument and silently truncating
/// one into a plausible number.
fn optional_count(args: &Map<String, Value>, name: &str) -> Result<Option<usize>, Refused> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_u64().and_then(|n| usize::try_from(n).ok()) {
            Some(count) => Ok(Some(count)),
            None => Err(Refused::caller(format!(
                "{name} must be a whole number that is not negative"
            ))),
        },
    }
}

/// One end of a time window, validated before it can reach a comparison.
///
/// Through the CLI's own rule, so `--since` typed at a terminal and `since`
/// passed by a model mean the same thing (D-23) - including that a bare
/// `YYYY-MM-DD` extends to the first instant of its day for `since` and the last
/// for `until`.
fn optional_time(args: &Map<String, Value>, name: &'static str) -> Result<Option<String>, Refused> {
    let Some(raw) = optional_string(args, name)? else {
        return Ok(None);
    };
    match crate::cmd::time_bound(name, &raw) {
        Ok(bound) => Ok(Some(bound)),
        Err(Failure::Misuse(message) | Failure::Operational(message)) => {
            Err(Refused::caller(message))
        }
        Err(Failure::Silent) => Err(Refused::caller(format!("{name} is not a time"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exactly three, each named, each `readOnlyHint`, each with an object
    /// schema - the shape AC7 asserts at the process boundary, checked here
    /// where a failure names the field rather than the spawn.
    #[test]
    fn there_are_exactly_three_read_only_tools() {
        let tools = descriptors();
        assert_eq!(tools.len(), 3);

        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, [RECALL_SEARCH, RECALL_CONTEXT, RECALL_GET]);

        for tool in &tools {
            assert_eq!(
                tool["annotations"]["readOnlyHint"],
                Value::from(true),
                "{tool}"
            );
            assert_eq!(tool["inputSchema"]["type"], Value::from("object"), "{tool}");
            let description = tool["description"].as_str().unwrap_or_default();
            assert!(!description.is_empty(), "{tool}");
            assert!(
                !description.contains('\n'),
                "a description is one line: {description:?}"
            );
        }
    }
}
