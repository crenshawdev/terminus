PLAN CHECKPOINT: structural
Plan: .planning/phases/1/PLAN-3.md
Tasks: 1 of 2
| Task | Commit | Note |
|---|---|---|
| 1: The state file names the turn the brief actually quoted | 3c6a55a | `cargo test -p verbatim --test suppression` 6 passed. Falsified both ways: with PLAN-2's `AND is_typed IS NOT 0` reverted the new case fails on the prose assertion, and with that assertion removed it fails on the id it recorded (`[16777218, 16777217] without 16777216`). Production code restored and re-verified green before the commit. Test-only, as D-12 predicts. |
| 2: The cold-start budget, measured on the real corpus | none - written, uncommitted | The measurement is implemented in `crates/verbatim/tests/corpus.rs` and works. Ungated arm passes and prints the skip line. Gated arm FAILS its own budget: `SessionStart: p99 10.52 ms is over 10 ms`. See the deviation. |

Deviations:
- [deviation] The plan asserted (Must-be-true 2, CONTEXT AC4) that 100 spawns of
  each of the four hook events stay under the same 10 ms p99 `hook.rs` asserts,
  against a store built from the real corpus. Measured, that is false in the
  DEBUG profile `cargo test` builds by default and true in RELEASE. Store built
  from `$HOME/.claude`: 3,580 sessions, 456,038 turns; the project the brief
  resolves to holds 498 sessions and 70,302 turns.
  - debug: SessionStart p50 10.38 ms, p99 10.85 ms; UserPromptSubmit p99 0.80;
    SessionEnd 0.64; PostCompact 0.63.
  - release, same store, same 100 runs: SessionStart p50 3.90 ms, p99 4.26 ms;
    the other three at or under 0.56 ms p99.
  - Not caused by this phase. With PLAN-2's `AND is_typed IS NOT 0` reverted the
    debug number is p50 10.24 ms against 10.38 ms with it - inside the noise,
    and consistent with D-11's 0.002 ms / 0.362 ms measurement of that lookup.
  - The cost is v0.1.0's INJ-01 index pointer, and it scales with the scoped
    project. Same store, same debug binary, only the payload's `cwd` changing:
    a `cwd` that resolves to no project (early return, no render) 3.83 ms p50; a
    project holding 4 turns (brief fully rendered) 4.09 ms; the 70,302-turn
    project 10.36 ms. So ~3.2 ms is the fixed store open plus
    `scope::resolve` -> `visible::projects`, and ~6.3 ms is
    `index_pointer`'s `SELECT count(*) FROM turns t JOIN session_meta m USING
    (session_key) WHERE m.project = ?` (1.74 ms warm in C SQLite alone,
    `crates/verbatim-core/src/inject/brief.rs:580`).
  - What I did: stopped rather than pick a budget or a profile. Both repairs
    are outside this plan's lease (`inject/brief.rs`) or change what AC4 asserts,
    and the plan's `Verify:` cannot be met as written.

Open items:
- The corpus-built store is still on disk at
  `/tmp/claude-1000/-code-verbatim/4a1f7226-2bce-4ccf-b1e3-bcaa916fc077/scratchpad/store`
  (2.2 GB, tmpfs), with the timing harness beside it as `bench.py` /
  `bench_release.py`. A continuation can re-measure in seconds instead of
  paying the 309 s ingest again.
- `UserPromptSubmit` injected nothing on all 100 runs against the real corpus
  (the fixture's prompt, scoped to the newest project). Expected under INJ-05's
  precision-first thresholds and not asserted either way, but it means only
  `SessionStart` is measured doing injection work.
