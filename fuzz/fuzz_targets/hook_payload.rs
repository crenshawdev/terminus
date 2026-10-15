//! A hook's stdin line, then the injection it drives, against an empty store in
//! a scratch directory. The first byte picks the event; the rest is the line.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((first, line)) = data.split_first() else {
        return;
    };
    let event = if first % 2 == 0 {
        "SessionStart"
    } else {
        "UserPromptSubmit"
    };
    let sandbox = terminus_fuzz::sandbox();
    let _ = terminus::fuzz::hook(event, line, &sandbox.data_dir, &sandbox.config);
    sandbox.reset();
});
