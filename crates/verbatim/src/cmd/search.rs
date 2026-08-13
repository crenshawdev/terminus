//! `verbatim search`: find a past turn from the terminal (RCL-05).
//!
//! The command parses and validates; [`verbatim_core::recall`] decides what
//! relevance is. That split is the whole reason this file is short: `verbatim
//! search` and PLAN-4's `recall_search` are two front ends over one query layer,
//! and a filter interpreted here rather than there would be a second definition
//! of what a search means.
//!
//! **The user's string never reaches SQLite (D-09).** `Query::parse` splits it
//! on the separators the index tokenizes on and rebuilds a quoted, conjoined
//! FTS5 expression. Measured on sqlite 3.53.4, `MATCH 'src/worker/S.ts'` fails
//! with `fts5: syntax error near "/"` - so the most natural command in the
//! product is exactly the one a raw `MATCH` would refuse, and RCL-06 forbids a
//! non-zero exit for a query that simply found nothing.

use verbatim_core::recall::{search, Filters, Query, Request, MAX_RESULTS};
use verbatim_core::Error;

use super::json::Document;
use super::read::{self, Opened};
use super::Failure;

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "search";

/// How many hits a terminal page shows when the caller does not say.
///
/// Lower than the library's [`verbatim_core::recall::DEFAULT_RESULTS`] on
/// purpose: this one is read by a person scrolling a terminal, where twenty
/// two-line entries is a screenful and a half.
const DEFAULT_LIMIT: usize = 10;

pub fn run(args: Args) -> Result<(), Failure> {
    // Parsed before the store is opened, so both exits carry the same shape: a
    // document whose fields depend on whether a store was there would be a
    // second shape for a caller to handle.
    let query = Query::parse(&args.query);
    let truncated = query.truncated();

    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        Opened::Nothing(reason) => {
            return read::empty(document(&args.query, &[], truncated), &reason, args.json)
        }
    };

    if truncated {
        // On stderr, and the search still runs. The tokens are conjoined, so a
        // truncated query is BROADER than the one asked for rather than wrong in
        // a direction the user cannot see - but they are owed the fact.
        eprintln!(
            "verbatim: only the first {} tokens of the query were used",
            query.tokens().len()
        );
    }

    let request = Request::new(query, read::scope(args.project.as_deref())?)
        .filters(args.filters)
        .limit(args.limit);

    let response = match search::run(reader.store().conn(), reader.config(), &request) {
        Ok(response) => response,
        // A malformed `--since` is a mistake in the command line, not an empty
        // result: every string compares cleanly against every other, so the
        // alternative is a plausible wrong answer that nothing reports (D-23).
        Err(Error::InvalidTimeFilter { field, value }) => {
            return Err(Failure::Misuse(format!(
                "--{field} {value:?} is not a time; expected YYYY-MM-DD or \
                 YYYY-MM-DDTHH:MM:SS.mmmZ"
            )))
        }
        Err(other) => return Err(Failure::Operational(other.to_string())),
    };

    if args.json {
        let hits: Vec<serde_json::Value> = response
            .hits
            .iter()
            .map(|hit| {
                serde_json::json!({
                    "turn_id": hit.turn_id,
                    "session_key": hit.session_key,
                    "project": hit.project,
                    "record_type": hit.record_type,
                    "ts": hit.ts,
                    "sidechain": hit.sidechain,
                    "relevance": hit.relevance,
                    "entity_score": hit.entity_score,
                    "excerpt": hit.excerpt,
                })
            })
            .collect();
        document(&args.query, &hits, truncated)
            .maybe_because(response.reason.as_ref())
            .emit();
        return Ok(());
    }

    for hit in &response.hits {
        // The id first, and unmissable: it is the argument `verbatim show`
        // takes, and following a hit into the conversation around it is what
        // the next command a reader runs does.
        println!(
            "{}  {}  {}  {}{}",
            hit.turn_id,
            day(hit.ts.as_deref()),
            session_of(&hit.session_key),
            hit.project.as_deref().unwrap_or("(no project)"),
            if hit.sidechain { "  (subagent)" } else { "" },
        );
        println!("    {}", hit.excerpt);
    }

    // Commentary on stderr, so stdout stays the hits and nothing else. It is
    // also what makes an empty result visibly empty rather than silent.
    match &response.reason {
        Some(reason) => eprintln!("verbatim: {reason}"),
        None => eprintln!("{} hit(s)", response.hits.len()),
    }
    Ok(())
}

