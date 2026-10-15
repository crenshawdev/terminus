//! A `terminus` command line, parsed and never run. Arguments are separated by
//! NUL bytes and need not be UTF-8.
#![no_main]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    terminus_fuzz::sandbox();
    let args = data
        .split(|b| *b == 0)
        .map(|arg| OsStr::from_bytes(arg).to_owned())
        .collect();
    terminus::fuzz::command_line(args);
});
