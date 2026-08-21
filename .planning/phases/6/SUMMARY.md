---
phase: 6
status: complete
completed: 2026-08-20
---

# Phase 6: Feedback Loop - Summary

Every prompt now leaves a decision record that ingest drains into SQLite, labels against what the session went on to reference, and that `verbatim replay` re-scores under different thresholds and `verbatim stats` reduces to precision, misses and chars injected versus referenced.

## What shipped

- The decision record - `crates/verbatim-core/src/inject/decision.rs`: one JSON file per `UserPromptSubmit` under a `decisions/` subdirectory, tmp+rename on the thread the hook never joins, carrying spellings, scored candidates with their matched `(kind, value_norm)` pairs, injected turns with per-turn chars, suppressions with reasons, and the thresholds in force
- `decisions` and `labels` tables - `crates/verbatim-core/src/store/schema.rs`: neither derived nor brought forward, so a reindex leaves a decision row byte-identical
- The drain - `crates/verbatim-core/src/feedback/mod.rs`: one transaction per batch, files unlinked only after it commits, unreadable files deleted and named in `runs.error`
- Outcome labels - `crates/verbatim-core/src/feedback/label.rs`: `hit`, `false positive`, `wasted budget` and `miss` computed entirely in SQL over `json_each` of the decision's injected list, gated on a session the six-hour idle rule marked final
- Recall-call entity extraction - `crates/verbatim-core/src/index/entity.rs`: a `recall_search` tool call leaves what it searched for, which is what a `miss` joins against
- Historical bound on search - `Request::before_turn_id`, so replay scores a decision against the archive as it stood
- `verbatim replay` - `crates/verbatim/src/cmd/replay.rs`: one flag per threshold, opened read-only, reports the label diff without writing a byte to the store
- `verbatim stats` - `crates/verbatim/src/cmd/stats.rs`: decisions (non-fires included), hits, false positives, precision (null rather than 0.0 when nothing is judged), misses, wasted budget, chars injected versus referenced

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 0adb1de | `Hit::matched_on` carries the matched (kind, value_norm) pairs; `search --json` gains the field |
| 1 | 2 | 89b4b61 | `decisions` and `labels` join `CREATE_SQL`, in neither derived nor bring-forward sets |
| 1 | 3 | 704c584 | The decision record: shape, tmp+rename writer, directory reader |
| 1 | 4 | a2e8de3 | Every `UserPromptSubmit` opens, fills and saves one record |
| 1 | 5 | 7afaaba | Ingest drains decision files into the table, counts and notes into `runs.error` |
| 1 | 6 | 73a2ab9 | Three prompt shapes through the real hook leave three rows |
| 2 | 1 | eb7951f | `recall_search` tool calls extract the query and its path-shaped words |
| 2 | 2 | 9e28328 | The six-hour idle rule writes `is_final` |
| 2 | 3 | 6d196dd | Hit, false positive, miss and wasted budget as three SQL inserts in one transaction |
| 2 | 4 | ee847c9 | Fixture proof end to end, on a pass that read zero blob bytes |
| 3 | 1 | 63ab2c4 | The six injection thresholds become a `Thresholds` value replay can vary |
| 3 | 2 | 509f8b5 | `Request::before_turn_id` bounds a search to the archive as it stood |
| 3 | 3 | fe49d24 | The replay engine, sharing one SQL definition with the ingest labeller |
| 3 | 4 | 0c8fdb7 | `verbatim replay`, read-only, one flag per threshold |
| 3 | 5 | f94922b | `verbatim stats` - precision, misses, chars injected versus referenced |
| 3 | 6 | 215ac51 | `stats` and `replay` join the swept CLI contract |

## Deviations

- [deviation] Plan 1 task 4 named `crates/verbatim-core/tests/inject_prompt.rs`, which sits on a task `Files:` continuation line and so is in no lease the checker reads. The four assertion sets the `Verify:` asks for landed in `crates/verbatim-core/tests/feedback.rs` against the same fixture and entry point; `inject_prompt.rs` is untouched and still passes (a2e8de3).
- [deviation] D-11's "every UserPromptSubmit writes a record" is false where no data directory exists: `Decision::save` will not create one, because phase 5's INJ-06 test asserts a hook leaves nothing behind on a machine that has never ingested. Records are written whenever the data directory exists; the unlogged prompts are the ones before a machine's first pass, whose decisions have no archive to be judged against. CONTEXT D-11 corrected (a2e8de3).
- [deviation] Plan 3 task 2 asked for the historical bound on `Filters`, but `cmd/mcp/tools.rs:332` builds `Filters` with an exhaustive struct literal and is in no lease, so any new field breaks the build there. The bound is a field of `Request`, constructed only through `Request::new`, which keeps the "every existing caller untouched" half of the Action exactly (509f8b5).
- [deviation] Plan 3 task 3 asked replay to apply the recorded compacted pool. The `decisions` table carries neither `compacted` nor `dropped` - plan 1's drain inserts eleven columns and those are not among them - and `store/schema.rs` was outside plan 3's lease. Replay scores every decision through the ordinary ranked window, stated in the module header; nothing in the task's `Verify:` or AC4 asks for the compacted pool (fe49d24).
- [deviation] Plan 3 task 4 asked the test to assert no `-wal`/`-shm` growth. A first read-only open of a WAL database necessarily materializes a 32 KiB `-shm` and an empty `-wal`, measured against a real store, and no read command can avoid it. Asserted instead: `verbatim.db` byte-identical, the WAL zero bytes, and a second replay growing neither sidecar (0c8fdb7).

