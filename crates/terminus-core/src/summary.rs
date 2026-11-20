//! The summary: answers the resume brief needs, kept ready at ingest.
//!
//! Two tables, `project_summary` and `session_summary`, so the brief's numbers
//! and its pick of session exist before any session starts and the hook reads
//! a row instead of grouping the archive. Ingest writes both inside each file's
//! transaction, from the rows as they stand after the file's writes and never
//! from the scan, so a row is a function of committed rows alone and a killed
//! pass leaves the summary equal to what the committed rows give.
//!
//! Every function takes the transaction's `&Connection`, the way
//! [`crate::derive`]'s do, and opens none of its own.

use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;

/// The project keys a `session_meta` row names, a null key being a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub project: Option<String>,
    pub pre_worktree: Option<String>,
}

/// What a session's stored rows said before this file's writes.
///
/// Read inside the transaction ahead of the `session_meta` upsert, because the
/// upsert can move the session to another pair and the old pair has to give its
/// counts back.
#[derive(Debug)]
pub struct Before {
    pair: Pair,
    turns: i64,
}

impl Before {
    /// `None` for a session the store has not seen.
    pub fn read(conn: &Connection, session_key: &str) -> Result<Option<Before>> {
        let pair = stored_pair(conn, session_key)?;
        pair.map(|pair| {
            Ok(Before {
                pair,
                turns: turn_count(conn, session_key)?,
            })
        })
        .transpose()
    }
}

fn stored_pair(conn: &Connection, session_key: &str) -> Result<Option<Pair>> {
    Ok(conn
        .prepare_cached(
            "SELECT project, project_pre_worktree FROM session_meta WHERE session_key = ?1",
        )?
        .query_row([session_key], |r| {
            Ok(Pair {
                project: r.get(0)?,
                pre_worktree: r.get(1)?,
            })
        })
        .optional()?)
}

fn turn_count(conn: &Connection, session_key: &str) -> Result<i64> {
    Ok(conn
        .prepare_cached("SELECT count(*) FROM turns WHERE session_key = ?1")?
        .query_row([session_key], |r| r.get(0))?)
}

/// Bring the summary up to date with the file's writes. Call it after the
/// `session_meta` upsert and the file's turns, inside the same transaction.
pub fn write(conn: &Connection, session_key: &str, before: Option<Before>) -> Result<()> {
    keep_counts(conn, session_key, before)?;
    keep_session(conn, session_key)
}

/// The main session's one row, written whole from the stored rows.
///
/// A main session is a `session_meta` row with no parent, the brief's own test.
/// A subagent's file writes no row here and leaves its parent's alone. The row
/// is replaced whole on every commit, so a tail pass leaves it equal to one pass
/// over the whole file. Every value is read back from rows the transaction has
/// already written, never from the scan, and none of them is prompt or reply
/// text.
fn keep_session(conn: &Connection, session_key: &str) -> Result<()> {
    let Some((last_turn_at, branch, project, pre_worktree, cwd)) = conn
        .prepare_cached(
            "SELECT last_turn_at, branch, project, project_pre_worktree, cwd
             FROM session_meta WHERE session_key = ?1 AND parent_session_key IS NULL",
        )?
        .query_row([session_key], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .optional()?
    else {
        return Ok(());
    };
    let title: Option<String> = conn
        .prepare_cached("SELECT title FROM session_titles WHERE session_key = ?1")?
        .query_row([session_key], |r| r.get(0))
        .optional()?;
    let typed = conn
        .prepare_cached("SELECT 1 FROM turns WHERE session_key = ?1 AND is_typed = 1 LIMIT 1")?
        .exists([session_key])?;

    conn.prepare_cached(
        "INSERT OR REPLACE INTO session_summary (
            session_key, last_turn_at, branch, project, project_pre_worktree, cwd,
            title, has_typed_prompt, turn_count, prompts,
            reply_id, reply_stream_offset, reply_byte_len, edited_files
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '[]', NULL, NULL, NULL, '[]')",
    )?
    .execute(rusqlite::params![
        session_key,
        last_turn_at,
        branch,
        project,
        pre_worktree,
        cwd,
        title,
        typed,
        turn_count(conn, session_key)?,
    ])?;
    Ok(())
}

/// Move this file's sessions and turns onto the pair the stored row names now.
///
/// Counts are adjusted, never recounted. A pair changes when a later pass
/// supplies a key the first lacked, and the whole session's count moves with
/// it. An old pair left with no session loses its row.
fn keep_counts(conn: &Connection, session_key: &str, before: Option<Before>) -> Result<()> {
    let Some(after) = stored_pair(conn, session_key)? else {
        return Ok(());
    };
    let turns = turn_count(conn, session_key)?;
    match before {
        Some(before) if before.pair == after => {
            let changed = conn
                .prepare_cached(
                    "UPDATE project_summary SET turn_count = turn_count + ?3
                     WHERE pair_key = json_array(?1, ?2)",
                )?
                .execute(rusqlite::params![
                    after.project,
                    after.pre_worktree,
                    turns - before.turns
                ])?;
            if changed == 0 {
                add(conn, &after, 1, turns)?;
            }
        }
        Some(before) => {
            conn.prepare_cached(
                "UPDATE project_summary
                 SET session_count = session_count - 1, turn_count = turn_count - ?3
                 WHERE pair_key = json_array(?1, ?2)",
            )?
            .execute(rusqlite::params![
                before.pair.project,
                before.pair.pre_worktree,
                before.turns
            ])?;
            conn.prepare_cached(
                "DELETE FROM project_summary
                 WHERE pair_key = json_array(?1, ?2) AND session_count <= 0",
            )?
            .execute(rusqlite::params![
                before.pair.project,
                before.pair.pre_worktree
            ])?;
            add(conn, &after, 1, turns)?;
        }
        None => add(conn, &after, 1, turns)?,
    }
    Ok(())
}

/// Add to a pair's counts, creating its row when it has none.
///
/// `json_array` is the key because `UNIQUE` treats nulls as distinct: it is one
/// value for a null key and a different one for every string, `''` and `'null'`
/// included.
fn add(conn: &Connection, pair: &Pair, sessions: i64, turns: i64) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO project_summary
            (pair_key, project, project_pre_worktree, session_count, turn_count)
         VALUES (json_array(?1, ?2), ?1, ?2, ?3, ?4)
         ON CONFLICT(pair_key) DO UPDATE SET
            session_count = session_count + excluded.session_count,
            turn_count = turn_count + excluded.turn_count",
    )?
    .execute(rusqlite::params![
        pair.project,
        pair.pre_worktree,
        sessions,
        turns
    ])?;
    Ok(())
}
