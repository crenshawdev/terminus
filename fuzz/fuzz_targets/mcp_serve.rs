//! The MCP server reading its stdin: every message parsed and every tool call's
//! arguments checked, against an empty store in a scratch directory.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    terminus_fuzz::sandbox();
    terminus::fuzz::mcp_serve(data);
});
