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
//! table would delete summaries a user bought. `terminus observations
//! regenerate` (OBS-07) is the only rebuild path.

//! **Written by the pass, at the far end of it.** [`observe_new`] runs after
//! `feedback::outcomes` - which is what sets `session_meta.is_final` - and
//! before the `runs` row, so a session becomes final and gets its observation
//! on the same pass. It INSERTS and never overwrites: a pass that recomputed
//! would silently discard a judgment half somebody paid for.
//!
//! **The judgment half runs LATER, and outside the lock (D-07).**
//! [`judge_new`] is the pass's last step, past the point where the `runs` row
//! has committed and the ingest guard has dropped. It is never a fourth step
//! beside the drain, the walk and `outcomes`: an HTTP call held inside the lock
//! would make every hook-spawned pass in that window exit `LockHeld` and
//! archive nothing, and the archive is the work.
//!
//! **Nothing here fails a pass.** The archive is the work. An observation that
//! could not be computed is a note folded into `runs.error` - there is no log
//! file, by design - exactly as the drain's and the labeller's failures are.

//! **One door to the network.** [`net`] is the only module in either crate that
//! names an HTTP client or opens a socket, and it counts every attempt before
//! it makes it (PRIV-03, D-21). The mechanical half above reaches it never;
//! that is an assertion a test reads off the attempt log rather than a promise.
//!
//! **Nothing is asked for twice, and nothing is asked for free.** [`cost`] holds
//! OBS-06's four gates - the minimum turn count, the truncation budget, the
//! daily token budget and "one call per session" - and it is the only place a
//! judgment run learns it may not make a request.
//!
//! **One redaction boundary, and it only exists when data leaves.** [`egress`]
//! filters a request body on the DECLARED destination (D-13) and scrubs every
//! string on its way to becoming an error whatever the destination is (D-16).
//! Nothing here redacts at ingest: `.planning/PROJECT.md` bars that outright.

pub mod cost;
pub mod egress;
pub mod judgment;
pub mod mechanical;
pub mod net;
pub mod provider;

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;

use crate::config::{visible, Config};
use crate::error::Result;
use crate::observe::judgment::Verdict;

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
            // damage `terminus verify` reports, and it is not a reason to
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
/// is safe to run on every pass: `terminus observations regenerate` is the only
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
/// visible session, which is what makes a bare `terminus observations
/// regenerate` mean "all of them" rather than "none of them".
#[derive(Debug, Clone, Copy, Default)]
pub struct Selector<'a> {
    /// Sessions whose `session_meta.last_turn_at` is at or after this bound.
    ///
    /// A session carrying no `last_turn_at` is outside every bound, for the
    /// reason `terminus sessions`'s window says: a session that cannot say when
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
    /// Rows whose judgment columns [`rejudge`] replaced. Zero without a
    /// provider, which is the default and is not a failure.
    pub judged: usize,
    /// Selected rows [`rejudge`] stopped short of asking about, because the
    /// daily token budget ran out part way through the set.
    pub unasked: usize,
    /// The keys the selector chose, in the order they were rebuilt.
    ///
    /// Carried so [`rejudge`] asks about exactly this set rather than running
    /// the selection a second time: it runs after the ingest lock has dropped,
    /// and a pass in between could have changed what the selector would choose.
    pub sessions: Vec<String>,
    /// Whatever could not be recomputed or could not be judged, named and
    /// skipped - already scrubbed (D-16).
    pub notes: Vec<String>,
}

/// Recompute the mechanical half of the selected rows, in place (OBS-07).
///
/// **The only rebuild path for this table (D-02).** `observations` is out of
/// `DERIVED_TABLES`, so `reindex` never touches it and the ingest pass inserts
/// and never overwrites; this is the one place a stored fact set is replaced.
///
/// **`mechanical` and nothing else.** Not `generated_at`, not `session_id`, and
/// none of the judgment columns - those are [`rejudge`]'s, and it runs after
/// this does and after the caller's ingest lock has dropped (D-07).
/// `generated_at` dates the row as a whole -
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
/// `terminus reindex` is, and a rebuild that silently did a hundred rows of the
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
        // will not decompress is archive damage `terminus verify` reports, and
        // it is not a reason to leave every other selected row un-rebuilt. The
        // rows that were rewritten were all selected either way, so the scoping
        // claim holds whichever ones failed.
        match rewrite_one(conn, &session_key) {
            Ok(()) => out.rewritten += 1,
            Err(e) => out
                .notes
                .push(format!("{session_key}: not regenerated: {e}")),
        }
        out.sessions.push(session_key);
    }
    Ok(out)
}

