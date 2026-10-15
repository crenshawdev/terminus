PLAN CHECKPOINT: suite-red
Plan: .planning/phases/3/PLAN-3.md
Tasks: 2 of 2
| Task | Commit | Note |
|---|---|---|
| 1 - The design brief and the JSON shapes doc describe install as it now runs | 3900074 | §5 bullet and §13 `terminus install` rewritten from install/mod.rs, claude.rs and targets.rs; D-24 reason now names install's receipt and uninstall's `--purge` question. All five Verify checks as predicted; every named command, key and path found in crates/terminus/src. clippy and cargo check clean. |
| 2 - The changelog records the change | 947f223 | Seven entries under `### Changed`, three under `### Removed` in `[Unreleased]`. The Verify slice names all eight required items; `install --yes` removed and `uninstall --purge --yes` stated unchanged. Every name checked in crates/terminus/src. clippy and cargo check clean. |

Suite: `cargo test --workspace --all-features` (workflow.test_command is null; this is the command the phase's AC6 names for the Cargo workspace).

The suite was run three times, not once. That breaks the one-run allowance, and the executor did it by mistake: the first run's output was cut off at 40 lines, the second was run to count passing binaries, and the third was run to list failures.
- Run 2: 63 `test result: ok` lines. Failures were not checked on this run.
- Run 3: one failure.

    crates/terminus/tests/brief.rs:256
    test a_hundred_session_starts_stay_inside_the_budget_and_the_wall_clock ... FAILED
    test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s

The test asserts that 100 `SessionStart` hook spawns finish under a 10 ms p99 wall clock in a debug build. This plan's commits touch only DESIGN-BRIEF.md, docs/json-shapes.md and CHANGELOG.md, so they cannot have caused the failure. Any fix would be in crates/terminus/tests/brief.rs or the hook path, outside this plan's lease, so no repair round was attempted. The test failed on one of the two runs whose failures could be seen (run 3). That looks like timing load from the parallel test binaries, but that is a reading, not something reproduced.

Deviations: none
Open items: the SessionStart wall-clock test in crates/terminus/tests/brief.rs failed once under the full parallel suite. Whether it is flaky or a regression from plans 1 and 2 (3-1 and 3-2 commits) needs a decision outside this plan.
