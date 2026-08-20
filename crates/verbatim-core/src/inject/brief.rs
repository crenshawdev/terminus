//! The `SessionStart` resume brief (INJ-01, INJ-02).
//!
//! What a session opens knowing: where the last session in this project left
//! off - its date, the branch it ended on, the last thing said in either
//! direction - and how much of the project is archived and reachable. Every
//! value comes from a stored row, so two runs against an unchanged store render
//! the same bytes (INJ-02).
//!
//! The counts are read the way D-09 requires: the project-only projection
//! `scope::resolve` has already run, plus one targeted `count(*)` per number,
//! and never `config::visible::sessions`. Measured on a store shaped like the
//! real one - 2,000 sessions, 250,000 turns, warm - `sessions()` costs 6.2-6.8
//! ms against 0.56-0.58 ms for the project-only join, because of its
//! per-session `count(*)` over `turns` and its watermark lookup. The whole
//! budget here is single digit milliseconds, so a brief built on it would spend
//! the budget before reading a useful row.
//!
//! **The working-state delta is the branch and nothing else (D-10).**
//! `session_meta.branch` is the only git fact the archive holds, and
//! `DESIGN-BRIEF.md:230` asks for HEAD-then-versus-now and a dirty-file list as
//! well. Both need `git`, whose 10-30 ms process spawn is the whole budget
//! several times over - phase 3 D-12 already bars `project::Resolver` from the
//! cold-start path for the same reason. So no `std::process::Command` appears
//! anywhere in this module; the model can check live git itself in one cheap
//! tool call, and this brief tells it what the archive knew.
//!
//! **Nothing here fails.** Every block is an `Option` and the brief is the ones
//! that rendered: a session with no meta row, no branch, no timestamp or a blob
//! that will not open drops that block and keeps the rest, because a brief that
//! returned an error would be a `SessionStart` that emitted nothing at all.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::Payload;
use crate::blob::BlobReader;
use crate::config::Config;
use crate::recall::scope::Scoped;
use crate::recall::{excerpt, Query};

/// Render the resume brief for one `SessionStart`, or nothing.
///
/// Nothing is the ordinary answer on a machine with no archive, in a directory
/// no archived project covers, and in an excluded project. It is also every
/// failure: see the module doc on `inject`.
pub fn session_start(data_dir: &Path, config: &Config, payload: &Payload) -> Option<String> {
    let store = super::open(data_dir)?;
    let scoped = super::scoped(store.conn(), config, payload)?;
    render(store.conn(), &scoped)
}

/// The brief's blocks, in order, joined by a blank line.
///
/// The last session first and the index pointer last: continuity is what the
/// session is resuming, and the pointer is the standing fact that outlives it.
/// The shape is a list so that a block which cannot be rendered is dropped
/// rather than failing the brief.
fn render(conn: &Connection, scoped: &Scoped) -> Option<String> {
    let blocks: Vec<String> = [last_session(conn, scoped), index_pointer(conn, scoped)]
        .into_iter()
        .flatten()
        .collect();
    if blocks.is_empty() {
        return None;
    }
    Some(blocks.join("\n\n"))
}

/// What the brief calls the project it is about.
///
/// `None` is the every-project scope, which reaches here only from a config
/// that excludes nothing and a `cwd` that resolved to no key.
fn project_label(scoped: &Scoped) -> &str {
    scoped.project().unwrap_or("this machine")
}

/// The `session_meta` row the brief is about, and the two coordinates it needs.
struct LastSession {
    session_key: String,
    last_turn_at: Option<String>,
    branch: Option<String>,
}

/// INJ-01's continuity blocks: which session was last here, when it ended, on
/// what branch, and the last thing said in each direction.
fn last_session(conn: &Connection, scoped: &Scoped) -> Option<String> {
    let row = last_session_row(conn, scoped).ok()??;
    let project = project_label(scoped);

    let day = row.last_turn_at.as_deref().map(day);
    let mut block = match (day, row.branch.as_deref()) {
        (Some(day), Some(branch)) => {
            format!("The last session in {project} ended {day}, on branch {branch}.")
        }
        (Some(day), None) => format!("The last session in {project} ended {day}."),
        (None, Some(branch)) => format!("The last session in {project} was on branch {branch}."),
        (None, None) => format!("The last session in {project} is archived."),
    };

    let (prompt, reply) = last_exchange(conn, &row.session_key);
    if let Some(prompt) = prompt {
        block.push_str(&format!("\nIt last asked: {prompt}"));
    }
    if let Some(reply) = reply {
        block.push_str(&format!("\nIt last answered: {reply}"));
    }
    Some(block)
}

