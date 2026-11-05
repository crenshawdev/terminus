//! Ranked search: the `turns_fts` match joined back to the rows that say which
//! turn, in which session, of which project it was.
//!
//! Both front ends, `terminus search` and `recall_search`, sit on this search,
//! so there is one definition of relevance rather than two that drift.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::Result;
use crate::observe::egress::Redaction;
use crate::recall::excerpt;
use crate::recall::query::EntityMatch;
use crate::recall::scope::{self, Reason, Scope};
use crate::recall::Query;

/// The most hits one search may return, whatever the caller asks for.
///
/// A caller may lower it and may not raise it. The MCP server needs a
/// server-side budget it does not have to trust a client to respect,
/// and every hit costs an excerpt read out of its session's block rows: the
/// blocks its turn covers, fetched and decompressed.
pub const MAX_RESULTS: usize = 50;

/// What a caller gets when it does not say.
///
/// Injection is precision-first and a terminal page is short; a caller that
/// wants more asks for more, up to [`MAX_RESULTS`].
pub const DEFAULT_RESULTS: usize = 20;

/// The filters: everything that narrows a search without changing what it is
/// asking for.
///
/// Each one is independently optional and they combine conjunctively, so a
/// default [`Filters`] narrows nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    /// `turns.tool_name`. Exactly one `tool_use` block was measured on every
    /// tool-bearing record of a 19,809-turn sample, so the column is the whole
    /// tool filter and not a first guess at one.
    pub tool: Option<String>,
    /// `turns.record_type`: `user`, `assistant`, `system`, `attachment`.
    pub kind: Option<String>,
    /// Only what a person typed: `turns.is_typed = 1` in a main session. `kind`
    /// can't answer this, Claude Code writes tool results and notices as `user`
    /// records too, and a subagent's prompts pass `is_typed` but Claude wrote them.
    /// A script's prompt still gets through: nothing stored says how a session started.
    pub typed: bool,
    /// Turns carrying any of these in the `paths` table. Structural, not
    /// textual: a turn that merely names a path in prose has no `paths` row and
    /// is not a match here.
    pub paths: Vec<String>,
    /// Inclusive lower bound on `turns.ts`.
    pub since: Option<String>,
    /// Inclusive upper bound on `turns.ts`.
    pub until: Option<String>,
}

/// One search, as asked for.
#[derive(Debug, Clone)]
pub struct Request {
    pub query: Query,
    /// Which project's turns may come back. Defaults to the project the caller
    /// is standing in, which is the only default that makes `terminus search`
    /// answer about the work in front of the user.
    pub scope: Scope,
    pub filters: Filters,
    /// How many hits the caller wants. Silently clamped to [`MAX_RESULTS`] -
    /// see [`Request::effective_limit`].
    pub limit: usize,
    /// Whole spellings to match ANY of, instead of every token of
    /// [`Request::query`]. Empty for every caller but injection.
    ///
    /// A `terminus search` query is a search string: the user typed the terms
    /// they want and conjoining them is what "search" means. A prompt is a
    /// SENTENCE, and conjoining every word of it asks the archive for a turn
    /// that repeats the sentence - measured, not assumed: `who edited
    /// docs/RETRY.md yesterday` returns zero hits against a store whose `Read`
    /// call opened exactly that file, because no turn also says `who`,
    /// `yesterday` and `edited`. Injection built on that expression would be
    /// permanent silence with every fixture green.
    ///
    /// So injection hands over the spellings it wants candidates FOR - the
    /// resolved paths and the identifier-shaped tokens of the prompt - each
    /// conjoined within itself and the lot of them disjoined, and the prose
    /// stays in [`Request::query`], where it still weighs and excerpts what
    /// comes back. Nothing loosens for either front end: an empty list is the
    /// expression this module always built.
    ///
    /// This is a recall net and not a decision. What may actually be injected
    /// is decided structurally afterwards, on [`Hit::entity_match`] and
    /// [`Hit::entity_count`], so a candidate that matched only as free text
    /// costs a row here and injects nothing.
    pub candidates: Vec<Query>,
    /// Whether every returned hit gets an excerpt cut from its session's block rows.
    ///
    /// On for both front ends, because a hit with no text under it is not a
    /// search result. Off for injection until its threshold has fired: an
    /// excerpt materializes a WHOLE compressed session - p50 273 KB, p90
    /// 1.03 MB, p99 2.6 MB, max 10.1 MB uncompressed over the real corpus -
    /// and the prompt path stays silent most of the time by design, so paying
    /// for text nobody will read is the one cost it must not carry. The caller
    /// attaches them itself, through [`excerpt::attach`], for the at-most-three
    /// turns that fired.
    pub excerpts: bool,
    /// An exclusive upper bound on `turns.id`: only turns strictly below it may
    /// come back. `None` for every caller but `terminus replay`, and then this
    /// narrows nothing.
    ///
    /// **This is how "the index as it stood" is reconstructed.** A turn
    /// id is `session_no << TURN_SEQ_BITS | turn_seq`
    /// (`store::schema::turn_id`), and `sessions.session_no` is assigned in
    /// ingest order and survives a rebuild - so every turn of every session up
    /// to and including a watermark sits below `turn_id(watermark + 1, 0)` and
    /// every later-ingested session sits above it. That makes one integer per
    /// decision a sound bound on what was archived when the decision was taken,
    /// with no database snapshot anywhere.
    ///
    /// **Not `turns.ts`.** A transcript timestamp says when a turn HAPPENED, not
    /// when it was indexed: a backfill run today archives turns stamped last
    /// month, and a replay filtering on `ts` would score a historic prompt
    /// against rows that did not exist when it was submitted - which is exactly
    /// the hindsight replay exists to rule out.
    pub before_turn_id: Option<i64>,
}

