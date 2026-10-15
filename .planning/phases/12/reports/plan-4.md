PLAN COMPLETE
Plan: .planning/phases/12/PLAN-4.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Call the shared chars and clip from brief.rs (#35) | cce2eb2 | brief.rs private pair deleted, imports `super::{chars, clip, Payload}`; inject_brief 15, inject_compaction 4, inject_prompt 10, brief 4, inject 7 pass; clippy, fmt, check clean |
| 2 - Rename the lock test that claims a process death (#10) | f47cc3f | renamed to `a_held_lock_refuses_a_second_attempt_and_frees_when_dropped`, body unchanged; comment keeps the OS-lock reason (`DESIGN-BRIEF.md:135`) and names `killing_ingest_anywhere_leaves_a_store_that_converges` in crash.rs; pass.rs test name unchanged; lock_race 2 pass; clippy, fmt clean |
| 3 - Mark the nullable sessions and show fields in json-shapes.md (#25) | 92e973a | all 15 D-14 fields in a bullet that says when null, read from the emitting code; `body` bullet extended, `context` bullet already stated its null case; no other field called nullable, JSON examples unchanged; cli 21 pass |
| 4 - Write the Fixed entries in CHANGELOG.md and run the phase gate | 94e34d7 | `### Fixed` under `[Unreleased]`, seven entries matching D-15, specifics taken from b45feb4, 5bef6e2, 0f9d5c7, d1246eb, 74fd89b, 1ad97e9, dcf66b0; diff is additions only in one hunk; the `1e3` -> `1000.0` echo claim checked against the pinned serde_json; clippy, fmt clean; full suite `cargo test --workspace --all-features` green, 66 binaries, 877 passed, 0 failed |
Deviations: none
Open items: `ts` (show and context turns) is nullable by column type but never null in practice, since `parse::record` makes a turn only of a record carrying `timestamp` and `derive_turn` is the one writer of `turns`; the doc says so rather than inventing a null case.
