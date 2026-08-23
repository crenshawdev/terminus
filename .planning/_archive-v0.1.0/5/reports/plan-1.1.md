CHECKPOINT: human-verify
Plan: .planning/phases/5/PLAN-1.md
Tasks: 4 of 5
| Task | Commit | Note |
|---|---|---|
| 1. Injection settings in `verbatim.toml` | 24bf33b | `[injection]` table with `brief_chars` (6000) / `prompt_chars` (4000) defaults; `cargo test -p verbatim-core --test config` 19 passed; `cargo test --workspace` green with no other test edited. |
| 2. The hook reads its payload instead of dropping it | 624b47a | `cmd::hook::Payload` off a `serde_json::Value`, five optional fields; two new `#[cfg(test)]` cases over the four recorded fixtures (bin unit tests 42 -> 44 passed); `cargo test -p verbatim --test hook` 7 passed unedited. |
| 3. One JSON object on stdout, on the two events that have one | d102635 | `verbatim_core::inject` with `brief`/`prompt` arms; `cargo test -p verbatim --test inject` 4 passed; whole workspace green (37 test binaries, 0 failed). |
| 4. Every way injection can fail is silence, inside the deadline | 5489773 | 50 ms watchdog thread + caught panic in the binary, 3 ms busy timeout and a version gate in core; `cargo test -p verbatim --test inject` 7 passed (3 AC6 cases), `cargo test -p verbatim-core --lib` 3 passed; workspace 37 binaries, 0 failed. |
| 5. What a real compaction actually sends | - | Not run: the plan marks it human-verify. It needs a live Claude Code session, a temporary `SessionStart` entry in `~/.claude/settings.json` that appends its stdin to a file, and a `/compact` - none of which an executor can perform. |

Deviations:
- [deviation] Task 4's Action asserts that lowering the read-only connection's busy timeout makes a prompt submitted against a held store "get an immediate `SQLITE_BUSY`". Measured otherwise: `Store::open_read_only` applies its own 5 s timeout and runs a `sqlite_master` query BEFORE returning, so the lowered timeout arrives too late to bound the open. Against a store held under `PRAGMA locking_mode = EXCLUSIVE`, a holder in the SAME process makes the open wait the full 5,011 ms, while a holder in ANOTHER process (the real case, and AC6's) is reported back in about 6 ms. Both mechanisms were still built as the plan asks; the constant's doc and both tests now state what each one actually bounds - the busy timeout covers every statement after the open, and the hook's watchdog is the only general bound on the wait. AC6 is met either way and no criterion changed. Bounding the open itself would need `crates/verbatim-core/src/store/open.rs`, which is outside this plan's lease.
Open items:
- The AC6 exclusive-writer case is bounded by SQLite rather than by the 50 ms watchdog on Linux, so the watchdog's timeout arm has no test that forces it. Forcing it needs a fault point in the injection path (phase 4's `testkit` pattern); not built here.
- `crates/verbatim/tests/hook.rs` flakes with `Text file busy` (ETXTBSY) at its `spawn the hook` sites when its parallel tests `fs::copy` the binary and exec the copy while a sibling test is forking. Reproduced at 1 of 6 runs on the pre-phase-5 baseline `3fd5b19` in a scratch worktree, and at 1 of 4 on this branch, so it pre-dates this phase; the file is outside this plan's lease and was not touched.
- `cargo fmt --check` reports pre-existing diffs in `crates/verbatim-core/src/ingest/backfill.rs:250` and `crates/verbatim/tests/hook.rs:252,681` under rustfmt 1.9.0. Neither file is touched by this plan.
