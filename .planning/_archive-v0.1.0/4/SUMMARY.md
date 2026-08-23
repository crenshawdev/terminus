---
phase: 4
status: complete
completed: 2026-08-20
---

# Phase 4: Hooks And Install - Summary

`verbatim install` wires this build into Claude Code in one command: four exec-form
hook entries plus an MCP registration pointed at a stable binary path, a shown-and-
confirmed diff of every file it touches, a detached backfill of the user's whole
history, and `doctor` / `uninstall` / `npx verbatim` around it.

## What shipped

- The detached hook spawn - `crates/verbatim/src/cmd/spawn.rs`, `cmd/hook.rs`. Four events, p99 0.65-0.76 ms against a 10 ms budget, surviving both of Claude Code's kills.
- `verbatim install` - `crates/verbatim/src/cmd/install/`. Stable path, order-preserving settings editor, LCS diff, one confirmation, `--yes`, idempotent on rerun and on upgrade.
- `verbatim doctor` - `crates/verbatim/src/cmd/doctor.rs`. Read-only, 14 named checks, `--json`, creates nothing.
- `verbatim uninstall` (`--purge`) - removes only what install added, restores the backup when the file is otherwise unchanged, deletes the archive only after showing its size and confirming.
- `verbatim backfill` - `crates/verbatim-core/src/ingest/backfill.rs`, `crates/verbatim/src/cmd/backfill.rs`. Estimate first, detached, chunked, resumable, bounded parallelism with a single SQLite writer.
- The npm distribution - `npm/`. Thin package with a JS shim, five per-platform optional dependencies, no postinstall script anywhere.

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 1a4cb50 | A detached spawn that survives both of Claude Code's kills |
| 1 | 2 | 1f074c9 | `verbatim hook <event>` for the four events |
| 1 | 3 | b11547e | The four real payloads, the budget and the clean stdout |
| 1 | 4 | 0daae3d | The kill Claude Code actually performs |
| 1 | F1 | 9c8f408 | Bound the hook's stdin drain in bytes and in time |
| 1 | F2 | 437c458 | Select the reparenting hand-off by argument, not by environment |
| 2 | 1-6 | c8d8120, f66e37e | `verbatim install` wires this build into Claude Code, and its tests |
| 2 | F1 | a0bf2a3 | Diff the settings file, not a re-rendered copy of it |
| 2 | F2 | c5ea16c | Refuse a settings file that repeats a key |
| 2 | F3 | 07a59aa | Create install's temporary files exclusively |
| 2 | F4 | 84f149e | Write the backup through a temporary and take its name atomically |
| 2 | F5 | 465d093 | Look at the stable path again before renaming over it |
| 2 | F6 | 6cb789d | Repair an entry of ours in the wrong shape rather than skip it |
| 3 | 1 | 569818e | `verbatim doctor` reports the wiring and the command that fixes it |
| 3 | 2 | 9b76dff | Doctor reports the store and the last run without creating either |
| 3 | 3 | b18ed95 | Doctor names the two Claude Code settings it will never change |
| 3 | 4 | eccb89c | `doctor --json` emits one document of named checks |
| 3 | 5 | 6756a42 | Uninstall removes what install added and restores the rest |
| 3 | 6 | 1ad6320 | `uninstall --purge` asks before it deletes the archive |
| 5 | 1 | 1ad3555 | The npm package, one thin manifest and five per-platform |
| 5 | 2 | b4c671c | The shim that finds the platform binary and gets out of the way |
| 5 | 3 | acc5474 | Pack it, install the tarball, prove the version |
| 4 | 1 | 8f32c8c | Backfill says how much there is before it reads any of it |
| 4 | 2 | d57e61f | Backfill hands the shell back and keeps working |
| 4 | 3 | 90af030 | Backfill parses and compresses on workers, one thread writes |
| 4 | 4 | 1dad34f | Backfill runs the pipeline, and converges after a kill |
| 4 | 5 | 7718115 | Install starts the backfill and hands the shell back |
| 4 | R1 | 977d0b4 | A panicking worker is one skipped file, not a wedged pass |

Reports: `.planning/phases/4/reports/plan-1.md` .. `plan-5.md`.
Range `59aa49c..977d0b4`, 32 commits. Plans 4 and 5 ran in parallel worktrees and
merged at `4f4c8a3` and `cd096ce`.

## Deviations