impl Request {
    /// A request scoped to the process's own working directory.
    ///
    /// Fallible only because reading the working directory is: a process whose
    /// `cwd` was deleted out from under it has no project to be standing in.
    pub fn here(query: Query) -> Result<Self> {
        Ok(Request::new(query, Scope::current_directory()?))
    }

    pub fn new(query: Query, scope: Scope) -> Self {
        Request {
            query,
            scope,
            filters: Filters::default(),
            limit: DEFAULT_RESULTS,
            candidates: Vec::new(),
            excerpts: true,
            before_turn_id: None,
        }
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Match any of these whole spellings - see [`Request::candidates`].
    pub fn candidates(mut self, candidates: Vec<Query>) -> Self {
        self.candidates = candidates;
        self
    }

    /// Cut an excerpt for every hit, or none - see [`Request::excerpts`].
    pub fn excerpts(mut self, excerpts: bool) -> Self {
        self.excerpts = excerpts;
        self
    }

    pub fn filters(mut self, filters: Filters) -> Self {
        self.filters = filters;
        self
    }

    /// Score only against turns archived before a watermark - see
    /// [`Request::before_turn_id`].
    pub fn before_turn_id(mut self, bound: Option<i64>) -> Self {
        self.before_turn_id = bound;
        self
    }

    /// The limit actually applied.
    ///
    /// Clamping rather than rejecting: an over-large `limit` is a client being
    /// optimistic, not a caller error, and it gets an answer rather than a
    /// refusal.
    pub fn effective_limit(&self) -> usize {
        self.limit.min(MAX_RESULTS)
    }
}

/// One `(kind, value_norm)` pair of the `entities` table that a query matched
/// on a turn.
///
/// The pair itself rather than a count of pairs, because the decision log has
/// to be able to say WHICH rule produced a candidate: "matched 2 entities"
/// cannot attribute a replayed label change to a path rule versus a symbol
/// rule, which is the one question offline replay exists to answer. `serde` is
/// derived here and on nothing else in this module: a matched pair is the one
/// part of a hit that outlives the query, because the prompt path writes it
/// into a decision record and ingest reads it back.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MatchedEntity {
    /// The `entities.kind`: `path`, `command`, `error`, `symbol`, `tool`.
    pub kind: String,
    /// The `entities.value_norm` as stored - the normalized spelling, never the
    /// user's.
    pub value: String,
}

