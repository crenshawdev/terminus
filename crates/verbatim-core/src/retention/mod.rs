//! Retention: how much of each project's history the store keeps
//! (RET-01..RET-03).
//!
//! **Off unless `verbatim.toml` says otherwise.** The resolved policy is the
//! off state by construction ([`crate::config::RetentionPolicy`]), so a store
//! whose user never wrote a `[retention]` table cannot act on anything - and
//! this module issues no query at all in that state.
//!
//! # One evaluation, two callers
//!
//! [`evaluate`] names what retention WOULD act on and mutates nothing;
//! [`apply`] does it. The ingest pass runs both, back to back, under the lock
//! it already holds; `verbatim retention --dry-run` runs the first alone and
//! prints what it named (D-04). They share the function rather than each
//! spelling the rule, because two independently written evaluations of a
//! `'now'`-relative predicate can legitimately disagree by one session and
//! there would be no way to tell that from a bug.
//!
//! **Sharing the function pins the RULE, not the instant.** `'now'`
//! re-evaluates on every call, so the instant is computed ONCE by the caller
//! ([`evaluated_now`]) and passed in, and every caller reports the instant it
//! used ([`Selection::evaluated_at`]). Two calls straddling an age boundary
//! still name different sets; reporting the instant is what makes that
//! difference explainable instead of invisible.
//!
//! # The age test
//!
//! `session_meta.last_turn_at` against `strftime('%Y-%m-%dT%H:%M:%fZ', <now>,
//! '-N days')`, which is the statement shape
//! [`crate::feedback::finalize::finalize`] already uses and for the reason it
//! states: stored timestamps are fixed-width ISO-8601 UTC text, so string order
//! is time order and no date parsing happens anywhere. The cutoff is computed
//! by SQLite from the caller's instant rather than from `'now'`, so every row
//! in one evaluation is measured against the same clock reading.
//!
//! The comparison itself is a Rust string comparison and not a SQL `WHERE`
//! clause, because the policy is per-project and resolved in Rust: the age
//! differs by row, so the query selects the candidates with their project keys
//! and the policy decides each one. Both sides are the same fixed-width ASCII,
//! so the ordering is the one the SQL clause would have applied.
//!
//! Only sessions the idle rule has already closed (`session_meta.is_final = 1`)
//! are eligible, and a session with no `last_turn_at` is never eligible: it is
//! not idle, it is unknown. Evicting a transcript that is still being appended
//! to would fail the next pass's `blob::append` checksum precondition and
//! record that file as a per-file failure on every pass thereafter.

pub mod apply;

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::config::{visible, Config, RetentionAction};
use crate::error::Result;
use crate::store::Store;

pub use apply::{apply, Applied};

/// How many sessions one evaluation may name, per action.
///
/// Mirrors [`crate::observe::MAX_PER_PASS`] and exists for the reason that
/// constant's own doc block gives: this work runs inside the ingest lock, so it
/// is bounded the way the rest of the pass is bounded, and the hook spawn is
/// the scheduler - so "the next pass" is the next prompt rather than a day
/// away. Whatever is left over is counted ([`Selection::over`]) and said out
/// loud, which is what lets a reader tell "there is nothing left to do" from
/// "the rest is coming next pass".
pub const MAX_PER_PASS: usize = 100;

/// What retention would do, named and not yet done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The instant every age in this evaluation was measured from, ISO-8601
    /// UTC. Reported rather than implied (see the module docs).
    pub evaluated_at: String,
    /// Session keys to evict, oldest first.
    pub evict: Vec<String>,
    /// Session keys to delete, oldest first.
    pub delete: Vec<String>,
    /// Eligible sessions [`MAX_PER_PASS`] left for the next pass.
    pub over: usize,
    /// Sessions an excluded project holds, which this evaluation passed over.
    ///
    /// Counted rather than silently dropped. Exclusion means never read, on the
    /// ingest and the read paths both (ING-08), and a delete is the most
    /// extreme thing that could be done to bytes a user said not to look at -
    /// so retention leaves them alone, exactly as
    /// [`crate::observe::observe_new`] does. The count is what stops that from
    /// being invisible: a user whose policy reclaims nothing gets a line saying
    /// which rule spared it, rather than a store that reads as broken.
    pub excluded: usize,
}

impl Selection {
    /// Would this act on anything at all?
    pub fn is_empty(&self) -> bool {
        self.evict.is_empty() && self.delete.is_empty()
    }
}

/// The instant an evaluation measures every age from.
///
/// Read off SQLite rather than off the system clock in Rust, in the same
/// `strftime` spelling that wrote every timestamp in this store, so the cutoff
/// and the stored values cannot drift apart in format or in timezone.
pub fn evaluated_now(conn: &Connection) -> Result<String> {
    Ok(
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
            r.get(0)
        })?,
    )
}

/// One candidate row, as the query hands it over.
struct Candidate {
    session_key: String,
    project: Option<String>,
    project_pre_worktree: Option<String>,
    last_turn_at: String,
    transcript_path: String,
    is_evicted: bool,
}

