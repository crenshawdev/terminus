---
phase: 1
plan: 1
requirements:
  - STOR-01
  - STOR-05
files:
  - Cargo.toml
  - Cargo.lock
  - LICENSE
  - .gitignore
  - crates/verbatim-core/Cargo.toml
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/error.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/src/store/mod.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/src/store/schema.rs
  - crates/verbatim-core/src/blob/mod.rs
  - crates/verbatim-core/src/blob/writer.rs
  - crates/verbatim-core/src/blob/reader.rs
  - crates/verbatim-core/tests/sqlite_capabilities.rs
  - crates/verbatim-core/tests/store_open.rs
  - crates/verbatim-core/tests/schema.rs
  - crates/verbatim-core/tests/blob.rs
  - crates/verbatim-core/tests/fixtures.rs
  - crates/verbatim/Cargo.toml
  - crates/verbatim/src/main.rs
  - tests/fixtures/README.md
  - tests/fixtures/session-basic.jsonl
  - tests/fixtures/session-large-record.jsonl
  - tests/fixtures/session-continuation.jsonl
  - tests/fixtures/session-truncated.jsonl
  - tests/fixtures/subagents/agent-alpha.jsonl
  - tests/fixtures/subagents/workflows/wf_demo/journal.jsonl
---

# Phase 1: Archive Core - Plan 1 of 2 (storage substrate)

**SEQUENTIAL: PLAN-1 must complete before PLAN-2 starts.** They share
`Cargo.toml`, `Cargo.lock`, `crates/verbatim-core/Cargo.toml`,
`crates/verbatim-core/src/lib.rs`, `crates/verbatim-core/src/testkit.rs`,
`crates/verbatim-core/src/store/open.rs`, `crates/verbatim/Cargo.toml` and
`crates/verbatim/src/main.rs`. Do not run them in parallel.

## Goal

A store that can hold a session's turns losslessly, read one turn cheaply,
prove its own integrity, and rebuild everything derived from the blobs alone.
This plan builds the substrate that makes that possible: the workspace, the
fixtures, the store and its version gate, and the block-framed zstd blob with
its per-blob checksum.

## Must be true when done

- The workspace builds one `verbatim` binary whose dependency tree contains no
  async runtime, no HTTP client and no rayon.
- A fresh store opens at a resolvable data directory in WAL mode and carries
  two version integers, `archive_format` and `derived_schema`, in `meta`.
- Opening a store whose `meta.archive_format` is one higher than the binary
  knows exits non-zero with a message naming both versions and leaves
  `verbatim.db`, `verbatim.db-wal` and the `runs` table unchanged (AC5).
- Any byte stream written through the blob writer reads back byte-identical,
  and the BLAKE3 the writer reports equals BLAKE3 computed independently over
  the same uncompressed bytes.
- Reading a byte range that lies inside one 64 KB block decompresses exactly
  one block; a range spanning N blocks decompresses exactly N (AC1, at the
  blob layer).
- Appending to an existing blob recompresses only the trailing partial block:
  the compressed bytes of every already-completed block are unchanged.
- The in-repo synthetic fixture set covers all fifteen record types, an
  out-of-order timestamp pair, a record larger than 64 KB, a subagent sidecar,
  a non-transcript `journal.jsonl` and a mid-line truncation.

## Context

Greenfield: the repo holds `DESIGN-BRIEF.md` and `.planning/` only, no
`Cargo.toml`, no source. Every decision D-01..D-20 in
`.planning/phases/1/CONTEXT.md` is locked; this plan implements D-01, D-04,
D-05, D-06, D-08, D-09, D-10 (the schema half), D-11 (the schema half), D-17
and D-19. Out of scope here and left to PLAN-2: parsing, tailing, the ingest
transaction, the LOCK file, `verify`, `reindex`. Out of scope for the whole
phase: tree discovery, project identity from `cwd`, exclusions, compaction
boundaries, `status` (phase 2); text expansion, entity extraction, search,
MCP (phase 3); hooks and install (phase 4).

## Tasks

### Task 1: Create the cargo workspace and its dependency floor

- **Files:** Cargo.toml, Cargo.lock, LICENSE, .gitignore,
  crates/verbatim-core/Cargo.toml, crates/verbatim-core/src/lib.rs,
  crates/verbatim/Cargo.toml, crates/verbatim/src/main.rs