/// One matching turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub turn_id: i64,
    pub session_key: String,
    pub record_type: String,
    pub ts: Option<String>,
    /// The project key ingest resolved for this turn's session, or `None` when
    /// the session has no meta row at all.
    pub project: Option<String>,
    /// A subagent turn: its session carries a `parent_session_key`.
    pub sidechain: bool,
    /// What the exact-match half of the ranking added to [`Hit::relevance`].
    /// Zero when the query matched this turn only as free text.
    ///
    /// Reported rather than folded away: it is the difference between "this
    /// turn ran that command" and "this turn mentions that word", and a caller
    /// that cannot see it cannot show it or test it.
    pub entity_score: f64,
    /// How the query matched this turn's entities at its strongest, or `None`
    /// when it matched no entity on this turn at all.
    ///
    /// [`EntityMatch::Exact`] outranks [`EntityMatch::Covered`]: a turn
    /// carrying one value the query asked for whole reports `Exact` however
    /// many other values it merely covered. This is the fact
    /// [`Hit::entity_score`] cannot carry - a scalar built from an IDF times a
    /// weight cannot say which of the two produced it, and the injection's
    /// structural threshold is written on the kind rather than on a score
    /// cutoff, since bm25 scores are not comparable across queries.
    pub entity_match: Option<EntityMatch>,
    /// How many DISTINCT `(kind, value_norm)` pairs on this turn the query
    /// matched.
    ///
    /// Distinct, and that is load-bearing: one path named by three tool calls
    /// of one turn is one piece of evidence, and the injection threshold's
    /// second condition is about independent entities co-occurring in one
    /// turn - so counting rows would let a
    /// single repeated value pass a threshold meant for two different facts.
    pub entity_count: usize,
    /// The distinct pairs [`Hit::entity_count`] counts, in `(kind, value)`
    /// order. Empty for a hit the query reached only as free text.
    ///
    /// Reported beside the count rather than instead of it: the count is what
    /// the injection threshold reads on every prompt and the pairs are what the
    /// decision log records, and `entity_count` must always equal
    /// `matched_on.len()`.
    pub matched_on: Vec<MatchedEntity>,
    /// Higher is a better match: the negated bm25 (see [`HEAD`]) plus
    /// [`Hit::entity_score`].
    pub relevance: f64,
    /// The matched text, cut from the session's block rows. Empty only when the
    /// archive would not give it up - see [`crate::recall::excerpt`].
    pub excerpt: String,
    /// The turn's `turns.is_typed`: `Some(false)` for a `user` row the harness
    /// wrote, such as a tool result, `Some(true)` for one a person typed, and
    /// `None` where nothing derived it. Carried so the injection paths can drop
    /// tool results without a second query (see
    /// [`crate::inject::prompt::quotable`]).
    pub is_typed: Option<bool>,
}

/// What a search answered.
///
/// An empty `hits` with a `reason` is how an empty answer is given: a search
/// that could not match anything says so, rather than throwing or looking like
/// an archive with nothing in it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Response {
    pub hits: Vec<Hit>,
    pub reason: Option<Reason>,
    /// Block rows the excerpts fetched and blocks they decompressed. An
    /// instrument, not an answer: see [`crate::recall::excerpt::Reads`].
    pub reads: excerpt::Reads,
}

/// The projection and the ordering, with the one place the bm25 sign is fixed.
///
/// `bm25()` returns a NEGATIVE number and more negative is a better match. The
/// negation happens here, once, in the projection, and every ordering below is
/// written against the negated value - because a sign error here ranks the
/// worst match first and every test that only checks "hits came back" still
/// passes.
///
/// The join to `session_meta` is a LEFT join on purpose. An inner join would
/// drop every turn of a session that has no meta row, which is the damage
/// `config::visible::sessions` deliberately keeps visible: a session archived
/// without meta is a bug to see, not a bug to hide.
const HEAD: &str = "
    SELECT t.id,
           t.session_key,
           t.record_type,
           t.ts,
           m.project,
           m.parent_session_key IS NOT NULL AS sidechain,
           -bm25(turns_fts) AS relevance,
           t.is_typed
    FROM turns_fts
    JOIN turns t ON t.id = turns_fts.rowid
    LEFT JOIN session_meta m ON m.session_key = t.session_key
    WHERE turns_fts MATCH ?
";

