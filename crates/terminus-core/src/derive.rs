//! The one seam between a turn and everything derived from it.
//!
//! Ingest and rebuild both call [`derive_turn`] and neither writes a derived
//! row any other way, so a rebuild cannot drift from what ingest wrote. That is
//! the whole point of the module: STOR-04 says the blobs alone reproduce the
//! derived tables, and the cheapest way to keep that true is to give both paths
//! one function.
//!
//! What the function writes: one `turns_fts` row per turn at rowid `turns.id`,
//! carrying [`crate::index::project`]'s text projection of the record rather
//! than the record's raw line (D-01), and one `compaction_boundaries` row for
//! the rare turn that is a boundary (D-21).
//!
//! The record is parsed to a `serde_json::Value` **here**, once, and handed to
//! every rule that needs it. `parse::Record` deliberately does not keep the
//! parsed value: a `Scan` holds every record of a session, and one `Value` each
//! would hold the session's whole JSON tree in memory for the length of an
//! ingest pass.
//!
//! Every write here is a delete followed by an insert at a **known** rowid,
//! which is what `contentless_delete=1` exists for (D-10): re-deriving a turn
//! is idempotent by construction rather than by a uniqueness check.

use rusqlite::Connection;

use crate::error::Result;
use crate::parse::Turn;
use crate::store::schema;

/// One turn, and where its bytes live in the session's uncompressed stream.
#[derive(Debug, Clone, Copy)]
pub struct TurnRow<'a> {
    pub session_key: &'a str,
    /// The session's stable surrogate. `turns.id` is derived from it and
    /// `turn_seq` (D-10), and `sessions` is never dropped, so a rebuild
    /// reproduces the same ids.
    pub session_no: i64,
    pub turn: &'a Turn,
    /// Uncompressed-stream coordinates (D-04), excluding the record's newline.
    pub stream_offset: u64,
    pub byte_len: u64,
    /// The record's own bytes, exactly as they sit in the blob.
    pub record: &'a [u8],
    /// The record's `subtype`, from [`crate::parse::Record`] (D-21).
    ///
    /// Carried in rather than re-read out of [`TurnRow::record`] here: the
    /// parser is the one place a line's fields are extracted, and a second parse
    /// site inside the seam would be a second place for ingest and rebuild to
    /// disagree about what the same bytes mean.
    pub subtype: Option<&'a str>,
    /// The record's `compactMetadata` bytes, verbatim (D-08).
    pub compact_metadata: Option<&'a [u8]>,
}

impl TurnRow<'_> {
    /// D-21: is this turn the record a compaction left behind?
    fn is_compact_boundary(&self) -> bool {
        self.subtype == Some(crate::parse::COMPACT_BOUNDARY)
    }
}

/// Write a turn and everything derived from it, returning `turns.id`.
///
/// Runs inside the caller's transaction and never opens one of its own: ingest
/// commits the blob, the turn rows and the watermark together (STOR-02), and a
/// nested transaction here would be a second commit point inside that.
pub fn derive_turn(tx: &Connection, row: TurnRow<'_>) -> Result<i64> {
    let id = schema::turn_id(row.session_no, row.turn.turn_seq);

    // `is_typed` is named in BOTH halves, like every other column: a re-derive
    // of a turn whose classification changed - a rule this build extended, a
    // record the previous one read differently - would otherwise keep the old
    // value while everything around it moved (INJ-07, phase 1 D-03).
    tx.execute(
        "INSERT INTO turns (
            id, session_key, turn_seq, uuid, parent_uuid, record_type, tool_name, ts,
            stream_offset, byte_len, is_typed
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
            uuid = excluded.uuid, parent_uuid = excluded.parent_uuid,
            record_type = excluded.record_type, tool_name = excluded.tool_name,
            ts = excluded.ts, stream_offset = excluded.stream_offset,
            byte_len = excluded.byte_len, is_typed = excluded.is_typed",
        rusqlite::params![
            id,
            row.session_key,
            row.turn.turn_seq,
            row.turn.uuid,
            row.turn.parent_uuid,
            row.turn.record_type,
            row.turn.tool_name,
            row.turn.timestamp,
            row.stream_offset as i64,
            row.byte_len as i64,
            // Off the `Turn` the parser produced, never re-read out of
            // `row.record` here - the rule `TurnRow::subtype` states above, and
            // for the same reason: a second parse site inside the seam is a
            // second place for ingest and rebuild to disagree.
            row.turn.is_typed,
        ],
    )?;

    // One parse, for every rule that reads the record's structure. A record
    // that will not parse projects to empty text and never to the raw line:
    // that path is unreachable for a turn - D-03 classifies a turn only when
    // `uuid` and `timestamp` parsed out of it - and exists so no fallback can
    // put JSON scaffolding back in the index (D-01).
    let value: Option<serde_json::Value> = serde_json::from_slice(row.record).ok();
    let body = value
        .as_ref()
        .map(crate::index::project)
        .unwrap_or_default();

    // Delete before insert, at the rowid the turn id fixes. On a fresh insert
    // the delete is a no-op; on a re-derive it is what keeps the row count
    // stable instead of doubling the index.
    tx.execute("DELETE FROM turns_fts WHERE rowid = ?1", [id])?;
    tx.execute(
        "INSERT INTO turns_fts (rowid, body) VALUES (?1, ?2)",
        rusqlite::params![id, body],
    )?;

    // Cleared before anything is written, at a known turn id, which is what
    // makes a re-derive idempotent by construction rather than by a uniqueness
    // check the tables do not carry. A value that stopped being extracted must
    // stop having a row.
    tx.execute("DELETE FROM entities WHERE turn_id = ?1", [id])?;
    tx.execute("DELETE FROM paths WHERE turn_id = ?1", [id])?;
    for entity in value
        .as_ref()
        .map(crate::index::entities)
        .unwrap_or_default()
    {
        tx.execute(
            "INSERT INTO entities (turn_id, kind, value_norm) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, entity.kind, entity.value],
        )?;
        // `paths` exists for exactly this lookup - "which turns touched this
        // file" - so a path entity without its row would leave the table
        // describing a subset of what the index knows.
        if entity.kind == crate::index::entity::PATH {
            tx.execute(
                "INSERT INTO paths (turn_id, path) VALUES (?1, ?2)",
                rusqlite::params![id, entity.value],
            )?;
        }
    }

    // The boundary row, on the same delete-then-insert footing as everything
    // above (D-21). Clearing first is what makes a re-derive idempotent by
    // construction rather than by an upsert that has to guess: a turn that is
    // no longer a boundary must not keep a row saying it is.
    //
    // The metadata goes in as the bytes the parser lifted out of the line, and
    // nothing here reads them (D-08). No dropped-turn set is computed: the
    // complement of `preservedMessages.uuids` is derived at query time in phase
    // 5 (INJ-05), where a wrong reading of an upstream format that has exactly
    // one real example costs a query and not a reingest.
    tx.execute("DELETE FROM compaction_boundaries WHERE turn_id = ?1", [id])?;
    if row.is_compact_boundary() {
        tx.execute(
            "INSERT INTO compaction_boundaries (turn_id, metadata) VALUES (?1, ?2)",
            rusqlite::params![id, row.compact_metadata],
        )?;
    }

    Ok(id)
}
