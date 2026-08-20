//! The `UserPromptSubmit` relevance injection (INJ-03, INJ-04, INJ-05).
//!
//! Silence is the answer this arm gives most of the time and the one it is
//! built to protect: three near-misses are worse than nothing at all
//! (`DESIGN-BRIEF.md:245`), so a turn is injected only on a structural match -
//! a rank-1-to-3 exact entity, or two independent entities co-occurring - and
//! never on a BM25 score, which is not comparable across queries.
//!
//! PLAN-3 fills the retrieval in and PLAN-4 the suppressions. What is here is
//! the seam: the signature the binary calls, which does not change as those
//! land, and the answer this task gives to every prompt.

use std::path::Path;

use super::Payload;
use crate::config::Config;
use crate::index::entity::normalize_path;
use crate::recall::Query;

/// Render the injection for one `UserPromptSubmit`, or nothing.
///
/// Nothing, always, until PLAN-3 lands the retrieval. That is the correct
/// behaviour for this commit rather than a placeholder: an event with no
/// injection is an event that writes nothing to stdout and exits 0, which is
/// exactly what the four events did before this phase started.
pub fn user_prompt_submit(
    _data_dir: &Path,
    _config: &Config,
    _payload: &Payload,
) -> Option<String> {
    None
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

/// The path-shaped words of a prompt that are not already absolute, in the
/// order they were typed and without repeats.
///
/// Path-shaped means it survives [`normalize_path`] - which trims the quotes
/// and the trailing `:line:col` a user pastes - and carries a separator. A bare
/// word is as likely to be a subcommand as a file, which is the same judgement
/// `entity::path_words` makes about a shell command line.
fn relative_paths(prompt: &str) -> Vec<String> {
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
        if Path::new(&path).is_absolute() {
            continue;
        }
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
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
