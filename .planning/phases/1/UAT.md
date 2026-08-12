---
status: testing
phase: 1
fields_version: 1
started: 2026-08-12
updated: 2026-08-12
---

## Items

### 1. Cold start from scratch
expected: With no existing data directory, `verbatim ingest <fixture>` creates the store, schema initializes, and one primary query (e.g. `verbatim reindex` then a turn read / FTS query) returns real data.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Live cold run of the built binary into an empty temp dir: `ingest` exit 0 creating LOCK + verbatim.db, `reindex` exit 0 (`rebuilt 8 turn(s) across 1 session(s)`), `verify` exit 0 (`1 session(s) checked, 0 failed`), and SQL on the result gives sessions=1, turns=8, FTS MATCH 'brillig' -> rowid 7, turn rows with real stream offsets/lengths.

### 2. Turn read decompresses only the blocks it occupies
expected: Ingesting a fixture transcript and reading a turn that fits inside one block decompresses exactly 1 block; a turn spanning N blocks decompresses exactly N, counted by an instrumented reader.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --workspace --test ingest --test blob`: `reading_a_turn_decompresses_only_the_blocks_it_occupies` (4 blocks for the oversized record, 1 for the small one) and `the_block_count_matches_the_windows_touched_for_random_ranges` over 200 random ranges, so the counter in blob/reader.rs is falsified rather than trusted. Also `every_turn_row_addresses_its_own_record` across all five fixtures.

### 3. Crash at 20+ randomized points converges
expected: A harness that kills ingest at 20 or more randomized points and reopens the store finds, for every session, turn/FTS/entity rows only where a blob with a matching BLAKE3 checksum exists, and no watermark past the last committed block boundary.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --workspace --test crash` -> 2 passed: 23 SIGKILLs (15 aimed at named in-transaction fault points across 3 seeded rounds, 8 timed), each followed by full invariant checks (blob checksums via verify, orphan checks on turns/turns_fts/entities/paths, watermark <= committed bytes) and a convergence assertion against an uninterrupted reference snapshot; the negative test proves the check can fail. Fault points are testkit-gated and inert in the shipped build. Narrowing carried to phase 2: kills only ever hit a first pass into a fresh data dir, never an append/resume pass.

### 4. verify names the corrupt session and no other
expected: `verbatim verify` on a store with one byte flipped inside one session's blob exits non-zero, prints that session id, and prints no other session id.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --workspace --test cli --test verify`: `verify_names_the_corrupt_session_and_no_other` flips one byte inside the sidecar's blob of D-01's colliding pair and asserts exit 1, stdout naming that session, not naming the parent, and never mentioning integrity_check; `verify_exits_zero_and_says_nothing_on_a_clean_store` is the control; the library suite adds two-corrupt-session and sidecar/parent discrimination cases.

### 5. reindex rebuilds to byte-identical output
expected: Dropping `turns`, `turns_fts` and the entity tables and running `verbatim reindex` produces byte-identical `--json` output for a fixed query set, compared before and after.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --workspace --test cli --test reindex`: all four derived tables dropped, `verbatim reindex` run as a process (exit 0, empty stdout), and both the fixed query set's serialized output (FTS hit rowids plus a full turn scan with coordinates) and a digest over sessions/session_meta compare equal to before. AC4's literal `--json` flag does not exist in phase 1 by scope (RCL-06 is phase 3, and `verify --json` is an asserted exit-2 misuse), so the query set is serialized by the harness against the store the binary rebuilt.

### 6. A newer archive format is refused without touching the store
expected: Opening a store whose `meta.archive_format` is one higher than the binary knows exits non-zero with a message naming both versions, and leaves the store file, its WAL and the `runs` table unchanged.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --workspace --test cli --test store_open`: both `verify` and `ingest` against a store one format ahead carrying a hot non-empty WAL exit non-zero with stderr naming both versions, and verbatim.db plus the WAL compare byte-for-byte equal afterwards, with `runs` still at 1 row and both meta integers unmoved. The gate reads through a SQLITE_OPEN_READ_ONLY connection before any writable one (store/open.rs:209,241,270).

### 7. Contended ingest exits fast and writes nothing
expected: A second `verbatim ingest` launched while the first holds `LOCK` exits 0 in under 50 ms over 10 consecutive runs and adds no row to any table.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --workspace --test lock_race` -> `a_contended_ingest_exits_zero_fast_and_writes_nothing` passed: 10 consecutive spawns of the real binary against a held OS try-lock, each exit 0, each under the 50 ms budget, each leaving every table's row count unchanged, followed by a control run with the lock free that does change the counts.

### 8. Every transcript line reads back byte-identical
expected: Every line of a fixture transcript, including the eleven state record types that get no turn row, reads back from the blob byte-identical to the source file.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --workspace --test ingest --test fixtures --test derive`: `the_blob_reproduces_the_transcript_byte_for_byte` compares the decompressed stream against the source file bytes and asserts all fifteen record types survive while only the four turn types produce turn rows (8 of them); the fixture suite independently pins that session-basic carries all fifteen types; `the_stored_checksum_is_blake3_over_the_uncompressed_stream` pins what D-06 hashes.

## Summary

total: 8
passed: 8
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 0