/// The order is total, so two runs over an unchanged store agree - injection
/// reads rank 1..3 and a nondeterministic tie would make its threshold fire on
/// a different turn each time. Relevance first, then the rule that a sidechain
/// turn sorts last at equal score, then the newer turn, then the lower id.
///
/// This orders the CANDIDATES. The entity weighting below then moves rows
/// within that set and the same comparison is applied again in Rust, which is
/// why it is written twice: SQL cannot see a score that is computed from a
/// second table's document frequencies.
const TAIL: &str = "
    ORDER BY relevance DESC, sidechain ASC, t.ts DESC, t.id ASC
    LIMIT ?
";

/// How many turns are ranked before the caller's limit is applied.
///
/// The entity weighting can only re-order rows it was given, so a pool no
/// larger than the limit would mean a turn that carries the exact command the
/// user asked for could never climb past a turn that merely says the word.
/// Four times the maximum is a bounded read - the rows are eight small columns
/// and no blob is touched until after the truncation - and it is the whole of
/// what a re-rank can reach.
const CANDIDATE_POOL: usize = MAX_RESULTS * 4;

/// Run one search.
///
/// The order is fixed. The time bounds are checked first, through
/// [`checked_bound`], so a bound that names no instant is
/// [`crate::error::Error::InvalidTimeFilter`] wherever the caller stands (#24).
/// Then the scope is resolved, and a scope that leaves nothing to search -
/// everything excluded, nothing archived, an unknown or excluded project -
/// answers with its reason. Then a query that reduced to no tokens returns no
/// hits without asking SQLite anything: `MATCH ''` is itself an fts5 error, so
/// the empty result is produced rather than caught, with
/// [`Reason::NothingSearchable`] saying why.
pub fn run(conn: &Connection, config: &Config, request: &Request) -> Result<Response> {
    let bounds = request.filters.bounds()?;
    let scoped = scope::resolve(conn, config, &request.scope)?;
    if let Some(reason) = scoped.reason() {
        return Ok(Response {
            hits: Vec::new(),
            reason: Some(reason.clone()),
            reads: excerpt::Reads::default(),
        });
    }

    let Some(expression) = expression(request) else {
        return Ok(Response {
            reason: Some(Reason::NothingSearchable),
            ..Response::default()
        });
    };

    let mut sql = String::from(HEAD);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(expression)];

    scoped.push_filter(&mut sql, &mut params);
    request.filters.push_onto(&bounds, &mut sql, &mut params);
    // Conjunctive with everything above and applied inside the ranking query,
    // not after it: a bound applied to the results would let a later-archived
    // turn take a slot in the candidate pool and push a turn that WAS indexed
    // out of the window a replay then judges.
    if let Some(bound) = request.before_turn_id {
        sql.push_str(" AND t.id < ?\n");
        params.push(Box::new(bound));
    }
    sql.push_str(TAIL);
    params.push(Box::new(CANDIDATE_POOL as i64));

    let mut statement = conn.prepare(&sql)?;
    let mut hits = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(Hit {
                turn_id: row.get(0)?,
                session_key: row.get(1)?,
                record_type: row.get(2)?,
                ts: row.get(3)?,
                project: row.get(4)?,
                sidechain: row.get::<_, i64>(5)? != 0,
                entity_score: 0.0,
                entity_match: None,
                entity_count: 0,
                matched_on: Vec::new(),
                relevance: row.get(6)?,
                excerpt: String::new(),
                is_typed: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<Hit>>>()?;
    drop(statement);

    weight_by_entities(conn, &request.query, &mut hits)?;
    hits.sort_by(rank);
    hits.truncate(request.effective_limit());
    // After the truncation, never before: an excerpt costs a whole decompressed
    // session and the candidate pool is four times what the caller asked for.
    // And not at all when the caller said so - see [`Request::excerpts`].
    let reads = if request.excerpts {
        // The knob is read HERE, at the entry point that already holds the
        // config, and travels down as a value.
        excerpt::attach(conn, &request.query, &mut hits, &Redaction::of(config))?
    } else {
        excerpt::Reads::default()
    };

    Ok(Response {
        hits,
        reason: None,
        reads,
    })
}

/// The fts5 expression one request asks for, or `None` when it asks nothing.
///
/// Every term still goes through [`Query::match_expression`], which is the one
/// place a token becomes a quoted fts5 string: a caller hands over tokenized
/// spellings and never an expression, so there is no path by which a prompt -
/// which is untrusted text - reaches `MATCH` as written. Each spelling is
/// parenthesized before the `OR`, because fts5 binds `AND` tighter than `OR`
/// and a missing group would silently make one spelling's last token an
/// alternative to the whole of the next.
fn expression(request: &Request) -> Option<String> {
    if request.candidates.is_empty() {
        return request.query.match_expression();
    }
    let parts: Vec<String> = request
        .candidates
        .iter()
        .filter_map(Query::match_expression)
        .map(|part| format!("({part})"))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" OR "))
}

