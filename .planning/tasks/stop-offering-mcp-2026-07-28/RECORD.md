# Task: stop-offering-mcp-2026-07-28

## What shipped

- The MCP server stops agreeing to protocol 2026-07-28, which it does not implement; a client asking for it gets 2025-06-18. Full MCP 2 support is issue #45. Group 4 capture todos resolved.

## Commits

| Task | Commit | Description |
| --- | --- | --- |
| 1 | 69518cac350e69a9034412911ea01bb4961e4eb0 | fix(mcp): stop agreeing to MCP 2026-07-28 until the server speaks it |
| 1 | 1dea6dc9f3a3df6b14b54332d028b0628d75cd6f | docs: drop eleven stale todos, move five to issues #41-#45, record the Fixes #13 note |

## Files

### Task 1: stop-offering-mcp-2026-07-28

- **Files:** .planning/CAPTURE.md, crates/terminus/src/cmd/mcp/mod.rs
