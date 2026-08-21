//! What a decision turned out to be worth (FEED-02).
//!
//! Four labels, and every one of them is a SQL join over rows the store already
//! holds - `decisions`, `entities`, `turns`, `session_meta`. **No blob is
//! decompressed and no transcript is re-read** (D-07). That is a budget rather
//! than a preference: real sessions are p90 1.03 MB and reach 10.1 MB
//! compressed, this runs inside the ingest lock, and a steady-state pass is 92
//! ms - so a labelling step that decompressed a session per decision would turn
//! the cheapest pass in the product into a per-session decompression sweep.
//!
//! **The proxy.** A decision injected some turns; if a turn that came *after* it
//! in the same session names one of the same things that turn named, the model
//! used what it was given. Measured 2026-08-20 over 150 transcripts: 41.1% of a
//! prompt's path mentions recur later in the same session, so the proxy
//! separates the two populations. `DESIGN-BRIEF.md:298` calls it a weak proxy
//! and it is one - which is exactly why the auto-tuner is gated behind evidence
//! from these labels rather than shipped on top of them.
//!
//! **Incremental by construction.** Every statement carries its own "and there
//! is no such row yet" clause, so a second pass over the same store writes
//! nothing. A label is never revised either: a session is labelled once it is
//! final, and a final session is not gaining turns.

use rusqlite::Connection;

use crate::error::Result;

/// The injected turn was used: something it named came up again afterwards.
pub const HIT: &str = "hit";
/// The injected turn was never referred to again.
pub const FALSE_POSITIVE: &str = "false positive";
/// The model went to recall for something the prompt had named and the
/// injector had declined to give it.
pub const MISS: &str = "miss";
/// Characters were spent and no injected turn earned a [`HIT`].
pub const WASTED_BUDGET: &str = "wasted budget";

/// Every label this module writes, for a report that needs the closed set.
pub const LABELS: [&str; 4] = [HIT, FALSE_POSITIVE, MISS, WASTED_BUDGET];

/// The entity kinds a reference is counted in.
///
/// `tool` is deliberately absent. A `tool` entity is `Read` or `Bash` - a value
/// that recurs in very nearly every session - so counting it would make a later
/// match structurally certain and every decision in a busy session a `hit`,
/// which is precisely the failure D-04 warns about. The four that remain name
/// something specific: a file, an identifier, a failure, a program.
const REFERENCE_KINDS: &str = "('path', 'symbol', 'error', 'command')";

/// The decisions this step may consider at all.
///
/// A decision belongs to a session only through `session_id`, never a session
/// key (D-04) - the injector cannot know which transcript file it is inside.
/// Joining that way also, deliberately, admits a sidecar's turns as downstream
/// evidence: 812 sidecar files report their parent's `sessionId`, and work an
/// agent did after an injection is still work that followed it.
const ELIGIBLE: &str = "\
    d.session_id IS NOT NULL
    AND d.ts IS NOT NULL
    AND EXISTS (
        SELECT 1 FROM session_meta f
         WHERE f.session_id = d.session_id AND f.is_final = 1
    )";

/// A decision's injected list, as JSON a table function can walk.
///
/// A column that will not parse yields an empty list rather than an error: the
/// drain writes `NULL` for a payload it could not serialize, and one unreadable
/// row of a log must not fail the pass that found it.
const INJECTED: &str = "CASE WHEN json_valid(d.injected) THEN d.injected ELSE '[]' END";

/// The same, for the spellings the prompt was asked about.
const SPELLINGS: &str = "CASE WHEN json_valid(d.spellings) THEN d.spellings ELSE '[]' END";

