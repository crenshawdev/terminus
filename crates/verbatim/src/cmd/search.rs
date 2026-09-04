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
//!
//! **`--raw` is the owner's way back to their own archive (PRIV-03, D-07).**
//! With `[privacy] redact_recall` set, the excerpt this prints is filtered like
//! every other projection that reaches a model; `--raw` spends
//! [`verbatim_core::Config::without_recall_redaction`] for this one invocation
//! and prints what the store holds. It is a flag on this loop and not an
//! environment variable on purpose: the hook and the MCP server load their own
//! `Config` in-process, so a variable would be inherited by them and would
//! unfilter the model-facing paths the knob exists to cover.

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

    // `--raw` changes which config the query layer reads and nothing else about
    // opening the store: `search::run` takes the `&Config` as its own argument,
    // and the forced-off copy is one direction only, so this can turn the
    // filter off for this process and can never turn it on.
    let unfiltered = args.raw.then(|| reader.config().without_recall_redaction());
    let config = unfiltered.as_ref().unwrap_or_else(|| reader.config());

    let response = match search::run(reader.store().conn(), config, &request) {
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
                    // The pairs behind `entity_score`, so a caller can see WHICH
                    // stored values it came from rather than only how much they
                    // weighed (D-05).
                    "matched_on": hit
                        .matched_on
                        .iter()
                        .map(|matched| serde_json::json!({
                            "kind": matched.kind,
                            "value": matched.value,
                        }))
                        .collect::<Vec<_>>(),
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
        // `get` and not `&ts[..10]`: `len` counts bytes, the slice needs a char
        // boundary, and a stored `ts` is any JSON string the transcript carried
        // (`parse::record` validates neither shape nor encoding). A timestamp
        // with a multi-byte character across byte 10 panicked the whole command
        // - exit 101, outside the documented 0/1/2 vocabulary - and took the
        // project's every hit with it. Falling back to the raw string is what
        // the short-timestamp arm already does.
        Some(ts) => ts.get(..10).unwrap_or(ts),
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
    /// `--raw`: hand back the archive unfiltered for this invocation alone.
    ///
    /// Governs the projection in both renderings. `--raw --json` emits the same
    /// keys with unfiltered values, because the flag changes a value and never
    /// a key (D-08).
    pub raw: bool,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut words: Vec<String> = Vec::new();
    let mut project = None;
    let mut filters = Filters::default();
    let mut limit = DEFAULT_LIMIT;
    let mut json = false;
    let mut raw = false;

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
            // Spelled out here rather than shared through `cmd::mod`: a
            // parser two commands reach through is how `verify --json` came
            // to look supported before it was, and this one belongs to
            // exactly the commands that print a projection (D-07).
            Long("raw") => raw = true,
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
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::day;

    /// A stored timestamp is any JSON string the transcript carried, so the
    /// day cut has to be total over arbitrary bytes rather than over the one
    /// shape D-23 measured.
    #[test]
    fn the_day_cut_survives_a_timestamp_that_is_not_the_measured_shape() {
        // The shape 22,412 of 22,412 sampled turns actually carry.
        assert_eq!(day(Some("2026-08-12T09:14:21.000Z")), "2026-08-12");

        // A multi-byte character across byte 10. `len()` is 10 or more, so the
        // old length guard admitted it and the slice panicked.
        let ragged = "2026-08-1\u{e9}9:00:00.000Z";
        assert!(ragged.len() > 10, "the premise: the length guard passes");
        assert!(
            !ragged.is_char_boundary(10),
            "the premise: byte 10 is inside a character"
        );
        assert_eq!(day(Some(ragged)), ragged, "the whole string, not a panic");

        // Shorter than the cut, and empty, both already fell through.
        assert_eq!(day(Some("2026")), "2026");
        assert_eq!(day(Some("")), "");
        assert_eq!(day(None), "(no time)");
    }
}
