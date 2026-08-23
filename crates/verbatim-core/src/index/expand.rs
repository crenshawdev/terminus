//! RCL-01's expansion: the extra tokens that let a query for one component find
//! the whole.
//!
//! The design bars a custom FTS5 tokenizer (`.planning/PROJECT.md`) precisely so
//! every rule here stays an ordinary function with an input and an answer.
//! Normalization happens in Rust at ingest and the table keeps plain
//! `unicode61`, which means both query shapes hit: the whole token, because it
//! is in the projected text, and each component, because it is appended here.
//!
//! **Case boundaries are the only rule that changes recall (D-13).** Measured on
//! sqlite 3.53.4 against the table `crates/verbatim-core/src/store/schema.rs`
//! declares: `MATCH 'manager'` returns a `search_manager_new` row and does *not*
//! return a `SearchManager` row, `MATCH 'worker'` returns `src/worker/S.ts`,
//! `MATCH 'sources'` returns `--setting-sources`. So snake, kebab and path
//! components are already reachable - `unicode61` splits on `_`, `-`, `/` and
//! `.` by itself - and RCL-01 names them for completeness rather than for reach.
//!
//! They are still a rule here, in [`separator_components`], and it is the rule
//! that decides what counts as *already present*. Emitting a token the body
//! already carries would double its term frequency and buy nothing under BM25
//! except body bytes, so the separator rule feeds the dedup set and the case
//! rule feeds the output. On this tokenizer the separator rule therefore emits
//! nothing, which is not a gap: it is D-13's measurement, expressed as code.

use std::collections::BTreeSet;

use super::text::MAX_BODY_BYTES;

/// How many bytes of expansion tokens one turn's body may gain.
///
/// D-14 measured full camel+snake+kebab+path expansion of per-turn-unique tokens
/// at +37.2% of turn bytes and camel-only at +15.7%, against the brief's
/// 1.3-1.6x budget, so half the projection budget is headroom rather than a
/// constraint. It exists so one pathological token soup cannot make an FTS row
/// grow without limit after [`MAX_BODY_BYTES`] has already bounded the text.
pub const MAX_EXPANSION_BYTES: usize = MAX_BODY_BYTES / 2;

/// Append the expansion tokens a body needs, in place.
///
/// One `turns_fts.body` value carries both the projected text and the tokens, on
/// purpose: a second FTS column would change the table declaration and the
/// `contentless_delete=1` rebuild property with it.
pub fn append_to(body: &mut String) {
    let mut remaining = MAX_EXPANSION_BYTES;
    for token in expansion_tokens(body) {
        let cost = token.len() + 1;
        if cost > remaining {
            break;
        }
        body.push('\n');
        body.push_str(&token);
        remaining -= cost;
    }
}

/// Every token `text` does not already contain but a component query needs, in
/// the order the text produces them.
///
/// Order is text order and the dedup is exact, so the same body always expands
/// to the same tokens - which is what lets a rebuild reproduce a byte-identical
/// FTS row (AC3).
pub fn expansion_tokens(text: &str) -> Vec<String> {
    // What `unicode61` will already index, folded the way it folds. Seeding the
    // dedup set with this is the snake/kebab/path rule doing its work: those
    // components are in here, so nothing re-emits them.
    let mut seen: BTreeSet<String> = separator_components(text).map(str::to_lowercase).collect();

    let mut out = Vec::new();
    for token in separator_components(text) {
        for piece in case_components(token) {
            if seen.insert(piece.to_lowercase()) {
                out.push(piece.to_owned());
            }
        }
    }
    out
}

/// The snake, kebab and path rule: the pieces `unicode61` splits a text into.
///
/// One function for three of RCL-01's four rules because on this tokenizer they
/// are one rule - `_`, `-`, `/`, `\` and `.` are all separators, as is every
/// other non-alphanumeric character - and writing them as three would be three
/// spellings of `is_alphanumeric` pretending to be independent.
pub fn separator_components(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|piece| !piece.is_empty())
}

/// The camelCase / PascalCase rule: one token's pieces at its case boundaries.
///
/// A token with no internal boundary yields itself, so a caller can run this
/// over every token without asking first. Digits stay attached to the piece they
/// sit in - `S3Client` is `S3` and `Client` - because nothing in this phase
/// specifies bare-integer handling and splitting them would put every version
/// number in the index twice.
pub fn case_components(token: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = token.char_indices().collect();
    let mut cuts: Vec<usize> = Vec::new();

    for index in 1..chars.len() {
        let (at, current) = chars[index];
        let (_, previous) = chars[index - 1];

        // `searchManager` / `search3Manager`: a rise out of lowercase or a digit.
        if current.is_uppercase() && (previous.is_lowercase() || previous.is_numeric()) {
            cuts.push(at);
            continue;
        }
        // `HTTPServer`: the last capital of a run belongs to the word after it,
        // not to the acronym before it.
        if current.is_lowercase() && previous.is_uppercase() && index >= 2 {
            let (previous_at, before) = chars[index - 1];
            if before.is_uppercase() && chars[index - 2].1.is_uppercase() {
                cuts.push(previous_at);
            }
        }
    }

    if cuts.is_empty() {
        return vec![token];
    }
    let mut pieces = Vec::with_capacity(cuts.len() + 1);
    let mut from = 0;
    for cut in cuts {
        pieces.push(&token[from..cut]);
        from = cut;
    }
    pieces.push(&token[from..]);
    pieces
}