/// The greatest `last_turn_at` among the scoped project's own sessions.
///
/// Three things this query is not. It is not `is_final`: that column is
/// declared in the schema and never written, so ordering by it would name a
/// session for a reason nothing establishes (D-17). It is not a date
/// comparison: 22,412 turns of a 180-file sample carry exactly the shape
/// `NNNN-NN-NNTNN:NN:NN.NNNZ` (phase 3 D-23), which sorts lexicographically, so
/// no parsing happens on the stored side. And it does not consider sidecars: a
/// session with a `parent_session_key` is a subagent transcript, and the brief
/// carries no subagent turns (`DESIGN-BRIEF.md:239`).
///
/// The tie-break on `session_key` is INJ-02's, not SQL's: two sessions whose
/// last turn carries the same timestamp must not be ordered by whatever the
/// query planner did that run, or the brief changes without the archive
/// changing.
///
/// A null `last_turn_at` sorts last under `DESC`, so a session that has one
/// always wins over one that does not, and a project where nothing has a
/// timestamp still names a session.
fn last_session_row(
    conn: &Connection,
    scoped: &Scoped,
) -> crate::error::Result<Option<LastSession>> {
    let mut sql = String::from(
        "SELECT m.session_key, m.last_turn_at, m.branch FROM session_meta m \
         WHERE m.parent_session_key IS NULL",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(project) = scoped.project() {
        sql.push_str(" AND m.project = ?");
        params.push(Box::new(project.to_owned()));
    }
    super::push_exclusion(
        &mut sql,
        &mut params,
        "m.project",
        scoped.excluded_projects(),
    );
    super::push_exclusion(
        &mut sql,
        &mut params,
        "m.project_pre_worktree",
        scoped.excluded_pre_worktree(),
    );
    sql.push_str(" ORDER BY m.last_turn_at DESC, m.session_key DESC LIMIT 1");

    let row = conn
        .query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| {
            Ok(LastSession {
                session_key: r.get(0)?,
                last_turn_at: r.get(1)?,
                branch: r.get(2)?,
            })
        })
        .optional()?;
    Ok(row)
}

/// The day of a stored timestamp, at day resolution (INJ-02).
///
/// `get` and not `&ts[..10]`, for the reason `cmd::search::day` states: `len`
/// counts bytes, the slice needs a char boundary, and a stored `ts` is any JSON
/// string the transcript carried - `parse::record` validates neither shape nor
/// encoding. A timestamp with a multi-byte character across byte 10 panicked a
/// whole command once already, and a panic here is a `SessionStart` that emits
/// nothing.
fn day(ts: &str) -> &str {
    ts.get(..10).unwrap_or(ts)
}

/// The last user turn and the last assistant turn of one session, as text.
///
/// **One blob read for the pair.** Both turns are in the same session and
/// rusqlite's incremental blob I/O is behind a feature this workspace does not
/// enable, so selecting the blob twice would decompress the same session twice -
/// up to 10.1 MB uncompressed (phase 1 D-05) on the path with the tightest
/// budget in the product.
///
/// `turn_seq` order and never `ts` order: 31 of 62 sampled real transcripts
/// carry a record whose timestamp runs backwards (phase 3 D-22), so the last
/// turn by clock is not the last turn of the conversation.
///
/// The text is cut through [`excerpt::of_record`], which is the same projection
/// ingest indexed these turns with and the search path reads them through -
/// `DESIGN-BRIEF.md:230` asks for the last prompt and the last assistant turn
/// "truncated", and a second definition of what a turn said would be a second
/// thing to keep true. The query is empty because there is no query here: the
/// window then falls back to the head of the turn, which is what the turn
/// opened with.
fn last_exchange(conn: &Connection, session_key: &str) -> (Option<String>, Option<String>) {
    let prompt = last_turn(conn, session_key, "user");
    let reply = last_turn(conn, session_key, "assistant");
    if prompt.is_none() && reply.is_none() {
        return (None, None);
    }

    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [session_key],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    let Some(blob) = blob else {
        return (None, None);
    };
    let Ok(reader) = BlobReader::open(&blob) else {
        return (None, None);
    };

    let query = Query::parse("");
    let cut = |coordinates: Option<(i64, i64)>| -> Option<String> {
        let (offset, len) = coordinates?;
        let bytes = reader.read_range(offset as u64, len as u64).ok()?;
        let text = excerpt::of_record(&query, &bytes);
        (!text.trim().is_empty()).then_some(text)
    };
    (cut(prompt), cut(reply))
}

/// `(stream_offset, byte_len)` of one session's last turn of a record type.
///
/// D-04: both address the UNCOMPRESSED session stream, never the blob.
/// `UNIQUE (session_key, turn_seq)` covers the lookup.
fn last_turn(conn: &Connection, session_key: &str, record_type: &str) -> Option<(i64, i64)> {
    conn.query_row(
        "SELECT stream_offset, byte_len FROM turns \
         WHERE session_key = ?1 AND record_type = ?2 \
         ORDER BY turn_seq DESC LIMIT 1",
        rusqlite::params![session_key, record_type],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .ok()
    .flatten()
}

/// INJ-01's index pointer: how much of this project is archived, and that there
/// are tools to reach it with.
///
/// The tool names are spelled here rather than taken from the MCP module
/// because that module is in the binary and this is the library; the two lists
/// are held together by `crates/verbatim/tests/inject.rs`. A project with
/// nothing archived renders no pointer at all - "0 sessions indexed" is a line
/// that costs context and tells the model to stop asking.
fn index_pointer(conn: &Connection, scoped: &Scoped) -> Option<String> {
    let sessions = super::count(conn, SESSIONS, scoped).ok()?;
    let turns = super::count(conn, TURNS, scoped).ok()?;
    if sessions == 0 {
        return None;
    }
    let project = project_label(scoped);
    Some(format!(
        "Verbatim has {} and {} archived for {project}, searchable at turn granularity.\n\
         Reach them with the recall_search, recall_context and recall_get tools \
         rather than guessing.",
        plural(sessions, "session"),
        plural(turns, "turn"),
    ))
}

/// Sessions of the scoped project. `idx_session_meta_project` covers it.
const SESSIONS: &str = "SELECT count(*) FROM session_meta m";

/// Turns of those sessions. The join is on `session_key`, which is
/// `session_meta`'s primary key and the leading column of `turns`'
/// `UNIQUE (session_key, turn_seq)` index, so neither side is a scan.
const TURNS: &str = "SELECT count(*) FROM turns t JOIN session_meta m USING (session_key)";

/// `1 session` / `2 sessions`, for a brief that reads as English.
fn plural(count: i64, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}
