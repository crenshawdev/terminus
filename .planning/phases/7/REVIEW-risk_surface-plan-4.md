{
  "findings": [
    {
      "file": "crates/terminus/tests/cli.rs",
      "line": 329,
      "severity": "high",
      "claim": "The format-1 fixture is stamped with the current `DERIVED_SCHEMA`, so the test does not exercise opening a real older store whose archive and derived-schema versions are both old.",
      "failure_scenario": "If store opening checks or rebuilds a mismatched derived schema before rejecting the archive format, an actual format-1 store can execute the CREATE TABLE/INDEX migration and then fail as half-upgraded. Every case in this test still passes because the fixture's derived schema already equals the current binary's version, bypassing the write-producing migration path."
    },
    {
      "file": "crates/terminus/tests/cli.rs",
      "line": 287,
      "severity": "medium",
      "claim": "The asserted every-command coverage explicitly omits `mcp`, even though its different response protocol does not prevent testing refusal and byte preservation.",
      "failure_scenario": "If MCP startup opens a format-1 store through a path that migrates or writes before returning its protocol-level error, this test remains green. The test can start MCP, send a request or initialization transcript, assert the refusal reason, terminate it, and compare the database and WAL bytes just as it does for exit-based commands."
    }
  ]
}
