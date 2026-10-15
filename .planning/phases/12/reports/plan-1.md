PLAN COMPLETE
Plan: .planning/phases/12/PLAN-1.md
Tasks: 3 of 3
| Task | Commit | Note |
|---|---|---|
| 1 - Stop offering protocol revision 2025-03-26 (#30) | b45feb4 | `PROTOCOL_VERSIONS` now four revisions; doc comment and batch-branch comment in `rpc::parse` rewritten; new test `the_revision_that_requires_batches_is_not_offered`. `cmd::mcp` 8 passed. Initialize asking 2025-03-26 answered `2025-06-18`; a two-ping batch answered with one line, `-32600`, `id` null. Clippy (workspace, all features, all targets) and fmt clean. |
| 2 - Refuse a request id that is not an integer that fits (#28) | 5bef6e2 | Id accepted only as a string or `is_i64()`/`is_u64()`; message now "a request id is a string or an integer"; `Request.id` doc says "Echoed back verbatim." again. New test `a_request_id_that_is_not_an_integer_that_fits_is_refused` covers `1.5`, `1.0`, `1e3`, `-0`, 2^64, i64::MIN-1 refused and i64::MIN, 0, u64::MAX kept. `cmd::mcp` 9 passed. The four-line stdin run printed three `-32600` with `id` null and a `result` with id `18446744073709551615`. Clippy clean; fmt needed one reflow of the new test's array, applied before commit. |
| 3 - Remove duplicate recall_get ids before the cap (#29) | 0f9d5c7 | `run_get` drops repeats with a `HashSet` and `retain` (first place kept, linear in the client-supplied length) before `MAX_IDS`; `served` and the reason's M now count distinct ids; doc comment says why. New test `repeated_ids_are_removed_before_the_cap`: 30 ids / 20 distinct returns the 20 in asked order with null `reason`; 35 ids / 30 distinct serves the first 25 and the reason reads "only the first 25 of 30 ids". `--test mcp` 27 passed, including the two named existing tests; `cmd::mcp` 9 passed. With the `tools.rs` change set aside the new test fails ("only the first 25 of 30" on twenty distinct ids), so it pins the fix. Clippy and fmt clean. |
Deviations: none
Open items: none
