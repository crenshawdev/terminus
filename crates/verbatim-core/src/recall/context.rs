//! RCL-08: the turns around a hit, in the order they happened.
//!
//! **`turn_seq` order, never timestamp (D-22).** 31 of 62 sampled real
//! transcripts carry a record whose timestamp goes backwards, so a timestamp
//! sort would show a reply before the prompt it answers in half of all
//! sessions. `turns` carries `UNIQUE (session_key, turn_seq)`, which is the
//! covering index this window reads through.
//!
//! **The window stops at the session (D-06).** It does not follow
//! `continues_from` and it does not follow `parent_session_key`: continuation
//! is a fan-out with zero, one or many successors, 2 of 173 real files name a
//! predecessor that was never ingested, and `parent_session_key` is a different
//! namespace again. A lineage walk here could cross into a sibling fork and
//! return turns from a conversation that never happened. What the window does
//! instead is say which end it stopped at, and hand back the `continues_from`
//! link unfollowed, so a caller can decide to make a second call.

use rusqlite::{Connection, OptionalExtension};

use crate::blob::BlobReader;
use crate::config::Config;
use crate::error::Result;
use crate::recall::scope::{self, Reason, Scope};
use crate::recall::{excerpt, Query};

/// The most turns one call may ask for on either side of the anchor.
///
/// A window is read out of one decompressed session and every turn in it is
/// projected, so an unbounded `before` returns the whole session as prose - the
/// thing `recall_get` exists for, arrived at by accident.
pub const MAX_CONTEXT_SIDE: usize = 25;

/// One turn of a context window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextTurn {
    pub turn_id: i64,
    pub turn_seq: i64,
    pub record_type: String,
    pub tool_name: Option<String>,
    pub ts: Option<String>,
    /// The turn's own text, projected the way the index projected it. Empty
    /// when the archive would not give the bytes up.
    pub text: String,
    /// The turn the window was asked about.
    pub is_anchor: bool,
}

/// A chronological window around one turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Window {
    /// The turns, in `turn_seq` order, the anchor among them.
    pub turns: Vec<ContextTurn>,
    /// The window reaches the first turn of the session: there is nothing
    /// earlier to ask for, in this file.
    pub at_session_start: bool,
    /// The window reaches the last turn of the session.
    pub at_session_end: bool,
    /// The session this one continues from, when it names one - returned and
    /// NOT followed (D-06). A caller that wants the turns before the start of
    /// this file resolves this itself and issues a second call.
    pub continues_from: Option<String>,
    /// Set when the window is empty for a reason rather than by accident.
    pub reason: Option<Reason>,
}

impl Window {
    fn nothing(reason: Reason) -> Window {
        Window {
            reason: Some(reason),
            ..Window::default()
        }
    }
}

/// The turns around `anchor`, `before` of them earlier and `after` later.
///
/// Exclusion is applied on the same projection the search scopes through
/// (D-21): a turn of an excluded project returns an empty window with a reason,
/// because a context call that answered would be the read-path half of
/// exclusion leaking through a second door.
pub fn window(
    conn: &Connection,
    config: &Config,
    anchor: i64,
    before: usize,
    after: usize,
) -> Result<Window> {
    let scoped = scope::resolve(conn, config, &Scope::Everything)?;
    if let Some(reason) = scoped.reason() {
        return Ok(Window::nothing(reason.clone()));
    }

    let Some((session_key, seq)) = conn
        .query_row(
            "SELECT session_key, turn_seq FROM turns WHERE id = ?1",
            [anchor],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?
    else {
        return Ok(Window::nothing(Reason::NoSuchTurn { turn_id: anchor }));
    };

    let meta: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT project, project_pre_worktree, continues_from FROM session_meta
             WHERE session_key = ?1",
            [&session_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (project, pre_worktree, continues_from) = meta.unwrap_or((None, None, None));

    let hidden = |key: &Option<String>, excluded: &[String]| {
        key.as_deref()
            .is_some_and(|k| excluded.iter().any(|e| e == k))
    };
    if hidden(&project, scoped.excluded_projects())
        || hidden(&pre_worktree, scoped.excluded_pre_worktree())
    {
        return Ok(Window::nothing(Reason::ProjectExcluded {
            project: project.or(pre_worktree).unwrap_or_default(),
        }));
    }

    let low = seq - before.min(MAX_CONTEXT_SIDE) as i64;
    let high = seq + after.min(MAX_CONTEXT_SIDE) as i64;

    let mut statement = conn.prepare(
        "SELECT id, turn_seq, record_type, tool_name, ts, stream_offset, byte_len
         FROM turns
         WHERE session_key = ?1 AND turn_seq BETWEEN ?2 AND ?3
         ORDER BY turn_seq",
    )?;
    let rows = statement
        .query_map(rusqlite::params![&session_key, low, high], |row| {
            Ok((
                ContextTurn {
                    turn_id: row.get(0)?,
                    turn_seq: row.get(1)?,
                    record_type: row.get(2)?,
                    tool_name: row.get(3)?,
                    ts: row.get(4)?,
                    text: String::new(),
                    is_anchor: row.get::<_, i64>(0)? == anchor,
                },
                (row.get::<_, i64>(5)?, row.get::<_, i64>(6)?),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    // The session's own ends, which is what "cut short" is measured against.
    let (first, last): (i64, i64) = conn.query_row(
        "SELECT min(turn_seq), max(turn_seq) FROM turns WHERE session_key = ?1",
        [&session_key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let mut turns: Vec<ContextTurn> = rows.iter().map(|(turn, _)| turn.clone()).collect();
    fill_text(conn, &session_key, &rows, &mut turns)?;

    let at_session_start = turns.first().is_some_and(|turn| turn.turn_seq == first);
    let at_session_end = turns.last().is_some_and(|turn| turn.turn_seq == last);

    Ok(Window {
        turns,
        at_session_start,
        at_session_end,
        // Returned whether or not the window reached the start, because that is
        // a fact about the session rather than about this call.
        continues_from,
        reason: None,
    })
}

/// Project every turn of the window out of the one session blob.
///
/// One `SELECT blob` for the whole window, which is free here in a way it is
/// not for a search: a context window is one session by definition (D-06).
fn fill_text(
    conn: &Connection,
    session_key: &str,
    rows: &[(ContextTurn, (i64, i64))],
    turns: &mut [ContextTurn],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [session_key],
            |r| r.get(0),
        )
        .optional()?;
    let Some(blob) = blob else { return Ok(()) };
    let Ok(reader) = BlobReader::open(&blob) else {
        return Ok(());
    };

    // An empty query, so the projection is taken from its head rather than
    // windowed around a match: a context turn is being read for what it says,
    // not for why it matched.
    let query = Query::default();
    for (index, (_, (offset, len))) in rows.iter().enumerate() {
        let Ok(bytes) = reader.read_range(*offset as u64, *len as u64) else {
            continue;
        };
        turns[index].text = excerpt::of_record(&query, &bytes);
    }
    Ok(())
}
