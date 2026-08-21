PLAN COMPLETE
Plan: .planning/phases/6/PLAN-2.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Extract entities from recall tool calls | eb7951f | `Collector::recall_search` plus a public `is_recall_search` (suffix with a `_` separator, so `mcp__verbatim__recall_search` and the bare name both match, `xrecall_search` does not). Query treated as a `Grep` pattern plus its path-shaped words; `paths` entries normalized as paths. No `DERIVED_SCHEMA` bump. 16 index tests pass. |
| 2 - The idle rule writes `is_final` | 9e28328 | `feedback::finalize` - one UPDATE, `IDLE_HOURS = 6`, lexicographic against `strftime('now', '-6 hours')`, null `last_turn_at` left alone. Reached through a new `feedback::outcomes` into a new `Summary::outcomes` field, wired after the walk and before the `runs` row. `write_session_meta` doc corrected. New `tests/label.rs`. |
| 3 - The label join | 6d196dd | `feedback::label` - three INSERTs in one transaction, `json_each` over the decision's injected list, per-statement "no such row yet" gates, counts read back off the label id range. Hand-built store: `hit` 1 / `false positive` 2 / `wasted budget` 1 / `miss` 0; a non-final session's decision gets zero rows; a second pass writes none. |
| 4 - Fixture proof of AC2 and AC3 | ee847c9 | Two ingest-driven tests through `inject::decision::save` -> drain -> finalize -> label. Both label on a pass reporting `files_committed = 0` and `bytes_read = 0`, which is the assertion that no blob was read. `cargo fmt` applied to the two files this plan created. |
Deviations: none
Open items:
- `runs.error` now carries a routine `labelled 1 hit, 1 false positive` line whenever a pass labels anything, per task 3's Action ("report the label counts through `Summary` into the pass notes only if a count is nonzero"). `verbatim status` prints that channel under an `error` heading, so a pass that worked correctly can read as one that did not. PLAN-3's `stats` reads the `labels` table directly and does not need the note; if the noise matters, the note is one line in `feedback::Labeled::lines`.
- The `miss` join matches a recall tool by `tool_name LIKE '%recall\_search'`, a plain suffix, while extraction uses `index::entity::is_recall_search`, which additionally requires a `_` before the suffix. A tool genuinely named `xrecall_search` would therefore be admitted by the join and extracted from by nothing, which reads zero either way; noted rather than fixed because the two rules live in different layers and the plan specified the suffix.
- Pre-existing `cargo fmt` drift in `crates/verbatim-core/src/ingest/backfill.rs:250` and `crates/verbatim/tests/hook.rs:252,681`, outside this plan's lease and left untouched.
