PLAN CHECKPOINT: structural
Plan: .planning/phases/7/PLAN-1.md
Tasks: 3 of 5
| Task | Commit | Note |
|---|---|---|
| 1 - Declare the `observations` table | 571393b | Added to `CREATE_SQL` and `schema::TABLES`, absent from `DERIVED_TABLES`, no `DERIVED_SCHEMA` bump, no reference to `turns`. Three new tests in `tests/schema.rs`, one in `tests/reindex.rs`; 15 + 8 pass. |
| 2 - Compute the mechanical facts for one session | 837c0ab | `observe::mechanical::observe` - SQL over `paths`/`entities`/`session_meta`/`turns`/`compaction_boundaries` plus one blob read for commands-with-arguments and commit subjects. Lists capped at 64, values at 512 bytes, cuts named in the document. `--test observe` 4 pass, 7 new unit tests pass. |
| 3 - Ingest writes an observation per newly finalized session | d936601 | `observe::observe_new` behind `config::visible::sessions`, insert-only (`ON CONFLICT DO NOTHING`), bounded at 100 sessions per pass, notes folded into `runs.error`. Called in `pass::run_with` after `feedback::outcomes` and before `record_pass`. `--test observe` 7 pass. Verify met. |

Deviations: none. Nothing the plan asserted turned out false; the checkpoint
below is a lease question, not a wrong criterion.

Open items:
- `crates/verbatim-core/tests/backfill.rs::the_pipeline_lands_on_the_same_store_as_the_sequential_pass`
  now fails, and it is the only failing test in the workspace (full suite run
  2026-08-21, every other target green). It compares every table's row count
  between `backfill::run_with` and `pass::run_with` over one tree, and the
  count list is built from `schema::TABLES`, so a new table joins the
  comparison automatically. `observations` is 0 after a backfill and 36 after a
  sequential pass. This is a PRE-EXISTING deliberate asymmetry that a row count
  can now see: `backfill::run_with` runs neither `feedback::drain` nor
  `feedback::outcomes`, so nothing sets `session_meta.is_final` there and
  nothing is observed. It was invisible before because `decisions` and `labels`
  are both 0 in that fixture tree and because `testkit::archive_digest`
  deliberately omits `is_final`.
