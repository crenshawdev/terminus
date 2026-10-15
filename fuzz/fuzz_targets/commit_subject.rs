//! A Bash command line from a transcript, read for the subject of the commit it
//! made. The command is whatever Claude ran, so the transcript decides it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terminus_core::observe::mechanical;

fuzz_target!(|data: &[u8]| {
    let command = String::from_utf8_lossy(data);
    let _ = mechanical::commit_subject(&command);
});
