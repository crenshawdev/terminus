//! The `UserPromptSubmit` relevance injection (INJ-03, INJ-04, INJ-05).
//!
//! Silence is the answer this arm gives most of the time and the one it is
//! built to protect: three near-misses are worse than nothing at all
//! (`DESIGN-BRIEF.md:245`), so a turn is injected only on a structural match -
//! a rank-1-to-3 exact entity, or two independent entities co-occurring - and
//! never on a BM25 score, which is not comparable across queries.
//!
//! PLAN-4 adds the suppressions; the retrieval and the threshold are here.
//!
//! **What a prompt is asked about.** Not the sentence. A `Query` is
//! conjunctive by construction (D-09), so handing the whole prompt to
//! `MATCH` asks for a turn that repeats the prompt, which is silence for
//! anything a person would actually type. The candidates come from the
//! spellings a prompt names - its paths, resolved against the payload's `cwd`
//! (D-05), and its identifier-shaped tokens - each conjoined within itself and
//! disjoined across, which is `DESIGN-BRIEF.md`'s own signal hierarchy: exact
//! matches on rare tokens, never a sentence-level similarity. A prompt naming
//! none of those never opens the store at all.
//!
//! **The decision is structural.** The search is a net and the threshold is
//! the judgement: a hit is injected when it matched at least one stored entity
//! AND sits in the top three of the ranked order, or when two independent
//! entities co-occur on it wherever it ranks. A hit the query reached only
//! through its text is never eligible, whatever its relevance - a score cutoff
//! is exactly the approximation `DESIGN-BRIEF.md:245` forbids, since bm25
//! scores are not comparable across queries, and free text is the weakest
//! signal in the hierarchy and the one that fires the most false positives.
//!
//! **Nothing decompresses a session until something has fired.** One excerpt
//! materializes a whole compressed session - p90 1.03 MB, max 10.1 MB over the
//! real corpus - so the search runs with excerpts off (D-13) and the at most
//! three turns that survive the threshold get theirs afterwards.

use std::collections::BTreeSet;
use std::path::Path;

use super::Payload;
use crate::config::Config;
use crate::index::entity::{self, normalize_path};
use crate::recall::search::{self, Request};
use crate::recall::{excerpt, Hit, Query, Scope};

/// How many ranked hits the threshold is applied over.
///
/// Wider than the cap of three, so a hit that two entities corroborate can be
/// seen below the top three, and far narrower than `CANDIDATE_POOL`, which the
/// re-rank already fixes: ten rows of seven small columns, no blob touched.
const RANKED: usize = 10;

/// The rank within which one matched entity is enough (INJ-03).
const ENTITY_RANK: usize = 3;

/// How many distinct entities make a hit eligible wherever it ranks (INJ-03).
///
/// Not a subset of the rank condition: corroboration by two independent facts
/// is evidence that bm25's ordering may have got this one wrong.
const CO_OCCURRING: usize = 2;

/// The most turns one prompt may ever be given (INJ-03).
pub const MAX_TURNS: usize = 3;

/// How many spellings one prompt may ask the archive about.
///
/// A pasted stack trace names dozens of files and identifiers, and every one
/// of them is another disjunct fts5 evaluates under a hard deadline. The
/// strongest signals are first - paths before symbols, in the order they were
/// typed - so the cut falls on the weakest.
const MAX_CANDIDATES: usize = 8;

/// The turns one prompt fired on, and what reading them cost.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Fired {
    /// At most [`MAX_TURNS`], in the ranked order. Empty is the ordinary
    /// answer.
    pub hits: Vec<Hit>,
    /// How many session blobs were materialized to produce them. An
    /// instrument, not an answer: a prompt that fires on nothing must read
    /// none, and that is a claim about a number (D-13).
    pub reads: excerpt::Reads,
}

/// Render the injection for one `UserPromptSubmit`, or nothing.
///
/// Nothing is the ordinary answer and the one this arm exists to protect.
pub fn user_prompt_submit(
    _data_dir: &Path,
    _config: &Config,
    _payload: &Payload,
) -> Option<String> {
    None
}

