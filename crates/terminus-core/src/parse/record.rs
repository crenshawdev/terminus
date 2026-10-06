//! One transcript line, classified.
//!
//! The classification rule is D-03's and nothing more: a record is a turn when
//! it carries **both** `uuid` and `timestamp` **and** its type is one of `user`,
//! `assistant`, `attachment`, `system`. The other eleven types produce no turn,
//! no FTS row and no entity row, while remaining part of the byte stream the
//! blob holds verbatim (D-13).
//!
//! Nothing here rejects a line. A line that is not JSON at all, or is JSON
//! without turn identity, becomes a record with no turn: `journal.jsonl` files
//! under `subagents/workflows/wf_*/` hold `{agentId, key, result, type}` and
//! phase 2's recursive discovery reaches them (D-12).

use serde_json::Value;

/// The four record types that can become a turn (D-03).
pub const TURN_TYPES: [&str; 4] = ["user", "assistant", "attachment", "system"];

/// The `subtype` a compaction boundary carries (D-21).
///
/// The record itself is a `type: "system"` record, which [`TURN_TYPES`] already
/// covers, so this names a *subtype* and adds no record class.
pub const COMPACT_BOUNDARY: &str = "compact_boundary";

/// The field whose bytes are kept verbatim (D-08).
const COMPACT_METADATA_FIELD: &str = "compactMetadata";

/// The turn-level fields, extracted only for records that qualify as turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// Position among the turns of this transcript, in **byte order** (D-02).
    ///
    /// Never derived from `timestamp`: 31 of 62 sampled real transcripts carry
    /// at least one record whose timestamp goes backwards, and a rebuild that
    /// sorted on timestamp would reproduce the same wrong order on both sides,
    /// so AC4 would still pass while every turn's neighbours were scrambled.
    pub turn_seq: i64,
    pub uuid: String,
    /// Message-level threading within one conversation (D-11). It is not the
    /// file-level lineage signal; [`Record::foreign_session_id`] is.
    pub parent_uuid: Option<String>,
    pub record_type: String,
    /// The first `tool_use` name in `message.content`, when there is one.
    pub tool_name: Option<String>,
    /// Did the PERSON type this turn, or did the harness write it (INJ-07)?
    ///
    /// `Some(true)` for a prompt someone typed, `Some(false)` for a tool
    /// result, an `isMeta` caveat or a harness envelope, and `None` for every
    /// record type that is not `user` - the question is only asked of the one
    /// type whose records have two authors (phase 1 D-02, D-04).
    pub is_typed: Option<bool>,
    pub timestamp: String,
}

/// One line of a transcript, with its coordinates in the uncompressed stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Byte offset of the record's first byte in the transcript stream.
    ///
    /// Uncompressed-stream coordinates (D-04): the blob header translates them
    /// to blocks, so a future block-size change rewrites no turn row.
    pub offset: u64,
    /// The record's bytes, **excluding** the terminating newline.
    pub len: u64,
    /// The `type` field, for any record that is a JSON object carrying one.
    pub record_type: Option<String>,
    /// `sessionId`: the session this file's records report as their own.
    pub session_id: Option<String>,
    /// `session_id` (snake_case): a *different* session this file continues
    /// from, and D-11's file-level lineage signal. 187 of 1,221 real top-level
    /// transcripts carry one.
    pub foreign_session_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    /// The `subtype` field, for the records that carry one.
    ///
    /// The only value this phase acts on is [`COMPACT_BOUNDARY`] (D-21). It is
    /// carried for every record that has one rather than tested here, because
    /// the parser's job is to say what the line holds, not what a caller cares
    /// about.
    pub subtype: Option<String>,
    /// The bytes of the record's `compactMetadata` object, exactly as they sit
    /// in the line - never a re-serialization (D-08).
    ///
    /// The upstream semantics of `preservedMessages.uuids` versus `allUuids`
    /// versus `preservedSegment` are unsettled: exactly one `compact_boundary`
    /// record exists in 300,556 measured records, and its own token counts
    /// (`preTokens` 45,500, `cumulativeDroppedTokens` 38,064 against 6 preserved
    /// uuids of 8) contradict `DESIGN-BRIEF.md:140`'s reading of it. Keeping the
    /// bytes is what makes a wrong reading fixable in phase 5 (INJ-05) without a
    /// reingest, so nothing here interprets them and nothing computes a
    /// dropped-turn set.
    pub compact_metadata: Option<Vec<u8>>,
    /// `Some` exactly when D-03 classifies this record as a turn.
    pub turn: Option<Turn>,
}

impl Record {
    pub fn is_turn(&self) -> bool {
        self.turn.is_some()
    }

