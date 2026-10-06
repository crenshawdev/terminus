//! D-09: the user's string is tokenized and rebuilt in Rust, and a raw query
//! never reaches `MATCH`.
//!
//! Measured on sqlite 3.53.4 against the table
//! `crates/terminus-core/src/store/schema.rs` declares: `MATCH 'src/worker/S.ts'`
//! fails with `fts5: syntax error near "/"` and exit 1, and `MATCH 'foo AND
//! (bar'` and `MATCH '"unbalanced'` fail the same way. None of the three returns
//! an empty result - they return an error - and RCL-06 forbids a non-zero exit
//! for a query that simply found nothing. Searching for a path is this phase's
//! own headline case, so the most natural command in the product is exactly the
//! one that would fail.
//!
//! The rule is therefore: split the user's string on the separators the index
//! tokenizes on, quote each surviving token, and require all of them. Every FTS5
//! metacharacter is a separator under that split, so there is no operator left
//! to be unbalanced and no syntax for a user to get wrong.
//!
//! **Query tokens are not expanded.** [`crate::index::expand`] already ran over
//! the indexed text, so the stored body of a turn containing `SearchManager`
//! carries `Search` and `Manager` as tokens of their own and a query for
//! `manager` matches it directly. Expanding here as well would turn a specific
//! query into its own components - a search for `SearchManager` would start
//! matching every turn that says `manager` - which is the opposite of what the
//! expansion was for.

use std::collections::BTreeSet;

use crate::index::expand::separator_components;

/// How many tokens one query may carry into `MATCH`.
///
/// A pasted stack trace or a whole error log is a plausible query - it is what a
/// user has in the clipboard when they go looking - and each token is a term the
/// FTS5 expression evaluates. The bound is on the expression, not on what the
/// user may type: tokens past it are dropped, the query still runs, and because
/// the tokens are combined conjunctively a truncated query is *broader* than the
/// one asked for rather than wrong in an unpredictable direction.
pub const MAX_QUERY_TOKENS: usize = 32;

/// How a query matched one stored `entities.value_norm` (RCL-04).
///
/// The comparison is between the query and a WHOLE stored value, never between
/// one query token and a whole value: `path` and `error` values are multi-token
/// by construction - `src/worker/S.ts`, and a whole normalized stderr line - so
/// a token-equality rule could never fire for the two kinds this phase
/// headlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityMatch {
    /// The value tokenizes to exactly this query, in order: the user asked for
    /// this stored value and nothing else.
    Exact,
    /// Every token of the value is in the query, which asks for more besides.
    Covered,
}

/// A user's raw string, reduced to the tokens the index can be asked about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    tokens: Vec<String>,
    /// The same tokens, case-folded, in the same order. Kept rather than
    /// recomputed because [`Query::matches_entity`] runs once per distinct
    /// entity value on every candidate turn of every search.
    folded: Vec<String>,
    truncated: bool,
}

impl Query {
    /// Tokenize a raw query string. Never fails: every input is either some
    /// tokens or none.
    pub fn parse(raw: &str) -> Query {
        let mut tokens: Vec<String> = Vec::new();
        let mut folded: Vec<String> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut truncated = false;

        for piece in separator_components(raw) {
            // Case-insensitively, because `unicode61` folds case and asking for
            // `Cargo` and `cargo` as two terms is one term twice.
            let lowered = piece.to_lowercase();
            if !seen.insert(lowered.clone()) {
                continue;
            }
            if tokens.len() == MAX_QUERY_TOKENS {
                truncated = true;
                break;
            }
            tokens.push(piece.to_owned());
            folded.push(lowered);
        }

        Query {
            tokens,
            folded,
            truncated,
        }
    }

    /// Does this query ask for one whole stored entity value?
    ///
    /// The value is tokenized by the same rule the query was, and it matches
    /// when every one of its tokens is in the query - so a query for
    /// `src/worker/S.ts` matches the stored path and a query for `worker` alone
    /// does not. `None` for a value that tokenizes to nothing, which cannot be
    /// asked for at all.
    ///
    /// Equality is compared over the token sequences rather than the raw
    /// strings. Two spellings of one path differing only in a separator or in
    /// case are the same value to the index, and a rule that called them
    /// different would make the strongest match depend on punctuation the
    /// tokenizer already threw away.
    pub fn matches_entity(&self, value: &str) -> Option<EntityMatch> {
        let value_tokens: Vec<String> =
            separator_components(value).map(str::to_lowercase).collect();
        if value_tokens.is_empty() {
            return None;
        }
        if !value_tokens.iter().all(|token| self.folded.contains(token)) {
            return None;
        }
        Some(if value_tokens == self.folded {
            EntityMatch::Exact
        } else {
            EntityMatch::Covered
        })
    }

    /// The tokens, in the order the raw string produced them and in the
    /// spelling the user wrote.
    ///
    /// The spelling is kept because the excerpt reader looks for these in the
    /// turn's own text, where the user's casing is a better first guess than a
    /// folded one, and because a `--json` shape that echoed a lowercased query
    /// back would be reporting something the user did not type.
    pub fn tokens(&self) -> &[String] {
        &self.tokens
    }

    /// Did the raw string carry more than [`MAX_QUERY_TOKENS`] distinct tokens?
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Does this query ask for nothing at all?
    ///
    /// True for an empty string and for a string of pure punctuation. The
    /// distinction matters because `MATCH ''` is itself an fts5 error, so the
    /// caller must skip the query rather than run it and catch the failure.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The FTS5 expression, or `None` when there is nothing to ask.
    ///
    /// Every token is a quoted FTS5 string, which is the form that carries no
    /// operator meaning: inside double quotes fts5 reads a string and tokenizes
    /// it with the table's own tokenizer, so a token can neither open a group
    /// nor be a bare `AND`. `AND` between them is explicit rather than left to
    /// fts5's implicit conjunction, because the default is a compile-time option
    /// of the amalgamation and a query that silently became a disjunction would
    /// return the whole archive for one common word.
    pub fn match_expression(&self) -> Option<String> {
        if self.tokens.is_empty() {
            return None;
        }
        let mut out = String::new();
        for token in &self.tokens {
            if !out.is_empty() {
                out.push_str(" AND ");
            }
            out.push('"');
            // Cannot fire: a token is a run of alphanumerics by construction.
            // Written anyway, because the cost is one pass over a short string
            // and the alternative is an injection into a query language.
            out.push_str(&token.replace('"', "\"\""));
            out.push('"');
        }
        Some(out)
    }
}
