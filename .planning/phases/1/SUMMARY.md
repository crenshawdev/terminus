---
phase: 1
status: complete
completed: 2026-08-12
---

# Phase 1: Archive Core - Summary

A SQLite-backed archive that stores a session as one block-framed zstd blob with
a BLAKE3 checksum over its uncompressed bytes, reads a single turn by
decompressing only the blocks it occupies, commits an ingest pass in one
transaction, and rebuilds `turns`, `turns_fts`, `entities` and `paths` from the
blobs alone.

## What shipped

- Cargo workspace, two crates, no async runtime or HTTP in the dependency floor -
  `Cargo.toml`, `crates/verbatim-core`, `crates/verbatim`
- Store open behind the D-09 two-integer format gate, read-only so a refusal
  writes nothing - `crates/verbatim-core/src/store/open.rs`
- Nine-table schema with deterministic `turns.id = (session_no << 24) | turn_seq`
  and a contentless deletable FTS5 table - `crates/verbatim-core/src/store/schema.rs`
- Block-framed zstd blob format: writer, range reader with a block counter, and
  an append that recompresses only the trailing partial block -
  `crates/verbatim-core/src/blob/`
- Streaming JSONL parser with mid-line resume at the last newline -
  `crates/verbatim-core/src/parse/`
- Single-transaction ingest behind an immediate-fail OS file lock -
  `crates/verbatim-core/src/ingest/`
- The one derive seam ingest and reindex share - `crates/verbatim-core/src/derive.rs`
- `verbatim ingest`, `verbatim verify`, `verbatim reindex` - `crates/verbatim/src/cmd/`
- Fault-injection points and a randomized SIGKILL harness with a runnable
  negative - `crates/verbatim-core/src/ingest/mod.rs` (`fault`), `crates/verbatim/tests/crash.rs`
- Synthetic transcript fixture set including D-01's colliding sidecar pair -
  `tests/fixtures/`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 88474a8 | Cargo workspace and its dependency floor |
| 1 | 2 | 111622c | Synthetic transcript fixture set |
| 1 | 3 | 7d6253a | Store open behind the D-09 format version gate |
| 1 | 4 | 1d74f34 | Store schema |
| 1 | 5 | 4eae97d | Block-framed zstd blobs with a BLAKE3 checksum |
| 1 | 6 | ab1604a | Read a byte range by decompressing only its blocks |
| 1 | 7 | 61d24a3 | Append without recompressing completed blocks |
| 1 | gate | 9c43b83 | Refuse corrupt blobs instead of certifying them |
| 1 | gate | 321b9a8 | Decide a store exists by its tables, not its file length |
| 2 | 1 | 616d067 | Parse a transcript range into records and a resume offset |
| 2 | 2 | f7359d0 | Ingest lock with an immediate-fail OS try-lock |
| 2 | 3 | 5aad7c0 | Ingest one named transcript in a single transaction |
| 2 | 4 | 0d14d1a | Derive FTS rows through the seam ingest and reindex share |
| 2 | 5 | da5735f | `verbatim verify` walks blob checksums |
| 2 | 6 | 751d228 | `verbatim reindex` rebuilds derived tables from blobs alone |
| 2 | 7 | 5e9a6f9 | Randomized kill harness proving crash atomicity |
| 2 | gate | 3e5d9ff | Stop a damaged session from destroying the archive around it |

17 commits, `573f325..3e5d9ff`. Five of them are gate fixes rather than plan
tasks; see Deviations.

## Deviations

- [deviation] D-19 asserts `rusqlite` needs the `fts5` cargo feature. That
  feature does not exist and never did; cargo refuses to resolve it.
  `libsqlite3-sys-0.38.1/build.rs:132` compiles the amalgamation with
  `-DSQLITE_ENABLE_FTS5`, so `bundled` alone is what puts FTS5 in. Used
  `features = ["bundled"]` and kept D-19's own mitigation, the runtime
  assertion in `crates/verbatim-core/tests/sqlite_capabilities.rs`. (88474a8)
