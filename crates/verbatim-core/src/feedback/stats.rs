//! Whether injection helps, as numbers rather than as an opinion (FEED-04).
//!
//! Four counts and two sums over `decisions` and `labels`, and the whole point
//! is what is in the denominators. **Every prompt is a decision, non-fires
//! included** (D-11), so [`Stats::decisions`] is how often the injector was
//! asked and not how often it answered - a precision computed only over the
//! prompts that fired would be a number about a subset that flatters itself.
//! And [`Stats::chars_referenced`] against [`Stats::chars_injected`] is the
//! question the incumbent cannot ask at all: not "how much context did we add"
//! but "how much of it was worth adding".
//!
//! **Characters, never tokens (D-12).** The same proxy the injection budget is
//! spent in, and the output names them chars - the workspace has no tokenizer
//! and will not grow one on the cold-start path for a report.
//!
//! Read-only, and no ingest lock: reading while a pass runs must work, which is
//! what WAL is on for.

use rusqlite::Connection;

use super::label;
use crate::error::Result;

/// What the decision log adds up to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    /// Prompts that produced a record, including every one that injected
    /// nothing (D-11).
    pub decisions: usize,
    /// Turns handed to the model across all of them.
    pub injected_turns: usize,
    /// Injected turns something later referred to.
    pub hits: usize,
    /// Injected turns nothing ever referred to again.
    pub false_positives: usize,
    /// Hits over hits plus false positives, or `None` when no injected turn has
    /// been labelled at all.
    ///
    /// `None` and not `0.0`: an archive whose sessions are all still open has
    /// no precision yet, and reporting zero there would read as "injection
    /// never helps" - the exact opposite of "nothing has been measured".
    pub precision: Option<f64>,
    /// Decisions where the model went to recall for something the prompt had
    /// named and the injector had declined to give it. Reads zero on real
    /// history today (D-13).
    pub misses: usize,
    /// Decisions that spent characters and earned no hit.
    pub wasted_budget: usize,
    /// Characters injected across every decision - the emitted total each one
    /// recorded, not the sum of its turns' shares.
    pub chars_injected: i64,
    /// Of those, the characters carried by turns that turned out to be hits.
    ///
    /// The per-turn counts come off the decision record, so this is the
    /// injection's own accounting rather than a second measurement of the same
    /// text.
    pub chars_referenced: i64,
}

/// The decision log, added up.
///
/// Four statements rather than one join: the label counts are a `GROUP BY` over
/// one table, the decision counts are aggregates over another, and forcing them
/// into a single query would multiply rows through the join and make every
/// count wrong in a way that still returns numbers.
pub fn stats(conn: &Connection) -> Result<Stats> {
    let (decisions, chars_injected) = conn.query_row(
        "SELECT count(*), coalesce(sum(chars_injected), 0) FROM decisions",
        [],
        |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)?)),
    )?;

    // `json_array_length` over the same guarded expression the labeller walks,
    // so a row whose payload would not serialize counts zero turns rather than
    // failing the report.
    let injected_turns: i64 = conn.query_row(
        &format!(
            "SELECT coalesce(sum(json_array_length({})), 0) FROM decisions d",
            INJECTED
        ),
        [],
        |r| r.get(0),
    )?;

    let mut counted =
        conn.prepare("SELECT label, count(*) FROM labels GROUP BY label ORDER BY label")?;
    let mut stats = Stats {
        decisions,
        injected_turns: injected_turns as usize,
        chars_injected,
        ..Stats::default()
    };
    let rows = counted
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<rusqlite::Result<Vec<(String, usize)>>>()?;
    for (name, count) in rows {
        match name.as_str() {
            label::HIT => stats.hits = count,
            label::FALSE_POSITIVE => stats.false_positives = count,
            label::MISS => stats.misses = count,
            label::WASTED_BUDGET => stats.wasted_budget = count,
            // A label no build of this product writes is not folded into one of
            // the four; the closed set is `label::LABELS`.
            _ => {}
        }
    }
    drop(counted);

    let judged = stats.hits + stats.false_positives;
    stats.precision = (judged > 0).then(|| stats.hits as f64 / judged as f64);
    stats.chars_referenced = referenced(conn)?;
    Ok(stats)
}

/// The same guarded read of a decision's injected list the labeller uses.
const INJECTED: &str = "CASE WHEN json_valid(d.injected) THEN d.injected ELSE '[]' END";

/// The characters carried by the injected turns that earned a [`label::HIT`].
///
/// Joined back through the decision's own injected list on `turn_id`, because
/// that list is the only place a per-turn character count exists: a `hit` row
/// names the turn and the labeller has no reason to copy what it cost.
fn referenced(conn: &Connection) -> Result<i64> {
    let sql = format!(
        "SELECT coalesce(sum(json_extract(i.value, '$.chars')), 0)
           FROM labels l, decisions d, json_each({INJECTED}) i
          WHERE d.id = l.decision_id
            AND l.label = '{}'
            AND json_extract(i.value, '$.turn_id') = l.turn_id",
        label::HIT
    );
    let chars: i64 = conn.query_row(&sql, [], |r| r.get(0))?;
    Ok(chars)
}
