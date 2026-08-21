//! Scoring the logged history again, under different rules (FEED-03).
//!
//! Every `decisions` row carries what its prompt asked about, when it was
//! asked, and how much of the archive existed at the time. That is enough to
//! ask the counterfactual: with these thresholds, which turns would that prompt
//! have been given, and what would those turns have been labelled? The answer
//! diffed against the labels actually stored is what makes a retrieval change
//! testable against history rather than shipped on a hunch.
//!
//! **Bounded to the index as it stood (D-10).** Each decision's search is
//! capped by [`Request::before_turn_id`] derived from the watermark the prompt
//! recorded, so a turn archived after the prompt was submitted can never be
//! offered to it. Without that, every replay would certify on hindsight: the
//! turns that answer a question tend to be written after it is asked.
//!
//! **Suppressions are re-applied, never re-derived.** INJ-04's three refusals
//! are read out of a session state file that is disposable scratch - capped,
//! overwritten every prompt, deleted or long stale by the time a replay runs -
//! so this re-applies the refusals the decision itself recorded. Re-deriving
//! them would attribute unreproducible session state to the rule under test,
//! which is the one thing a diff must not do.
//!
//! **Reads only.** Every statement here is a `SELECT`; the caller opens the
//! store and `verbatim replay` opens it with `Store::open_read_only`, so
//! "without touching the live store" is enforced by connection flags rather
//! than by discipline (D-15).
//!
//! **What it cannot reconstruct.** The compacted pool (INJ-05) is not replayed:
//! whether the one prompt after a compaction was in force, and which turns had
//! fallen out of context, live in the decision FILE and not in the `decisions`
//! table, so a replay would have to re-derive them from session state - exactly
//! what the paragraph above rules out. Those decisions are replayed through the
//! ordinary ranked window, which is what they would have got had the compaction
//! not happened.

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::label;
use crate::config::Config;
use crate::error::Result;
use crate::inject::prompt::{self, Thresholds};
use crate::recall::search::{self, Request};
use crate::recall::Scope;
use crate::store::{schema, Store};

/// One label's count before and after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Movement {
    /// One of [`label::LABELS`].
    pub label: &'static str,
    /// How many of this label the store holds today.
    pub old: usize,
    /// How many the replayed rules produce.
    pub new: usize,
}

/// What one replay found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replayed {
    /// Decisions scored: the ones the labeller was allowed to judge, and no
    /// others - see [`label::ELIGIBLE`].
    pub decisions: usize,
    /// One entry per label in [`label::LABELS`] order, present even at zero.
    /// A per-label diff with holes in it would read as "no such label" rather
    /// than "none of those".
    pub labels: Vec<Movement>,
    /// The decisions whose label set moved, in id order. The ids are the point:
    /// a diff that said only "four more hits" cannot be followed back into the
    /// prompts that produced them.
    pub changed: Vec<i64>,
}

/// One logged decision, as replay reads it back.
struct Logged {
    id: i64,
    cwd: Option<String>,
    prompt: String,
    watermark_session_no: Option<i64>,
    /// The turns this prompt refused, as the decision recorded them.
    refused: BTreeSet<i64>,
}

/// Re-score every eligible decision under `thresholds` and diff the labels.
///
/// Deterministic by construction: the decisions are walked in id order, the
/// search's order is total (`recall::search::TAIL`), and nothing here reads a
/// clock. Two replays of an unchanged store under equal thresholds produce
/// equal output, which is what makes a diff evidence rather than an anecdote.
pub fn replay(store: &Store, config: &Config, thresholds: &Thresholds) -> Result<Replayed> {
    let conn = store.conn();
    let logged = eligible(conn)?;

    let mut totals: Vec<usize> = vec![0; label::LABELS.len()];
    let mut stored: Vec<usize> = vec![0; label::LABELS.len()];
    let mut changed: Vec<i64> = Vec::new();

    for decision in &logged {
        let new = recomputed(conn, config, thresholds, decision)?;
        let old = counts(conn, decision.id)?;
        if new != old {
            changed.push(decision.id);
        }
        for index in 0..label::LABELS.len() {
            totals[index] += new[index];
            stored[index] += old[index];
        }
    }

    Ok(Replayed {
        decisions: logged.len(),
        labels: label::LABELS
            .iter()
            .enumerate()
            .map(|(index, name)| Movement {
                label: name,
                old: stored[index],
                new: totals[index],
            })
            .collect(),
        changed,
    })
}