/// The same total order [`TAIL`] applies, over scores SQL could not compute.
fn rank(a: &Hit, b: &Hit) -> std::cmp::Ordering {
    b.relevance
        .partial_cmp(&a.relevance)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.sidechain.cmp(&b.sidechain))
        // Descending, and `None` last, which is where SQL puts it too.
        .then_with(|| b.ts.cmp(&a.ts))
        .then_with(|| a.turn_id.cmp(&b.turn_id))
}

/// An exact match on a whole stored value counts double one that merely
/// covers it.
///
/// The user who typed the command asked for the command; the user who typed it
/// inside a longer question asked for something the command is part of.
const EXACT_WEIGHT: f64 = 2.0;
const COVERED_WEIGHT: f64 = 1.0;

/// What the query matched on one candidate turn, beside the score it earned.
///
/// The score is what re-ranks; the other two are what the injection's
/// structural threshold reads. They are collected in the same pass because they
/// come off the same rows: recomputing them later would mean a second scan of
/// `entities` and a second definition of "matched".
#[derive(Default)]
struct Matched {
    score: f64,
    /// The strongest [`EntityMatch`] seen on this turn.
    kind: Option<EntityMatch>,
    /// The distinct `(kind, value_norm)` pairs matched, as a set rather than a
    /// counter: `entities` declares no uniqueness constraint, so two identical
    /// rows for one turn are a shape the table permits and must not count
    /// twice.
    values: BTreeSet<(String, String)>,
}

impl Matched {
    /// Fold in one matched `(kind, value_norm)` row.
    ///
    /// The score accumulates per ROW and the set per VALUE, which is the
    /// arithmetic that was here before this reporting was added: a change to
    /// the summing is a re-ranking, and a re-ranking hidden inside a reporting
    /// change is the regression every existing test still passes.
    fn saw(&mut self, key: &(String, String), matched: EntityMatch, weight: f64) {
        self.score += weight;
        self.values.insert(key.clone());
        // `Exact` outranks `Covered`, and the strongest wins whatever order
        // the rows arrived in.
        if matched == EntityMatch::Exact || self.kind.is_none() {
            self.kind = Some(matched);
        }
    }
}

