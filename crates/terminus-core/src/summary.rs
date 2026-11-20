//! The summary: answers the resume brief needs, kept ready at ingest.
//!
//! Two tables, `project_summary` and `session_summary`, so the brief's numbers
//! and its pick of session exist before any session starts and the hook reads
//! a row instead of grouping the archive. Four things write them, each in the
//! transaction that changes the rows under them:
//!
//! - ingest, through [`write`], inside each file's transaction, from the rows as
//!   they stand after the file's writes and never from the scan;
//! - retention's delete, through `remove`, inside the delete's own transaction,
//!   before the session's rows go;
//! - `reindex`, which empties both tables and writes every session again
//!   through `rewrite` inside its one transaction, and stamps
//!   [`META_SUMMARY_FILLED`] last;
//! - the pass's fill, which does the same through `rewrite` on a store whose
//!   `meta` lacks [`META_SUMMARY_FILLED`], and stamps it last.
//!
//! A row is a function of committed rows alone, so a killed pass leaves the
//! summary equal to what the committed rows give. An eviction changes nothing
//! the summary holds and leaves it alone.
//!
//! Every function takes the transaction's `&Connection`, the way
//! [`crate::derive`]'s do, and opens none of its own.

use rusqlite::{Connection, OptionalExtension};

use crate::error::Result;
use crate::observe::mechanical::WRITING_TOOLS;

/// `meta` key whose row says the summary has been written whole.
///
/// A store that holds the row has had every session's summary written by the
/// pass's fill or by `reindex`. Only those two write it, each last in its own
/// transaction, and code only asks whether the row exists. A pass fills a store
/// whose `meta` lacks it.
pub const META_SUMMARY_FILLED: &str = "summary_filled";

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

/// Empty both tables and write every stored session as new, from the rows the
/// transaction can see. The one rewrite `reindex` and the fill share.
pub(crate) fn rewrite(conn: &Connection) -> Result<()> {
    conn.execute_batch("DELETE FROM session_summary; DELETE FROM project_summary;")?;
    let keys: Vec<String> = conn
        .prepare("SELECT session_key FROM sessions ORDER BY session_no")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for session_key in &keys {
        write(conn, session_key, None)?;
    }
    Ok(())
}

/// Fill the summary when `meta` lacks [`META_SUMMARY_FILLED`], and write that
/// row last. A store that has the row is left exactly as it is.
///
/// Whether the row exists is the whole test, never whether its value parses.
pub(crate) fn fill(conn: &Connection) -> Result<()> {
    let filled = conn
        .prepare_cached("SELECT 1 FROM meta WHERE key = ?1")?
        .exists([META_SUMMARY_FILLED])?;
    if filled {
        return Ok(());
    }
    rewrite(conn)?;
    conn.prepare_cached("INSERT INTO meta (key, value) VALUES (?1, '1')")?
        .execute([META_SUMMARY_FILLED])?;
    Ok(())
}

/// Take a session off the summary ahead of its delete: its own row, and its
/// share of its pair.
///
/// Call it first in the delete's transaction, while `session_meta` and `turns`
/// still hold what the session counted. The pair's counts are adjusted and
/// never recounted, and stop at zero, so a row that is already missing or short
/// is not an error. A subagent has no row of its own and only lowers its pair.
pub(crate) fn remove(conn: &Connection, session_key: &str) -> Result<()> {
    conn.prepare_cached("DELETE FROM session_summary WHERE session_key = ?1")?
        .execute([session_key])?;
    let Some(pair) = stored_pair(conn, session_key)? else {
        return Ok(());
    };
    let turns = turn_count(conn, session_key)?;
    conn.prepare_cached(
        "UPDATE project_summary
         SET session_count = max(session_count - 1, 0), turn_count = max(turn_count - ?3, 0)
         WHERE pair_key = json_array(?1, ?2)",
    )?
    .execute(rusqlite::params![pair.project, pair.pre_worktree, turns])?;
    conn.prepare_cached(
        "DELETE FROM project_summary WHERE pair_key = json_array(?1, ?2) AND session_count <= 0",
    )?
    .execute(rusqlite::params![pair.project, pair.pre_worktree])?;
    Ok(())
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

    let prompts = newest_prompts(conn, session_key)?;
    let reply = newest_reply(conn, session_key)?;
    let files = edited_files(conn, session_key, [&cwd, &pre_worktree, &project])?;

    conn.prepare_cached(
        "INSERT OR REPLACE INTO session_summary (
            session_key, last_turn_at, branch, project, project_pre_worktree, cwd,
            title, has_typed_prompt, turn_count, prompts,
            reply_id, reply_stream_offset, reply_byte_len, edited_files
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
        serde_json::json!(prompts).to_string(),
        reply.map(|(id, _, _)| id),
        reply.map(|(_, offset, _)| offset),
        reply.map(|(_, _, len)| len),
        serde_json::json!(files).to_string(),
    ])?;
    Ok(())
}

