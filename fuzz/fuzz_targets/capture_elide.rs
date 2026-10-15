//! `[capture] mode`, which can rewrite records before they are archived.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terminus_core::capture;
use terminus_core::config::CaptureMode;

fuzz_target!(|data: &[u8]| {
    // `full` keeps every byte. The archive's byte-for-byte guarantee rests on it.
    assert_eq!(&*capture::elide_stream(data, CaptureMode::Full), data);
    for mode in [CaptureMode::Lean, CaptureMode::Minimal] {
        // Elision only removes bytes: a record that would not shrink is kept as it was.
        let out = capture::elide_stream(data, mode);
        assert!(out.len() <= data.len(), "{mode:?} grew the stream");
    }
});
