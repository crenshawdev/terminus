//! RCL-09: the whole verbatim record behind a turn id.
//!
//! This is the read that answers "what exactly was said". A search returns an
//! excerpt - a projection, windowed around the match - and a context window
//! returns projections of its neighbours; this returns the record's **own
//! bytes**, read at the `stream_offset` and `byte_len` stored on the turn row.
//! The blob is truth (`.planning/PROJECT.md`, D-13) and this is the one path
//! that hands the truth back unaltered - unless `[privacy] redact_recall` says
//! otherwise, and then only for this answer: see [`Record::body`].
//!
//! **One blob per session per request (D-20).** rusqlite's incremental blob I/O
//! sits behind a `blob` feature this workspace does not enable, so reading one
//! turn materializes that session's whole compressed blob - up to 10.1 MB
//! uncompressed (phase 1 D-05). Ids are therefore grouped by `session_key` and
//! each session's blob is selected once, which bounds the cost per request
//! without eliminating it. The block counter at [`crate::blob::BlobReader`]
//! would report one block decompressed either way, so the number that says this
//! worked is [`Fetched::reads`] and not that one.
//!
//! **An evicted body is read off the column, never off a failed blob read
//! (D-08).** `session_meta.is_evicted` stays null until retention lands in
//! phase 8, and it is the only thing that produces [`Record::body_evicted`].
//! A blob that will not decompress is what `terminus verify` exists to report;
//! letting it look like an eviction would report silent data loss as retention
//! working correctly.

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::blob::BlobReader;
use crate::config::{visible, Config};
use crate::error::Result;
use crate::observe::egress::Redaction;
use crate::recall::excerpt::Reads;
use crate::recall::scope::{self, Reason, Scope};

/// One requested turn, as the archive holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub turn_id: i64,
    pub session_key: String,
    pub turn_seq: i64,
    pub record_type: String,
    pub tool_name: Option<String>,
    pub ts: Option<String>,
    /// The project key ingest resolved for this turn's session, or `None` when
    /// the session has no meta row at all.
    pub project: Option<String>,
    /// The record's own line, byte for byte as it was archived.
    ///
    /// `Vec<u8>` and not `String` on purpose. A transcript line is JSON and JSON
    /// is UTF-8 by specification, but the archive is verbatim and a lossy
    /// conversion here would make this read the one place a byte could change
    /// between what was written and what is shown. The caller decides how to
    /// render it - a terminal takes the bytes, a JSON document takes a lossy
    /// string - and says so where it does.
    ///
    /// With `[privacy] redact_recall` set these bytes are the archived line put
    /// through the egress filter, so the byte-for-byte promise above is what the
    /// default answers and not what every answer does; the archive itself is
    /// never touched, and the same read with the knob absent still returns the
    /// stored line unchanged (phase 4 D-04).
    ///
    /// `None` when the body was not read: the session is evicted, or the blob
    /// would not give the range up.
    pub body: Option<Vec<u8>>,
    /// The session's body has been evicted by retention (D-08). No blob read
    /// was attempted for this record.
    pub body_evicted: bool,
}

/// A requested id that produced no record, and why.
///
/// A value rather than an error: asking for five ids and getting four records
/// and one reason is an answer, where a failed call would lose the four.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Absent {
    pub turn_id: i64,
    pub reason: Reason,
}

/// What one `get` request answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fetched {
    /// The records, in the order the ids were asked for.
    pub records: Vec<Record>,
    /// Ids that named no record this caller may see.
    pub absent: Vec<Absent>,
    /// Set when the whole request could return nothing, whatever the ids were -
    /// the scope itself is empty or excluded.
    pub reason: Option<Reason>,
    /// How many session blobs this request materialized.
    pub reads: Reads,
}

