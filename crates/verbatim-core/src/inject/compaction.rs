//! The turns that fell out of the model's context (INJ-05, D-07).
//!
//! A compaction rewrites what the session is carrying and appends one
//! `compact_boundary` record saying which messages survived. Ingest stored that
//! record's `compactMetadata` object verbatim in `compaction_boundaries.metadata`
//! and derived nothing from it (phase 2 D-08), deliberately: the format has
//! exactly one real example across 300,556 measured records, and a wrong
//! reading derived at ingest would cost a reingest to correct while a wrong
//! reading derived here costs a query.
//!
//! **The dropped set is the complement of `preservedMessages.uuids`**, over the
//! session's own turns before the boundary. Three other readings are available
//! in the same object and all three are wrong, measured on 2026-08-20 against
//! the one real boundary in the corpus:
//!
//! - `allUuids` names messages that are not in the transcript at all - two of
//!   its eight resolve to no record - so its complement silently omits turns
//!   that really did fall out.
//! - `preservedSegment.anchorUuid` resolves to a record AFTER the boundary, so
//!   a segment read from it describes the compacted context rather than what
//!   preceded it.
//! - "everything before `headUuid`" assumes the preserved set is a suffix, and
//!   it is not: the preserved records sat at indices 31-35 and 41, while record
//!   43 preceded the boundary and was not preserved. That record's whole class
//!   of turn - recent, dropped, and the most likely thing the session was just
//!   working on - is exactly what INJ-05 exists to offer back.
//!
//! **Every failure is an empty set, never an error.** A session with no
//! boundary, a boundary whose metadata is null, bytes that are not JSON, JSON
//! that is not an object and an object with no `preservedMessages.uuids` all
//! read as "nothing is known to have been dropped", which is the same silence
//! the rest of injection answers with. The format is one undocumented example
//! wide; a reader that threw would turn a shape change upstream into a prompt
//! that fails.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension};

/// The turns of `session_key` that the most recent compaction dropped.
///
/// Empty for a session that has never been compacted, and empty for every
/// unreadable boundary - see the module doc. The ids are `turns.id`, so a
/// caller filters candidates against this set without a second query.
///
/// **The most recent boundary and not every boundary.** A session compacted
/// twice dropped the first boundary's turns long ago and the model has been
/// through a whole context since; what INJ-05 is about is what fell out just
/// now. Ordered by `turn_seq`, which is byte order within the transcript and
/// total, rather than by `ts` - 31 of 62 sampled real transcripts carry a
/// record whose timestamp runs backwards.
pub fn dropped(conn: &Connection, session_key: &str) -> BTreeSet<i64> {
    let Some((turn_seq, metadata)) = boundary(conn, session_key) else {
        return BTreeSet::new();
    };
    let preserved = preserved_uuids(metadata.as_deref());
    if preserved.is_empty() {
        // Not "everything before the boundary was dropped": an unreadable
        // metadata blob says nothing about what survived, and reading it as
        // "all of it" would offer the model back the turns it is still looking
        // at.
        return BTreeSet::new();
    }
    turns_before(conn, session_key, turn_seq)
        .into_iter()
        .filter(|(_, uuid)| match uuid {
            Some(uuid) => !preserved.contains(uuid),
            // A turn the transcript gave no uuid cannot be shown to have been
            // dropped, only to be absent from a list it could never appear in.
            // Injection is precision-first (`DESIGN-BRIEF.md:245`), so the
            // unprovable case is left out.
            None => false,
        })
        .map(|(id, _)| id)
        .collect()
}

/// `(turn_seq, metadata)` of the session's most recent compaction boundary.
///
/// The join is the boundary's own turn: `compaction_boundaries.turn_id` is a
/// `turns.id`, and `turns` is where the session and the ordinal live.
/// `UNIQUE (session_key, turn_seq)` covers the lookup.
fn boundary(conn: &Connection, session_key: &str) -> Option<(i64, Option<Vec<u8>>)> {
    conn.query_row(
        "SELECT t.turn_seq, b.metadata FROM compaction_boundaries b \
         JOIN turns t ON t.id = b.turn_id \
         WHERE t.session_key = ?1 \
         ORDER BY t.turn_seq DESC LIMIT 1",
        [session_key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .ok()
    .flatten()
}

/// The uuids `preservedMessages.uuids` lists, or nothing at all.
///
/// Read out of the stored bytes with `serde_json` rather than through a typed
/// struct: the surrounding object carries seven other fields whose shapes are
/// undocumented, and a `Deserialize` over the whole thing would fail the whole
/// read the day one of them changes type. Only this one path is asked for, and
/// anything that is not an array of strings at it is nothing.
fn preserved_uuids(metadata: Option<&[u8]>) -> BTreeSet<String> {
    let Some(bytes) = metadata else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return BTreeSet::new();
    };
    value
        .get("preservedMessages")
        .and_then(|preserved| preserved.get("uuids"))
        .and_then(serde_json::Value::as_array)
        .map(|uuids| {
            uuids
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `(id, uuid)` of every turn of this session before `turn_seq`.
///
/// Strictly before, which is what keeps the boundary record itself out of the
/// dropped set: it is a turn like any other (phase 2 D-21) and it is the one
/// record of the compaction the model definitely still has.
fn turns_before(conn: &Connection, session_key: &str, turn_seq: i64) -> Vec<(i64, Option<String>)> {
    let Ok(mut statement) = conn.prepare(
        "SELECT id, uuid FROM turns WHERE session_key = ?1 AND turn_seq < ?2 ORDER BY turn_seq",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map(rusqlite::params![session_key, turn_seq], |r| {
        Ok((r.get(0)?, r.get(1)?))
    }) else {
        return Vec::new();
    };
    rows.filter_map(std::result::Result::ok).collect()
}