- **Action:** Create a two-member cargo workspace per D-17: `verbatim-core`
  (store, blob, ingest) and `verbatim` (the CLI, later the MCP server), both
  under `crates/`, resolver 2, edition 2021 or newer, `license = "Apache-2.0"`
  in both manifests and the full Apache-2.0 text in `LICENSE` at the repo root
  (DESIGN-BRIEF.md:512-513 makes this public OSS, an explicit override of the
  private-by-default rule). `verbatim-core` depends on `rusqlite` with features
  `["bundled", "fts5"]` pinned to the current release per D-19 (FTS5 is not
  compiled in by `bundled` alone, it needs the cargo feature), plus `zstd`,
  `blake3`, `serde`, `serde_json` and `thiserror`. `verbatim` depends on
  `verbatim-core` and an argument parser. Declare a `testkit` cargo feature on
  `verbatim-core`, off by default, which later tasks use to expose test-support
  code without shipping it in the binary. Add no `rayon`, no HTTP client and no
  async runtime anywhere in the tree: cold start is the product
  (DESIGN-BRIEF.md:39, PROJECT.md constraints), and a runtime pulled in
  transitively is the failure this constraint exists to prevent. Commit
  `Cargo.lock` (this workspace ships a binary). Extend `.gitignore` with
  `/target` without disturbing the existing `.planning/trace.jsonl` line. Give
  `main.rs` only what makes the binary runnable: `--version` prints the crate
  version to stdout and exits 0, an unrecognized argument prints to stderr and
  exits 2 (misuse), stdout stays empty on misuse. Do not add subcommands here;
  each command task in PLAN-2 adds its own.
- **Verify:** `cargo build --workspace` succeeds; `cargo run -p verbatim --
  --version` prints a version to stdout and exits 0; `cargo run -p verbatim --
  bogus` exits 2 with empty stdout; `cargo tree -p verbatim --edges normal | grep
  -Eic 'tokio|async-std|smol|futures-executor|reqwest|hyper|rayon'` prints 0.

### Task 2: Commit the synthetic transcript fixture set

- **Files:** tests/fixtures/README.md, tests/fixtures/session-basic.jsonl,
  tests/fixtures/session-large-record.jsonl,
  tests/fixtures/session-continuation.jsonl,
  tests/fixtures/session-truncated.jsonl,
  tests/fixtures/subagents/agent-alpha.jsonl,
  tests/fixtures/subagents/workflows/wf_demo/journal.jsonl,
  crates/verbatim-core/src/testkit.rs, crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/tests/fixtures.rs
- **Action:** Real transcripts are private and cannot be committed
  (DESIGN-BRIEF.md:500), so build the small synthetic set every later task
  tests against, shaped to the facts CONTEXT measured. `session-basic.jsonl`
  contains records of all fifteen types D-03 names: the four turn types
  (`user`, `assistant`, `attachment`, `system`) each carrying both `uuid` and
  `timestamp`, and the eleven state types (`last-prompt`, `ai-title`, `mode`,
  `permission-mode`, `file-history-snapshot`, `queue-operation`,
  `bridge-session`, `file-history-delta`, `agent-name`, `agent-setting`,
  `frame-link`) which carry no turn identity; include at least one adjacent
  pair of turn records whose `timestamp` values decrease in file order (D-02:
  31 of 62 sampled real transcripts have one), at least one
  `file-history-snapshot` record with no `sessionId` at all, `parentUuid`
  chaining across the turn records, and `cwd` plus `gitBranch` on every record
  that would carry them upstream. `session-large-record.jsonl` holds one turn
  record whose text field is roughly 200 KB of deterministic filler so it spans
  four 64 KB blocks (D-05). `session-continuation.jsonl` carries a foreign
  `session_id` field naming `session-basic`'s session id, which is the D-11
  lineage signal, plus its own `parentUuid` chain. `subagents/agent-alpha.jsonl`
  reports the parent session's `sessionId` on every record with
  `isSidechain: true` (D-01: this is the collision case). `journal.jsonl` holds
  `{agentId, key, result, type}` records with none of `sessionId`, `uuid`,
  `timestamp` or `cwd` (D-12). Every file except `session-truncated.jsonl` ends
  with a trailing `\n` and contains no `\r` byte (D-14);
  `session-truncated.jsonl` is `session-basic.jsonl` cut mid-record with no
  trailing newline. `README.md` states what each fixture exists to exercise.
  Add `testkit.rs` behind the `testkit` feature holding only fixture path
  resolution from `CARGO_MANIFEST_DIR` and a helper that copies a fixture into
  a temp dir; it is the shared home for test support so both crates can reach
  the same fixtures without a third workspace member.
