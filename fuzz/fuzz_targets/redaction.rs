//! The `[privacy] redact_recall` filter, on prose and on archived JSON lines.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terminus_core::observe::egress::{self, Redaction};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = egress::for_model_context(&text);
    let redaction = Redaction::of(terminus_fuzz::redacting());
    let _ = redaction.apply(&text);
    let _ = redaction.apply_json_line(&text);
});