/// The query-time half of entity matching: weight each matched entity by how
/// rare its value is, and add it to the relevance of every candidate carrying
/// it.
///
/// Nothing is rejected and there is no stop-list. "Too common" changes as the
/// corpus grows and an index-time rejection is irreversible, which is why
/// commonness is weighed here: a value carried by half the archive still
/// matches, it just moves a hit almost not at all.
///
/// Deterministic, and dependent only on stored rows: the document frequency is
/// a count over `entities` and the total is a count over `turns`, so two runs
/// against an unchanged store produce the same order. The injection's
/// structural threshold reads rank 1..3, and a nondeterministic tie would make
/// it fire on a different turn each time.
fn weight_by_entities(conn: &Connection, query: &Query, hits: &mut [Hit]) -> Result<()> {
    if hits.is_empty() || query.is_empty() {
        return Ok(());
    }

    // One JSON array, so the id list is one bound parameter (#108).
    let ids: serde_json::Value = hits.iter().map(|hit| hit.turn_id).collect();
    let mut statement = conn.prepare(
        "SELECT turn_id, kind, value_norm FROM entities
         WHERE turn_id IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = statement
        .query_map([ids.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<(i64, String, String)>>>()?;
    drop(statement);
    if rows.is_empty() {
        return Ok(());
    }

    // The N of the IDF. Every turn is a document whether or not it carries an
    // entity, so a value on one turn in the archive is rare in the archive and
    // not merely rare among the turns that happen to have entities.
    let total: f64 =
        conn.query_row("SELECT count(*) FROM turns", [], |r| r.get::<_, i64>(0))? as f64;

    let mut weights: BTreeMap<(String, String), Option<(EntityMatch, f64)>> = BTreeMap::new();
    let mut by_turn: BTreeMap<i64, Matched> = BTreeMap::new();
    let mut frequency = conn.prepare(
        "SELECT count(DISTINCT turn_id) FROM entities WHERE kind = ?1 AND value_norm = ?2",
    )?;

    for (turn_id, kind, value) in rows {
        let key = (kind, value);
        let scored = match weights.get(&key) {
            Some(scored) => *scored,
            None => {
                let scored = match query.matches_entity(&key.1) {
                    None => None,
                    Some(matched) => {
                        // `idx_entities_lookup` is on exactly (kind, value_norm).
                        let df: i64 =
                            frequency.query_row(rusqlite::params![&key.0, &key.1], |r| r.get(0))?;
                        let idf = (1.0 + total / df.max(1) as f64).ln();
                        let weight = idf
                            * match matched {
                                EntityMatch::Exact => EXACT_WEIGHT,
                                EntityMatch::Covered => COVERED_WEIGHT,
                            };
                        Some((matched, weight))
                    }
                };
                weights.insert(key.clone(), scored);
                scored
            }
        };
        // A match with a zero weight is still a match. The two were one fact
        // while `entity_score` was the only thing reported, and separating
        // them means a value the whole archive carries scores
        // almost nothing and is still the query naming a stored value.
        if let Some((matched, weight)) = scored {
            by_turn
                .entry(turn_id)
                .or_default()
                .saw(&key, matched, weight);
        }
    }

    for hit in hits.iter_mut() {
        if let Some(matched) = by_turn.get(&hit.turn_id) {
            hit.entity_score = matched.score;
            hit.entity_match = matched.kind;
            hit.entity_count = matched.values.len();
            // The same set the count is taken from, so the two cannot disagree.
            hit.matched_on = matched
                .values
                .iter()
                .map(|(kind, value)| MatchedEntity {
                    kind: kind.clone(),
                    value: value.clone(),
                })
                .collect();
            hit.relevance += matched.score;
        }
    }
    Ok(())
}

/// The time bounds of one search, each already through [`checked_bound`].
struct Bounds {
    since: Option<String>,
    until: Option<String>,
}

impl Filters {
    /// Check both time bounds, once per search.
    fn bounds(&self) -> Result<Bounds> {
        let check = |field, bound: &Option<String>| {
            bound
                .as_deref()
                .map(|raw| checked_bound(field, raw))
                .transpose()
        };
        Ok(Bounds {
            since: check(FIELD_SINCE, &self.since)?,
            until: check(FIELD_UNTIL, &self.until)?,
        })
    }

    /// Append every filter that is set, conjunctively, with the time bounds as
    /// [`Filters::bounds`] checked them.
    fn push_onto(
        &self,
        bounds: &Bounds,
        sql: &mut String,
        params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    ) {
        if let Some(tool) = &self.tool {
            sql.push_str(" AND t.tool_name = ?\n");
            params.push(Box::new(tool.clone()));
        }
        if let Some(kind) = &self.kind {
            sql.push_str(" AND t.record_type = ?\n");
            params.push(Box::new(kind.clone()));
        }
        // Here and not on the results: filtering afterward would let tool
        // results fill the candidate pool and cut the typed turns out of it.
        if self.typed {
            sql.push_str(" AND t.is_typed = 1 AND m.parent_session_key IS NULL\n");
        }
        if !self.paths.is_empty() {
            // EXISTS rather than a join: a turn carrying three of the named
            // paths is still one hit, and a join would return it three times
            // and spend three excerpt reads saying so. The list is one JSON
            // array read through `json_each`, so its length is not bounded by
            // SQLite's variable limit (#108).
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM paths p WHERE p.turn_id = t.id \
                 AND p.path IN (SELECT value FROM json_each(?)))\n",
            );
            let paths: serde_json::Value = self.paths.iter().map(String::as_str).collect();
            params.push(Box::new(paths.to_string()));
        }
        // A turn with no timestamp is outside every window, because SQL's
        // `NULL >= x` is NULL. That is the wanted answer: a turn that cannot say
        // when it happened cannot be shown as evidence of when something did.
        if let Some(since) = &bounds.since {
            sql.push_str(" AND t.ts >= ?\n");
            params.push(Box::new(since.clone()));
        }
        if let Some(until) = &bounds.until {
            sql.push_str(" AND t.ts <= ?\n");
            params.push(Box::new(until.clone()));
        }
    }
}

/// The `field` [`checked_bound`] reads as the lower end of a window.
pub const FIELD_SINCE: &str = "since";
/// The `field` [`checked_bound`] reads as the upper end of a window.
pub const FIELD_UNTIL: &str = "until";

/// One end of a time window, checked, as a string that can be compared against
/// `turns.ts` directly.
///
/// This is the one date check in the workspace: [`Filters`] runs every bound
/// through it, and so do `terminus search`, `terminus sessions`, `terminus
/// observations regenerate` and the MCP server's `recall_search` when they read
/// their arguments. `field` is [`FIELD_SINCE`] or [`FIELD_UNTIL`]; it names the
/// bound in the error and decides how a bare date extends.
///
/// The comparison is lexicographic and there is no date parsing on the
/// stored side. All 22,412 turns of a 180-file sample carry exactly the shape
/// `NNNN-NN-NNTNN:NN:NN.NNNZ` - one format, UTC only - and `turns(ts)` is
/// already indexed, so a text comparison is both correct and the fast plan.
///
/// A bare `YYYY-MM-DD` is extended to the first instant of that day for `since`
/// and the last for `until`, which is what makes a one-day window include its
/// own day. Extending both ends to midnight would make `--since 2026-08-12
/// --until 2026-08-12` return the single turn that happened at exactly
/// 00:00:00.000 and look like an empty day.
///
/// A value of either shape must also be a real date, and in the full form a
/// real time of day: month 01 to 12, a day within that month's Gregorian
/// length, hour 00 to 23, minute and second 00 to 59. The milliseconds may be
/// any three digits. The shape alone is not enough, because the comparison
/// never parses: `2026-02-30` is well shaped, sorts between 28 February and 1
/// March, and would answer a plausible window that no calendar has (#24).
///
/// Anything else is a caller error. Comparing it would succeed - every string
/// orders against every other - and return an answer that looks right.
pub fn checked_bound(field: &'static str, raw: &str) -> Result<String> {
    let refused = || crate::error::Error::InvalidTimeFilter {
        field,
        value: raw.to_owned(),
    };
    let full = is_shaped(raw, "NNNN-NN-NNTNN:NN:NN.NNNZ");
    if !full && !is_shaped(raw, "NNNN-NN-NN") {
        return Err(refused());
    }
    if !is_real_date(raw) || (full && !is_real_time(raw)) {
        return Err(refused());
    }
    if full {
        return Ok(raw.to_owned());
    }
    let tail = if field == FIELD_SINCE {
        "T00:00:00.000Z"
    } else {
        "T23:59:59.999Z"
    };
    Ok(format!("{raw}{tail}"))
}

/// Is the `YYYY-MM-DD` that `shaped` starts with a day on the Gregorian
/// calendar? `shaped` has already passed [`is_shaped`], so every field is
/// ASCII digits.
fn is_real_date(shaped: &str) -> bool {
    let year = digits(shaped, 0, 4);
    let month = digits(shaped, 5, 2);
    let day = digits(shaped, 8, 2);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=length).contains(&day)
}

/// Is the `HH:MM:SS` of a full-form timestamp a time of day? No leap second:
/// transcripts are stamped by a clock that never writes one.
fn is_real_time(shaped: &str) -> bool {
    digits(shaped, 11, 2) <= 23 && digits(shaped, 14, 2) <= 59 && digits(shaped, 17, 2) <= 59
}

/// The number `len` ASCII digits starting at byte `start` spell.
fn digits(shaped: &str, start: usize, len: usize) -> u32 {
    shaped.as_bytes()[start..start + len]
        .iter()
        .fold(0, |n, byte| n * 10 + u32::from(byte - b'0'))
}

/// Does `value` match `shape`, where `N` stands for one ASCII digit and every
/// other character stands for itself?
fn is_shaped(value: &str, shape: &str) -> bool {
    value.len() == shape.len()
        && value
            .bytes()
            .zip(shape.bytes())
            .all(|(byte, expected)| match expected {
                b'N' => byte.is_ascii_digit(),
                other => byte == other,
            })
}
