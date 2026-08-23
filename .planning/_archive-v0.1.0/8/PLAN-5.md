---
phase: 8
plan: 5
requirements: [ING-07]
files:
  - crates/verbatim-core/src/capture.rs
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/store/schema.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/src/ingest/backfill.rs
  - crates/verbatim-core/src/recover.rs
  - crates/verbatim-core/src/testkit.rs
  - tests/fixtures/session-capture.jsonl
  - crates/verbatim-core/tests/capture.rs
  - crates/verbatim-core/tests/schema.rs
  - crates/verbatim-core/tests/ingest.rs
  - crates/verbatim-core/tests/recover.rs
  - crates/verbatim-core/tests/config.rs
---

# Phase 8: Retention And Lifecycle - Plan 5 (capture mode: how much of each record to keep)

## Goal

The user chooses how much of each record the archive stores - `full`, `lean` or
`minimal` - the elision happens on the record's own JSON line before it ever
reaches the blob, every elided record says so in its stored bytes, and `full`
still reproduces the transcript byte for byte.

## Must be true when done

- Ingesting one transcript under `full`, then `lean`, then `minimal`, into three
  separate stores, leaves strictly decreasing `length(sessions.blob)`.
- Every record elided under `lean` or `minimal` carries an elision mark in its
  stored bytes, and every stored line is still valid JSON that classifies as the
  same record type, with the same `uuid`, `timestamp` and turn ordinal it had.
- The `full` store still reproduces the transcript byte for byte - the existing
  `the_blob_reproduces_the_transcript_byte_for_byte` test in
  `crates/verbatim-core/tests/ingest.rs` still passes, now as a statement about
  `full` alone.
- `session_meta.capture_mode` says which mode a session was captured under, and
  reads `full` only while every append to it has been full.
- `verbatim verify` finds nothing wrong with an elided store, `verbatim reindex`
  rebuilds it from its own stored bytes, and a second ingest pass appends to an
  elided session without re-reading or rewriting a byte it already holds.
- A store written by an earlier binary gains the new column on its next
  write-mode open, with no forced rebuild of the derived tables.

## Context

- Two things must NOT become true. There is no elision on the `full` path, which
  is the default: a store on defaults behaves exactly as it does today, down to
  the bytes. And nothing here redacts - elision drops a whole named subtree and
  marks that it did, it never rewrites content in place, and ingest-time
  redaction is barred outright by `.planning/PROJECT.md`.

- D-05 fixes the mechanism: elision happens at ingest on the record's JSON line
  BEFORE it enters the blob, targeting the top-level `toolUseResult` and
  `attachment` objects and replacing each with a marker so the line stays valid
  JSON. Those two are named in `crates/verbatim-core/src/index/text.rs`'s module
  doc as identifiable without parsing a message body, and `toolUseResult` is 41%
  of the corpus by bytes. This NARROWS phase 1 D-13 and phase 2's byte-for-byte
  invariant to `full` rather than overturning them.
- D-12 fixes what may not be used: "tool result" is not identifiable from the
  derived tables. `SELECT count(*) FROM turns WHERE record_type='user' AND
  tool_name IS NOT NULL` returns 0 on the live store while `user` records hold
  57.3% of the stream, so an implementation filtering on `turns.tool_name` elides
  nothing at all.
- D-13 and D-19 fix the column: a new `session_meta.capture_mode`, appended at
  the END of the table in both `CREATE_SQL` and `BRING_FORWARD_COLUMNS`, with no
  `DERIVED_SCHEMA` bump. Per-session rather than a store-wide `meta` key because
  the byte-for-byte claim for `full` has to be assertable about a session in an
  existing store.
- `DESIGN-BRIEF.md:163` fixes what the two reduced modes mean: `lean` elides
  LARGE tool-result and attachment bodies, `minimal` keeps prompts, assistant
  text, tool names and args - which is those same two subtrees, elided
  unconditionally.
- Depends on PLAN-1 and PLAN-4 for `crates/verbatim-core/src/config.rs`, and on
  PLAN-2 for `crates/verbatim-core/src/ingest/mod.rs`.

## Tasks

### Task 1: Record the mode a session was captured under