/// Name the sessions retention would evict and the ones it would delete.
///
/// Mutates nothing, opens no transaction and writes no row - it is what the
/// `--dry-run` report prints and what the pass step then acts on.
pub fn evaluate(conn: &Connection, config: &Config, now: &str) -> Result<Selection> {
    let mut out = Selection {
        evaluated_at: now.to_owned(),
        ..Selection::default()
    };
    // The state every user is in. No query, no listing, no clock: "retention is
    // off" is a fact about the config and costs a pass nothing to establish.
    if config.retention_selects_nothing() {
        return Ok(out);
    }

    // Oldest first, which both result lists inherit: a bounded evaluation then
    // makes the same progress every time rather than picking a different
    // hundred each run, and the sessions it picks are the ones the policy has
    // been true of for longest.
    let pre_worktree = match visible::pre_worktree_column(conn)? {
        "project_pre_worktree" => "m.project_pre_worktree",
        absent => absent,
    };
    let mut statement = conn.prepare(&format!(
        "SELECT m.session_key, m.project, {pre_worktree}, m.last_turn_at,
                m.transcript_path, m.is_evicted
           FROM session_meta m
          WHERE m.is_final = 1 AND m.last_turn_at IS NOT NULL
          ORDER BY m.last_turn_at, m.session_key"
    ))?;
    let rows = statement.query_map([], |r| {
        Ok(Candidate {
            session_key: r.get(0)?,
            project: r.get(1)?,
            project_pre_worktree: r.get(2)?,
            last_turn_at: r.get(3)?,
            transcript_path: r.get(4)?,
            is_evicted: r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
        })
    })?;

    // One cutoff per distinct configured age, computed from the caller's
    // instant and reused for every row that age governs.
    let mut cutoffs: BTreeMap<u32, String> = BTreeMap::new();
    for row in rows {
        let row = row?;
        if is_excluded(config, &row) {
            out.excluded += 1;
            continue;
        }
        let policy = config.retention_for(row.project.as_deref());
        if policy.selects_nothing() {
            continue;
        }
        let cutoff = match cutoffs.get(&policy.age_days) {
            Some(cutoff) => cutoff,
            None => {
                let cutoff = cutoff_at(conn, now, policy.age_days)?;
                cutoffs.entry(policy.age_days).or_insert(cutoff)
            }
        };
        if row.last_turn_at.as_str() >= cutoff.as_str() {
            continue;
        }
        match policy.action {
            // Unreachable: `selects_nothing` above already took it. Spelled out
            // rather than swept into a wildcard so a fourth action added later
            // cannot silently fall through to "do nothing".
            RetentionAction::Keep => {}
            RetentionAction::Evict => {
                // Already emptied. Offering it again would make every pass
                // report an eviction that reclaims nothing.
                if !row.is_evicted {
                    out.evict.push(row.session_key);
                }
            }
            RetentionAction::Delete => {
                // D-02, and it is the whole of the delete rule beyond the age:
                // a session whose transcript is still on disk is rediscovered
                // by `discover::discover` on the next hook spawn and re-ingested
                // from offset 0, so deleting it reclaims nothing measurable.
                if transcript_is_gone(&row.transcript_path) {
                    out.delete.push(row.session_key);
                }
            }
        }
    }

    out.over = out.evict.len().saturating_sub(MAX_PER_PASS)
        + out.delete.len().saturating_sub(MAX_PER_PASS);
    out.evict.truncate(MAX_PER_PASS);
    out.delete.truncate(MAX_PER_PASS);
    Ok(out)
}

/// `now` less `age_days` days, in the stored timestamp's own spelling.
fn cutoff_at(conn: &Connection, now: &str, age_days: u32) -> Result<String> {
    Ok(conn.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1, ?2)",
        rusqlite::params![now, format!("-{age_days} days")],
        |r| r.get(0),
    )?)
}

/// Is this session inside an excluded project, under either of its keys?
///
/// Both keys, for the reason `config::visible` gives: a worktree session
/// carries the folded parent repo in `project` and the worktree path in the
/// pre-mapping column, and a user may reasonably have excluded either one.
fn is_excluded(config: &Config, row: &Candidate) -> bool {
    [&row.project, &row.project_pre_worktree]
        .into_iter()
        .flatten()
        .any(|key| config.excludes_path(std::path::Path::new(key)))
}

/// Has Claude Code's own cleanup already removed this transcript (D-02)?
///
/// **Only `NotFound` counts as gone.** A permission error, an unreadable mount
/// or a path this platform cannot represent all leave the question open, and
/// the safe answer for a deletion is that the file is still there: being wrong
/// that way costs a session that is reclaimed on a later pass, and being wrong
/// the other way destroys an archive whose transcript is still on disk.
fn transcript_is_gone(path: &str) -> bool {
    match std::fs::metadata(path) {
        Ok(_) => false,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Evaluate and apply, in one call, reporting instead of failing.
///
/// The one entry point the ingest pass uses, and the reason it exists is the
/// error handling rather than the composition: **nothing here may fail a
/// pass.** The archive is the work, a retention step that could not run is a
/// note, and a `?` propagated out of here is exactly what would let one damaged
/// row stop every future ingest of every other transcript in the tree.
///
/// The evaluation instant is read once, here, and every age in the resulting
/// selection is measured from it.
pub fn retain(store: &mut Store, config: &Config) -> Applied {
    // The state every user is in. No clock read, no query, no note - which is
    // what leaves `runs.error` null on a pass that had no retention to do.
    if config.retention_selects_nothing() {
        return Applied::default();
    }

    let now = match evaluated_now(store.conn()) {
        Ok(now) => now,
        Err(e) => {
            return Applied {
                notes: vec![format!("retention could not read the clock: {e}")],
                ..Applied::default()
            }
        }
    };
    let selection = match evaluate(store.conn(), config, &now) {
        Ok(selection) => selection,
        Err(e) => {
            return Applied {
                notes: vec![format!("retention could not be evaluated: {e}")],
                ..Applied::default()
            }
        }
    };

    let mut applied = apply(store, &selection);
    applied.over = selection.over;
    applied.excluded = selection.excluded;
    applied
}