    /// Is this the record a compaction wrote where the dropped turns used to be
    /// (D-21)?
    ///
    /// Not a new record class and not a classification rule: `system` is already
    /// in [`TURN_TYPES`] and the real boundary record carries both `uuid` and
    /// `timestamp`, so D-03 has already made it a turn. This only names the
    /// subtype, so the one caller that writes a derived boundary row does not
    /// spell the literal itself.
    pub fn is_compact_boundary(&self) -> bool {
        self.subtype.as_deref() == Some(COMPACT_BOUNDARY)
    }

    /// Classify one line. `offset` is where it starts in the stream and
    /// `turn_seq` is the ordinal it takes if it turns out to be a turn.
    pub(crate) fn parse(line: &[u8], offset: u64, turn_seq: i64) -> Record {
        let mut record = Record {
            offset,
            len: line.len() as u64,
            record_type: None,
            session_id: None,
            foreign_session_id: None,
            cwd: None,
            git_branch: None,
            subtype: None,
            compact_metadata: None,
            turn: None,
        };

        // A line that is not JSON, or is JSON but not an object, is kept as a
        // record with no fields rather than dropped: its bytes are in the blob
        // either way, and a scan that aborted here would stop ingesting a
        // transcript because of one malformed line.
        let value: Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => return record,
        };
        let Some(object) = value.as_object() else {
            return record;
        };

        record.record_type = string(object.get("type"));
        record.session_id = string(object.get("sessionId"));
        record.foreign_session_id = string(object.get("session_id"));
        record.cwd = string(object.get("cwd"));
        record.git_branch = string(object.get("gitBranch"));
        record.subtype = string(object.get("subtype"));
        // Asked of the parsed object first, so a line that merely mentions the
        // name in some string never sends the scanner looking; the scanner then
        // takes the bytes out of the line rather than re-serializing the parsed
        // value, because a round trip through `serde_json` renormalizes
        // whitespace, escapes and number formatting and would make "verbatim"
        // mean "equivalent" (D-08).
        if object.contains_key(COMPACT_METADATA_FIELD) {
            record.compact_metadata = raw_field(line, COMPACT_METADATA_FIELD).map(<[u8]>::to_vec);
        }

        let uuid = string(object.get("uuid"));
        let timestamp = string(object.get("timestamp"));
        // Both fields, and the type: `ai-title` carries a uuid and no
        // timestamp, `mode` a timestamp and no uuid, so a check on either field
        // alone classifies state records as turns.
        if let (Some(kind), Some(uuid), Some(timestamp)) =
            (record.record_type.as_deref(), uuid, timestamp)
        {
            if TURN_TYPES.contains(&kind) {
                record.turn = Some(Turn {
                    turn_seq,
                    uuid,
                    parent_uuid: string(object.get("parentUuid")),
                    record_type: kind.to_owned(),
                    tool_name: tool_name(&value),
                    is_typed: (kind == "user").then(|| is_typed(&value)),
                    timestamp,
                });
            }
        }

        record
    }
}

/// A JSON string field, or `None` for absent, null or non-string.
fn string(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::to_owned)
}

/// The bytes of one **top-level** field's value, exactly as they appear in
/// `line`.
///
/// Structural rather than a substring search: it walks the object's own
/// key/value pairs, so a `"compactMetadata"` appearing inside some nested
/// message content or inside a string value can never be mistaken for the field.
/// Only the first occurrence of a key is returned; `serde_json` keeps the last
/// of a duplicated key, and a line with duplicate top-level keys is malformed
/// enough that either answer is a guess.
///
/// `None` for anything this cannot answer exactly - a line that is not a JSON
/// object, a truncated value, a key written with an escape - and the caller
/// stores nothing rather than storing bytes it is not sure of.
fn raw_field<'a>(line: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let mut at = skip_ws(line, 0);
    if *line.get(at)? != b'{' {
        return None;
    }
    at = skip_ws(line, at + 1);
    if line.get(at) == Some(&b'}') {
        return None;
    }

    loop {
        let name_end = string_end(line, at)?;
        // The quoted content, undecoded: the keys this is ever asked for are
        // plain ASCII, and an escaped key simply does not match rather than
        // being decoded to something that might.
        let name = &line[at + 1..name_end - 1];

        at = skip_ws(line, name_end);
        if *line.get(at)? != b':' {
            return None;
        }
        at = skip_ws(line, at + 1);
        let end = value_end(line, at)?;
        if name == key.as_bytes() {
            return Some(&line[at..end]);
        }

        at = skip_ws(line, end);
        if line.get(at) != Some(&b',') {
            return None;
        }
        at = skip_ws(line, at + 1);
    }
}

