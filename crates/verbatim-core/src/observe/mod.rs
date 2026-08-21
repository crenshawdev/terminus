//! Observations: an auditable account of what happened in a finalized session
//! (OBS-01..OBS-04).
//!
//! **Two halves, one row.** The mechanical half is [`mechanical`]: parser-
//! derived facts that are always available, never involve a model and open no
//! network connection. The judgment half is a later plan's: one optional call
//! per finalized session, whose every claim carries a `turn_id` resolving to a
//! real turn. They share a row because they describe one session and are read
//! back together.
//!
//! **Archival, never derived (D-02).** `observations` is deliberately absent
//! from [`crate::store::schema::DERIVED_TABLES`]: the judgment half is a paid
//! model call that no blob replay reproduces, so a `reindex` that dropped the
//! table would delete summaries a user bought. `verbatim observations
//! regenerate` (OBS-07) is the only rebuild path.

//! **Written by the pass, at the far end of it.** [`observe_new`] runs after
//! `feedback::outcomes` - which is what sets `session_meta.is_final` - and
//! before the `runs` row, so a session becomes final and gets its observation
//! on the same pass. It INSERTS and never overwrites: a pass that recomputed
//! would silently discard a judgment half somebody paid for.
//!
//! **Nothing here fails a pass.** The archive is the work. An observation that
//! could not be computed is a note folded into `runs.error` - there is no log
//! file, by design - exactly as the drain's and the labeller's failures are.

//! **One door to the network.** [`net`] is the only module in either crate that
//! names an HTTP client or opens a socket, and it counts every attempt before
//! it makes it (PRIV-03, D-21). The mechanical half above reaches it never;
//! that is an assertion a test reads off the attempt log rather than a promise.
//!
//! **One redaction boundary, and it only exists when data leaves.** [`egress`]
//! filters a request body on the DECLARED destination (D-13) and scrubs every
//! string on its way to becoming an error whatever the destination is (D-16).
//! Nothing here redacts at ingest: `.planning/PROJECT.md` bars that outright.

pub mod egress;
pub mod mechanical;
pub mod net;

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::config::{visible, Config};
use crate::error::Result;

pub use mechanical::{observe, Mechanical};

/// How many sessions one pass may observe.
///
/// This step reads one blob per newly finalized session (p90 1.0 MB, largest
/// measured 10.1 MB uncompressed) and it runs inside the ingest lock, so it is
/// bounded the way the rest of the pass is bounded. Whatever is left over is
/// named in the note and picked up by the next pass; the hook spawn is the
/// scheduler, so "the next pass" is the next prompt rather than a day away. A
/// first pass over a whole backfilled history is the case this exists for.
pub const MAX_PER_PASS: usize = 100;

/// What one pass's observation step did (OBS-01).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observed {
    /// Rows written on this pass, never the number that exist: a session that
    /// already had an observation is not touched again.
    pub written: usize,
    /// Whatever could not be done, said rather than raised. There is no log
    /// file, so these travel into `runs.error` with the pass's other notes.
    pub notes: Vec<String>,
}

impl Observed {
    /// The reported lines, in the shape `record_pass` writes.
    pub fn lines(&self) -> Vec<String> {
        self.notes.clone()
    }
}

/// Write one observation for every visible session that is final and has none.
///
/// **Through `config::visible::sessions`, not straight off `session_meta`.**
/// Exclusion is retroactive by design (ING-08): a project excluded after its
/// sessions were archived must stop being read, and this step reads a blob. The
/// listing costs a few milliseconds on a real store, which is a price a
/// background pass can pay and a prompt cannot.
pub fn observe_new(conn: &Connection, config: &Config) -> Observed {
    let mut out = Observed::default();

    let visible = match visible::sessions(conn, config) {
        Ok(sessions) => sessions,
        Err(e) => {
            out.notes
                .push(format!("the sessions to observe could not be listed: {e}"));
            return out;
        }
    };
    let mut pending = match pending(conn) {
        Ok(pending) => pending,
        Err(e) => {
            out.notes
                .push(format!("the finalized sessions could not be read: {e}"));
            return out;
        }
    };
    if pending.is_empty() {
        return out;
    }

    // Ingest order, because `visible::sessions` is ordered by `session_no`: the
    // oldest unobserved session goes first, so a bounded pass makes the same
    // progress every time rather than picking a different hundred each run.
    let mut candidates: Vec<(String, Option<String>)> = Vec::new();
    for session in visible {
        if let Some(session_id) = pending.remove(&session.session_key) {
            candidates.push((session.session_key, session_id));
        }
    }

    let over = candidates.len().saturating_sub(MAX_PER_PASS);
    for (session_key, session_id) in candidates.into_iter().take(MAX_PER_PASS) {
        match write_one(conn, &session_key, session_id.as_deref()) {
            Ok(()) => out.written += 1,
            // Named and skipped, the way one damaged transcript is skipped by
            // the walk it sits in: a blob that will not decompress is archive
            // damage `verbatim verify` reports, and it is not a reason to
            // observe none of the other sessions.
            Err(e) => out
                .notes
                .push(format!("{session_key}: no observation was written: {e}")),
        }
    }
    if over > 0 {
        out.notes.push(format!(
            "{over} more finalized session(s) are waiting to be observed; the next ingest \
             takes the next {MAX_PER_PASS}"
        ));
    }
    out
}