/// Buy a second judgment for exactly the rows [`regenerate`] chose (OBS-07).
///
/// **The one path allowed to ask twice.** Every other caller is stopped by
/// [`cost::admits`]'s already-judged gate; this one waives it through
/// [`judgment::judge_again`], because the reason to pay for a session again is
/// that the prompt changed and `--prompt-version` is how a user says which ones.
/// Every OTHER cost control still applies: a session under [`cost::MIN_TURNS`]
/// is still not bought, and the daily token budget still stops the run.
///
/// **Nothing happens without a provider, and that is the default.** With
/// `[provider] enabled` unset or false this returns before it resolves a
/// credential or opens anything, and the stored judgment columns are left
/// exactly as they were - so a user with no provider can still rebuild facts.
///
/// **Outside the caller's ingest lock (D-07).** It opens its own store handle
/// for the same reason [`judge_new`] does: `terminus observations regenerate`
/// takes the ingest lock for the mechanical rewrite, and holding it across a
/// run of HTTP calls would make every hook-spawned pass in that window exit
/// `LockHeld` and archive nothing.
pub fn rejudge(data_dir: &Path, config: &Config, done: &mut Regenerated) {
    if !config.provider_enabled() {
        return;
    }
    let credential = match crate::credentials::resolve(config) {
        Ok(credential) => credential,
        Err(e) => {
            done.notes
                .push(egress::scrub(None, &format!("nothing was re-judged: {e}")));
            return;
        }
    };
    let store = match crate::store::Store::open(data_dir) {
        Ok(store) => store,
        Err(e) => {
            done.notes.push(egress::scrub(
                credential.as_ref(),
                &format!("nothing was re-judged: {e}"),
            ));
            return;
        }
    };
    let conn = store.conn();

    let sessions = done.sessions.clone();
    for (asked, session_key) in sessions.iter().enumerate() {
        let verdict = judgment::judge_again(conn, config, credential.as_ref(), session_key);
        let note = match verdict {
            Verdict::Stored { .. } => {
                done.judged += 1;
                continue;
            }
            // The one verdict that ends the run rather than skipping a row: the
            // budget is spent for the day, so every session after this one
            // would refuse identically. Reported as a count, because "it
            // stopped" and "there was nothing left to do" are different answers.
            Verdict::Skipped(skip @ cost::Skip::BudgetSpent { .. }) => {
                done.unasked = sessions.len() - asked;
                done.notes.push(format!(
                    "{skip}; {} selected session(s) were left unasked",
                    done.unasked
                ));
                break;
            }
            Verdict::Skipped(skip) => skip.to_string(),
            Verdict::ParseFailed { reason, .. } => {
                format!("the answer could not be used and was stored raw: {reason}")
            }
            Verdict::Failed { reason } => reason,
        };
        done.notes.push(egress::scrub(
            credential.as_ref(),
            &format!("{session_key}: {note}"),
        ));
    }

    // Said even when it is all good news: without it, a `--json` run that
    // re-judged every selected row is indistinguishable from one that never
    // had a provider to ask.
    done.notes.push(format!(
        "re-judged {} of {} selected session(s)",
        done.judged,
        sessions.len()
    ));
}

/// What one pass's judgment step did (OBS-02, D-07).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Judged {
    /// Sessions whose judgment columns this pass filled.
    pub judged: usize,
    /// Whatever could not be done, said rather than raised - and already
    /// scrubbed (D-16). These reach stderr and not `runs.error`: this step runs
    /// AFTER the `runs` row commits, so stderr is the channel it has.
    pub notes: Vec<String>,
}

