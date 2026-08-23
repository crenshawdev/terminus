# risk_surface - phase 3, plan 4

Range `40ff946..d457c06` (6 files, +2784/-2). Gate `blocking`, reviewer set
`["claude-subagent"]`, mode `adjudicated`. Matched surfaces: a JSON-RPC wire
contract over untrusted client input, scoping as an authorization boundary, and
a server-side budget.

**Round 1: FAIL** - 2 `high`, both driven against the running server. Fixed in
`f3f2076`; 30 suites green, clippy clean. Not re-reviewed: the `rearm` counter
is spent for this run, and the user asked for no further dispatches.

## Fixed

- **high** `recall/get.rs`, `recall/context.rs` - exclusion was checked before
  scope, so at the default scope an id inside an excluded project answered
  "<absolute path> is excluded by config" rather than `NoSuchTurn`, confirming
  both that the id exists and what the excluded project is named. Reachable by
  an untrusted client with no opt-out argument, 25 ids per call, over densely
  enumerable ids. Plan 3's gate tracked a weaker form of this as `low` under
  `--project '*'` and did not mention `recall_context`.
- **high** `cmd/mcp/tools.rs` - `recall_search`'s `paths` filter was unbounded
  while `Filters::push_onto` binds one placeholder per path, so an oversized
  array became an operational SQLite failure returned as a non-error empty
  result whose reason was the whole generated statement (2 MB for a
  1,000,000-element array). Both halves are wrong: a model reads it as "no
  matches", and it defeats the context budget outright.

## Open items (medium/low, queued in CAPTURE.md)

1. **medium** `cmd/mcp/tools.rs` - `optional_time` inherits `cmd::time_bound`'s
   shape-only validation, so `until: "2026-08-00"` or `since: "2026-02-30"` is
   accepted, filters the archive out, and returns `reason: null` with
   `isError: false`. The wrong-SHAPE case is handled; the impossible-CALENDAR
   case is not. Same root cause as plan 3's open item.
2. **medium** `cmd/mcp/mod.rs` - `serve` materializes one whole line before
   parsing with no size cap, and the wire-to-resident amplification is ~20x
   (raw bytes + `Value` + the arguments clone). Measured: one 12.9 MB line
   drives 278.7 MiB RSS. A client that never emits a newline pins memory.
3. **low** `cmd/mcp/rpc.rs` - an integer id outside i64/u64 is parsed as `f64`,
   so the echoed id is not the id sent (`123456789012345678901234567890` comes
   back `1.2345678901234568e+29`) and a client correlating by id hangs. A
   fractional id is accepted too, which MCP forbids.
4. **low** `cmd/mcp/tools.rs` - `time_bound`'s message hardcodes the CLI flag
   spelling, so a tool result names `--since`, an argument this surface's own
   `inputSchema` forbids.
5. **low** `cmd/mcp/tools.rs` - `recall_get` truncates to `MAX_IDS` before
   `get::records` deduplicates, so duplicate ids consume the budget and drop
   distinct ids that would have cost nothing.
6. **low** `cmd/mcp/rpc.rs` - `PROTOCOL_VERSIONS` offers `2025-03-26`, the one
   revision that mandates receiving JSON-RPC batches, and `initialize` echoes it
   as accepted, but a batch gets one `-32600` with a null id and every request
   inside it goes unanswered. Either drop the revision or handle the array.
