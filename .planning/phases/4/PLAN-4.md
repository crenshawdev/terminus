---
phase: 4
plan: 4
requirements:
  - PRIV-03
  - RCL-09
files:
  - crates/verbatim/tests/mcp.rs
  - crates/verbatim/tests/brief.rs
  - crates/verbatim/tests/hook.rs
---

# Phase 4: Redacted Recall - Plan 4

## Goal

The two surfaces Claude Code puts into the model's context - the MCP server's
three tools and the `SessionStart` hook - are proved filtered at the process
boundary, immune to anything the environment says, and still inside the wall
budget with the knob on.

## Must be true when done

- With the boolean absent, `recall_search`'s excerpt, `recall_context`'s window
  and `recall_get`'s body over the planted fixture each carry the secret's
  characters; with the boolean set true and nothing else changed, none of the
  three does and each carries that rule's named marker constant.
- The same holds for the `SessionStart` brief's quoted prompt, run as the hook
  the harness runs.
- Both surfaces stay filtered no matter what environment variables the spawn
  carries, including a variable spelled like the terminal raw flag.
- `crates/verbatim/tests/hook.rs` passes its p99 wall budget over 100 runs of
  every event with the boolean on.
- Every existing test in the three files passes unmodified, including
  `tests/mcp.rs`'s pinned `recall_get` record key set.

## Context

Locked: D-07 (the escape hatch is a per-invocation CLI flag and NOT an
environment variable, precisely because the hook and the MCP server load
`Config` in-process and would inherit one), D-08 (no `--json` or tool-result
shape gains a field in either setting - `tests/mcp.rs` pins the exact key set of
a `recall_get` record), D-03 (the filter does not consult `provider.local`).
All product behaviour under test lands in PLAN-1 and PLAN-2; this plan writes
tests only, and if a task here cannot be made to pass, the defect is in the core
and not in a test that should be relaxed. `session-secrets.jsonl` is a rooted
fixture whose project is `project-delta` and whose turns each carry one
credential shape with no surrounding name-keyed context, which is what keeps
each assertion attributable to the one rule that can catch it.

## Tasks

### Task 1: The three MCP tools at the process boundary

- **Files:** crates/verbatim/tests/mcp.rs
- **Action:** Add tests that spawn the server through that file's existing
  `Bench` and drive real JSON-RPC calls, covering all three tools over the
  `session-secrets.jsonl` project: `recall_search` for a token in the
  `Authorization` turn, `recall_context` anchored on that turn, and
  `recall_get` of its id. Run each pair twice - once with no `verbatim.toml`
  written into the bench's config directory, once with one that sets the knob -
  and assert the sentinel is present in the first and absent in the second, with
  `verbatim_core::config::REDACTED` present in the second. Add the AC4 half in
  the same file: a spawn carrying environment variables spelled like the
  terminal raw flag (both the bare uppercase name and a `VERBATIM_`-prefixed
  one) still answers filtered, because the escape hatch is a command-line flag
  and the server parses none. Add only new items - a config helper on `Bench` if
  one is needed and new `#[test]` functions - and change no existing test body,
  since AC2 turns on those passing unmodified.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --features testkit --test mcp` passes, including the existing record key-set
  assertion, and `git -C /code/verbatim diff` on that file shows additions only.
  The new tests fail if either direction breaks: the unset run asserts the exact
  string `sk-VBEGRESS-authz-9f2` is present, the knob-on run asserts no
  `VBEGRESS` fragment appears anywhere in the tool result.

### Task 2: The SessionStart brief at the hook boundary

- **Files:** crates/verbatim/tests/brief.rs
- **Action:** Add tests using that file's existing `Bench` - `ingest`, `project`
  and `hook`, which feed a payload on stdin and read the hook's stdout the way
  Claude Code does - seeded from `session-secrets.jsonl` so the brief's "It last
  asked" quotes that session's last typed turn, which carries the double-quoted
  `Cookie` shape. With no `verbatim.toml`, the emitted brief carries
  `sid-VBEGRESS-qcrumb-1f9`; with one that turns the knob on, it carries
  `verbatim_core::config::REDACTED` and none of that sentinel's characters, and
  the hook still exits 0 with a well-formed payload. Add the environment half
  here too: the same knob-on spawn with variables spelled like the terminal raw
  flag still emits the filtered brief. Assert on the spawned process's stdout,
  not on a library call, and add only new items to the file.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --features testkit --test brief` passes, with the existing byte-identity and
  budget tests unmodified and still green, and `git -C /code/verbatim diff` on
  that file showing additions only.

### Task 3: The hook budget with the knob on

- **Files:** crates/verbatim/tests/hook.rs
- **Action:** Add one test that repeats the shape of
  `every_event_exits_zero_with_an_empty_stdout_inside_the_budget` - 100 runs of
  each of the four events, each exiting 0 with an empty stdout, p99 printed and
  asserted under the same 10 ms budget - with a `verbatim.toml` turning the knob
  on written into the bench's config directory first (AC7). In this file rather
  than a new one on purpose: the `COPYING` RwLock this file holds between a
  binary copy and any spawn is what keeps ETXTBSY out of a fully parallel suite,
  and a second file copying the binary would sit outside that lock. Reuse the
  existing `hook()` bench, `feed`, `FIXTURES` and `drain` helpers rather than
  duplicating them; change no existing test body and no shared helper's
  behaviour.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --features testkit --test hook` passes, with the new test printing a p50 and
  p99 per event and asserting p99 under the same budget the existing test uses,
  and `git -C /code/verbatim diff` on that file showing additions only.

## Notes

- Runs after PLAN-1 and PLAN-2. It shares no file with PLAN-3, so the two may
  run in parallel.
- This plan writes tests only; no product file is declared. A failure here is a
  finding about PLAN-1 or PLAN-2's code, and the fix belongs there.
