//! Ranked search: the `turns_fts` match joined back to the rows that say which
//! turn, in which session, of which project it was.
//!
//! This is the search the phase exists for, and both front ends sit on it -
//! `verbatim search` (PLAN-3) and `recall_search` (PLAN-4) - so there is one
//! definition of relevance rather than two that drift.

use rusqlite::Connection;

use crate::error::Result;
use crate::recall::Query;

/// The most hits one search may return, whatever the caller asks for.
///
/// A caller may lower it and may not raise it. The MCP server needs a
/// server-side budget it does not have to trust a client to respect (RCL-10),
/// and every hit costs an excerpt read out of a session blob, which is a whole
/// decompressed session per session touched while rusqlite's incremental blob
/// I/O stays out of scope (D-20).
pub const MAX_RESULTS: usize = 50;

/// What a caller gets when it does not say.
///
/// Injection is precision-first and a terminal page is short; a caller that
/// wants more asks for more, up to [`MAX_RESULTS`].
pub const DEFAULT_RESULTS: usize = 20;

/// One search, as asked for.
#[derive(Debug, Clone)]
pub struct Request {
    pub query: Query,
    /// How many hits the caller wants. Silently clamped to [`MAX_RESULTS`] -
    /// see [`Request::effective_limit`].
    pub limit: usize,
}

impl Request {
    pub fn new(query: Query) -> Self {
        Request {
            query,
            limit: DEFAULT_RESULTS,
        }
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// The limit actually applied.
    ///
    /// Clamping rather than rejecting: an over-large `limit` is a client being
    /// optimistic, not a caller error, and RCL-10 wants an answer rather than a
    /// refusal.
    pub fn effective_limit(&self) -> usize {
        self.limit.min(MAX_RESULTS)
    }
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
    /// A subagent turn (D-07): its session carries a `parent_session_key`.
    pub sidechain: bool,
    /// Higher is a better match. See [`SQL`] for where the sign is fixed.
    pub relevance: f64,
}

/// The one query, with the one place the bm25 sign is fixed.
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
///
/// The order is total, so two runs over an unchanged store agree - phase 5
/// reads rank 1..3 and a nondeterministic tie would make its threshold fire on
/// a different turn each time. Relevance first, then D-07's rule that a
/// sidechain turn sorts last at equal score, then the newer turn, then the
/// lower id.
const SQL: &str = "
    SELECT t.id,
           t.session_key,
           t.record_type,
           t.ts,
           m.project,
           m.parent_session_key IS NOT NULL AS sidechain,
           -bm25(turns_fts) AS relevance
    FROM turns_fts
    JOIN turns t ON t.id = turns_fts.rowid
    LEFT JOIN session_meta m ON m.session_key = t.session_key
    WHERE turns_fts MATCH ?1
    ORDER BY relevance DESC, sidechain ASC, t.ts DESC, t.id ASC
    LIMIT ?2
";

/// Run one search.
///
/// A query that reduced to no tokens returns no hits without asking SQLite
/// anything: `MATCH ''` is itself an fts5 error (D-09), so the empty result is
/// produced rather than caught.
pub fn run(conn: &Connection, request: &Request) -> Result<Vec<Hit>> {
    let Some(expression) = request.query.match_expression() else {
        return Ok(Vec::new());
    };

    let mut statement = conn.prepare(SQL)?;
    let hits = statement
        .query_map(
            rusqlite::params![expression, request.effective_limit() as i64],
            |row| {
                Ok(Hit {
                    turn_id: row.get(0)?,
                    session_key: row.get(1)?,
                    record_type: row.get(2)?,
                    ts: row.get(3)?,
                    project: row.get(4)?,
                    sidechain: row.get::<_, i64>(5)? != 0,
                    relevance: row.get(6)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<Hit>>>()?;

    Ok(hits)
}
