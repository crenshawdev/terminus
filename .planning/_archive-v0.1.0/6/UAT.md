---
status: testing
phase: 6
fields_version: 1
started: 2026-08-21
updated: 2026-08-21
---

## Items

### 1. Cold start from scratch
expected: With ephemeral state cleared, a fresh `verbatim ingest` on a store built from nothing boots clean, creates decisions and labels, and one primary query (`verbatim status --json`) returns real data with no schema error.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Fresh data dir: ingest exit 0, decisions and labels created, `status --json` returns sessions 2 / turns 14 / files_committed 2 with ok:true and no schema error

### 2. Three prompt shapes each leave a decision row
expected: Feeding three UserPromptSubmit payloads through the real hook - one that injects, one suppressed on threshold, one whose candidates are empty before the store opens - yields three decision rows after the next ingest pass, each carrying entities, candidates with matched (kind, value_norm) pairs, suppressions with reasons, threshold values and char counts.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: Two live `hook UserPromptSubmit` runs produced two full decision rows (thresholds, spellings, watermark, chars); crates/verbatim/tests/decisions.rs:248 passes over all three shapes and asserts candidates[].matched_on, suppressions and thresholds

### 3. Hook p99 budget still passes
expected: The phase-4 hook latency test still passes with the decision-record write on the path: `cargo test -p verbatim --test hook` is green, including the 10 ms p99 assertion.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: `cargo test -p verbatim --test hook` 7 passed; the 10 ms assertion is live at crates/verbatim/tests/hook.rs:345,376

### 4. Idle rule labels hit and false positive
expected: On a fixture session past the six-hour idle threshold with one injected turn referenced downstream and one never referenced, an ingest pass sets is_final and labels them `hit` and `false positive`; a session younger than the threshold gets is_final unset and zero labels.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim-core/tests/label.rs:310 and :347 pass - is_final 1 vs None, hit 1 / false positive 2 / wasted budget 1, live session zero labels

### 5. A declined entity later searched labels miss
expected: A fixture transcript containing a `recall_search` tool call for an entity a logged decision declined to inject gets that decision labeled `miss`.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim-core/tests/label.rs:528 passes with miss count 1, detail naming the withheld path, and bytes_read 0

### 6. verbatim replay diffs without writing
expected: `verbatim replay` with a threshold override flag exits 0, reports a per-label diff across the logged history, and leaves verbatim.db byte-identical afterwards.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: Live `replay --entity-rank 5` and `--json` exit 0 with the four-label old->new diff; verbatim.db md5 unchanged. crates/verbatim/tests/replay.rs 4 tests pass

### 7. verbatim stats reports precision, misses and chars
expected: `verbatim stats` and `verbatim stats --json` report precision, miss count and chars injected versus referenced, matching the documented json-shapes envelope; on a fixture with a known outcome mix the numbers equal the hand-computed values.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: Live human and --json output match docs/json-shapes.md:180-203 field for field, precision null on an unlabelled archive; cli.rs:625 and feedback.rs:1105/:1132 pass

### 8. Reindex preserves decisions; old store gains tables
expected: `verbatim reindex` on a store holding logged decisions returns the same decisions rows after the rebuild; a store initialized by the phase-5 binary and opened by this one gains decisions/labels with no reindex triggered.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: decisions rows byte-identical across reindex; a store with both tables dropped regains them on ingest with turns preserved and zero files committed

### 9. search --json exposes matched_on
expected: `verbatim search <query> --json` emits a `matched_on` field on each hit carrying the matched (kind, value_norm) pairs, as documented in docs/json-shapes.md.
status: pass
first_pass: pass
source: verifier
evidence: Live search returns matched_on [{'kind':'path','value':'docs/RETRY.md'}], documented at docs/json-shapes.md:78,90

### 10. stats and replay on a store older than this build report a raw sqlite line instead of an answer
expected: behavior wrong - on a store that has not yet been opened in write mode by this build, both read-only commands fail on the missing table, and with --json they print a bare stderr line and write no envelope at all
origin: verifier
status: pass
first_pass: fail
source: model
evidence: Live binary probe on a real store with the tables dropped, after 00c6b53. Both absent: `verbatim stats --json` -> exit 0 with the full envelope `{command:stats, ok:true, reason:"this store predates the decision log...", data:{...}}`, and `replay --json` the same; no `no such table` anywhere on stderr. One absent (labels dropped, decisions kept): `stats --json` -> exit 1 with `ok:false` and the reason naming the missing table, human mode the same reason on one stderr line and exit 1. Regression tests: `crates/verbatim/tests/replay.rs` gains `stats_and_replay_on_a_store_without_the_decision_log_still_answer` and `stats_and_replay_call_half_a_decision_log_a_failure`, both green; `cargo test --workspace` 485 passed 0 failed, `cargo clippy --workspace --all-targets` clean.
reported: behavior wrong - on a store that has not yet been opened in write mode by this build, both read-only commands fail on the missing table, and with --json they print a bare stderr line and write no envelope at all
severity: minor
cause: Two seams miss the case together. (1) `read::open_in` (crates/verbatim/src/cmd/read.rs:97-105) gates its D-18 'older than this build' line on `Store::predates_this_build()`, which is `rebuild.is_some() || !missing.is_empty()` (store/open.rs:290-292); `missing` is computed by `missing_columns()` over `schema::BRING_FORWARD_COLUMNS` alone (open.rs:377-391), never over `schema::TABLES`. `decisions`/`labels` are whole new TABLES and D-02 deliberately took no `DERIVED_SCHEMA` bump, so a phase-5 store makes both terms false and the read path sees a perfectly current store. (2) The failure then surfaces from the query itself: `stats::stats(...).map_err(|e| Failure::Operational(e.to_string()))` (cmd/stats.rs:32-33, cmd/replay.rs:42) hands a raw rusqlite string to `main.rs:51`, which prints `verbatim: {msg}` on stderr and exits 1 with no knowledge of `--json` - so the envelope is never emitted. The missing-table condition is knowable at open time and is not checked there, which is why it escapes as an unenveloped operational error rather than the `Opened::Nothing` reason-and-exit-0 answer read.rs already defines for 'this machine has nothing logged yet'.
fix: 00c6b53, retest

## Summary

total: 10
passed: 10
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 1