- **Verify:** `cargo test -p verbatim-core --features testkit --test fixtures`
  passes, where the test asserts: every fixture except `session-truncated.jsonl`
  ends with byte 0x0a and contains no 0x0d; `session-basic.jsonl` contains all
  fifteen `type` values and at least one adjacent turn pair whose timestamps
  decrease; `session-large-record.jsonl` has exactly one line longer than 65536
  bytes; every record in `journal.jsonl` lacks all four of `sessionId`, `uuid`,
  `timestamp`, `cwd`; every record in `subagents/agent-alpha.jsonl` reports the
  same `sessionId` as `session-basic.jsonl`.

### Task 3: Open the store with the format version gate

- **Files:** crates/verbatim-core/src/store/mod.rs,
  crates/verbatim-core/src/store/open.rs, crates/verbatim-core/src/error.rs,
  crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/tests/sqlite_capabilities.rs,
  crates/verbatim-core/tests/store_open.rs
- **Action:** Resolve the data directory once at startup and pass it down
  explicitly, never re-deriving it (DESIGN-BRIEF.md:404): `VERBATIM_DATA_DIR`
  when set, otherwise `$XDG_DATA_HOME/verbatim` on Linux,
  `~/Library/Application Support/verbatim` on macOS, `%LOCALAPPDATA%\verbatim`
  on Windows. Open `verbatim.db` in WAL mode with a busy timeout. Implement the
  D-09 gate: two integers in the `meta` key/value table, `archive_format` and
  `derived_schema`, both read before any write to the store. On an existing
  store, read them inside a read transaction and run no DDL and no DML before
  the gate decides, because AC5 requires a refused open to leave the database
  file, its WAL and the `runs` table byte-for-byte unchanged. When the store's
  `archive_format` exceeds the binary's, return an error naming both the
  store's value and the binary's, and let the CLI exit non-zero (STOR-05). When
  either integer is lower than the binary's, do not rebuild here: return that
  condition to the caller as a distinct, inspectable outcome, and note in the
  code that PLAN-2's reindex task wires it to the actual derived rebuild, since
  the rebuild function does not exist yet. A non-existent store is created and
  initialized with the binary's current values. Introduce the crate's error
  type in `error.rs` here, the first place that needs one. Add
  `tests/sqlite_capabilities.rs` for the two D-19 assertions that must fail
  loudly in CI rather than at a user's first ingest: `SELECT sqlite_version()`
  is at or above 3.43, the floor `contentless_delete=1` needs, and creating a
  scratch FTS5 table with `content=''` and `contentless_delete=1` succeeds.
- **Verify:** `cargo test -p verbatim-core --test store_open --test
  sqlite_capabilities` passes, where the tests assert: a fresh store in a temp
  dir opens, reports `journal_mode = wal`, and has both `meta` integers set;
  `VERBATIM_DATA_DIR` pointed at a temp dir puts `verbatim.db` there; after
  setting `meta.archive_format` to the binary's value plus one, the SHA-256 of
  `verbatim.db`, the SHA-256 of `verbatim.db-wal` and the `runs` row count are
  identical before and after a failed open, and the error message text contains
  both version numbers; a store whose `derived_schema` is one lower opens with
  the rebuild-required outcome and no error; `sqlite_version()` >= 3.43 and the
  scratch contentless FTS5 table is created.

### Task 4: Create the store schema

- **Files:** crates/verbatim-core/src/store/schema.rs,
  crates/verbatim-core/src/store/mod.rs, crates/verbatim-core/tests/schema.rs
