# Task: ignore-a-closed-stdout-pipe

## What shipped

- Every stdout write in the binary goes through outln!/out!, which drop output once the reader closes the pipe; the command finishes and exits with its own code. show no longer exits 1 on a closed pipe. A CLI test holds five commands to the same exit code with the pipe closed. Also fixes a compaction test the earlier verify-message change missed.

## Commits

| Task | Commit | Description |
| --- | --- | --- |
| 1 | 5808f889756201d79bec5e4d8088ca35c21cd663 | fix(cli): drop stdout once the reader closes the pipe instead of panicking |
| 1 | 4c1fea6a6e3047bc9bc10caf93451b610a1d5ef1 | test(verify): expect the shortened-or-rewritten wording in the compaction test too |
| 1 | 6f2306713fd17b07d5817b0e028f439bf76a42da | test(cli): prove a closed stdout changes no exit code and prints no panic |
| 1 | 69ad764532504b7035b647474b1c30489dac543f | docs: drop the two closed-pipe todos now that they are fixed |
| 1 | 656f7e3adf996136c3e72ace993e9a11180fc418 | style(cli): format the closed-stdout test |
| 1 | 123035473a8b7e30c49abcc209bf34d347bef2b2 | docs: task plan ignore-a-closed-stdout-pipe |

## Files

### Task 1: ignore-a-closed-stdout-pipe

- **Files:** .planning/CAPTURE.md, .planning/tasks/ignore-a-closed-stdout-pipe/PLAN.md, crates/terminus-core/tests/compaction.rs, crates/terminus/src/cmd/backfill.rs, crates/terminus/src/cmd/compact.rs, crates/terminus/src/cmd/data.rs, crates/terminus/src/cmd/export.rs, crates/terminus/src/cmd/hook.rs, crates/terminus/src/cmd/install/mod.rs, crates/terminus/src/cmd/json.rs, crates/terminus/src/cmd/observations.rs, crates/terminus/src/cmd/replay.rs, crates/terminus/src/cmd/retention.rs, crates/terminus/src/cmd/search.rs, crates/terminus/src/cmd/sessions.rs, crates/terminus/src/cmd/show.rs, crates/terminus/src/cmd/stats.rs, crates/terminus/src/cmd/status.rs, crates/terminus/src/cmd/uninstall.rs, crates/terminus/src/cmd/usage.rs, crates/terminus/src/cmd/verify.rs, crates/terminus/src/main.rs, crates/terminus/src/out.rs, crates/terminus/tests/cli.rs