/// The turns one prompt fires on: the whole of INJ-03's decision.
///
/// Every failure is an empty [`Fired`] rather than an error, for the reason the
/// module doc on [`super`] gives: a missing, unreadable, busy or outdated store
/// is "no context this time" and not something a prompt can be told about.
pub fn select(data_dir: &Path, config: &Config, payload: &Payload) -> Fired {
    let Some(fired) = selected(data_dir, config, payload) else {
        return Fired::default();
    };
    fired
}

/// [`select`]'s body, written in the `?` its every step wants.
fn selected(data_dir: &Path, config: &Config, payload: &Payload) -> Option<Fired> {
    let prompt = payload.prompt.filter(|prompt| !prompt.trim().is_empty())?;
    let cwd = payload.cwd.filter(|cwd| !cwd.is_empty())?;

    // Before the store is opened, because a prompt naming nothing the archive
    // could match structurally is the common case, and the cheapest thing this
    // arm can do with it is nothing at all.
    let candidates = candidates(prompt, Some(cwd));
    if candidates.is_empty() {
        return None;
    }

    let store = super::open(data_dir)?;
    let conn = store.conn();
    let request = Request::new(query_of(prompt, Some(cwd)), Scope::Directory(cwd.into()))
        .limit(RANKED)
        .candidates(candidates)
        .excerpts(false);
    // D-12: the scope is the payload's `cwd` and `search::run` resolves it,
    // which is also where an excluded project becomes an empty result.
    let response = search::run(conn, config, &request).ok()?;

    let mut hits = eligible(response.hits);
    if hits.is_empty() {
        return None;
    }
    // Now, and only now, is a session worth decompressing.
    let reads = excerpt::attach(conn, &request.query, &mut hits).ok()?;
    Some(Fired { hits, reads })
}

/// INJ-03's threshold, applied over the ranked order.
///
/// The rank is the position in that order and nothing else - `recall::search`'s
/// order is total on purpose, so two runs over an unchanged store agree about
/// which turns were in the top three.
fn eligible(hits: Vec<Hit>) -> Vec<Hit> {
    hits.into_iter()
        .enumerate()
        .filter(|(rank, hit)| {
            // A free-text-only hit is never eligible, whatever its relevance.
            hit.entity_match.is_some() && (*rank < ENTITY_RANK || hit.entity_count >= CO_OCCURRING)
        })
        .map(|(_, hit)| hit)
        .take(MAX_TURNS)
        .collect()
}

/// The spellings this prompt asks the archive about, strongest first.
///
/// A path counts twice: as the absolute spelling the archive almost certainly
/// stored (D-05) and as the relative one the user typed, because the two are
/// different token sets and the second is what a turn quoting the short form
/// carries. An identifier-shaped token is `DESIGN-BRIEF.md`'s third signal and
/// is judged by exactly the rule the extractor judges a `Grep` pattern by, so a
/// prompt cannot ask about `the` or `should`.
fn candidates(prompt: &str, cwd: Option<&str>) -> Vec<Query> {
    let mut spellings: Vec<String> = Vec::new();
    for relative in relative_paths(prompt) {
        if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
            spellings.push(join(cwd, &relative));
        }
        spellings.push(relative);
    }
    for absolute in absolute_paths(prompt) {
        spellings.push(absolute);
    }
    for token in entity::identifier_tokens(prompt) {
        if entity::is_identifier_shaped(token) {
            spellings.push(token.to_owned());
        }
    }

    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut out: Vec<Query> = Vec::new();
    for spelling in spellings {
        let query = Query::parse(&spelling);
        if query.is_empty() {
            continue;
        }
        // Deduplicated on the TOKENS and not on the spelling: two spellings of
        // one path differing in a separator are one question to the index, and
        // asking it twice costs a disjunct and returns the same rows.
        if seen.insert(query.tokens().to_vec()) {
            out.push(query);
        }
        if out.len() == MAX_CANDIDATES {
            break;
        }
    }
    out
}