- **Files:** crates/verbatim-core/src/config.rs, crates/verbatim-core/src/store/schema.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/tests/schema.rs, crates/verbatim-core/tests/config.rs
- **Action:** Add a `[capture]` table to `FileConfig` in `config.rs` on the same
  `#[serde(default)]` terms as `[injection]` and `[provider]`, carrying a `mode`
  of `full`, `lean` or `minimal`, defaulting to `full` and resolving an
  unrecognized value to `full` under the same rule `ResponseFormat::parse` states
  - this file grows across phases and a typo must not silently start throwing
  away tool output. Resolve it onto `Config` with an accessor. Then add a
  `capture_mode TEXT` column to `session_meta`, declared at the END of that
  table in `schema::CREATE_SQL` and appended at the END of `session_meta`'s entry
  in `schema::BRING_FORWARD_COLUMNS`, which is what that constant's own doc block
  requires: `ALTER TABLE ADD COLUMN` appends, so a column inserted mid-table
  gives a fresh store and an upgraded store different column orders and any
  positional `r.get(n)` diverges between the two. No `DERIVED_SCHEMA` bump - this
  is additive to an archive table and `bring_forward` runs on every write-mode
  open of an existing store. Write it from `ingest::write_session_meta` by
  carrying the mode on the existing `MetaRow` struct, which exists so that a
  phase adding a column does not become ten positional arguments. The upsert's
  rule for this column is not a plain `coalesce`: the column reads `full` only
  while every append to that session has been full, so a null or `full` stored
  value is replaced by whatever this pass captured under, and a stored value that
  is already `lean` or `minimal` is never moved back to `full` - `blob::append`
  copies completed blocks across untouched, so bytes written under an earlier
  mode are never revisited and a store whose mode changes mid-life holds a mix
  inside one blob.
- **Verify:** `cargo test -p verbatim-core --test schema` and
  `cargo test -p verbatim-core --test config` pass with new cases showing (a) a
  fresh store and a store brought forward from a schema without the column have
  identical `PRAGMA table_info(session_meta)` output including column order,
  (b) bringing the column forward does not change `meta.derived_schema` and
  forces no rebuild, (c) no `[capture]` table and no `verbatim.toml` at all both
  resolve to `full`, an unrecognized mode resolves to `full`, and (d) a session
  first written under `full` and appended to under `lean` reads back as `lean`,
  and one first written under `lean` and appended to under `full` still reads
  back as `lean`.

### Task 2: Elide a record's JSON line, and mark that it happened

- **Files:** crates/verbatim-core/src/capture.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/src/testkit.rs, tests/fixtures/session-capture.jsonl, crates/verbatim-core/tests/capture.rs
- **Action:** Add a `capture` module, declared in `lib.rs` beside `derive` and
  `parse`, holding one function that takes one record's line bytes and the mode
  and returns the bytes to store. Under `full` it returns the input untouched
  and does not parse it - that is what keeps the default path byte-identical and
  free. Under `lean` and `minimal` it parses the line as
  `serde_json::Value`, and if the top-level object carries `toolUseResult` or
  `attachment` it replaces that key's VALUE with a marker object carrying one
  reserved key naming the elision and the byte length it stood in for, then
  re-serializes. `minimal` elides both keys unconditionally; `lean` elides one
  only when its serialized value exceeds a threshold constant declared in this
  module - set it to 8,192 bytes, the breakpoint the phase CONTEXT measured
  ("records over 8 KB 52.0%") and the reading of the design brief's "elide LARGE
  tool-result and attachment bodies" - which is also what makes `minimal`
  strictly smaller than `lean` on any transcript holding a small tool result. A
  line carrying neither key, or one that does not parse as a JSON object, is
  returned untouched and never re-serialized, so only elided records pay the
  round trip and only they change shape (`serde_json`'s map is ordered by key
  here, so a re-serialized line comes back with sorted keys - which nothing
  reads, since `full` is the only mode that promises the source bytes back).
  Nothing else in the line may move: the top-level `type`, `uuid`, `timestamp`,
  `sessionId`, `session_id`, `cwd`, `gitBranch`, `subtype` and `compactMetadata`
  fields are what `parse::record::Record::parse` classifies on and what
  `derive::derive_turn` and `ingest::write_session_meta` read, so an elided
  record must still be the same turn, of the same type, at the same ordinal.
  Add a fixture transcript at `tests/fixtures/session-capture.jsonl` carrying a
  `toolUseResult` well over the threshold, one well under it, an `attachment`
  over and one under, and a plain user and assistant record with neither key,
  with a constant naming it in `testkit.rs` beside `COMPACTED_FIXTURE` and
  `TRUNCATED_FIXTURE` - and deliberately NOT added to `TRANSCRIPT_FIXTURES`,
  whose members are ingested wholesale by the swept CLI and corpus tests, where a
  new member changes counts asserted elsewhere.
