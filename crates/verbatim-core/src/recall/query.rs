//! D-09: the user's string is tokenized and rebuilt in Rust, and a raw query
//! never reaches `MATCH`.
//!
//! Measured on sqlite 3.53.4 against the table
//! `crates/verbatim-core/src/store/schema.rs` declares: `MATCH 'src/worker/S.ts'`
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

/// A user's raw string, reduced to the tokens the index can be asked about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    tokens: Vec<String>,
    truncated: bool,
}

impl Query {
    /// Tokenize a raw query string. Never fails: every input is either some
    /// tokens or none.
    pub fn parse(raw: &str) -> Query {
        let mut tokens: Vec<String> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut truncated = false;

        for piece in separator_components(raw) {
            // Case-insensitively, because `unicode61` folds case and asking for
            // `Cargo` and `cargo` as two terms is one term twice.
            if !seen.insert(piece.to_lowercase()) {
                continue;
            }
            if tokens.len() == MAX_QUERY_TOKENS {
                truncated = true;
                break;
            }
            tokens.push(piece.to_owned());
        }

        Query { tokens, truncated }
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
