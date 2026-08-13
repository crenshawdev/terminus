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

use serde_json::{json, Value};

use verbatim_core::recall;

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
