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
    /// `Some` exactly when D-03 classifies this record as a turn.
    pub turn: Option<Turn>,
}

impl Record {
    pub fn is_turn(&self) -> bool {
        self.turn.is_some()
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
