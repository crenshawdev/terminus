//! Transcript JSONL as ingest reads it: every line parsed, the turns picked out,
//! and the session's lineage read off the result.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terminus_core::{lineage, parse};

fuzz_target!(|data: &[u8]| {
    let scan = parse::scan(data);
    // The watermark ingest stores comes from here; it can never point past
    // the bytes that were read.
    assert!(scan.resume_offset <= data.len() as u64);
    let _ = scan.consumed(0);
    let _ = scan.turn_count();
    let _ = scan
        .turns()
        .filter(|(record, _)| record.is_compact_boundary())
        .count();
    let _ = lineage::own_session_id(&scan);
    let _ = lineage::foreign_predecessor(&scan);
});
