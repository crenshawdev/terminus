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

/// How long an injection read waits behind a writer before giving up.
///
/// `store::open` sets five seconds, which is right for a writer and wrong here
/// by three orders of magnitude: backfill was measured holding the store for
/// 49 s over the real corpus (`.planning/phases/4/SUMMARY.md`), and a prompt
/// submitted during one would block the user for the whole busy timeout on
/// exactly the machine state that provoked it. Lowering it turns lock
/// contention into an immediate `SQLITE_BUSY`, which is silence - the first of
/// D-03's two mechanisms, and the one that acts before the hook's watchdog has
/// to.
///
/// A few milliseconds rather than zero so that the microsecond-scale overlap
/// with an ordinary ingest commit is still waited out rather than reported as a
/// missing brief.
///
/// **It bounds the queries and not the open, which is measured rather than
/// assumed.** [`Store::open_read_only`] applies its own five-second timeout and
/// then runs a statement against `sqlite_master` before it returns, so a
/// connection that cannot get in at all waits inside the open, where nothing
/// here has been able to lower anything yet. How long that wait is depends on
/// who is holding the file, and both answers were measured on 2026-08-20
/// against a store held under `PRAGMA locking_mode = EXCLUSIVE`: a holder in
/// ANOTHER process let the open fail in about 6 ms
/// (`crates/verbatim/tests/inject.rs`), while a holder in the SAME process took
/// the busy handler's full five seconds - 5,011 ms for
/// `a_store_held_by_an_exclusive_writer_is_nothing` below. So the wait is not
/// bounded by anything in this library, and the hook's watchdog is what bounds
/// it: exactly why D-03 asks for two mechanisms rather than one. What this
/// constant does bound is every statement after the open, which is where an
/// ordinary ingest's contention lands.
const BUSY_TIMEOUT_MS: u64 = 3;

/// Open the store injection reads, or nothing.
///
/// Read-only, and the failure arms are deliberately indistinguishable from each
/// other: a machine that has never ingested, a store on unreadable media, a
/// file that is not a database, a store another process is holding and a store
/// older than this build are all "no context this time".
///
/// The version gate is the least obvious of those and the most necessary. A
/// read command answers a store that predates this build by saying so on stderr
/// and querying the old-shape index anyway (D-18); a hook has no such channel -
/// its stderr is not shown and its stdout is a validated protocol - so a
/// degraded brief would be an assertion about the archive that nothing could
/// qualify. The next ingest rebuilds the derived tables and the brief comes
/// back.
pub(crate) fn open(data_dir: &Path) -> Option<Store> {
    let store = Store::open_read_only(data_dir).ok()?;
    store
        .conn()
        .busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS))
        .ok()?;
    if store.predates_this_build() {
        return None;
    }
    Some(store)
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

#[cfg(test)]
mod tests {
    use super::*;

    use rusqlite::OpenFlags;

    /// The three stores INJ-06 names, as directories a test can point an arm at.
    ///
    /// Every one of them is a way a real machine arrives here: nothing ingested
    /// yet, a pass or a backfill holding the file, and a `verbatim.db` that is
    /// not one - a half-restored backup, a synced-file conflict, a truncated
    /// copy.
    fn data_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Both arms, against one data directory, with a payload that would inject
    /// if the store were readable.
    fn both_arms_say_nothing(data_dir: &Path, what: &str) {
        let config = Config::default();
        let payload = Payload {
            session_id: Some("0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55"),
            transcript_path: Some("/home/user/.claude/projects/-p/s.jsonl"),
            cwd: Some("/code/verbatim"),
            prompt: Some("where did we settle the retry budget"),
            source: Some("resume"),
        };
        assert_eq!(
            brief::session_start(data_dir, &config, &payload),
            None,
            "the brief injected against {what}"
        );
        assert_eq!(
            prompt::user_prompt_submit(data_dir, &config, &payload),
            None,
            "the prompt arm injected against {what}"
        );
    }

    /// The ordinary state of a machine that has installed verbatim and not yet
    /// ingested. Nothing is created by asking, either: `Store::open` would have
    /// made a store here as the side effect of a question.
    #[test]
    fn a_data_directory_with_no_store_is_nothing_and_creates_nothing() {
        let dir = data_dir();
        let data_dir = dir.path().join("data");

        both_arms_say_nothing(&data_dir, "a data directory with no store");
        assert!(!data_dir.exists(), "the read created the data directory");
    }

    /// A `verbatim.db` whose bytes are not a database. `Store::open_read_only`
    /// reports it as `StoreUnreadable`, which a read command renders as exit 1;
    /// a hook has nowhere to render it and must stay silent.
    #[test]
    fn bytes_that_are_not_a_database_are_nothing() {
        let dir = data_dir();
        std::fs::write(
            dir.path().join(crate::store::DB_FILE_NAME),
            b"this is not a database, it is a note about one",
        )
        .unwrap();

        both_arms_say_nothing(dir.path(), "a verbatim.db that is not a database");
    }

    /// A store another connection is holding outright.
    ///
    /// `PRAGMA locking_mode = EXCLUSIVE` and not a bare `BEGIN EXCLUSIVE`,
    /// because the store is in WAL mode and a WAL writer deliberately does not
    /// block readers - a test that took the write lock alone would pass while
    /// proving nothing. Exclusive locking mode is the one that makes a reader
    /// see `SQLITE_BUSY`, and the assertion below is that it really does,
    /// before the arms are asked at all.
    ///
    /// **This test takes five seconds and the five seconds are the finding.**
    /// `Store::open_read_only` sets its own timeout and queries `sqlite_master`
    /// before it returns, so the busy timeout [`BUSY_TIMEOUT_MS`] lowers arrives
    /// too late to bound the open - and a holder in the same process is the
    /// shape that makes SQLite's busy handler wait the whole five seconds
    /// (measured: 5,011 ms here against about 6 ms for the same lock held by
    /// another process). The arms still say nothing, which is all INJ-06 asks
    /// of this library. Bounding the WAIT is the hook's watchdog, D-03's other
    /// mechanism, and `crates/verbatim/tests/inject.rs` is where that is
    /// asserted against a wall clock. Do not delete this test to make the
    /// suite faster: its cost IS the measurement.
    #[test]
    fn a_store_held_by_an_exclusive_writer_is_nothing() {
        let dir = data_dir();
        let path = dir.path().join(crate::store::DB_FILE_NAME);
        // A real store, written and closed, so what follows is contention and
        // not an empty file.
        drop(Store::open(dir.path()).expect("create the store"));

        let holder = rusqlite::Connection::open(&path).unwrap();
        holder
            .pragma_update(None, "locking_mode", "exclusive")
            .unwrap();
        holder
            .execute_batch("BEGIN EXCLUSIVE; CREATE TABLE IF NOT EXISTS held (x);")
            .unwrap();

        // The falsifying check: if this read succeeds, the lock is not held and
        // the assertions below would be true for the wrong reason.
        let blocked =
            rusqlite::Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .and_then(|c| {
                    c.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS))?;
                    c.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                        r.get::<_, i64>(0)
                    })
                });
        assert!(
            blocked.is_err(),
            "the holder is not actually keeping a reader out, so this test \
             proves nothing about a busy store"
        );

        both_arms_say_nothing(dir.path(), "a store held by an exclusive writer");
        drop(holder);
    }
}
