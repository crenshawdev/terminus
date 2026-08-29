PLAN COMPLETE
Plan: .planning/phases/1/PLAN-3.md
Tasks: 2 of 2
| Task | Commit | Note |
|---|---|---|
| 1: The state file names the turn the brief actually quoted | 3c6a55a | Committed by the previous dispatch; not re-run. `cargo test -p verbatim --test suppression` 6 passed. Falsified both ways: with PLAN-2's `AND is_typed IS NOT 0` reverted the new case fails on the prose assertion, and with that assertion removed it fails on the id it recorded. Test-only, as D-12 predicts. Full record in `plan-3.1.md`. |
| 2: The cold-start budget, measured on the real corpus | c313888 | Built on the previous dispatch's uncommitted measurement; only the wall changed, to the profile-aware one the user chose. Ungated arm: 2 passed, prints `skipping: VERBATIM_TEST_CORPUS is unset, so there is no real corpus to measure`. Gated arm against `$HOME/.claude`: passed in 685.55s. `SessionStart: p50 18.48 ms, p99 19.72 ms over 100 runs on the corpus store (debug build, wall 20.0 ms)`; `UserPromptSubmit` p99 0.86; `SessionEnd` 0.70; `PostCompact` 0.71. `SessionStart` stdout non-empty on all 100 runs. Grepped the whole 685 s log for paths, uuids and session keys: the only match is the pre-existing `corpus: <path>` echo of the env var the developer supplied, which is not derived from the store. `cargo clippy --all-targets -- -D warnings` and `cargo check --all-targets` both exit 0; `rustfmt --check` on `corpus.rs` exits 0. |

Deviations:
- [deviation] The plan's task-2 `Verify:` and its `## Must be true when done`
  bullet 2 both assert a flat 10 ms p99 for the gated arm, matching CONTEXT AC4.
  That is false in the DEBUG profile `cargo test` builds by default and true in
  RELEASE. The user, dispatched with the previous run's checkpoint, chose a
  profile-aware wall and it is what shipped: 10.0 ms asserted when
  `cfg!(debug_assertions)` is false, 20.0 ms when it is true, with the profile
  and the wall in force named on the printed per-event line. CONTEXT AC4 is left
  as written, per the dispatch. The overage is NOT this phase's `AND`: with
  PLAN-2's `AND is_typed IS NOT 0` reverted the previous run measured a debug
  p50 of 10.24 ms against 10.38 ms with it, inside the noise and consistent with
  D-11's 0.002 ms / 0.362 ms measurement of that lookup. It is v0.1.0's INJ-01
  index pointer, `crates/verbatim-core/src/inject/brief.rs:580`, whose
  `SELECT count(*) FROM turns JOIN session_meta` scales with the scoped project.
  Release, previous run, same store, same 100 runs: `SessionStart` p50 3.90 ms,
  p99 4.26 ms, the other three at or under 0.56 ms p99.

- [deviation] The premise the 20 ms debug wall was chosen on no longer holds.
  The user's instruction states "The measured debug p99 was 10.85 ms, so 20.0 ms
  is deliberate headroom." Re-measured today on a freshly built store, the debug
  numbers are p50 18.48 ms and p99 19.72 ms - the wall passes with 1.4% margin,
  not the 2x the decision assumed. What I did: shipped the 20 ms the user chose
  and did not adjust it, because the number is theirs to set and the dispatch
  explicitly said not to tune the wall to the measurement. See open items for
  what changed and why this is growth rather than noise.

Open items:
- The debug wall will not survive the next corpus growth, and the growth is this
  project's own. The previous dispatch's store held 3,580 sessions / 456,038
  turns and measured `SessionStart` at 10.85 ms p99. Today's store, built hours
  later from the same `$HOME/.claude`, holds 4,360 sessions / 661,622 records
  and measures 19.72 ms p99. This is corpus growth and not run-to-run noise: the
  p50-to-p99 spread is 1.24 ms, so the distribution is tight around a genuine
  ~18.5 ms cost, where contention noise would show a low p50 with a long tail.
  The cost scales with the SCOPED project, and the project the payload scopes to
  is `verbatim` itself, which every executor dispatch in this phase appends
  transcript to. So the measurement climbs each time the phase is worked on. A
  decision is owed on whether the debug arm should assert a wall at all, or only
  print; the release arm at 10 ms is unaffected and had 58% margin.
- `cargo fmt --check` fails across the workspace on files committed by plans 1
  and 2 - `crates/verbatim/tests/hook.rs`, `crates/verbatim/tests/retention.rs`,
  `crates/verbatim-core/tests/{inject_brief,parse,reindex,retention,verify}.rs`.
  Fourteen diffs, all line-wrapping of `assert!` and `format!` calls, plus one
  import order in `verbatim-core/tests/retention.rs`. Outside this plan's lease
  and not caught by the project's detected lint or typecheck commands, neither
  of which runs `rustfmt`.
- `crates/verbatim/tests/hook.rs`'s
  `the_ingest_survives_a_group_kill_and_a_descendant_sweep` is flaky under a
  full workspace run. It failed once on the first `cargo test --workspace`, then
  passed 3/3 targeted, 3/3 as the whole `hook` binary, and again on a clean
  `cargo test --workspace --no-fail-fast` where every binary in the workspace is
  green. Not attributable to this commit by construction: `corpus.rs` is a
  separate test binary and with `VERBATIM_TEST_CORPUS` unset it returns before
  any of the code this task added runs. The test reads the live process table
  for a `verbatim ingest` argv, which other binaries in the same suite also
  spawn, so a foreign ingest is the likely confound. Pre-existing and outside
  this plan's lease.
- `UserPromptSubmit` injected nothing on all 100 runs against the real corpus
  (the fixture's prompt, scoped to the newest project). Expected under INJ-05's
  precision-first thresholds and not asserted either way, but it means only
  `SessionStart` is measured doing injection work.
- The corpus-built store this run used is gone: the test builds it in a
  `tempfile::tempdir()` that is removed when the pass returns, and the previous
  dispatch's copy at its own scratchpad path had already been reclaimed. A
  further measurement pays the 355 s ingest again.