/// The envelope for this command, with its hits already in it.
fn document(query: &str, hits: &[serde_json::Value], truncated: bool) -> Document {
    Document::new(COMMAND)
        .field("query", query)
        .field("truncated", truncated)
        .field("hits", hits.to_vec())
}

/// A transcript timestamp at day resolution.
///
/// All 22,412 turns of a 180-file sample carry exactly `NNNN-NN-NNTNN:NN:NN.NNNZ`
/// (D-23), so the day is the first ten characters and there is no date parsing
/// on the read path. A turn with no timestamp says so rather than showing a
/// blank column that reads as an alignment bug.
fn day(ts: Option<&str>) -> &str {
    match ts {
        Some(ts) if ts.len() >= 10 => &ts[..10],
        Some(ts) => ts,
        None => "(no time)",
    }
}

/// The transcript's own file name, which is what a session key ends with.
///
/// The whole key is an absolute path and a terminal line of them is unreadable;
/// `--json` carries the key in full, because that is the identifier a caller
/// scripts against.
fn session_of(session_key: &str) -> &str {
    session_key
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(session_key)
}

/// The parsed command line for `search`.
#[derive(Debug)]
pub struct Args {
    /// The words to search for, as typed, joined back into one string.
    pub query: String,
    /// `--project`: a path inside the project, or the literal `*`.
    pub project: Option<String>,
    pub filters: Filters,
    pub limit: usize,
    pub json: bool,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut words: Vec<String> = Vec::new();
    let mut project = None;
    let mut filters = Filters::default();
    let mut limit = DEFAULT_LIMIT;
    let mut json = false;

    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long("project") => project = Some(super::value(parser, "project")?),
            Long("tool") => filters.tool = Some(super::value(parser, "tool")?),
            Long("kind") => filters.kind = Some(super::value(parser, "kind")?),
            // Repeatable: a turn carrying any of the named paths is a hit, and
            // it is one hit however many of them it carries.
            Long("path") => filters.paths.push(super::value(parser, "path")?),
            // Validated here rather than only inside the query layer: a bad
            // bound must be misuse whatever project the caller is standing in.
            Long("since") => {
                filters.since = Some(super::time_bound("since", &super::value(parser, "since")?)?)
            }
            Long("until") => {
                filters.until = Some(super::time_bound("until", &super::value(parser, "until")?)?)
            }
            Long("limit") => {
                let raw = super::value(parser, "limit")?;
                limit = raw
                    .parse::<usize>()
                    .map_err(|_| Failure::Misuse(format!("--limit {raw:?} is not a number")))?;
                if limit == 0 {
                    return Err(Failure::Misuse("--limit 0 asks for no results".into()));
                }
                // Clamped rather than refused, the way the library clamps it:
                // an over-large limit is a caller being optimistic.
                limit = limit.min(MAX_RESULTS);
            }
            Long(super::JSON_FLAG) => json = true,
            // Every remaining word is part of the query, which is what lets
            // `verbatim search cannot find module` work without quoting.
            Value(word) => words.push(word.to_string_lossy().into_owned()),
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    if words.is_empty() {
        return Err(Failure::Misuse(
            "search needs something to search for".into(),
        ));
    }

    Ok(Args {
        query: words.join(" "),
        project,
        filters,
        limit,
        json,
    })
}
