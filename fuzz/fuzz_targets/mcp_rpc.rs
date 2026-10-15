//! One MCP message, as the server's JSON-RPC layer parses it.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    terminus::fuzz::mcp_message(&String::from_utf8_lossy(data));
});
