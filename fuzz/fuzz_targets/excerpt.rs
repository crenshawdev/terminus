//! The excerpt of an archived record that search, recall and injection hand to
//! Claude Code. Input: the query, a newline, then the record.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terminus_core::observe::egress::Redaction;
use terminus_core::recall::{excerpt, Query};

fuzz_target!(|data: &[u8]| {
    let (query, record) = terminus_fuzz::split_line(data);
    let query = Query::parse(&String::from_utf8_lossy(query));
    let redaction = Redaction::of(terminus_fuzz::redacting());
    for text in [
        excerpt::of_record(&query, record),
        excerpt::of_record_with(&query, record, &redaction),
    ] {
        // One line, so quoted text cannot start a line of its own in an injection.
        assert!(
            text.chars().all(|c| c == ' ' || !c.is_whitespace()),
            "excerpt is not one line: {text:?}"
        );
    }
});