/// Read the verbatim record behind each id.
///
/// Scoped exactly the way a search and a context window are (RCL-10 auto-scopes
/// all three MCP tools, with `project: "*"` opting out). Ids are densely
/// enumerable - `session_no << 24 | turn_seq` - so a get that scoped nothing
/// would walk out of the caller's project one id at a time, which is the door
/// [`crate::recall::context::window`] closes for the same reason. An id outside
/// the scope is reported as [`Reason::NoSuchTurn`] rather than as a scope
/// violation, deliberately: out of the caller's scope the turn does not exist,
/// and a distinct reason would confirm that some other project holds that id.
///
/// A repeated id is answered once. The record is the same either way, and a
/// second copy costs the caller its own budget for nothing.
pub fn records(conn: &Connection, config: &Config, scope: &Scope, ids: &[i64]) -> Result<Fetched> {
    let scoped = scope::resolve(conn, config, scope)?;
    if let Some(reason) = scoped.reason() {
        return Ok(Fetched {
            reason: Some(reason.clone()),
            absent: ids
                .iter()
                .map(|turn_id| Absent {
                    turn_id: *turn_id,
                    reason: reason.clone(),
                })
                .collect(),
            ..Fetched::default()
        });
    }

    let mut wanted: Vec<i64> = Vec::with_capacity(ids.len());
    for id in ids {
        if !wanted.contains(id) {
            wanted.push(*id);
        }
    }
    if wanted.is_empty() {
        return Ok(Fetched::default());
    }

    let rows = rows_for(conn, &wanted)?;

    let mut fetched = Fetched::default();
    // `(record index, stream_offset, byte_len)` per session, so each blob is
    // opened once and every range wanted out of it is cut from the one copy.
    let mut by_session: BTreeMap<String, Vec<(usize, i64, i64)>> = BTreeMap::new();

    for id in wanted {
        let Some(row) = rows.get(&id) else {
            fetched.absent.push(Absent {
                turn_id: id,
                reason: Reason::NoSuchTurn { turn_id: id },
            });
            continue;
        };

        // Scope FIRST, then exclusion. Both arms refuse the id; only one of
        // them names a path. An excluded project is also outside the caller's
        // scope at every scope but `*`, so checking exclusion first answered
        // "<absolute path> is excluded by config" for an id the caller was
        // never entitled to hear about - confirming both that the id exists and
        // what the excluded project is called. Ids are densely enumerable
        // (`session_no << 24 | turn_seq`), so that reason mapped every excluded
        // project's name and live id ranges, one call at a time, with no
        // argument a client had to pass to reach it.
        if let Some(wanted_project) = scoped.project() {
            if row.project.as_deref() != Some(wanted_project) {
                fetched.absent.push(Absent {
                    turn_id: id,
                    reason: Reason::NoSuchTurn { turn_id: id },
                });
                continue;
            }
        }
        let hidden = |key: &Option<String>, excluded: &[String]| {
            key.as_deref()
                .is_some_and(|k| excluded.iter().any(|e| e == k))
        };
        if hidden(&row.project, scoped.excluded_projects())
            || hidden(&row.pre_worktree, scoped.excluded_pre_worktree())
        {
            fetched.absent.push(Absent {
                turn_id: id,
                reason: Reason::ProjectExcluded {
                    project: row
                        .project
                        .clone()
                        .or_else(|| row.pre_worktree.clone())
                        .unwrap_or_default(),
                },
            });
            continue;
        }

        let index = fetched.records.len();
        fetched.records.push(Record {
            turn_id: row.turn_id,
            session_key: row.session_key.clone(),
            turn_seq: row.turn_seq,
            record_type: row.record_type.clone(),
            tool_name: row.tool_name.clone(),
            ts: row.ts.clone(),
            project: row.project.clone(),
            body: None,
            body_evicted: row.evicted,
        });
        // The evicted arm reads no blob at all, which is the difference between
        // "this body is gone" and "this body would not decompress".
        if !row.evicted {
            by_session
                .entry(row.session_key.clone())
                .or_default()
                .push((index, row.stream_offset, row.byte_len));
        }
    }

    // Resolved once for the whole request, at the entry point that holds the
    // `Config` (phase 4 D-01). `recall_get`'s body is hook and MCP output like
    // every other projection, so the knob reaches it too - and the conversion it
    // needs happens HERE and not per renderer, because `terminus show` writes
    // these bytes with `write_all` while the MCP tool renders them lossily, and
    // a filter applied separately in each would let the two `body` values
    // diverge (D-04).
    let redaction = Redaction::of(config);
    for (session_key, ranges) in by_session {
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [&session_key],
                |r| r.get(0),
            )
            .ok();
        fetched.reads.blobs += 1;
        let Some(blob) = blob else { continue };
        let Ok(reader) = BlobReader::open(&blob) else {
            // Damage is `terminus verify`'s to report. Here it costs this record
            // its body and leaves every other record of the request intact.
            continue;
        };
        for (index, offset, len) in ranges {
            if let Ok(bytes) = reader.read_range(offset as u64, len as u64) {
                // The knob-off arm is the blob's own bytes with no conversion at
                // all, not a round trip that happens to be lossless: this is the
                // read whose whole claim is byte-exactness, and a `String` step
                // taken unconditionally would make that claim depend on the
                // corpus rather than on the code.
                fetched.records[index].body = Some(if redaction.is_on() {
                    redaction
                        .apply(&String::from_utf8_lossy(&bytes))
                        .into_owned()
                        .into_bytes()
                } else {
                    bytes
                });
            }
        }
    }

    Ok(fetched)
}

/// One turn row plus the session facts scoping and eviction are decided on.
struct Row {
    turn_id: i64,
    session_key: String,
    turn_seq: i64,
    record_type: String,
    tool_name: Option<String>,
    ts: Option<String>,
    stream_offset: i64,
    byte_len: i64,
    project: Option<String>,
    pre_worktree: Option<String>,
    evicted: bool,
}

/// Every requested id that names a turn, in one statement.
///
/// A LEFT join to `session_meta` for the same reason `search::run` uses one: a
/// session archived without a meta row is damage to see, not damage to hide.
fn rows_for(conn: &Connection, ids: &[i64]) -> Result<BTreeMap<i64, Row>> {
    // Named through the degraded-read helper: a read-only open of a store that
    // predates `project_pre_worktree` must not meet a raw `no such column`.
    let pre_worktree = match visible::pre_worktree_column(conn)? {
        "project_pre_worktree" => "m.project_pre_worktree",
        // The literal `NULL`, which takes no table qualifier.
        absent => absent,
    };
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = conn.prepare(&format!(
        "SELECT t.id, t.session_key, t.turn_seq, t.record_type, t.tool_name, t.ts,
                t.stream_offset, t.byte_len, m.project, {pre_worktree}, m.is_evicted
         FROM turns t LEFT JOIN session_meta m ON m.session_key = t.session_key
         WHERE t.id IN ({placeholders})"
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok(Row {
            turn_id: row.get(0)?,
            session_key: row.get(1)?,
            turn_seq: row.get(2)?,
            record_type: row.get(3)?,
            tool_name: row.get(4)?,
            ts: row.get(5)?,
            stream_offset: row.get(6)?,
            byte_len: row.get(7)?,
            project: row.get(8)?,
            pre_worktree: row.get(9)?,
            evicted: row.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0,
        })
    })?;

    let mut out = BTreeMap::new();
    for row in rows {
        let row = row?;
        out.insert(row.turn_id, row);
    }
    Ok(out)
}