/// How many of a session's newest typed prompts the row locates: the number the
/// brief looks at to find the few it quotes. The quoting, the length rule and
/// the read budget stay with the brief, which reads the text and redacts it.
const PROMPTS_KEPT: i64 = 12;

/// `(id, stream_offset, byte_len)` of one `turns` row: where a turn's bytes sit
/// in the session's stream, never what they say.
type Locator = (i64, i64, i64);

/// The newest typed `user` turns, newest first, or all when fewer. A `user`
/// record nobody typed is skipped, and the order is `turn_seq` and never `ts`.
fn newest_prompts(conn: &Connection, session_key: &str) -> Result<Vec<Locator>> {
    locators(
        conn,
        "SELECT id, stream_offset, byte_len FROM turns
         WHERE session_key = ?1 AND record_type = 'user' AND is_typed IS NOT 0
         ORDER BY turn_seq DESC LIMIT ?2",
        session_key,
        PROMPTS_KEPT,
    )
}

/// The newest `assistant` turn, whatever it holds: a turn that is only a tool
/// call is still the last reply.
fn newest_reply(conn: &Connection, session_key: &str) -> Result<Option<Locator>> {
    Ok(locators(
        conn,
        "SELECT id, stream_offset, byte_len FROM turns
         WHERE session_key = ?1 AND record_type = 'assistant'
         ORDER BY turn_seq DESC LIMIT ?2",
        session_key,
        1,
    )?
    .pop())
}

fn locators(conn: &Connection, sql: &str, session_key: &str, limit: i64) -> Result<Vec<Locator>> {
    Ok(conn
        .prepare_cached(sql)?
        .query_map(rusqlite::params![session_key, limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<std::result::Result<_, _>>()?)
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

/// Every path the session called a writing tool on that sits inside its
/// project, most recently edited first, relative to the root it sits under.
///
/// The brief's own query over the session's whole history. Redaction, the
/// clip, deduplication after the clip and the cap on how many are named stay
/// with the brief. A call that failed still counts: the record says it was
/// tried. `CROSS JOIN` starts from the session's turns, because left alone
/// SQLite starts from every `path` entity in the store.
fn edited_files(
    conn: &Connection,
    session_key: &str,
    roots: [&Option<String>; 3],
) -> Result<Vec<String>> {
    let tools: serde_json::Value = WRITING_TOOLS.iter().copied().collect();
    let mut roots: Vec<&str> = roots.into_iter().flatten().map(String::as_str).collect();
    roots.sort_by_key(|root| std::cmp::Reverse(root.len()));

    let mut statement = conn.prepare_cached(
        "SELECT e.value_norm FROM turns t CROSS JOIN entities e ON e.turn_id = t.id
         WHERE t.session_key = ?1 AND e.kind = 'path'
           AND t.tool_name IN (SELECT value FROM json_each(?2))
         GROUP BY e.value_norm ORDER BY max(t.turn_seq) DESC, e.value_norm",
    )?;
    let paths = statement.query_map(rusqlite::params![session_key, tools.to_string()], |r| {
        r.get::<_, String>(0)
    })?;
    let mut files = Vec::new();
    for path in paths {
        if let Some(inside) = inside_project(&path?, &roots) {
            files.push(inside.to_owned());
        }
    }
    Ok(files)
}

/// `path` relative to the longest root it sits under. A relative path stays as
/// written, and an absolute one under no root is outside the project.
fn inside_project<'p>(path: &'p str, roots: &[&str]) -> Option<&'p str> {
    let relative = roots
        .iter()
        .find_map(|root| path.strip_prefix(root)?.strip_prefix('/'));
    match relative {
        Some(relative) => Some(relative),
        None => (!path.starts_with(['/', '~'])).then_some(path),
    }
}
