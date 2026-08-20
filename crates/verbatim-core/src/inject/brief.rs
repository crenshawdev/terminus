//! The `SessionStart` resume brief (INJ-01, INJ-02).
//!
//! What a session opens knowing: how much of this project is archived and
//! reachable, and - once PLAN-2 lands its blocks - where the last session in it
//! left off. Every value comes from a stored row, so two runs against an
//! unchanged store render the same bytes (INJ-02).
//!
//! The counts are read the way D-09 requires: the project-only projection
//! `scope::resolve` has already run, plus one targeted `count(*)` per number,
//! and never `config::visible::sessions`. Measured on a store shaped like the
//! real one - 2,000 sessions, 250,000 turns, warm - `sessions()` costs 6.2-6.8
//! ms against 0.56-0.58 ms for the project-only join, because of its
//! per-session `count(*)` over `turns` and its watermark lookup. The whole
//! budget here is single digit milliseconds, so a brief built on it would spend
//! the budget before reading a useful row.

use std::path::Path;

use rusqlite::Connection;

use super::Payload;
use crate::config::Config;
use crate::recall::scope::Scoped;

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
/// PLAN-2 adds the last-session block ahead of the pointer and the budget over
/// the whole; the shape is a list so that a block which cannot be rendered is
/// dropped rather than failing the brief.
fn render(conn: &Connection, scoped: &Scoped) -> Option<String> {
    let blocks: Vec<String> = [index_pointer(conn, scoped)]
        .into_iter()
        .flatten()
        .collect();
    if blocks.is_empty() {
        return None;
    }
    Some(blocks.join("\n\n"))
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
    let project = scoped.project().unwrap_or("this machine");
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
