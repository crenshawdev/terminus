//! Claude Code's `settings.json` and `.claude.json`, as install, uninstall and
//! doctor parse them, and the rendering install writes back.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        terminus::fuzz::settings(text);
    }
});
