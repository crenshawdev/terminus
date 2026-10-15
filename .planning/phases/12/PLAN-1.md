---
phase: 12
plan: 1
requirements: ["#30", "#28", "#29"]
files:
  - crates/terminus/src/cmd/mcp/mod.rs
  - crates/terminus/src/cmd/mcp/rpc.rs
  - crates/terminus/src/cmd/mcp/tools.rs
  - crates/terminus/tests/mcp.rs
---

# Phase 12: Quick Issue Fixes - Plan 1

## Goal

The phase goal: twelve open GitHub issues that are small enough to fix inside one function or one doc section are fixed and closed before the release is announced, which cuts the open list from 39 to 27. #17 gets its type check here, and its second case moves to phase 13.

This plan fixes the three MCP server issues. The server stops offering `2025-03-26`, which requires batches it does not answer (#30). It refuses a request id that is not an integer that fits (#28). It removes duplicate `recall_get` ids before the 25-id cap (#29).

## Must be true when done

- An `initialize` that asks for `2025-03-26` is answered with `2025-06-18`, and `2025-03-26` is not in `PROTOCOL_VERSIONS`. Every revision still in the list is echoed back when a client asks for it.
- A JSON-RPC batch still gets exactly one response, a `-32600` error with a null id. The comment on that branch says why no offered revision needs batching.
- A request whose id is `1.5`, `1.0`, `1e3`, `-0`, or an integer outside both the i64 and the u64 range gets a `-32600` error with a null id. String ids, and integer ids anywhere from `i64::MIN` to `u64::MAX`, are still answered with the id echoed back.
- A `recall_get` call with 30 ids, 20 of them distinct, returns all 20 records in the order they were first asked for, with a null `reason`. A call with more than 25 distinct ids serves the first 25 of them, and its reason counts distinct ids.
- `cargo test -p terminus --all-features --bin terminus cmd::mcp` and `cargo test -p terminus --all-features --test mcp` pass.

## Context

- D-02, D-03 and D-04 in `.planning/phases/12/CONTEXT.md` bind this plan exactly. MCP 2 (`2026-07-28`) stays unoffered, which is phase 14's work. `get::records` keeps its own deduplication. The `recall_get` descriptor's `maxItems` is not changed.
- `mod.rs` denies `unwrap_used`, `expect_used`, `panic` and `indexing_slicing` outside test code, for this module and both of its children. New code obeys that, because a panic here writes a backtrace onto the transport.
- Every task leaves `cargo clippy --workspace --all-features --all-targets -- -D warnings` and `cargo fmt --all --check` clean. Both are clean on the tree today.
- Recalled prior art (`v0.1.0/phases/3/UAT.md`, phase 3): a malformed `recall_get` call returns an empty result carrying a reason and never throws. Duplicate ids stay an answer, and are never turned into a refusal.

## Tasks

### Task 1: Stop offering protocol revision 2025-03-26 (#30)

- **Files:** crates/terminus/src/cmd/mcp/mod.rs (`PROTOCOL_VERSIONS` and `initialize` and the tests module), crates/terminus/src/cmd/mcp/rpc.rs (the not-an-object branch of `parse`)
- **Action:** Remove `2025-03-26` from `PROTOCOL_VERSIONS`, which leaves four revisions in newest-first order. `2024-10-07` stays in the list, per the CONTEXT assumption that it has no batching. `initialize` needs no new branch. A requested revision that is not in the list already falls through to `DEFAULT_PROTOCOL_VERSION` (`2025-06-18`), and that is the answer D-02 locks. Rewrite the `PROTOCOL_VERSIONS` doc comment so it describes the revisions this server agrees to, not every revision the client accepts. Add the reason `2025-03-26` is left out: JSON-RPC batching was added in that revision and is absent from `2024-11-05` and `2025-06-18`, and this server answers a batch with one `-32600`. Agreeing to the one revision that requires batches would promise something the server does not do, which is the same reasoning the MCP 2 paragraph already gives. In `rpc::parse`, correct the comment on the branch a batch reaches. It must say that no offered revision uses batching, that `2025-03-26` is not offered for that reason, and that a batch gets the single `-32600` with a null id rather than half an answer. That branch's behaviour does not change. Add a test in `mod.rs`'s tests module asserting two things: `PROTOCOL_VERSIONS` does not contain `2025-03-26`, and an `initialize` asking for it is answered with `DEFAULT_PROTOCOL_VERSION`. The test is needed because the loop in `initialize_negotiates_a_version_both_sides_know` iterates the list and stops covering the removed revision once it is gone.
- **Verify:** `cargo test -p terminus --all-features --bin terminus cmd::mcp` passes, including the new test, and `grep -n '2025-03-26' crates/terminus/src/cmd/mcp/mod.rs` finds the revision only in comments and in that test, never inside the `PROTOCOL_VERSIONS` array. Running `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}' | cargo run -q -p terminus -- mcp` prints one line whose `result.protocolVersion` is `2025-06-18`. Running `printf '%s\n' '[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","id":2,"method":"ping"}]' | cargo run -q -p terminus -- mcp` prints exactly one line, with `error.code` `-32600` and `id` null. Clippy and fmt are clean.

### Task 2: Refuse a request id that is not an integer that fits (#28)

- **Files:** crates/terminus/src/cmd/mcp/rpc.rs (the id check in `parse` and the tests module)
- **Action:** In `parse`, accept an id only when it is a string, or a number serde_json classifies as `is_i64()` or `is_u64()`. Any other number, meaning every number serde_json 1.0.151 parsed as f64, is refused as `Incoming::Invalid` with `id: None` and `code::INVALID_REQUEST`, the same shape the object-id case already returns. That covers a fractional id (`1.5`), an integer written as a float (`1.0`), an exponent (`1e3`), negative zero (`-0`, which serde_json parses as f64) and any integer outside both the i64 and the u64 range. Change the refusal message so it says a request id is a string or an integer, because "a number" now names something the check refuses. Do not enable serde_json's `arbitrary_precision` and do not add a dependency. The rule rests on the parser's default classification, and the MCP `2025-06-18` basic spec requires a string or integer id. Add tests in this file's tests module. For each of the refused forms above, `parse` must return `Incoming::Invalid` with `id: None` and `code::INVALID_REQUEST`. An integer id at each bound (`-9223372036854775808` and `18446744073709551615`) must still parse as a `Request` whose id is that value, and so must `0`. The bound cases pin both sides of the rule, so a check that refused large integers would fail too. The doc comment on `Request.id` says "Echoed back terminus." because the rename replaced the word. Make it say "Echoed back verbatim." again.
- **Verify:** `cargo test -p terminus --all-features --bin terminus cmd::mcp` passes, including the new cases. `grep -n 'Echoed back' crates/terminus/src/cmd/mcp/rpc.rs` prints `Echoed back verbatim.` Running `printf '%s\n' '{"jsonrpc":"2.0","id":1.5,"method":"ping"}' '{"jsonrpc":"2.0","id":1e3,"method":"ping"}' '{"jsonrpc":"2.0","id":18446744073709551616,"method":"ping"}' '{"jsonrpc":"2.0","id":18446744073709551615,"method":"ping"}' | cargo run -q -p terminus -- mcp` prints four lines. The first three are errors with code `-32600` and `"id":null`. The fourth is a `result` whose `id` is `18446744073709551615`. Clippy and fmt are clean.

### Task 3: Remove duplicate recall_get ids before the cap (#29)

- **Files:** crates/terminus/src/cmd/mcp/tools.rs (`run_get` and its doc comment), crates/terminus/tests/mcp.rs (beside `a_list_longer_than_the_cap_is_truncated_and_said_so`)
- **Action:** In `run_get`, remove duplicate ids from the list `required_ids` returns before `MAX_IDS` is applied. Keep the first occurrence of each id, in the order the client sent them. `served` then counts distinct ids that were read. The "only the first N of M ids were read" reason fires only when the distinct count is above `MAX_IDS`, and its M is the distinct count, not the raw list length. `get::records` keeps its own deduplication unchanged, per D-04. Write the deduplication with iterators, not indexing, because the module denies `indexing_slicing`. Do not refuse a list with repeats. A repeated id is still a valid call and gets an ordinary answer (phase 3 UAT prior art). Update `run_get`'s doc comment, which says the over-long list is truncated, so it also says repeats are removed first and why: a list that repeats one id 30 times names one record, and reading it as over the cap dropped real ids. In `tests/mcp.rs`, add a test on a `long_session` with at least 25 turns. A call naming 30 ids, 20 of them distinct, must return exactly those 20 records in first-asked order with a null `reason`. A call naming more than 25 distinct ids with repeats among them must return the first 25 distinct ids, and its reason must name the distinct count as the M.
- **Verify:** `cargo test -p terminus --all-features --test mcp` passes, including the new test and the unchanged `a_list_longer_than_the_cap_is_truncated_and_said_so` and `an_over_large_argument_is_lowered_to_the_budget_rather_than_refused`. `cargo test -p terminus --all-features --bin terminus cmd::mcp` passes. Clippy and fmt are clean.