/// The stored timestamp shape, produced the same way every other one is.
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// Label every eligible decision, reporting what was written by label.
///
/// One transaction for the three statements, because they read each other:
/// `wasted budget` is defined against the `hit` rows the first statement just
/// wrote, so a half-applied batch would be a decision labelled wasteful on the
/// strength of labels that rolled back.
pub fn label(conn: &mut Connection) -> Result<Vec<(String, usize)>> {
    // The high-water mark, so "what this pass wrote" is a range of ids rather
    // than a sum of `execute` counts - one INSERT below produces two different
    // labels and cannot report them apart.
    let before: i64 =
        conn.query_row("SELECT coalesce(max(id), 0) FROM labels", [], |r| r.get(0))?;

    let tx = conn.transaction()?;
    tx.execute(&hit_or_false_positive(), [])?;
    tx.execute(&miss(), [])?;
    tx.execute(&wasted_budget(), [])?;
    tx.commit()?;

    let mut counted = conn.prepare(
        "SELECT label, count(*) FROM labels WHERE id > ?1 GROUP BY label ORDER BY label",
    )?;
    let counts = counted
        .query_map([before], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<rusqlite::Result<Vec<(String, usize)>>>()?;
    Ok(counts)
}

/// One row per injected turn: did anything it named come up again?
///
/// `json_each` over the decision's own injected list is what makes this "still
/// entirely in SQL" while operating per turn - the alternative is a Rust loop
/// issuing a statement per decision, inside the ingest lock.
///
/// The join is driven from the injected turn's entities outward: its own rows
/// (by `idx_entities_turn`), then every later turn naming the same
/// `(kind, value_norm)` (by `idx_entities_lookup`), rather than scanning the
/// downstream turns and asking about each.
fn hit_or_false_positive() -> String {
    format!(
        "INSERT INTO labels (decision_id, turn_id, label, labeled_at)
         SELECT d.id,
                json_extract(i.value, '$.turn_id'),
                CASE WHEN EXISTS (
                    SELECT 1
                      FROM entities gave
                      JOIN entities used
                        ON used.kind = gave.kind
                       AND used.value_norm = gave.value_norm
                      JOIN turns t        ON t.id = used.turn_id
                      JOIN session_meta m ON m.session_key = t.session_key
                     WHERE gave.turn_id = json_extract(i.value, '$.turn_id')
                       AND gave.kind IN {REFERENCE_KINDS}
                       AND m.session_id = d.session_id
                       AND t.ts > d.ts
                ) THEN '{HIT}' ELSE '{FALSE_POSITIVE}' END,
                {NOW}
           FROM decisions d, json_each({INJECTED}) i
          WHERE {ELIGIBLE}
            AND NOT EXISTS (
                SELECT 1 FROM labels l
                 WHERE l.decision_id = d.id
                   AND l.turn_id = json_extract(i.value, '$.turn_id')
            )"
    )
}

/// One row per decision: the model asked recall for something this prompt had
/// named and this decision did not hand over.
///
/// Reached through `turns.tool_name` and the recall call's own entities, which
/// is why [`crate::index::entity`] extracts a `recall_search` query at all -
/// without those rows there is nothing in the store that says what was searched
/// for, and D-07 bars opening the blob to find out. Zero such calls exist across
/// the 3,217 transcripts measured on 2026-08-20 (D-13), so on real history today
/// this reads zero and only a fixture moves it.
///
/// The sought value is computed once, in the `SELECT`, and the outer query keeps
/// the rows where there was one - so the condition and the `detail` it records
/// cannot say different things. `min` because a decision may have missed several
/// and a label has one detail; the choice is arbitrary but it is stable.
fn miss() -> String {
    format!(
        "INSERT INTO labels (decision_id, turn_id, label, detail, labeled_at)
         SELECT decision_id, NULL, '{MISS}', sought, {NOW}
           FROM (
             SELECT d.id AS decision_id,
                    (SELECT min(sought.value_norm)
                       FROM entities sought
                       JOIN turns t        ON t.id = sought.turn_id
                       JOIN session_meta m ON m.session_key = t.session_key
                      WHERE m.session_id = d.session_id
                        AND t.ts > d.ts
                        AND (t.tool_name LIKE '%recall\\_search' ESCAPE '\\'
                          OR t.tool_name LIKE '%recall\\_get' ESCAPE '\\')
                        AND EXISTS (
                            SELECT 1 FROM json_each({SPELLINGS}) s
                             WHERE s.value = sought.value_norm
                        )
                        AND NOT EXISTS (
                            SELECT 1
                              FROM json_each({INJECTED}) i
                              JOIN entities gave
                                ON gave.turn_id = json_extract(i.value, '$.turn_id')
                             WHERE gave.value_norm = sought.value_norm
                        )
                    ) AS sought
               FROM decisions d
              WHERE {ELIGIBLE}
                AND NOT EXISTS (
                    SELECT 1 FROM labels l
                     WHERE l.decision_id = d.id AND l.label = '{MISS}'
                )
           )
          WHERE sought IS NOT NULL"
    )
}

/// One row per decision: characters were spent and nothing came of them.
///
/// Read off the `hit` rows [`hit_or_false_positive`] just wrote rather than
/// recomputed, which is why the three statements share a transaction. The
/// `turn_id IS NOT NULL` clause is what stops a decision being judged before it
/// has been judged: it says this decision's injected turns have been labelled at
/// all, so "none of them is a hit" is an answer rather than an absence.
fn wasted_budget() -> String {
    format!(
        "INSERT INTO labels (decision_id, turn_id, label, detail, labeled_at)
         SELECT d.id, NULL, '{WASTED_BUDGET}', CAST(d.chars_injected AS TEXT), {NOW}
           FROM decisions d
          WHERE {ELIGIBLE}
            AND d.chars_injected > 0
            AND EXISTS (
                SELECT 1 FROM labels l
                 WHERE l.decision_id = d.id AND l.turn_id IS NOT NULL
            )
            AND NOT EXISTS (
                SELECT 1 FROM labels l
                 WHERE l.decision_id = d.id AND l.label = '{HIT}'
            )
            AND NOT EXISTS (
                SELECT 1 FROM labels l
                 WHERE l.decision_id = d.id AND l.label = '{WASTED_BUDGET}'
            )"
    )
}