/// The index just past a JSON string's closing quote.
fn string_end(line: &[u8], start: usize) -> Option<usize> {
    if *line.get(start)? != b'"' {
        return None;
    }
    let mut at = start + 1;
    while at < line.len() {
        match line[at] {
            // A backslash escapes whatever follows, including a quote and
            // including another backslash, so both bytes are consumed together.
            b'\\' => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

/// The index just past a JSON value that begins at `start`.
fn value_end(line: &[u8], start: usize) -> Option<usize> {
    match *line.get(start)? {
        b'"' => string_end(line, start),
        open @ (b'{' | b'[') => {
            let close = if open == b'{' { b'}' } else { b']' };
            let mut depth = 0usize;
            let mut at = start;
            while at < line.len() {
                match line[at] {
                    // Strings are skipped whole, so a brace inside one never
                    // moves the depth.
                    b'"' => {
                        at = string_end(line, at)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return (line[at] == close).then_some(at + 1);
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            None
        }
        // A number, `true`, `false` or `null`: it ends where the enclosing
        // object does, at a comma, or at whitespace.
        _ => {
            let mut at = start;
            while at < line.len()
                && !matches!(line[at], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
            {
                at += 1;
            }
            (at > start).then_some(at)
        }
    }
}

fn skip_ws(line: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < line.len() && matches!(line[at], b' ' | b'\t' | b'\n' | b'\r') {
        at += 1;
    }
    at
}

/// The tags a harness-written `user` record's text opens with (D-02).
///
/// Measured on 2026-08-23 over 400 top-level transcripts and 12,379 `user` turn
/// records: `command-message` leads 143 of them, `command-name` 119,
/// `local-command-caveat` 119 (every one of those also `isMeta`),
/// `task-notification` 64, `local-command-stdout` 41 and `bash-stdout` 13. The
/// vocabulary is Claude Code's own and can change under us, so it is one
/// `const` and a new upstream shape is one line.
///
/// `bash-input` is measured (13 records) and deliberately absent: it is the
/// command the person typed after `!`, which the harness only wrapped.
const HARNESS_ENVELOPE_TAGS: [&str; 6] = [
    "command-message",
    "command-name",
    "local-command-caveat",
    "task-notification",
    "local-command-stdout",
    "bash-stdout",
];

/// Did the person type this `user` record (D-02)?
///
/// Read off the record's OWN `message.content` and its own `isMeta`, never the
/// top-level `toolUseResult` (D-01, D-06): the block rule is a strict superset
/// of the key rule - over 400 sampled transcripts the key implies the block in
/// 12,234 of 12,234 cases and the block appears without the key in 282 more -
/// and the key's own shape is a dict on 6,265 records, a string on 522 and a
/// list on 35, so any test that treated it as an object would misclassify 8.2%
/// of them. Reading message content and nothing else is also what makes the
/// answer survive `[capture]` elision (D-07), which replaces `toolUseResult`
/// and `attachment` and never touches `message.content`, so a `lean` or
/// `minimal` ingest and a later blob-only rebuild agree.
///
/// `promptSource` is not consulted: it is present on 57 of 400 records and on
/// no tool-result record at all, so it cannot carry the distinction.
fn is_typed(value: &Value) -> bool {
    // Every command caveat the harness inserts, whatever its content shape.
    if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return false;
    }

    let text = match value.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(text)) => text.as_str(),
        Some(Value::Array(blocks)) => {
            // The tool-result test wins over the text one (D-05): a record
            // carrying both blocks is a tool result the harness wrote, and the
            // two real examples of it are fork boilerplate in `agent-*.jsonl`.
            if blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            {
                return false;
            }
            match blocks
                .iter()
                .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|block| block.get("text"))
                .and_then(Value::as_str)
            {
                Some(text) => text,
                None => return true,
            }
        }
        _ => return true,
    };

    !opens_with_envelope_tag(text)
}

/// Does this text OPEN with one of [`HARNESS_ENVELOPE_TAGS`]?
///
/// The leading tag and never "the text contains a tag": roughly 200
/// person-typed prompts in the same 400-transcript sample carry `<objective>`,
/// `<execution_context>` and `<process>` tags inside them, and every one of
/// them is a prompt someone typed. The tag closes immediately - measured, all
/// 345 leading tags in a 300-transcript sample are `<name>` with no attribute -
/// so a `<command-message-of-my-own>` a person wrote does not match.
fn opens_with_envelope_tag(text: &str) -> bool {
    let Some(rest) = text.trim_start().strip_prefix('<') else {
        return false;
    };
    HARNESS_ENVELOPE_TAGS.iter().any(|tag| {
        rest.strip_prefix(tag)
            .is_some_and(|after| after.starts_with('>'))
    })
}

/// The first `tool_use` block's name in `message.content`.
///
/// Everything else a tool record carries is phase 3's (RCL-02 extracts
/// entities); phase 1 stores only the name, because `turns.tool_name` exists
/// and filling it from the blob is exactly as cheap here as it is in a rebuild.
fn tool_name(value: &Value) -> Option<String> {
    value
        .get("message")?
        .get("content")?
        .as_array()?
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .and_then(|block| block.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