- **Verify:** `cargo test -p verbatim-core --test capture` passes with new cases
  showing, over that fixture, (a) under `full` every line comes back byte-
  identical, (b) under `minimal` every line carrying either key is shorter, is
  valid JSON, carries the mark, and reports the same
  `parse::record::Record::parse` classification - record type, `uuid`,
  `timestamp`, `subtype` - as the source line, (c) under `lean` the over-
  threshold records are elided and the under-threshold ones are byte-identical,
  (d) the total bytes across the fixture are strictly decreasing from `full` to
  `lean` to `minimal`, and (e) a line that is not a JSON object is returned
  untouched under every mode.

### Task 3: Elide before the blob, and keep the two coordinate systems straight

- **Files:** crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/tests/ingest.rs, crates/verbatim-core/tests/capture.rs
- **Action:** Wire the elision into `ingest::prepare` and `ingest::apply`. Today
  `prepare` scans the raw tail with `parse::scan_from(&tail, existing.watermark,
  existing.turn_count)`, takes `fresh = &tail[..consumed]`, hands `fresh` to
  `blob::write`/`blob::append`, and `apply` indexes into `fresh` with
  `record.offset - existing_watermark`. That works because the stored stream and
  the transcript file share one coordinate system, and elision is exactly what
  breaks that. So: scan the RAW tail first, as today, purely to establish
  `resume_offset` and `consumed` - the watermark is a FILE offset,
  `read_tail` seeks the file by it, and it must keep meaning what it means. Then
  build the bytes to store by applying task 2's function line by line across
  `fresh`, preserving the line framing exactly (every complete line keeps its
  terminating newline, an empty line is passed through, and the trailing
  incomplete line is already excluded by `consumed`). Then scan the STORED bytes
  with `parse::scan_from(&stored, stream_base, existing.turn_count)`, where
  `stream_base` is the uncompressed length the session's blob already holds -
  read `session_meta.uncompressed_len` in `Existing::read` alongside the flags it
  already reads there, so this costs no extra query - and zero for a session with
  no blob. That second scan is what produces the records `apply` derives turns
  from, so `turns.stream_offset` and `byte_len` address the stored stream, which
  is what `blob::BlobReader`, `recall::get` and `reindex` all read. In `apply`,
  index into the stored buffer with `record.offset - stream_base` and hand the
  stored buffer to `blob::append`. Under `full` the stored bytes are the raw
  bytes and `stream_base` equals `existing.watermark`, so every offset is
  arithmetically what it is today and phase 1 and 2's tests are unchanged. Carry
  the configured mode into `prepare` and on to `MetaRow` for task 1's column.
  `Pass::bytes_read` keeps meaning file bytes read past the watermark, which is
  what `runs.bytes_read` and `status` report.
- **Verify:** `cargo test -p verbatim-core --test ingest` still passes unchanged,
  including `the_blob_reproduces_the_transcript_byte_for_byte`, and
  `cargo test -p verbatim-core --test capture` passes with new cases showing,
  over three separate temp stores fed the same fixture transcript, (a)
  `length(sessions.blob)` is strictly decreasing from `full` to `lean` to
  `minimal`, (b) in the `lean` and `minimal` stores every turn read back through
  `recall::get` returns the stored line at its recorded offset and length, and
  every elided one carries the mark, (c) `verify::verify` reports no findings on
  all three, (d) `reindex::reindex` on the `lean` store reproduces the same turn
  ids, offsets and lengths and the same `testkit::query_set_json`, and (e)
  appending more lines to the transcript and re-ingesting under the same mode
  adds turns whose stored offsets continue from the previous stored length, with
  the watermark still equal to the file's byte length.

