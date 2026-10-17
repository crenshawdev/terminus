PLAN COMPLETE
Plan: .planning/phases/13/PLAN-4.md
Tasks: 3 of 3
| Task | Commit | Note |
|---|---|---|
| 1 - Read a diverged session's prefix and tail through one file handle | 7de4982 | New tests saw 2 opens before the change and 1 after; compaction 16/16, capture 23/23 |
| 2 - Key boundary rows by stream position, in a new derived table | 1bbac27 | `compactions` keyed by (session_key, stream_offset); DERIVED_SCHEMA 5; two version-4 upgrade tests pass; targeted runs green (core: compaction, derive, inject_compaction, reindex, retention, snapshot, schema, store_open; terminus: crash, suppression, replay); the `--workspace --all-features` criterion is met by the end-of-plan suite run (`cargo test --workspace --all-features --locked`: 67 binaries, 896 passed, 0 failed) |
| 3 - Give a boundary with no uuid or timestamp a row at its own offset | 9606ed1 | `derive::derive_boundary`, called from ingest's apply and from reindex; compaction 19/19, suppression 7/7; mutation checks: disabling the reindex call fails both field tests, disabling the ingest call fails the new suppression test. Commit first made as 2d04312 with scope 17-4 by mistake, amended to 13-4 before anything else ran |
Deviations: none
Open items: `Record::is_compact_boundary`'s doc in crates/terminus-core/src/parse/record.rs still says the boundary record is already a turn because it carries `uuid` and `timestamp`; true of the one real record, but it no longer describes every boundary. The file is outside this plan's lease and D-19 keeps `parse` unchanged, so it is left for a docs pass.
