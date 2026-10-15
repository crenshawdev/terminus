---
phase: 12
status: complete
completed: 2026-10-15
---

# Phase 12: Quick Issue Fixes - Summary

Twelve open GitHub issues are fixed on `hardin` (#10, #14, #20, #25, #28, #29, #30, #33, #35, #39, #41, #42) and #17 has its type check, each in its own commit, with a `### Fixed` section under `[Unreleased]` in CHANGELOG.md.

## What shipped

- The MCP server no longer offers `2025-03-26` (#30), refuses a request id that is not a string or an integer that fits i64 or u64 (#28), and removes repeated `recall_get` ids before the 25-id cap (#29) - `crates/terminus/src/cmd/mcp/mod.rs`, `rpc.rs`, `tools.rs`
- `Projector::push` charges its separator, so an indexed turn body stays inside `MAX_BODY_BYTES` (#20) - `crates/terminus-core/src/index/text.rs`
- A compaction boundary needs `type` `system` (#17, the type check only) - `crates/terminus-core/src/derive.rs`, `crates/terminus-core/src/parse/record.rs`
- The miss-labelling query drops its `recall_get` arm and matches `recall_search` by the extractor's rule (#39) - `crates/terminus-core/src/feedback/label.rs`
- `initialize` takes the write lock before it writes a fresh store (#33) - `crates/terminus-core/src/store/open.rs`
- A single-file ingest records its committed and failed counts in its runs row (#14) - `crates/terminus-core/src/ingest/mod.rs`
- `reindex` reports how many evicted sessions it carried across, as `preserved` in JSON and as a stderr line when the count is above 0 (#41) - `crates/terminus/src/cmd/reindex.rs`, `docs/json-shapes.md`, `docs/commands.md`
- `verify` fails an evicted session that still has block rows, once per session (#42) - `crates/terminus-core/src/verify.rs`
- `brief.rs` calls the shared `chars` and `clip` (#35) - `crates/terminus-core/src/inject/brief.rs`
- The lock test that claimed a process death is renamed `a_held_lock_refuses_a_second_attempt_and_frees_when_dropped` and points to the crash harness (#10) - `crates/terminus/tests/lock_race.rs`
- `docs/json-shapes.md` says when each nullable `sessions` and `show` field is null (#25)
- Seven `### Fixed` entries under `[Unreleased]` - `CHANGELOG.md`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | b45feb4 | stop offering 2025-03-26, the revision that requires batches (#30) |
| 1 | 2 | 5bef6e2 | refuse a request id that is not an integer that fits (#28) |
| 1 | 3 | 0f9d5c7 | drop repeated recall_get ids before the cap is applied (#29) |
| 2 | 1 | 20dd4ac | charge the separator so a turn body stays inside its cap (#20) |
| 2 | 2 | 92a088c | require type system before a compact_boundary gets its row (#17) |
| 2 | 3 | 99491f4 | match recall_search in the miss query the way the extractor does (#39) |
| 3 | 1 | d1246eb | take the write lock before initialize writes a fresh store (#33) |
| 3 | 2 | 74fd89b | count the file a single-file ingest committed in its runs row (#14) |
| 3 | 3 | 1ad97e9 | report the evicted sessions reindex carried across (#41) |
| 3 | 4 | dcf66b0 | fail an evicted session that still has block rows (#42) |
| 4 | 1 | cce2eb2 | let the brief count and cut with the shared chars and clip (#35) |
| 4 | 2 | f47cc3f | name the lock test for the guard drop it checks, not a process death (#10) |
| 4 | 3 | 92e973a | say when each sessions and show field is null (#25) |
| 4 | 4 | 94e34d7 | record the phase's user-visible fixes under Unreleased |

## Deviations

None - plans executed as written.

## Open items

- The twelve issues are fixed but not closed. They close when the pull request from `hardin` merges with a `Fixes #N` line for each (D-15, AC7). #17 gets a reference line and no `Fixes`, so it stays open for its second case in phase 13.

## Goal check

The commits plausibly deliver the goal's code half. Each of the twelve issues and #17's type check maps to one commit in `git log --oneline 074dd4b..94e34d7`, as the table above shows, and every task's report records its new test failing against the old code and passing after the fix (`reports/plan-1.md` through `plan-4.md`). The full suite `cargo test --workspace --all-features` passed after plans 3 and 4 with 877 tests and 0 failures (`reports/plan-3.md`, `reports/plan-4.md`). Both risk-surface reviews passed. Plan 1's range matched `api_contract`, and the one medium finding, which asked the server to accept `1e3` and `-0` as ids, was refuted because D-03 refuses them on purpose (`ADJUDICATION-risk_surface-plan-1.json`). Plan 3's range matched `migrations` and drew no findings (`ADJUDICATION-risk_surface-plan-3.json`). What is missing is the "closed" half of the goal: no issue is closed yet, because closing them is the pull request's job under D-15, and that is the open item above.
