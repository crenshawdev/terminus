PLAN COMPLETE
Plan: .planning/phases/12/PLAN-2.md
Tasks: 3 of 3
| Task | Commit | Note |
|---|---|---|
| 1 - Charge the separator to the body budget (#20) | 20dd4ac | `push` charges the separator, writes it only when a character fits after it, and sets `full` on a cut; `leaves` stops on `full`. Four tests in `index::text::tests` pass, and all four failed with the old `push` body put back (restored, not committed). index, recall, recall_golden green; clippy, fmt, check clean. |
| 2 - Require type system for a compaction boundary (#17, type check only) | 92a088c | `TurnRow::is_compact_boundary` reads `row.turn.record_type`; `Record::is_compact_boundary` uses the same rule and its doc comment is corrected. New `a_boundary_subtype_on_a_record_that_is_not_system_gets_no_row` (compaction.rs) and `only_a_system_record_is_a_compact_boundary` (parse.rs). compaction 14, inject_compaction 4, derive 10, parse 23 all pass. The new compaction test failed with the type check removed (restored). `DERIVED_SCHEMA: i64 = 4` still at open.rs:50. Clippy, fmt, check clean. |
| 3 - Match recall_search in the miss query by the extractor's rule (#39) | 99491f4 | `sought_sql` drops the `recall_get` arm and matches `tool_name = 'recall_search' OR tool_name GLOB '*_recall_search'`, with a SQL comment naming `index::entity::is_recall_search`. New `the_miss_query_matches_recall_search_by_the_extractors_rule` (label.rs) covers the five names through `label::sought`. label 5, feedback 12, terminus replay 6 pass. With the old two `LIKE` arms restored the new test failed on `recall_get` (found `docs/RETRY.md`, expected None); fix restored. Clippy, fmt, check clean. |
Suite: `cargo test --workspace --all-features` (workflow.test_command is null; AC6 and the Cargo workspace name it), exit 0, 66 test binaries, 873 passed, 0 failed.
Deviations: none
Open items: none
