PLAN COMPLETE
Plan: .planning/phases/5/PLAN-2.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1. The last session in this project, and the branch it ended on | a6e423c | Last non-subagent `session_meta` row by greatest `last_turn_at` (tie-broken on `session_key`), day-resolution date, branch, and last user/assistant turn cut from one blob read via `excerpt::of_record`. New `crates/verbatim-core/tests/inject_brief.rs`: 4 passed (later-session-wins, sidecar-excluded, null-branch, and a source check that `src/inject/` names no `Command` and no `observations`). `cargo test -p verbatim --test inject` 7 passed unedited; clippy `--all-targets -D warnings` clean. |
| 2. The brief inside its configured budget | 116dae2 | `brief_chars` enforced in characters over the whole brief, cutting only the two quoted turns and marking every cut with `excerpt::ELISION`; `MAX_BRIEF_CHARS = 10_000` clamps the config. `cargo test -p verbatim-core --features testkit --test inject_brief` 6 passed, including a falsifying check that the unbudgeted brief really is over the budget, and a ceiling case driven by an 11,000-character project key. |
| 3. Nothing volatile, byte for byte | ef45d2b | New `crates/verbatim/tests/brief.rs`: two `verbatim hook SessionStart` spawns against a store ingested to completion first, stdout compared byte for byte, plus a character-window scan proving no `NN:NN` appears anywhere in the output. `cargo test -p verbatim --test brief` 1 passed. The no-clock and total-ordering rules are stated in `brief.rs`'s module doc. |
| 4. The brief inside its wall budget, over a hundred runs | fa4fbde | 100 `SessionStart` spawns against a store seeded to completion, each exit 0, exactly one JSON object under `hookEventName: SessionStart`, `additionalContext` inside the `brief_chars` the test wrote into `verbatim.toml`. Measured debug-build **p50 1.27 ms, p99 2.35 ms** against the 10 ms budget `tests/hook.rs` asserts; both printed. `cargo test --workspace --features verbatim-core/testkit` green, 0 failed, with `tests/hook.rs`'s three `stdout.is_empty()` sites unedited (D-11). |

Deviations: none
Open items:
- `cargo fmt -p verbatim-core` reformats `crates/verbatim-core/src/ingest/backfill.rs`, a pre-existing rustfmt diff plan 1 already recorded. It was reverted rather than committed: the file is outside this plan's lease.
