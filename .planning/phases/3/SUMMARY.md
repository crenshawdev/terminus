---
phase: 3
status: complete
completed: 2026-10-13
---

# Phase 3: Install Asks Nothing - Summary

`terminus install` now asks nothing and takes no `--yes`, places the MCP entry with `claude mcp add-json --scope user` instead of writing `~/.claude.json`, reports itself incomplete with the finishing command when `claude` is missing or fails, and ends with a receipt; uninstall removes the entry with `claude mcp remove`.

## What shipped

- A runner for `claude` (PATH lookup, no shell, null stdin, stderr drained, 30 s deadline) - `crates/terminus/src/cmd/install/claude.rs`
- MCP registration through `claude mcp`: nothing run when the entry is current, add, or remove-then-add - `crates/terminus/src/cmd/install/mod.rs:269-285`, `targets::mcp_entry_is_current`, `targets::mcp_entry`
- An incomplete install when registration cannot finish: binary, hooks and backfill still run, the summary names the cause and the `to finish, run` command, exit 1 - `crates/terminus/src/cmd/install/mod.rs`, `targets::finishing_command`
- No confirmation and no `--yes`: the diff-and-confirm path, `Options`, `offer`, `json_file::diff` and its helpers removed - `crates/terminus/src/cmd/install/`
- The receipt: `cleanupPeriodDays` at every value, that older transcripts are already gone, the line that raises it, the backfill estimate and `terminus status` as where to watch it - `crates/terminus/src/cmd/install/mod.rs:311-345`
- Uninstall removes the registration with `claude mcp remove --scope user terminus` and leaves `.claude.json` and an old `.claude.json.terminus-backup` untouched; confirmation and `--purge` unchanged - `crates/terminus/src/cmd/uninstall.rs:301`
- Doctor's MCP fix is `<exe> install` when `claude` is on PATH and the `claude mcp` command when it is not; doctor runs `claude --version` with nonessential traffic off - `crates/terminus/src/cmd/doctor.rs`
- A fake `claude` every install, uninstall and doctor test runs against - `crates/terminus-core/src/testkit.rs` (`fake_claude`, `fake_claude_calls`)
- DESIGN-BRIEF.md, docs/json-shapes.md and CHANGELOG.md describe install as it now runs

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | dd9fc8b | Run every install, uninstall and doctor test against a fake `claude` |
| 1 | 2 | fd16846 | Place the MCP entry through `claude mcp` |
| 1 | 3 | 8da4679 | Finish the rest of install when registration cannot, and print the finishing command |
| 1 | 4 | ee592ba | Install applies its defaults, asks nothing, and treats `--yes` as misuse |
| 2 | 1 | 51c61c3 | End install with a receipt stating `cleanupPeriodDays` and the line that raises it |
| 2 | 2 | 4f36378 | Uninstall removes the registration through `claude mcp remove` |
| 2 | 3 | 9df58e1 | Doctor's MCP fix names the `claude mcp` command when `claude` is not on PATH |
| 2 | suite repair | 01402d1 | Retry the fake `claude` while exec reports it busy in the runner unit tests |
| 3 | 1 | 3900074 | Design brief and JSON shapes doc describe install as it now runs |
| 3 | 2 | 947f223 | Changelog records the change |

## Deviations

- [deviation] Plan 1 task 1: `a_data_directory_that_does_not_exist_is_a_state_and_stays_absent` failed because doctor's `claude --version` made the fake create its log in the fixture root. The fake now creates its log up front, so the root holds the same names whether or not anything ran. dd9fc8b
- [deviation] Plan 1 task 3: the Verify check "output does not contain `second`" could not hold, because `backfill::print_estimate` prints "under a second" and the `cleanupPeriodDays` advisory says "second recovery path". The test asserts the intended property instead: the mcp line ends `(not registered: first)` and no output line reads `second`. 8da4679
- [deviation] Plan 2: the first full suite run failed `cmd::install::claude::tests::every_claude_gets_the_traffic_variable` with ETXTBSY, because another test thread's fork held the fake script's write descriptor. The runner unit tests now retry while exec reports error 26; the runner itself is unchanged. 01402d1
- [deviation] Plan 3: the first run's final suite failed `a_hundred_session_starts_stay_inside_the_budget_and_the_wall_clock` (`crates/terminus/tests/brief.rs:256`, 10 ms p99) once, and the executor ran the suite three times against its one-run allowance. Plans 3-1 and 3-2 touch no file on the SessionStart hook path; alone the test passed three times at p99 1.39, 2.35 and 2.35 ms. The user ruled it a wall-clock flake under parallel load. A continuation ran the suite once, green (764 passed), with no code change.

## Open items

- The SessionStart wall-clock test at `crates/terminus/tests/brief.rs:256` can fail under the full parallel suite while passing alone well under its 10 ms limit. Changing it is outside this phase.
- `terminus_core::testkit::fake_claude` writes its script the same way the runner unit tests did, so the integration tests are open to the same ETXTBSY window. Not seen in any run. The fix would be the same retry, or writing the fake once per fixture before any spawn.
- `Json::number` was removed with the `cleanupPeriodDays` offer, its only caller. It comes back if a later change writes a number.

## Goal check

The ten commits plausibly deliver the goal and all six success criteria, and the full suite passed (`cargo test --workspace --all-features`, 764 passed, per `reports/plan-3.md`). Install asks nothing and rejects `--yes`: `install_takes_no_yes_flag` and `install_reads_nothing_from_stdin` (`crates/terminus/tests/install.rs:928`, `:969`). The entry is placed with `claude mcp add-json --scope user` (`crates/terminus/src/cmd/install/mod.rs:285`), and a current entry runs no `claude` (`mod.rs:269`, test `a_current_mcp_registration_runs_no_claude` at `install.rs:784`). A missing or failing `claude` leaves the install incomplete with the finishing command (`install.rs:838`, `:850`, `:871`). The receipt states `cleanupPeriodDays`, the raise line, the backfill estimate and `terminus status` as where to watch it (`mod.rs:311-345`, tests at `install.rs:1059`, `:1124`). Uninstall runs `claude mcp remove --scope user terminus` (`crates/terminus/src/cmd/uninstall.rs:301`), and its four purge tests are unchanged per `reports/plan-2.md`. Criterion 4, the hook merge with a backup and a no-op rerun, was not reworked this phase. It rests on the existing settings backup path (`mod.rs:123-127`) and the INST-05 rerun test (`install.rs:626`), which still pass. Nothing in the goal looks missing. Two open items concern test reliability, not behaviour.