- **Action:** Create every table phase 1 needs, in one place, applied when a
  store is initialized. `sessions` is the archive and never migrates
  (DESIGN-BRIEF.md:94): a text `session_key` primary key holding the transcript
  file identity rather than the record's `session_id` (D-01: 812 sidecar files
  report their parent's `sessionId`, so keying on it silently overwrites the
  parent's blob), the blob bytes, and a stable integer surrogate that PLAN-2's
  reindex depends on for deterministic turn ids. `session_meta` carries the
  `session_key`, the record-reported `session_id`, the canonical transcript
  path, the BLAKE3 checksum of the uncompressed session bytes and their length
  (D-06), `continues_from` for file-level lineage (D-11), first and last turn
  timestamps, and nullable `project`, `cwd`, `branch`, `is_final`,
  `is_evicted`, `parent_session_key` columns that later phases fill: phase 1
  leaves `project` NULL because project identity from `cwd` is ING-05 in phase
  2. `turns` carries an integer id derived deterministically from the session's
  stable integer key and `turn_seq` so a drop-and-rebuild reproduces the same
  ids (D-10), plus `session_key`, `turn_seq`, `uuid`, `parent_uuid` (D-11 keeps
  this as the per-turn message-level column, distinct from `continues_from`),
  the record type, an optional tool name, the timestamp, and the offset and
  length of the record within the *uncompressed* session stream, never within
  the compressed blob (D-04: the blob header owns the translation, so a future
  block-size change never rewrites a turn row). `turns_fts` is an FTS5 table
  with `content=''` and `contentless_delete=1`, its rowid being `turns.id`
  (D-10). `entities` (turn_id, kind, value_norm) and `paths` (turn_id, path)
  are created with their indexes and stay empty in this phase. `watermarks`
  maps a canonical transcript path to a byte offset. `runs` records a pass:
  start, finish, duration, counts, error. `meta` is the key/value table task 3
  reads. Pick and document the deterministic turn-id encoding, keeping the
  per-session turn ordinal space large enough for the largest observed session
  (10.1 MB, p90 1.0 MB) with room to spare.
- **Verify:** `cargo test -p verbatim-core --test schema` passes, where the
  test asserts against a fresh temp store: `sqlite_master` lists exactly
  `sessions`, `session_meta`, `turns`, `turns_fts`, `entities`, `paths`,
  `watermarks`, `runs`, `meta` plus their indexes and FTS5 shadow tables; the
  `sql` for `turns_fts` contains both `content=''` and `contentless_delete=1`;
  `PRAGMA table_info` on `turns` shows `parent_uuid` and on `session_meta`
  shows `continues_from`; inserting a `turns_fts` row at an explicit rowid,
  matching it, deleting it, and matching again returns one hit then zero.

### Task 5: Write block-framed zstd blobs with a BLAKE3 checksum

- **Files:** crates/verbatim-core/src/blob/mod.rs,
  crates/verbatim-core/src/blob/writer.rs, crates/verbatim-core/tests/blob.rs
- **Action:** Define the blob format and its writer. The header carries a magic
  value, the archive format version, the codec id, the block size and a block
  offset table mapping each block's uncompressed start offset to its compressed
  extent in the blob (DESIGN-BRIEF.md:73, D-04). Blocks are 65536 uncompressed
  bytes each, the last one short, compressed independently with zstd level 3
  and no dictionary: D-08 measured level 3 at 4.21x against level 9's 4.49x on
  real block-framed transcripts, and a dictionary would become part of an
  archive format that never migrates. The writer reports the BLAKE3 hash of the
  uncompressed session bytes for `session_meta` (D-06: independent of SQLite's
  own checks and of the codec, so a decoder change is distinguishable from
  corruption). The blob holds the transcript bytes verbatim, never a parsed
  projection (D-13), so the writer takes bytes and imposes no record structure.
- **Verify:** `cargo test -p verbatim-core --test blob` passes, where a
  property test over the byte streams empty, 1 byte, 65535, 65536, 65537,
  200 KB and 1.2 MB (plus pseudorandom lengths from a printed seed) asserts:
  writing then reading the whole blob returns bytes identical to the input; the
  parsed header reports block size 65536 and a block count of
  `ceil(len/65536)`; the writer's reported BLAKE3 equals `blake3::hash` over
  the same input computed directly in the test.

### Task 6: Read a byte range by decompressing only the blocks it occupies