- [deviation] PLAN-2 task 6 said opening a store with a lower `derived_schema`
  runs the rebuild and bumps `meta`; PLAN-1's committed
  `store_open.rs::an_older_derived_schema_opens_with_a_rebuild_outcome` asserts
  the opposite, and that file was outside PLAN-2's lease. Resolved so that
  `Store::open` stays side-effect-free and reports the outcome, and
  `reindex::open_up_to_date` is the caller that acts on it. No file outside the
  lease was touched. (751d228)
- [deviation] PLAN-2 tasks 5 and 6 put process-level assertions in
  `crates/verbatim-core/tests/`, which cannot spawn the binary:
  `CARGO_BIN_EXE_verbatim` exists only for integration tests of the package
  declaring the bin. Process-level assertions moved to a new
  `crates/verbatim/tests/cli.rs`, declared in PLAN-2's frontmatter with the
  reason. (da5735f, 751d228)
- [deviation] AC3's "names that session id and no other" was undecidable as
  written, because D-01 establishes the record-reported `session_id` is not
  unique across sessions. `verify` names sessions by `session_key`, and both the
  library and CLI tests ingest the colliding pair and corrupt the sidecar, so
  naming its parent would fail. (da5735f)
- [deviation] AC5's CLI half was asserted by no task. Added: both
  `verbatim verify` and `verbatim ingest` run against a hot-WAL copy of a store
  one archive format ahead and must exit non-zero naming both versions, leaving
  `verbatim.db`, the WAL, `runs` and `meta` byte-identical. (da5735f)
- [deviation] PLAN-2 task 7's harness had no mechanism for landing a kill inside
  the ingest transaction - an uninterrupted release pass over 5 MB measures
  15 ms - and asked for its negative confirmation as a manual edit-and-revert.
  Added six `testkit`-gated fault points and made the negative a recorded test.
  The shipped release binary has no such branch, confirmed by running it with
  the fault variables set. (5e9a6f9)
- [deviation, gate] The blocking `risk_surface` review on plan 1's range
  reproduced three defects, all fixed before plan 2 was dispatched: `blob::append`
  laundered corruption in a completed block into a fresh self-consistent
  checksum (it now takes and verifies the recorded checksum); an unbounded
  header drove a multi-gigabyte allocation and aborted the process instead of
  returning an error; and `initialize` ran as two transactions, bricking the
  data directory on a crash. The gate re-armed once on that fix and the brick
  survived, because `PRAGMA journal_mode=wal` grows a fresh file to 4096 bytes
  before `initialize` runs - so `Store::open` now decides a store exists from
  whether `sqlite_master` holds tables. (9c43b83, 321b9a8)
- [deviation, gate] The blocking `risk_surface` review on plan 2's range
  reproduced three more, all fixed: re-ingesting a session whose `session_meta`
  row was missing overwrote its archived blob with only the tail and exited 0;
  `reindex` aborted the whole rebuild on the first undecompressable blob, which
  through `open_up_to_date` stopped every future ingest; and `reindex` dropped
  all four derived tables without taking the ingest lock. (3e5d9ff)

## Open items

- `cargo test -p verbatim-core` runs **zero** of the five `#![cfg(feature = "testkit")]`
  test files and exits 0. They only run because `verbatim`'s dev-dependency
  enables the feature and resolver-v2 unifies it on a whole-workspace run. Any
  `-p`-scoped CI step is a green run over an empty test set.
- The crash harness's negative test asserts the weaker of the two watermark
  checks: `check_invariants` complains via the "watermark with no archived
  session" branch, and the assertion only matches the word "watermark", so
  deleting the per-session "watermark past the bytes the committed blob holds"
  check would leave it green. Every kill also targets a first pass into a fresh
  data dir, so the append/resume path - the one a live session takes on every
  pass, and the only one `blob::append`'s checksum verification guards - is
  never killed.