/// The visible-agnostic half: final sessions carrying no observation yet.
fn pending(conn: &Connection) -> Result<BTreeMap<String, Option<String>>> {
    let mut statement = conn.prepare(
        "SELECT m.session_key, m.session_id
           FROM session_meta m
           LEFT JOIN observations o ON o.session_key = m.session_key
          WHERE m.is_final = 1 AND o.session_key IS NULL",
    )?;
    let rows = statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (key, id) = row?;
        out.insert(key, id);
    }
    Ok(out)
}

/// One row, inserted and never overwritten (D-02).
///
/// `DO NOTHING` rather than an upsert, and that is the whole of why this step
/// is safe to run on every pass: `verbatim observations regenerate` is the only
/// rebuild path, so a pass that found a row already there must leave it - the
/// judgment half of it is a model call somebody paid for.
fn write_one(conn: &Connection, session_key: &str, session_id: Option<&str>) -> Result<()> {
    let facts = mechanical::observe(conn, session_key)?;
    conn.execute(
        "INSERT INTO observations (session_key, session_id, generated_at, mechanical)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?3)
         ON CONFLICT(session_key) DO NOTHING",
        rusqlite::params![session_key, session_id, facts.to_json().to_string()],
    )?;
    Ok(())
}

/// Which rows a regenerate acts on (OBS-07).
///
/// Two independent narrowings that AND together. Neither given selects every
/// visible session, which is what makes a bare `verbatim observations
/// regenerate` mean "all of them" rather than "none of them".
#[derive(Debug, Clone, Copy, Default)]
pub struct Selector<'a> {
    /// Sessions whose `session_meta.last_turn_at` is at or after this bound.
    ///
    /// A session carrying no `last_turn_at` is outside every bound, for the
    /// reason `verbatim sessions`'s window says: a session that cannot say when
    /// it happened cannot be shown as evidence of when something did.
    pub since: Option<&'a str>,
    /// Rows carrying exactly this value in `observations.prompt_version`.
    ///
    /// The column is null until PLAN-3 fills it, so this narrows to nothing on
    /// a store no provider has answered for - which is the honest answer, not
    /// an empty selector that would rebuild everything.
    pub prompt_version: Option<&'a str>,
}

/// What one regenerate did (OBS-07).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Regenerated {
    /// Rows the selector named AND the exclusion gate allows.
    pub selected: usize,
    /// Rows whose `mechanical` column was rewritten.
    pub rewritten: usize,
    /// Whatever could not be recomputed, named and skipped.
    pub notes: Vec<String>,
}

/// Recompute the mechanical half of the selected rows, in place (OBS-07).
///
/// **The only rebuild path for this table (D-02).** `observations` is out of
/// `DERIVED_TABLES`, so `reindex` never touches it and the ingest pass inserts
/// and never overwrites; this is the one place a stored fact set is replaced.
///
/// **`mechanical` and nothing else.** Not `generated_at`, not `session_id`, and
/// none of the judgment columns: `generated_at` dates the row as a whole -
/// including a judgment half somebody paid for - so a mechanical recompute that
/// moved it would misdate the part it did not touch. One column, on exactly the
/// selected rows, is what makes a run under a narrowed selector provably scoped.
///
/// **Through `config::visible::sessions`.** This reads one blob per selected
/// session, and exclusion is retroactive (ING-08): a project excluded after its
/// sessions were archived must stop being read, by a rebuild as much as by a
/// query.
///
/// **Unbounded, unlike [`observe_new`].** The pass is bounded because it runs
/// inside the hook path on every prompt; this is asked for explicitly, the way
/// `verbatim reindex` is, and a rebuild that silently did a hundred rows of the
/// set it was handed would be the wrong answer.
pub fn regenerate(
    conn: &Connection,
    config: &Config,
    selector: &Selector<'_>,
) -> Result<Regenerated> {
    let mut out = Regenerated::default();

    let allowed: std::collections::BTreeSet<String> = visible::sessions(conn, config)?
        .into_iter()
        .map(|session| session.session_key)
        .collect();

    // `o.prompt_version` is qualified for the reason the schema comment gives
    // about `o.decisions`: the column and the table share a name.
    let selected: Vec<String> = {
        let mut statement = conn.prepare(
            "SELECT o.session_key
               FROM observations o
               LEFT JOIN session_meta m ON m.session_key = o.session_key
              WHERE (?1 IS NULL OR m.last_turn_at >= ?1)
                AND (?2 IS NULL OR o.prompt_version = ?2)
              ORDER BY o.session_key",
        )?;
        let rows = statement.query_map(
            rusqlite::params![selector.since, selector.prompt_version],
            |r| r.get::<_, String>(0),
        )?;
        let mut keys = Vec::new();
        for row in rows {
            let key = row?;
            if allowed.contains(&key) {
                keys.push(key);
            }
        }
        keys
    };
    out.selected = selected.len();

    for session_key in selected {
        // Per row and not one transaction over the set: a session whose blob
        // will not decompress is archive damage `verbatim verify` reports, and
        // it is not a reason to leave every other selected row un-rebuilt. The
        // rows that were rewritten were all selected either way, so the scoping
        // claim holds whichever ones failed.
        match rewrite_one(conn, &session_key) {
            Ok(()) => out.rewritten += 1,
            Err(e) => out
                .notes
                .push(format!("{session_key}: not regenerated: {e}")),
        }
    }
    Ok(out)
}

/// One row's `mechanical` column, recomputed and written over.
fn rewrite_one(conn: &Connection, session_key: &str) -> Result<()> {
    let facts = mechanical::observe(conn, session_key)?;
    conn.execute(
        "UPDATE observations SET mechanical = ?2 WHERE session_key = ?1",
        rusqlite::params![session_key, facts.to_json().to_string()],
    )?;
    Ok(())
}
