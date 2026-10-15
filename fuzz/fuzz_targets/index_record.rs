//! Transcript records as the index reads them: the searchable text projected
//! out of each one, its expansion tokens, and its entities.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terminus_core::{capture, index};

fuzz_target!(|data: &[u8]| {
    for line in data.split(|b| *b == b'\n') {
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        let mut text = index::text::project(&record);
        index::expand::append_to(&mut text);
        let entities = index::entity::extract(&record);
        assert!(entities.len() <= index::MAX_ENTITIES_PER_TURN);
        let _ = capture::elided_bytes(&record);
    }
});