- [deviation] Plan 4 task 1: the plan's real-corpus yardsticks are both looser than what they measure. `find -name '*.jsonl'` counts 3,119 to backfill's 3,115, and all four extra are the `journal.jsonl` files D-16 documents as non-transcripts; `du -sb` overcounts by 6.6% because it counts directory inodes. Backfill's numbers are the correct ones. No code changed. (8f32c8c)
- [deviation] Plan 4 task 1: the plan calibrates the time estimate as the sequential rate divided by the worker count. Measured, that model is wrong - the same 1.41 GB corpus took 49,089 ms through four workers and 53,007 ms sequentially, an 8% win, because the single SQLite writer plus derived rows and FTS dominate the stages that moved. Dividing by `WORKERS` would print "about 10s" for a fifty-second job, so the estimate is calibrated on the pipeline's own measurement and a unit test asserts it is not the divided figure. (8f32c8c)
- [deviation] Plan 4 task 3: `pass::record_pass` is private and `ingest/pass.rs` is outside this plan's lease, so the `runs` INSERT and the per-file failure arm are a second copy carrying a comment that says so. See open items. (90af030)
- [deviation] Plan 4 task 5: starting the backfill unconditionally broke AC6's "doctor against a data directory that does not exist creates nothing". Fixed at the cause - `backfill::start` returns `false` without spawning when the estimate finds no transcripts at all - rather than in the test, which is outside the lease. (7718115)
- [deviation] Plan 5 tasks 2-3: the plan names `target/release/verbatim`, which does not exist here (`CARGO_TARGET_DIR=/mnt/ramdisk/cargo-target`). `pack-local.sh` now reads `target_directory` from `cargo metadata` instead of hardcoding `target/`. (acc5474)
- [deviation] Plan 5 task 3: npm 12.0.2 does not honour `--prefix` for `pack`, and emits `pack --json` as an object rather than an array. Used the positional spec `./npm/verbatim` and `[.[]][0]`, making the identical assertion. (acc5474)
- [deviation] Plans 4 and 5: the dispatch prompts named branches `cadence/phase-4-plan-4` and `-plan-5`; the host provisioned `worktree-agent-<id>` branches instead. Both forked from `1ad6320` and were merged from their real names.
- [deviation] Post-merge: the blocking `risk_surface` gate matched `concurrency` on plan 4 and raised one `high` finding, fixed in 977d0b4. See Goal check.

## Open items

- The shim does not forward signals to the child. `spawnSync` blocks the event loop, so a `SIGTERM` aimed at the shim's own PID kills node and orphans the binary; Ctrl-C and job control signal the whole process group and are unaffected. Raised by the `risk_surface` review on plan 5, adjudicated `downgraded` (medium, below the blocker/high bar this gate fixes) - `.planning/phases/4/ADJUDICATION-risk_surface-plan-5.json`.
- Hoist `pass::record_pass` and `pass::walk`'s per-file failure arm so `pass` and `backfill` share one `runs`-row writer and one skip rule. Needs a plan whose lease covers `crates/verbatim-core/src/ingest/pass.rs`.
- The four-worker pipeline is only 8% faster than the sequential pass. The remaining cost is the writer's: batching derived-row inserts, or moving `derive_turn`'s text expansion and entity extraction onto the workers. Worth a measurement-led task.
- `verbatim status` can fail with "database is locked" in the ~1 ms window while a backfill creates the store and sets `journal_mode=wal` (1 of 20 polls, at t=1 ms). Pre-existing in `Store::open`.
- AC3's Windows half - no console window, no inherited handle - is unrunnable on Linux and stays a human-verify on a Windows machine.
- npm's `os`/`cpu` selection of an optionalDependency cannot be exercised locally, only the shim's resolution of an already-placed package. AC9's remaining risk lives in the publish step (D-21).
- Four of the five platform packages carry no binary; they arrive with cross-compilation in a later shipping step.
- The npm name `verbatim` and the `@verbatim` scope have not been checked for availability, and crates.io's `verbatim` is taken. Publish-step question for the human.
- `ingest::backfill` does not honour `fault::pass_fails_after`; the sequential walk still does.
- `cargo fmt --check` reports two pre-existing diffs in `crates/verbatim/tests/hook.rs`, both on lines committed in plan 1 and neither in code any later pass wrote.
- Declined an `engines.node` field on the thin package; add one when a task states a minimum.
- `shellcheck` is not installed here, so `pack-local.sh` got `bash -n` only.

## Goal check

The phase goal is verbatim wired into Claude Code by a single install command,
keeping itself current with no user action and no measurable cost at the hook,
and the commits deliver all three clauses with one caveat. *Single command*:
`c8d8120`/`f66e37e` place the binary at the stable path and append four exec-form
hook entries plus `.mcpServers.verbatim`, behind one confirmation, and rerunning
leaves exactly four entries and one registration with one backup (plan-2 report,
tasks 3-4). *No measurable cost at the hook*: `b11547e` measured debug-build p99
per event at 0.76 / 0.65 / 0.69 / 0.68 ms against a 10 ms budget, and `0daae3d`
shows the spawned ingest surviving the group kill and the descendant sweep that
Claude Code actually performs. *Keeping itself current* is INST-05, and plan 2
task 3's upgrade test replaces the bytes at the stable path and asserts
`settings.json` comes back byte-identical - the entries point at a path, so an
upgrade never rewrites one. The caveat is that "with no user action" is carried
by npm rather than by anything in this repo: nothing here polls or self-updates,
so currency is exactly as automatic as the user's npm update habit, which is a
distribution decision and not a gap in the code. ING-11 is met end to end -
`verbatim backfill` over the real 1.41 GB / 3,123-transcript corpus returned the
shell in 6.3-7.4 ms against a 100 ms budget and printed an estimate matching the
49 s the pipeline took, and `1dad34f`'s eight SIGKILLs at seeded random delays
each converged on the uninterrupted run's blobs, turns, FTS rowids and
watermarks. The one real defect the phase produced was found after the merge, not
by the executors: the backfill's drain loop waits for exactly as many `Done`
messages as it sent and its disconnect arm only fires once every worker is gone,
so a single worker panicking inside `prepare` hung the pass with no error and no
exit. `977d0b4` catches the unwind and records that file as a per-file failure;
the falsification is a 60 s timeout with the fix reverted and nothing else
changed. What is genuinely not proven here is the Windows half of AC3 and npm's
platform selection at publish time - both are named above and both need a machine
or a registry this phase does not have.