/// The decisions a replay may score: exactly the ones the labeller judged.
///
/// [`label::ELIGIBLE`] rather than every row, and the same constant rather than
/// a second copy of the predicate. A decision of a session that is not final
/// carries no stored label, so scoring it would diff a recomputed label against
/// an absence and report movement for every open session in the archive.
fn eligible(conn: &Connection) -> Result<Vec<Logged>> {
    let sql = format!(
        "SELECT d.id, d.cwd, coalesce(d.prompt, ''), d.watermark_session_no,
                CASE WHEN json_valid(d.suppressed) THEN d.suppressed ELSE '[]' END
           FROM decisions d
          WHERE {}
          ORDER BY d.id",
        label::ELIGIBLE
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok(Logged {
                id: row.get(0)?,
                cwd: row.get(1)?,
                prompt: row.get(2)?,
                watermark_session_no: row.get(3)?,
                refused: refused(&row.get::<_, String>(4)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<Logged>>>()?;
    Ok(rows)
}

/// The turn ids out of a decision's recorded suppression list.
///
/// Read as JSON values rather than as `inject::state::Suppressed`, because the
/// reason is not wanted here and a reason spelling this build does not know
/// would otherwise discard the whole list - turning a refusal into a turn the
/// replay happily injects. An unreadable list refuses nothing, which is the
/// same answer the drain gives a payload it could not serialize.
fn refused(document: &str) -> BTreeSet<i64> {
    serde_json::from_str::<Vec<serde_json::Value>>(document)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| entry.get("turn_id").and_then(serde_json::Value::as_i64))
        .collect()
}

/// The labels this decision would carry under `thresholds`, counted by label.
fn recomputed(
    conn: &Connection,
    config: &Config,
    thresholds: &Thresholds,
    decision: &Logged,
) -> Result<Vec<usize>> {
    let (spellings, injected) = would_inject(conn, config, thresholds, decision)?;

    let mut counts: Vec<usize> = vec![0; label::LABELS.len()];
    let mut any_hit = false;
    for turn_id in &injected {
        if label::referenced(conn, decision.id, *turn_id)? {
            any_hit = true;
            counts[index_of(label::HIT)] += 1;
        } else {
            counts[index_of(label::FALSE_POSITIVE)] += 1;
        }
    }

    // The spellings are the REPLAYED ones: `--max-candidates` changes what a
    // prompt asked the archive about, and a miss is defined against what was
    // asked and withheld.
    let asked = serde_json::to_string(&spellings).unwrap_or_else(|_| "[]".to_owned());
    let handed = handed_over(&injected);
    if label::sought(conn, decision.id, &asked, &handed)?.is_some() {
        counts[index_of(label::MISS)] += 1;
    }

    // "Characters were spent and nothing came of them". Ingest reads
    // `chars_injected > 0`; a replay has no rendered text to count, and the two
    // agree anyway - `prompt::render` returns nothing for an empty hit list and
    // a non-empty one always costs characters.
    if !injected.is_empty() && !any_hit {
        counts[index_of(label::WASTED_BUDGET)] += 1;
    }
    Ok(counts)
}

/// The turns this decision's prompt would be given today, under `thresholds`,
/// beside the spellings it would ask about.
///
/// The same four steps the live arm takes, through the same functions
/// (`prompt::candidates`, `prompt::query_of`, `prompt::eligible`,
/// `prompt::capped`) and with the same request shape - candidates, the scope of
/// the stored `cwd`, excerpts off. A second implementation of any of them would
/// make the diff a measurement of the replay engine.
fn would_inject(
    conn: &Connection,
    config: &Config,
    thresholds: &Thresholds,
    decision: &Logged,
) -> Result<(Vec<String>, Vec<i64>)> {
    let Some(cwd) = decision.cwd.as_deref().filter(|cwd| !cwd.is_empty()) else {
        // A record with no working directory has no project to be scoped to,
        // which is where the live arm returns as well.
        return Ok((Vec::new(), Vec::new()));
    };
    let (spellings, candidates) =
        prompt::candidates(&decision.prompt, Some(cwd), thresholds.max_candidates);
    if candidates.is_empty() {
        // The prompt named nothing structural: the live arm never opened the
        // store for it, and neither does this.
        return Ok((spellings, Vec::new()));
    }

    let request = Request::new(
        prompt::query_of(&decision.prompt, Some(cwd)),
        Scope::Directory(cwd.into()),
    )
    .limit(thresholds.ranked)
    .candidates(candidates)
    .excerpts(false)
    .before_turn_id(bound(decision.watermark_session_no));

    let hits = search::run(conn, config, &request)?.hits;
    let admitted = prompt::eligible(hits, thresholds);
    let injected = prompt::capped(admitted, &decision.refused, thresholds.max_turns)
        .iter()
        .map(|hit| hit.turn_id)
        .collect();
    Ok((spellings, injected))
}

/// A watermark as the exclusive turn-id bound it implies (D-10).
///
/// `turn_id(watermark + 1, 0)` is the first id the session after the watermark
/// could take, so every turn archived up to and including it sits below.
/// `None` for a decision with no watermark and for one whose watermark would
/// overflow the id space: an unbounded replay of one row is a wrong answer
/// worth having over a panic that takes the whole command.
fn bound(watermark: Option<i64>) -> Option<i64> {
    let watermark = watermark?;
    // `turn_id` asserts on both ends, and an assert here is a panic inside a
    // read-only report over a log nothing decides on yet.
    let ceiling = i64::MAX >> schema::TURN_SEQ_BITS;
    if watermark < 0 || watermark >= ceiling {
        return None;
    }
    Some(schema::turn_id(watermark + 1, 0))
}

/// The would-inject set in the shape [`label::sought`] reads: the same document
/// the `injected` column carries, so one JSON walk serves both callers. The
/// character counts are absent because nothing here can know them and nothing
/// there reads them.
fn handed_over(injected: &[i64]) -> String {
    let document: Vec<serde_json::Value> = injected
        .iter()
        .map(|turn_id| serde_json::json!({"turn_id": turn_id}))
        .collect();
    serde_json::to_string(&document).unwrap_or_else(|_| "[]".to_owned())
}

/// The labels one decision carries today, counted the same way.
fn counts(conn: &Connection, decision_id: i64) -> Result<Vec<usize>> {
    let mut statement =
        conn.prepare("SELECT label, count(*) FROM labels WHERE decision_id = ?1 GROUP BY label")?;
    let mut counts: Vec<usize> = vec![0; label::LABELS.len()];
    let rows = statement
        .query_map([decision_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<rusqlite::Result<Vec<(String, usize)>>>()?;
    for (name, count) in rows {
        // A label no build of this product writes is left out rather than
        // counted somewhere arbitrary; the closed set is `label::LABELS`.
        if let Some(index) = label::LABELS.iter().position(|known| *known == name) {
            counts[index] += count;
        }
    }
    Ok(counts)
}

/// Where one label sits in [`label::LABELS`].
///
/// Panics on a label not in that array, which is unreachable: the only
/// arguments are the four constants the array is built from, and a fifth added
/// without extending the array is a bug at compile time in every other respect.
fn index_of(label: &str) -> usize {
    label::LABELS
        .iter()
        .position(|known| *known == label)
        .expect("every label this module counts is in LABELS")
}
