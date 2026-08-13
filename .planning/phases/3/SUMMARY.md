---
phase: 3
status: complete
completed: 2026-08-13
---

# Phase 3: Recall - Summary

Turn text is indexed as a projection with case/separator expansion and five
kinds of exact-match entity; a scoped, ranked, IDF-weighted query layer sits
under both `verbatim search|show|sessions` and a hand-written stdio MCP server
exposing `recall_search`, `recall_context` and `recall_get`.

## What shipped

- Text projection and expansion - `crates/verbatim-core/src/index/{text,expand}.rs`
- Entity extraction and error normalization - `crates/verbatim-core/src/index/entity.rs`
- Query layer: parse, rank, scope, filter, IDF, excerpt, window, get -
  `crates/verbatim-core/src/recall/`
- Read-only store open with a degraded-read path - `crates/verbatim-core/src/store/open.rs`
- Terminal recall and the JSON envelope - `crates/verbatim/src/cmd/`, `docs/json-shapes.md`
- The MCP server - `crates/verbatim/src/cmd/mcp/`

## Commits

38 commits, `8e0e68d..d457c06`. Per-plan detail with one row per task is in
`reports/plan-{1,2,3,4}.md`; the gate fixes are the six commits below.

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | gate | f7db432 | Read the nested result path a `Read` leaves behind |
| 1 | gate | 5bb3f97 | Separate a path from the shell syntax it arrived welded to |
| 2 | gate | 1066d40 | Scope to the row that names the caller's directory directly |
| 2 | gate | 12c22c9 | Degrade the read on an aged store, and scope the context window |
| 3 | gate | 4e48c97 | Cut the day off a timestamp on a character boundary |
| - | - | 5d85f1b | Drop the transient risk diff from the tree |

## Deviations

- [deviation] Plan 1, task 4: `tests/ingest.rs` added to the lease by checkpoint,
  because `session-basic.jsonl` carries a `Bash` tool_use and a pass now moves
  the `entities` table too (365d600).
- [deviation] Plan 2, task 1: a read-only open leaves `verbatim.db` byte-identical
  but SQLite materializes `-shm` and an empty `-wal` for any WAL database opened
  read-only. Suppressing it needs `immutable=1`, which is false here because
  ingest writes concurrently. Asserted the meaning instead (6ca8efd).
- [deviation] Plan 3: tasks 1+7 and 2+4 landed as single commits. The lint gate
  (`-D warnings`) rejects a commit whose only consumer is a later task, and the
  alternative was an `#[allow(dead_code)]` naming a commit that had not happened.
- [deviation] Plan 3, task 7/8: `verify` and `reindex` printed diagnostics to both
  stdout and stderr in JSON mode - two accounts of one walk that can disagree.
  Adopted the uniform rule task 7's own text implies: with `--json` the document
  is the answer and stderr carries only warnings (b68355c).
- [deviation] Plan 4, task 5: the task's `Action:` says an over-large `limit`
  clamps and its `Verify:` two sentences later says it returns an empty result.
  Implemented clamping, matching the `Action:` and the existing
  `effective_limit`. The "reason" half of the criterion holds, the
  "empty result" half does not; neither the plan's own `## Must be true when
  done` nor CONTEXT AC6 asks for the empty-result arm.

## Open items

Five `high` findings were caught by the blocking `risk_surface` gate and fixed
in-phase (commits above). The medium/low findings are recorded per plan in
`REVIEW-risk_surface-plan-{1,2,3}.md` and queued in `CAPTURE.md`. The ones most
worth naming here:

- `MAX_BODY_BYTES` does not actually bound the projected body: the `'\n'`
  separator is uncharged and `take` is 0 when the budget is smaller than the
  next leaf's first character, so the short-circuit never fires.
- `path`, `command` and `symbol` entity values carry no length bound; only
  `error` does.
- The excerpt window is displaced for any character whose lowercase mapping
  changes length (`first_token` indexes the folded string, `window` slices the
  source), and excerpt cutting is unbounded in one record's size.
- `time_bound` validates shape but not the calendar, so `--since 2026-08-32` is
  accepted and silently hides the month with exit 0.
- `Document::emit` uses `println!`, which panics with exit 101 on a closed
  stdout pipe; the human path has a `broken_pipe` helper and the JSON path does
  not.
- `recall_get`'s exclusion arm names an excluded project's absolute path in a
  `reason`, where the scope arm deliberately answers `NoSuchTurn` so as not to
  confirm another project holds the id.
- The MCP tool result shape is not pinned in any document; `docs/json-shapes.md`
  is the CLI contract only.
- Plan 4's `risk_surface` gate was still in flight when this summary was
  written. Its findings land in `REVIEW-risk_surface-plan-4.md`.

## Goal check

The goal is finding a specific past turn, from the terminal and from inside
Claude Code, by identifier, path, error or free text, and the 38 commits deliver
each of those routes with a test naming it. Free text and identifier: `index`
projects turn text instead of the raw JSON line (0dc1796) and expands case and
separator components (42cdcd6), so `expansion_tokens("SearchManager")` yields
`["Search", "Manager"]` (`tests/index.rs:312`) and SC1's `manager` -> `SearchManager`
case is asserted there. Path: `entity.rs` writes a `paths` row per structured
path (7e98268), corrected twice by the gate for the `Read` result shape and for
shell punctuation (f7db432, 5bb3f97), measured over the real corpus at 149,684
path entities (c27e15e). Error: `normalize_error` collapses timestamps, UUIDs,
addresses and `:line:col` (5f6e10c) with the a/b fixture pair asserting one
shared value, SC2. Terminal: `success_and_an_empty_result_both_exit_zero` and
`every_data_command_emits_the_documented_shape` (`tests/cli.rs:795,684`) carry
SC3. Inside Claude Code: `mcp.rs` carries SC4 (`recall_search_is_scoped_to_the_
directory_the_server_was_started_in`, `the_star_project_reaches_more_than_one_
project`), SC5 (`an_evicted_body_is_a_flag_and_not_a_failed_call`, `a_malformed_
call_is_an_empty_result_with_a_reason_and_never_a_throw`) and SC6 (`the_handshake_
reports_three_read_only_tools_and_ends_at_eof`, plus the socket probe).
30 test binaries pass and clippy is clean at `d457c06`.

What is honestly not settled: the phase shipped five high-severity defects that
only a blocking adversarial gate caught, three of them cross-project read paths
that the plans' own tests passed over - `longest_prefix` resolving to the wrong
project, `context::window` reading across projects by enumerable id, and
`recall_get` naming an excluded project in a reason. Scoping is the phase's
authorization boundary and it was the phase's weakest tested surface; phase 4's
verification should treat it as such rather than trusting these tests. The
bounds noted in open items (`MAX_BODY_BYTES`, unbounded entity values, unbounded
excerpt cutting) are all cost rather than correctness on today's corpus, and
none is measured against a corpus larger than 984 MB.