### Task 4: The two places that assumed the blob and the file were the same bytes

- **Files:** crates/verbatim-core/src/recover.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/tests/recover.rs, crates/verbatim-core/tests/capture.rs
- **Action:** Two shipped checks compare a FILE offset against STORED bytes, and
  under any mode but `full` that comparison carries no information. First,
  `recover::recover`'s lowered-watermark sweep selects
  `WHERE w.byte_offset > m.uncompressed_len`, which is normally TRUE for an
  elided session - so left alone, recovery "repairs" every elided session on
  every pass, lowering its watermark and making the next pass re-read and
  re-append bytes it already holds, forever. Restrict that arm to sessions whose
  `capture_mode` is null or `full`. The orphan-watermark arm - a non-zero
  watermark for a path nothing archived - is untouched and must stay untouched:
  it is what removes the watermark of a session PLAN-1 deleted. Say in the module
  doc why the elided sessions are outside the sweep: blob and watermark commit in
  one transaction (STOR-02), so there is no divergence for it to find, and the
  comparison it uses is only meaningful while the two coordinate systems
  coincide. Second, `ingest::prefix_matches` compares the archived stream's first
  `watermark` bytes against the file's first `watermark` bytes, to decide whether
  a session already flagged `transcript_diverged` may be un-flagged. For a
  session not captured under `full` those two byte ranges are not comparable, so
  it answers `false` - still divergent - which leaves the flag set, leaves the
  file skipped, and leaves `verbatim verify` naming it. That is the safe answer
  and the one the surrounding code already chooses: the alternative is appending
  at a stale offset onto a blob whose middle no longer exists anywhere, with
  `verify` reporting it clean because the blob's checksum still matches itself.
- **Verify:** `cargo test -p verbatim-core --test recover` still passes
  unchanged, and `cargo test -p verbatim-core --test capture` passes with new
  cases showing (a) two consecutive passes over an unchanged transcript in a
  `lean` store leave the watermark, the blob bytes and the turn count identical
  and produce no `Recovered` repair, (b) a `full` session whose watermark is
  manually put ahead of its `uncompressed_len` is still lowered by the sweep,
  and (c) a `lean` session whose transcript is truncated below its watermark is
  flagged, is skipped on the following pass rather than re-ingested from offset
  0, and is named by `verify::verify`.

## Notes

- The doc comment at the top of `crates/verbatim-core/src/parse/mod.rs` states
  that what ingest hands to the blob stays byte-identical to the source
  "(D-13/AC7)". That sentence now describes `full` only and should say so, since
  D-05 narrows the invariant rather than overturning it.
- An elided record's searchable text shrinks with it: `index::text::project`
  reads the top-level `toolUseResult` and `attachment` subtrees, so under `lean`
  and `minimal` the tokens that were in them are no longer in `turns_fts`. That
  is the cost of choosing a reduced mode and is worth stating where the mode is
  documented, because it is the half a user will notice as "search stopped
  finding that".
- Restricting recovery's lowered-watermark arm to `full` sessions means an
  elided session has no independent second witness for its watermark. There is
  none available without a second `session_meta` column, and D-13 named exactly
  one; the arm was defence in depth against a state one transaction already
  prevents.
- PLAN-5 shares `crates/verbatim-core/src/config.rs` with PLAN-1 and PLAN-4 and
  `crates/verbatim-core/src/ingest/mod.rs` with PLAN-2, and is SEQUENTIAL with
  all three.
- `crates/verbatim-core/src/ingest/pass.rs` and
  `crates/verbatim-core/src/ingest/backfill.rs` were added to the `files:` list
  during execution. `prepare` and `ingest_locked` are the two functions the
  configured mode has to reach, and their only callers outside
  `ingest/mod.rs` are `pass::walk` and `backfill`'s worker - so without those two
  paths `[capture].mode` would be honoured by `verbatim ingest <path>` and
  silently ignored by the hook-driven pass, which is the product's ingest. One
  value is passed through in each and nothing else changes; no plan in this
  phase runs in parallel with this one, and plans 1-4 are already committed.
