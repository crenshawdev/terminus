PLAN COMPLETE
Plan: .planning/phases/7/PLAN-2.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Add the block codec | e765509 | Committed by an earlier run (see plan-2.2.md) |
| 2 - Refuse an older archive format | 7ff134e | Committed by an earlier run (see plan-2.2.md) |
| 3 - Store sessions as block rows | 2a6fd01 | Committed by an earlier run (see plan-2.3.md) |
| 4 - Remove the whole-session blob format | 8f50e45 | The earlier run's working-tree implementation (see plan-2.3.md) committed as is, plus the approved edit: crates/terminus-core/Cargo.toml line 3 now reads `description = "Terminus archive core: store, block rows, ingest."`, nothing else in that file changed. The Verify grep prints nothing (exit 1). `--test blob` 18/18. clippy --all-targets -D warnings and cargo check --all-targets clean. Lease check ok. The commit deletes only blob/reader.rs and blob/writer.rs, as planned. Full suite `cargo test --workspace --all-features` (the command the plan names; workflow.test_command is null): exit 0, 64 test binaries, 824 passed, 0 failed, 0 ignored, including crash, compaction, verify, reindex, retention and schema. |

Deviations:
- [deviation] (earlier run) The plan asserted Task 3's files covered the switch to block rows. crates/terminus-core/tests/ingest.rs had two assertions over the format-1 storage. Checkpointed; the owner approved adding it to the lease with exactly the two edits recorded in plan-2.3.md.
- [deviation] (earlier run) Task 4's Verify asserted its grep could print nothing from the files in the lease. crates/terminus-core/Cargo.toml:3 said "block-framed blobs" and was outside the lease. Checkpointed; the owner approved adding Cargo.toml to the lease and changing only line 3, which this run did.
Open items:
- `verify`'s failure text still says "blob does not decompress". Plan 3 replaces verify's messages with per-block ones.
- tests/verify.rs:346 still cites the deleted `blob::read_all` in a doc comment ("finds no header in zero bytes"). That file is outside this lease and in plan 3's, which restates those tests.
- tests/retention.rs:585 says `blob::append` "has no header to parse" out of an evicted session. `blob::append` still exists (it is the codec now), but the header reason is stale. Outside the lease.
- The `Display` text of `Error::BlobFormat` ("malformed session blob") and `Error::BlobChecksumMismatch` ("session blob failed its recorded checksum") still says "session blob". Task 4 asks only for the docs, and tests may match the text, so the messages were left alone.
- lib.rs's first sentence still says "The session blob is truth". Task 4 changes only the "block-framed" sentence; left for plan 4's docs pass.
- `cargo fmt --all --check` fails at HEAD on six test files outside the lease (tests/inject_state.rs, tests/recall_golden.rs, crates/terminus/tests/hook.rs, mcp.rs, observations.rs, retention.rs). Left untouched.
- reindex.rs comments still describe "the blobs" as the input. reindex.rs is not named in Task 3 or 4; left for plan 4's docs pass.
- The new `session_meta.block_size` column is nullable with no default, so the direct `session_meta` inserts in tests/retention.rs:79 and tests/pass.rs:777 still work. Readers and append refuse a null block size per session ("no block size recorded").
