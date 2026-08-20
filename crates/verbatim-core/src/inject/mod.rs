//! Context injection: what the model is told before it is asked (INJ-01..INJ-06).
//!
//! Two events out of the four Claude Code fires can carry text back, and each
//! has an arm here: [`brief::session_start`] renders the resume brief and
//! [`prompt::user_prompt_submit`] renders the relevance injection. Both take
//! the data directory, the loaded [`Config`] and all five payload fields, and
//! both return the text to inject or nothing at all. Nothing in this module
//! knows what the wire format is - the binary owns stdout, the exit code and
//! the JSON envelope (D-01) - and nothing in the binary knows what a turn is.
//!
//! **Every failure is silence.** These functions return an `Option`, never a
//! `Result`, and that is the requirement rather than a convenience: INJ-06 says
//! a missing store, an unreadable store, a busy store, a store older than this
//! build and a query error all leave the hook writing nothing and exiting 0. A
//! `Result` here would put a rendering decision - which errors are worth a
//! prompt - at every call site instead of once.
//!
//! **Nothing here writes, creates or locks.** The store is opened with
//! [`Store::open_read_only`] and never `Store::open`, which would
//! `create_dir_all`, initialize a fresh database, run the additive column
//! bring-forward and set two pragmas: a hook must not leave a store behind as
//! the side effect of a question, and the ingest lock belongs to the pass the
//! hook already spawned.
//!
//! **The scope comes off the payload (D-12).** [`Scope::Directory`] built from
//! the payload's `cwd`, never [`Scope::current_directory`]: the working
//! directory Claude Code chose for a hook may be a plugin root, a worktree or
//! `/`, and resolving it here would scope the most history-laden sessions to
//! the wrong project or to `Reason::UnknownProject`.

pub mod brief;
pub mod prompt;

use std::path::Path;

use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::recall::scope::{self, Scope, Scoped};
use crate::store::Store;

/// The hook payload fields injection reads, borrowed from whoever parsed them.
///
/// All five travel together even where an arm ignores one, so that the seam
/// between the binary and this module does not change shape as later plans
/// deepen an arm. Every field is optional: the harness sends a different set
/// per event, and an absent field is less injection rather than an error.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Payload<'a> {
    /// The Claude Code session this event belongs to.
    pub session_id: Option<&'a str>,
    /// The transcript file Claude Code is appending to.
    pub transcript_path: Option<&'a str>,
    /// The session's working directory, which is what scopes both arms (D-12).
    pub cwd: Option<&'a str>,
    /// The text the user just submitted. `UserPromptSubmit` only.
    pub prompt: Option<&'a str>,
    /// Why the session started. `SessionStart` only.
    pub source: Option<&'a str>,
}

/// Open the store injection reads, or nothing.
///
/// Read-only, and the failure arms are deliberately indistinguishable from each
/// other: a machine that has never ingested, a store on unreadable media and a
/// file that is not a database are all "no context this time".
pub(crate) fn open(data_dir: &Path) -> Option<Store> {
    Store::open_read_only(data_dir).ok()
}

/// The project the payload's `cwd` sits in, resolved against the keys the
/// archive already holds.
///
/// `None` when the payload carries no `cwd`, when the resolve failed, or when
/// the scope can match nothing at all - an empty archive, a directory no
/// archived project covers, or a project the config excludes. Every one of
/// those is silence.
pub(crate) fn scoped(conn: &Connection, config: &Config, payload: &Payload) -> Option<Scoped> {
    let cwd = payload.cwd?;
    let scoped = scope::resolve(conn, config, &Scope::Directory(cwd.into())).ok()?;
    if scoped.is_empty() {
        return None;
    }
    Some(scoped)
}

/// The `NOT IN` clause that keeps an excluded session out of a count.
///
/// ING-08 is enforced on the read path as well as on ingest, and
/// `scope::resolve` only rejects a scope whose own project is excluded - a
/// session of a visible project whose *pre-worktree* key is excluded is still
/// in the table and must not be counted. Written with the null guard because
/// SQL's `NULL NOT IN (...)` is NULL rather than true, which would drop every
/// session whose key is null: the one session nothing can say is excluded.
pub(crate) fn push_exclusion(
    sql: &mut String,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    column: &str,
    excluded: &[String],
) {
    if excluded.is_empty() {
        return;
    }
    sql.push_str(&format!(" AND ({column} IS NULL OR {column} NOT IN ("));
    for (index, value) in excluded.iter().enumerate() {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
        params.push(Box::new(value.clone()));
    }
    sql.push_str("))");
}

/// One count over the visible rows of a scoped project.
pub(crate) fn count(conn: &Connection, head: &str, scoped: &Scoped) -> Result<i64> {
    let mut sql = String::from(head);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    match scoped.project() {
        Some(project) => {
            sql.push_str(" WHERE m.project = ?");
            params.push(Box::new(project.to_owned()));
        }
        None => sql.push_str(" WHERE 1 = 1"),
    }
    push_exclusion(
        &mut sql,
        &mut params,
        "m.project",
        scoped.excluded_projects(),
    );
    push_exclusion(
        &mut sql,
        &mut params,
        "m.project_pre_worktree",
        scoped.excluded_pre_worktree(),
    );

    let count = conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| {
        r.get(0)
    })?;
    Ok(count)
}
