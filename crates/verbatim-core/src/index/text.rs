//! The text a turn contributes to `turns_fts.body` (D-01).
//!
//! A per-record-type projection of what the turn actually *said*, never the raw
//! transcript line. Phase 1 indexed `String::from_utf8_lossy(record)`, the whole
//! line, which put every JSON key and every record-type literal in the index, so
//! `MATCH 'assistant'` returned every assistant record on the strength of
//! `"type":"assistant"`, and BM25 ranked turns by how much scaffolding they
//! carried. Phase 5's structural rank-1..3 threshold reads those ranks, so the
//! noise would not have stayed in search.
//!
//! Four subtrees are read and nothing else (D-01, D-02):
//!
//! - `message.content` - a plain string, or an array whose `text` blocks give
//!   their text, whose `tool_use` blocks give the tool name and the string
//!   leaves of `input`, and whose `tool_result` blocks give their content.
//! - the top-level `content`, when it is a string: the `system` record type
//!   carries its text there rather than under `message`.
//! - the top-level `toolUseResult` - `stdout`, `stderr` and whatever else the
//!   tool put in it - which is 41% of the corpus by bytes and the half of a
//!   tool call that says what happened.
//! - the top-level `attachment` object's string leaves.
//!
//! Keys are never emitted, which is the whole of "a search for `toolUseResult`
//! returns nothing". Neither are numbers or booleans: an attachment's byte count
//! is not a word anybody searches for, and every digit run in the index costs
//! FTS bytes.

use serde_json::Value;

/// How deep the projection follows a nested value before it stops.
///
/// A bound rather than a measurement: `input` and `toolUseResult` are
/// tool-authored JSON of no fixed shape, so the depth is upstream's to choose
/// and a recursive walk with no floor is a stack overflow one malformed tool
/// away. Eight clears every shape measured (`toolUseResult.structuredPatch` is
/// the deepest at three).
pub const MAX_DEPTH: usize = 8;

/// How many bytes of projected text one turn may contribute.
///
/// One FTS row per turn is the schema, so an unbounded projection makes one row
/// as large as its record - and phase 1 measured records up to 205 KB, with a
/// whole session reaching 10.1 MB. The blob still holds every byte; this bounds
/// only what is *searchable*, and a turn that needs 256 KB of matching text to
/// be found is not a turn recall was going to find.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// The searchable text of one turn record.
///
/// The input is the record's parsed JSON. A caller with only bytes parses once
/// and passes the value in: `derive_turn` needs the same value for entity
/// extraction, and `parse::Record` deliberately keeps no parsed value, because a
/// `Scan` holds every record of a session and one `Value` each would hold the
/// session's whole JSON tree in memory.
pub fn project(record: &Value) -> String {
    let mut out = Projector::default();
    if let Some(message) = record.get("message") {
        out.message(message);
    }
    if let Some(content) = record.get("content").and_then(Value::as_str) {
        out.push(content);
    }
    if let Some(result) = record.get("toolUseResult") {
        out.leaves(result, 0);
    }
    if let Some(attachment) = record.get("attachment") {
        out.leaves(attachment, 0);
    }
    out.text
}

/// Accumulates projected text under [`MAX_BODY_BYTES`].
struct Projector {
    text: String,
    remaining: usize,
}

impl Default for Projector {
    fn default() -> Projector {
        Projector {
            text: String::new(),
            remaining: MAX_BODY_BYTES,
        }
    }
}

impl Projector {
    /// One string leaf, newline-separated from what came before.
    ///
    /// The separator matters: `unicode61` splits on it, so two leaves that
    /// happened to be `"cargo"` and `"build"` cannot run together into a token
    /// neither one is.
    fn push(&mut self, value: &str) {
        if value.is_empty() || self.remaining == 0 {
            return;
        }
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        let take = floor_char_boundary(value, self.remaining.min(value.len()));
        self.text.push_str(&value[..take]);
        self.remaining -= take;
    }

    /// `message.content`, per D-01's block rules.
    fn message(&mut self, message: &Value) {
        match message.get("content") {
            Some(Value::String(text)) => self.push(text),
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    self.block(block);
                }
            }
            _ => {}
        }
    }

    /// One `message.content` block.
    ///
    /// A block whose `type` is none of the three contributes nothing. That is a
    /// deliberate allowlist and not an oversight: the block vocabulary is
    /// upstream's and grows, and a fallback that projected any unknown block's
    /// string leaves would be exactly the "index whatever is there" rule this
    /// module exists to replace.
    fn block(&mut self, block: &Value) {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    self.push(text);
                }
            }
            Some("tool_use") => {
                // The name is a word a user searches with ("what did Grep
                // find"), and `turns.tool_name` holds it too - one for the
                // filter, one for the free-text query.
                if let Some(name) = block.get("name").and_then(Value::as_str) {
                    self.push(name);
                }
                if let Some(input) = block.get("input") {
                    self.leaves(input, 0);
                }
            }
            Some("tool_result") => match block.get("content") {
                Some(Value::String(text)) => self.push(text),
                Some(Value::Array(parts)) => {
                    for part in parts {
                        if part.get("type").and_then(Value::as_str) == Some("text") {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                self.push(text);
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Every string leaf of an arbitrary value, keys excluded.
    fn leaves(&mut self, value: &Value, depth: usize) {
        if depth >= MAX_DEPTH || self.remaining == 0 {
            return;
        }
        match value {
            Value::String(text) => self.push(text),
            Value::Array(items) => {
                for item in items {
                    self.leaves(item, depth + 1);
                }
            }
            Value::Object(map) => {
                for (_, item) in map {
                    self.leaves(item, depth + 1);
                }
            }
            _ => {}
        }
    }
}

/// The largest index at or below `at` that is a UTF-8 character boundary.
///
/// `str::floor_char_boundary` is unstable, and slicing a multi-byte character
/// in half panics - so a turn whose text runs past the budget mid-emoji would
/// take the ingest of the whole session with it.
pub(super) fn floor_char_boundary(text: &str, at: usize) -> usize {
    if at >= text.len() {
        return text.len();
    }
    let mut at = at;
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}