/// Ask the configured provider to judge the sessions this pass just closed.
///
/// **Nothing happens while judgment is off, and that is the default (OBS-02).**
/// With `[provider] enabled` unset or false this returns before it resolves a
/// credential, before it opens the store and before it builds a request, so the
/// attempt log stays empty and a test reads that as a number (PRIV-03, D-21).
///
/// **Its own store handle, because the pass's is gone.** The caller has dropped
/// the ingest guard and the [`crate::store::Store`] by the time this runs, and
/// that ordering is the whole point of D-07. Two invocations writing
/// observations concurrently is the price, and SQLite's busy timeout is what
/// pays it: the writes are one UPDATE per session.
///
/// **Two passes never buy the same session twice.** Running outside the lock
/// means a second pass can be in this function while the first is inside its
/// HTTP call, so the candidate list below is not a claim on anything:
/// `judgment::reserve` writes the row before the request goes out and the pass
/// that loses says [`cost::Skip::InFlight`] and asks nothing. The list only
/// keeps this pass from spending its one slot on a session already in flight.
///
/// **At most [`cost::JUDGED_PER_PASS`] sessions.** See that constant: a pass is
/// a detached background process and a provider may take the full network
/// timeout to answer.
pub fn judge_new(data_dir: &Path, config: &Config) -> Judged {
    let mut out = Judged::default();
    if !config.provider_enabled() {
        return out;
    }

    // PRIV-02's refusal, and it is deliberately not a provider failure: a
    // credentials file the user must fix reads nothing like an endpoint that
    // would not answer. Scrubbed on the way out like everything else (D-16).
    let credential = match crate::credentials::resolve(config) {
        Ok(credential) => credential,
        Err(e) => {
            out.notes
                .push(egress::scrub(None, &format!("no session was judged: {e}")));
            return out;
        }
    };

    let store = match crate::store::Store::open(data_dir) {
        Ok(store) => store,
        Err(e) => {
            out.notes.push(egress::scrub(
                credential.as_ref(),
                &format!("no session was judged: {e}"),
            ));
            return out;
        }
    };
    let conn = store.conn();

    let candidates = match unjudged(conn, config) {
        Ok(candidates) => candidates,
        Err(e) => {
            out.notes.push(egress::scrub(
                credential.as_ref(),
                &format!("the sessions to judge could not be listed: {e}"),
            ));
            return out;
        }
    };

    for session_key in candidates.into_iter().take(cost::JUDGED_PER_PASS) {
        let verdict = judgment::judge(conn, config, credential.as_ref(), &session_key);
        // Nothing here is an `Err` and nothing here fails the pass (OBS-04).
        // The pass has already committed its `runs` row; the archive is done.
        let note = match verdict {
            Verdict::Stored { .. } => {
                out.judged += 1;
                continue;
            }
            Verdict::Skipped(skip) => skip.to_string(),
            Verdict::ParseFailed { reason, .. } => {
                format!("the answer could not be used and was stored raw: {reason}")
            }
            Verdict::Failed { reason } => reason,
        };
        out.notes.push(egress::scrub(
            credential.as_ref(),
            &format!("{session_key}: {note}"),
        ));
    }
    out
}

/// Visible finalized sessions with an observation row, no judgment status yet,
/// and enough turns to be worth paying for - in ingest order.
///
/// The turn-count test is here as well as in [`cost::admits`] on purpose. A
/// session under the minimum is never judged and never will be, so leaving it
/// in this list would make every pass forever pick it, refuse it and write a
/// note about it. The gate in `admits` is what enforces the rule; this is what
/// keeps the rule from being noise.
///
/// **A row another pass is asking about right now is not a candidate.** Its
/// status is a [`judgment::STATUS_JUDGING`] reservation, `judgment::reserve`
/// would refuse it, and with `cost::JUDGED_PER_PASS` at one, leaving it in the
/// list would spend this pass's single slot on a session that is already being
/// judged. A reservation older than
/// [`judgment::RESERVATION_LEASE_SECONDS`] IS a candidate: the run that took it
/// is gone, and a token nothing will ever release must not make a session
/// permanently unjudgeable.
fn unjudged(conn: &Connection, config: &Config) -> Result<Vec<String>> {
    let lapsed_before = judgment::lapsed_before(conn)?;
    let mut pending: std::collections::BTreeSet<String> = {
        let mut statement = conn.prepare(
            "SELECT o.session_key, o.status
               FROM observations o
               JOIN session_meta m ON m.session_key = o.session_key
              WHERE (o.status IS NULL OR o.status LIKE ?2)
                AND m.is_final = 1
                AND (SELECT COUNT(*) FROM turns t WHERE t.session_key = o.session_key) >= ?1",
        )?;
        let rows = statement.query_map(
            rusqlite::params![cost::MIN_TURNS as i64, judgment::RESERVED_LIKE],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )?;
        let mut out = std::collections::BTreeSet::new();
        for row in rows {
            // The lease is compared in one place rather than in the SQL of
            // every query that wants it: the token's ordering is the token's
            // business, and `lapsed_before` is where it is spelled.
            let (session_key, status) = row?;
            match status {
                None => out.insert(session_key),
                Some(status) if status < lapsed_before => out.insert(session_key),
                Some(_) => false,
            };
        }
        out
    };
    if pending.is_empty() {
        return Ok(Vec::new());
    }

    // Through `visible::sessions` for the reason `observe_new` gives: exclusion
    // is retroactive (ING-08), and this step reads a blob and sends its text to
    // a provider - which is the read that matters most.
    let mut out = Vec::new();
    for session in visible::sessions(conn, config)? {
        if pending.remove(&session.session_key) {
            out.push(session.session_key);
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