## Open items

- ~~`verbatim stats` and `verbatim replay` against a store ingested before this phase exit 1 with a raw `sqlite: no such table: decisions`.~~ Closed at UAT by `00c6b53`: `Store::missing_tables` measures the absence at read-only open, both tables gone is the empty answer with a reason and exit 0, and exactly one gone is a damaged store reported with the envelope written and exit 1. Read commands still never migrate.
- `read::decision_log` reads the table list captured at open and does not re-check it before the query, so a writer dropping a table in between still reaches the raw sqlite path. Not closable from a read-only connection - the check and the query are two statements whatever their order - and the same window `Store::missing_columns` has always had (cross-model reviewer, medium, adjudicated as an open item rather than a blocker).
- Replay's `wasted budget` is "the would-inject set is non-empty and none of it is a hit", where ingest reads `chars_injected > 0`. The two agree in practice, but a replay has no rendered text to count and so cannot use the column's own definition.
- The `decisions` table carries neither `compacted` nor `dropped`, so replay and stats cannot tell a decision taken under a compacted pool from an ordinary one.
- A `.tmp` left by a hook that exited between write and rename is skipped by `read_all` and deleted by nothing, so the decisions directory accumulates orphans (cross-model reviewer, medium, adjudicated as an open item rather than a blocker).
- `runs.error` carries a routine `labelled 1 hit, 1 false positive` line whenever a pass labels anything, and `verbatim status` prints that channel under an `error` heading, so a correct pass can read as a failed one.
- The `miss` join matches a recall tool by `tool_name LIKE '%recall\_search'` while extraction requires a `_` before the suffix, so a tool named `xrecall_search` would be admitted by one rule and extracted from by neither.
- `MatchedEntity` is not re-exported from `crate::recall` (that module was outside plan 1's lease), so callers name `recall::search::MatchedEntity`.
- `PassOutcome` carries `#[allow(clippy::large_enum_variant)]`: `Summary` gained the drain result and crossed the 200-byte threshold; boxing would ripple `Box<Summary>` through `cmd/ingest.rs` and four test files outside the lease.
- Pre-existing `cargo fmt` drift in `crates/verbatim-core/src/ingest/backfill.rs:250` and `crates/verbatim/tests/hook.rs:252,681`, outside every plan's lease and untouched.

## Goal check

The sixteen commits plausibly deliver the phase goal. Measurement exists where opinion was: `verbatim stats` and `verbatim replay` are both shipped subcommands wired into `main.rs` and swept by the CLI contract test (215ac51, 17 `cli` tests green), the whole workspace is 483 tests passing with `cargo check --workspace --all-targets` clean, and the phase's four roadmap criteria each have a named test - AC1 by `crates/verbatim/tests/decisions.rs` feeding three payloads through the real `hook UserPromptSubmit` and asserting three rows including the two that injected nothing (73a2ab9), AC2/AC3 by the two ingest-driven fixture tests in plan 2 task 4 that label on a pass reporting `files_committed = 0` and `bytes_read = 0` (ee847c9), AC4 by `tests/replay.rs` running `--entity-rank 5 --json` and asserting the store file is byte-identical afterwards (0c8fdb7), and AC5 by the stats aggregation test writing every expected number as a literal (f94922b). Two gaps are worth naming. First, the tooling is unusable on this repo's own archive today: `verbatim stats` and `verbatim replay` both exit 1 with `sqlite: no such table: decisions`, because read commands deliberately never migrate (`cmd/read.rs`) and the live store predates this phase - it self-heals on the next write-mode ingest, but the error a user meets is a raw sqlite line rather than the "older than this build" answer D-18 promises. Second, the decision record carries `compacted` and `dropped` but the drain's eleven-column insert does not persist them (`feedback/mod.rs:107`), so the compaction case phase 5 built - AC5 of that phase - is invisible to every measurement this phase added. Neither blocks the goal; both are queued.
