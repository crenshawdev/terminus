//! Search text, from the command line, the MCP `query` argument or a prompt,
//! turned into an FTS5 MATCH expression.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terminus_core::recall::{Query, MAX_QUERY_TOKENS};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let query = Query::parse(&text);
    assert!(query.tokens().len() <= MAX_QUERY_TOKENS);
    let _ = query.matches_entity(&text);
    let Some(expression) = query.match_expression() else {
        return;
    };
    // Only double-quoted runs of alphanumerics joined by AND, so no FTS5
    // operator, column filter, prefix star or stray quote gets through.
    for term in expression.split(" AND ") {
        let inner = term
            .strip_prefix('"')
            .and_then(|t| t.strip_suffix('"'))
            .unwrap_or_else(|| panic!("unquoted term {term:?} in {expression:?}"));
        assert!(
            !inner.is_empty() && inner.chars().all(char::is_alphanumeric),
            "term {term:?} in {expression:?}"
        );
    }
});