- `crates/verbatim-core/src/store/schema.rs` documents that foreign keys are
  "deliberately not enforced" and that `PRAGMA foreign_keys` stays off. They are
  enforced: the bundled amalgamation is compiled with
  `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`, and `reindex.rs` relies on the opposite
  fact for its reverse drop order. Two committed comments contradict each other
  about a constraint that dictates correct code.
- `lock_race.rs::the_lock_is_released_by_process_death` never starts or kills a
  process; it acquires and drops a guard in-process, so it would pass unchanged
  against a PID file. The stale-lock-after-SIGKILL property it is named for is
  asserted nowhere.
- `session_meta.continues_from` is populated in the session-id namespace, with a
  `parentUuid` fallback that depends on the predecessor already being ingested.
  D-11 does not say which namespace the fallback writes. Phase 2 (ING-04)
  consumes the column and should confirm the choice.
- The 64 KB block size remains the flagged unmeasured assumption. It is a header
  field, not a constant in turn coordinates, so changing it costs a re-blob and
  no turn-row rewrite. D-08's 11.2% framing penalty was not re-measured.

## Goal check

The commits do deliver the phase goal, and the evidence is per-criterion rather
than per-feature. **Losslessly**: `ingest.rs::the_blob_reproduces_the_transcript_byte_for_byte`
holds the decompressed stream against the file over all fifteen record types,
and `the_stored_checksum_is_blake3_over_the_uncompressed_stream` pins D-06's
choice of what is hashed. **Read one turn cheaply** (AC1):
`ingest.rs::reading_a_turn_decompresses_only_the_blocks_it_occupies` reads
through the counter in `blob/reader.rs:19`, and
`blob.rs::the_block_count_matches_the_windows_touched_for_random_ranges` checks
that counter against an independently computed windows-touched figure over 200
random ranges, so the instrument is itself falsified rather than trusted.
**Crash atomicity** (AC2): `crash.rs::killing_ingest_anywhere_leaves_a_store_that_converges`
runs 23 SIGKILLs from a printed seed, 15 aimed at named fault points inside the
pass, and `the_harness_catches_a_watermark_committed_outside_the_transaction` is
the negative that makes it fail-capable. **Prove its own integrity** (AC3):
`cli.rs::verify_names_the_corrupt_session_and_no_other` corrupts the sidecar of
D-01's colliding pair, so naming the parent instead would fail. **Rebuild from
blobs alone** (AC4): `reindex.rs::the_rebuild_reads_nothing_but_the_blobs` and
`cli.rs::reindex_rebuilds_the_derived_tables_to_byte_identical_query_output`.
**Refuse a newer format** (AC5): `store_open.rs::a_newer_archive_format_is_refused_without_touching_the_store`
compares `verbatim.db` and a hot WAL byte for byte, and
`cli.rs::a_store_one_archive_format_ahead_is_refused_with_both_versions_named`
covers the process half. **Lock contention** (AC6):
`lock_race.rs::a_contended_ingest_exits_zero_fast_and_writes_nothing`, with a
control run afterwards proving the counts could have moved.

What is missing is narrower than the goal but not cosmetic. Six defects reached
committed code and were caught only by the two blocking reviews, four of them
data-destroying: an append that certified corruption into a fresh checksum, a
re-ingest that overwrote an archived blob with its own tail and exited 0, a
rebuild that one bad blob turned into a store-wide outage, and a destructive
`reindex` running with no lock. That the phase's own tests passed throughout is
the honest finding here - "prove its own integrity" is the phase goal, and the
proofs were weaker than the property until the reviews landed. Two of the open
items are the same shape and are unresolved: a `-p`-scoped test run silently
executes none of the five core test files, and the crash harness never kills an
append/resume pass, which is exactly the path a live session takes and the one
`blob::append`'s new checksum verification guards. Phase 2 ingests the real
1,896-file corpus over that path, so both should close before it does.
