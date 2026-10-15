# Task: verify-divergence-message

## What shipped

- verify now says a diverged transcript was shortened or rewritten since it was archived, instead of only shorter, because the same flag stays set for a file that grew back with a different start. The matching capture todo is removed.

## Commits

| Task | Commit | Description |
| --- | --- | --- |
| 1 | 6636cf835d18926cf27b44cf67cb4d314fde971a | fix(verify): say a diverged transcript was shortened or rewritten, not only shorter |
| 1 | c1ca8d5778331084626f06f3f9b3b8136a3776cc | docs: drop the verify message todo now that it is fixed |

## Files

### Task 1: verify-divergence-message

- **Files:** .planning/CAPTURE.md, crates/terminus-core/src/verify.rs, crates/terminus-core/tests/verify.rs