/// The query one prompt becomes, with every relative path in it resolved
/// against the payload's `cwd` (D-05).
///
/// **Why the resolution has to happen at all.** A stored `path` entity is
/// almost always absolute - 1,029 absolute against 2 relative over 120 sampled
/// real transcripts - while a user types the relative spelling, and
/// [`Query::matches_entity`] requires EVERY token of the stored value to be
/// present in the query. So `/home/u/proj/crates/x.rs` cannot be matched by a
/// query of `crates/x.rs` however obviously the two name one file, and AC3
/// would be permanent silence that every test written against an absolute-path
/// fixture still passes.
///
/// **The resolved spelling is added, never substituted.** The prose is still
/// there, because the query is also what weighs and excerpts the turns that
/// come back, and because the user's own spelling is what a turn quoting the
/// relative path carries.
///
/// **The resolved spellings go FIRST.** [`crate::recall::MAX_QUERY_TOKENS`]
/// drops the thirty-third distinct token onward, and a pasted stack trace ahead
/// of the path would otherwise drop exactly the tokens this exists to add.
///
/// **No filesystem is touched.** 52% of the real corpus's `cwd` directories no
/// longer exist (phase 2 D-05), so a path canonicalized today would key
/// differently tomorrow and one file would key two ways across one archive.
/// This is string joining, and `..` is left in place: token containment does
/// not care, since the tokens of the true path are a subset of the tokens of
/// the joined one either way.
pub fn query_of(prompt: &str, cwd: Option<&str>) -> Query {
    Query::parse(&resolved(prompt, cwd))
}

/// The prompt with the resolved spellings prepended, or the prompt itself.
///
/// A payload with no `cwd` asks what the user typed: there is nothing to
/// resolve against, and inventing a base would be inventing a file.
fn resolved(prompt: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return prompt.to_owned();
    };

    let mut out = String::new();
    for relative in relative_paths(prompt) {
        out.push_str(&join(cwd, &relative));
        out.push(' ');
    }
    if out.is_empty() {
        return prompt.to_owned();
    }
    out.push_str(prompt);
    out
}

/// The path-shaped words of a prompt, in the order they were typed and without
/// repeats.
///
/// Path-shaped means it survives [`normalize_path`] - which trims the quotes
/// and the trailing `:line:col` a user pastes - and carries a separator. A bare
/// word is as likely to be a subcommand as a file, which is the same judgement
/// `entity::path_words` makes about a shell command line.
fn path_words(prompt: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in prompt.split_whitespace() {
        // A URL is not a file, and joining one onto a working directory
        // produces a spelling that names nothing at all.
        if word.contains("://") {
            continue;
        }
        let Some(path) = normalize_path(word) else {
            continue;
        };
        if !path.contains('/') && !path.contains('\\') {
            continue;
        }
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// The path-shaped words that are not already absolute: the ones a `cwd` says
/// something about.
fn relative_paths(prompt: &str) -> Vec<String> {
    path_words(prompt)
        .into_iter()
        .filter(|path| !Path::new(path).is_absolute())
        .collect()
}

/// The path-shaped words that already name a file outright.
fn absolute_paths(prompt: &str) -> Vec<String> {
    path_words(prompt)
        .into_iter()
        .filter(|path| Path::new(path).is_absolute())
        .collect()
}

/// `cwd` and `relative`, joined with one separator between them.
///
/// The separator is a forward slash whatever the platform: the value is a query
/// string and never a path this process opens, and the tokenizer splits on both
/// separators anyway (`index::expand::separator_components`), so the spelling
/// changes nothing about what matches.
fn join(cwd: &str, relative: &str) -> String {
    let base = cwd.trim_end_matches(['/', '\\']);
    let relative = relative.trim_start_matches("./").trim_start_matches(".\\");
    format!("{base}/{relative}")
}