- **Files:** crates/verbatim-core/src/blob/reader.rs,
  crates/verbatim-core/src/blob/mod.rs, crates/verbatim-core/tests/blob.rs
- **Action:** Read an arbitrary `(offset, len)` range expressed in
  uncompressed-stream coordinates, translating through the header's block
  offset table and decompressing only the blocks the range overlaps. A range
  may span several blocks: STOR-01's "one block" holds for turns that fit in
  one, and 28 of 6,731 sampled records exceed 65536 bytes with a maximum of
  1,134,645 (D-05), so the reader must handle N blocks correctly rather than
  assume one. Expose a count of blocks decompressed on the reader so AC1 is
  measured rather than asserted by inspection; the counter is the instrument
  the phase's acceptance criterion names, so keep it in the normal build rather
  than behind a test feature. Reading past the end of the stream is an error,
  not a truncated read.
- **Verify:** `cargo test -p verbatim-core --test blob` passes, where the tests
  assert against a 200 KB blob: reading a 100-byte range wholly inside block 2
  returns the expected bytes and reports exactly 1 block decompressed; reading
  a range from inside block 0 to inside block 2 reports exactly 3; reading the
  whole stream reports exactly 4; a sweep of ranges straddling every block
  boundary returns bytes equal to the corresponding slice of the source; the
  block count reported for each range equals the number of distinct 64 KB
  windows the range touches.

### Task 7: Append to a blob without recompressing completed blocks

- **Files:** crates/verbatim-core/src/blob/writer.rs,
  crates/verbatim-core/src/blob/mod.rs, crates/verbatim-core/tests/blob.rs
- **Action:** Support appending bytes to an existing blob. Completed 64 KB
  blocks are immutable once committed and only the trailing partial block is
  recompressed and rewritten (D-07): `SessionEnd` does not fire on a crash
  (DESIGN-BRIEF.md:116), so a live session is re-ingested repeatedly while it
  grows, and recompressing the whole blob each pass would recompress a 10 MB
  session dozens of times and push the full blob size into the WAL on every
  pass. The append path extends the block offset table, rewrites the trailing
  block, and recomputes the header; it must never renumber or move an existing
  block's uncompressed start offset, because `turns` rows already point at
  those coordinates. Expose a count of blocks recompressed per append so the
  property is measurable.
- **Verify:** `cargo test -p verbatim-core --test blob` passes, where the test
  builds a 200 KB blob by appending 1 KB at a time and asserts: the decompressed
  result is byte-identical to a one-shot write of the same 200 KB; the
  compressed extent bytes of every completed block are identical between the
  incremental blob and the one-shot blob; no single append recompresses more
  than one block; and reading any range from the incrementally built blob
  returns the same bytes and the same block count as reading it from the
  one-shot blob.

## Notes

- **Plan shape.** CONTEXT directs "multiple plans, same phase" and this phase
  is split into two, but the slices are not independent: PLAN-2 edits
  `crates/verbatim-core/src/lib.rs`, `Cargo.toml`, `Cargo.lock`,
  `crates/verbatim-core/src/store/open.rs`, `crates/verbatim-core/src/testkit.rs`
  and `crates/verbatim/src/main.rs`, all created here. The split is by capacity
  (14 tasks against an 8-task ceiling), so the plans are SEQUENTIAL, not
  parallel.
- **Requirement split.** STOR-05's newer-format refusal lands in task 3 here;
  its older-format derived rebuild is wired in PLAN-2 task 6, where the rebuild
  function is born. STOR-01's block-accounting mechanism is task 6 here; its
  end-to-end form over an ingested turn is PLAN-2 task 3's verify.
- **Flagged assumption carried from CONTEXT.** The 64 KB block size comes from
  DESIGN-BRIEF.md:73 and was never measured against the real turn-size
  distribution (p50 726 bytes, p90 4,860). Tasks 5 and 6 make it a header field
  rather than a constant baked into `turns` coordinates, so changing it later
  costs a re-blob and no turn-row rewrite. Do not change the value in this
  phase.
- **Flagged assumption carried from CONTEXT.** The eleven state record types
  are undocumented and only Anthropic can confirm they are stable. D-13 is the
  mitigation: the bytes are always in the blob, so `reindex` can reclassify.
  Task 2's fixture pins the current set so drift shows up as a test failure.
