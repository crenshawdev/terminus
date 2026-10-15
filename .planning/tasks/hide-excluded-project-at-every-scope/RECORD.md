# Task: hide-excluded-project-at-every-scope

## What shipped

- recall_get and recall_context no longer name an excluded project at project "*"; both answer such a turn as no such turn. The MCP test now covers "*" and fails without the fix. The matching capture todo is removed.

## Commits

| Task | Commit | Description |
| --- | --- | --- |
| 1 | 4a938c272796ce3e2988366cad75d3db57d2e415 | fix(recall): stop naming an excluded project when a client asks at project "*" |
| 1 | f3055b24912a0a575ce7d087f925473428506bd7 | docs: drop the excluded-project todo now that it is fixed |

## Files

### Task 1: hide-excluded-project-at-every-scope

- **Files:** .planning/CAPTURE.md, crates/terminus-core/src/recall/context.rs, crates/terminus-core/src/recall/get.rs, crates/terminus-core/tests/recall.rs, crates/terminus/tests/mcp.rs
