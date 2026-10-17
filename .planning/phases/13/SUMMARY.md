---
phase: 13
status: complete
completed: 2026-10-15
---

# Phase 13: CI and Data Integrity - Summary

A new workflow, `.github/workflows/test.yml`, runs `cargo test --workspace --all-features --locked` on the four release runners for every push and pull request, and the archive and decision fixes landed under it: #44, #40, #38, the drain race, #15 and #17's second case, in 11 commits on `mallow`.

## What shipped

- The suite runs on `ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-15-intel` and `macos-15` for every push and pull request. A step before `cargo test` fails the job and names any tool the shell tests need that is missing from PATH (#47) - `.github/workflows/test.yml`
- README.md and SECURITY.md stop saying no workflow runs the tests (#47) - `README.md`, `SECURITY.md`
- The 27 test files gated as a whole on `testkit` are declared with `required-features`, so naming one without the feature is refused with a message that names `testkit` (#44) - `crates/terminus-core/Cargo.toml`, `crates/terminus/Cargo.toml`
- The `decisions` table stores `compacted` and `dropped`, and an existing store gains both columns when it is opened (#40) - `crates/terminus-core/src/store/schema.rs`, `crates/terminus-core/src/feedback/mod.rs`
- `replay` re-scores a compacted decision through the compacted window and its dropped turns, and still answers against a store without the new columns (#40) - `crates/terminus-core/src/feedback/replay.rs`, `crates/terminus-core/src/inject/prompt.rs`
- `terminus stats` counts compacted decisions apart from ordinary ones, as two new `--json` keys after `decisions` and two indented lines under it (#40) - `crates/terminus-core/src/feedback/stats.rs`, `crates/terminus/src/cmd/stats.rs`, `docs/json-shapes.md`, `docs/commands.md`
- Every drain deletes the writers' own `.tmp` decision and state files once they are ten minutes old (#38) - `crates/terminus-core/src/feedback/mod.rs`
- A zero-length decision `.json` younger than that age is left for the next pass, so a record renamed over its reservation while a drain runs is inserted rather than deleted unread (the drain race) - `crates/terminus-core/src/feedback/mod.rs`
- The comments that said the abandoned hook thread writes nothing and that state files hold prompt text are corrected (#38) - `crates/terminus/src/cmd/hook.rs`, `crates/terminus-core/src/inject/state.rs`, `crates/terminus-core/src/inject/decision.rs`
- A diverged session's prefix check and tail read go through one open file handle (#15) - `crates/terminus-core/src/ingest/mod.rs`
- Boundary rows move to a new derived `compactions` table keyed by stream position, `DERIVED_SCHEMA` goes to 5, and a boundary with no `uuid` or `timestamp` gets a row at its own offset (#17) - `crates/terminus-core/src/store/schema.rs`, `crates/terminus-core/src/derive.rs`, `crates/terminus-core/src/reindex.rs`, `crates/terminus-core/src/ingest/mod.rs`
- `[Unreleased]` entries for each fix - `CHANGELOG.md`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | a4f9e8a | make cargo refuse a named testkit test run without the feature (#44) |
| 1 | 2 | 6e8e0f5 | run the whole suite on every push and pull request on the four release runners (#47) |
| 2 | 1 | e7c0486 | keep a decision's compacted flag and dropped turns through the drain (#40) |
| 2 | 2 | 506a354 | replay a compacted decision through the compacted window and its dropped turns (#40) |
| 2 | 3 | 66396b2 | count compacted decisions apart from ordinary ones in terminus stats (#40) |
| 3 | 1 | 203c29e | sweep the writers' abandoned .tmp files on every drain once they are ten minutes old (#38) |
| 3 | 2 | 7e3415d | leave a decision reservation still in flight for the next pass (drain race) |
| 3 | 3 | 718341d | correct the comments that said the abandoned hook thread writes nothing and the drain leaves .tmp files (#38) |
| 4 | 1 | 7de4982 | read a diverged session's prefix and tail through one open file (#15) |
| 4 | 2 | 1bbac27 | key boundary rows by stream position in a new compactions table (#17) |
| 4 | 3 | 9606ed1 | give a compaction boundary with no uuid or timestamp a row at its own offset (#17) |

## Deviations

None - plans executed as written.

## Open items

- The workflow has not run on GitHub yet. Push the branch and confirm all four jobs run for the push and the pull request, that the tool check runs before `cargo test`, and that every job passes. A timing budget that fails on a hosted runner goes to the owner with the test name and its measured number (D-04).
- No Linux install step was added. The Ubuntu images are expected to carry every listed tool, and the check step names any that is missing. Add an apt step only if the first run shows a gap, most likely on `ubuntu-24.04-arm`.
- On macOS a missing `sha256sum` is supplied as one link to Homebrew's `gsha256sum` in a folder of its own, not by putting coreutils' gnubin first on PATH, so install.sh's tests still run against the BSD tools a Mac ships.
- README.md and SECURITY.md were opened in RustRover for the owner to review.
- The new `stats` lines read `  compacted` and `  ordinary`, indented under `decisions`, because `compacted decisions` is wider than the 18-character label column and widening it would change every line the command prints. The owner may prefer other wording.
- `crates/terminus-core/tests/inject_state.rs:267` still says the injection scratch is quoted prompt text. State holds turn ids, suppressions and the compaction flag. The file was outside plan 3's lease.
- `cargo fmt --all -- --check` reports diffs that were there before this phase in `crates/terminus/src/cmd/hook.rs:210` and `crates/terminus/tests/release_workflows.rs`.
- The drain-race test proves the drain-side rule, a young empty file left and the record inserted after the rename. It does not time a real rename between the read and the stat, the same call D-10 made for #15.
- The doc on `Record::is_compact_boundary` in `crates/terminus-core/src/parse/record.rs` still says a boundary record is already a turn because it carries `uuid` and `timestamp`. That is true of the one real record and no longer of every boundary. D-19 keeps `parse` unchanged, so it is left for a docs pass.
- No commit carries a closing keyword, so #47, #44, #15, #40, #38 and #17 are still open. They need closing from the pull request or by hand once the suite passes in CI.

## Goal check

The commits plausibly deliver the phase goal, with one part still unproven. Each success criterion has a commit behind it. `6e8e0f5` adds the workflow on all four runners. `a4f9e8a` makes all 27 named testkit runs exit non-zero with "requires the features: `testkit`" (plan 1 report). `7de4982` cuts a diverged session's opens from 2 to 1 under the testkit open log. `e7c0486`, `506a354` and `66396b2` store, replay and count the two decision columns, with tests against a store that lacks them. `203c29e` and `718341d` sweep the `.tmp` files and correct the comments. `7e3415d` leaves a young reservation for the next pass. `1bbac27` and `9606ed1` give the field-less boundary its row. The suite passed locally after every plan: `cargo test --workspace --all-features --locked`, 67 test binaries, 896 passed and 0 failed after plan 4. What is not shown is the suite passing in CI. The workflow has never run on GitHub, and none of the six issues is closed, so criterion 1's "a failing test fails the run" and the last criterion wait on the first push. Two cross-model risk reviews ran, on plans 2 and 4. Neither finding stood, and both are declined in `.planning/DECLINED.md`. Plan 4's finding, a transcript rewritten in place between the two reads, is the gap D-09 accepted, and `crates/terminus-core/src/ingest/mod.rs:1001-1002` documents it.
