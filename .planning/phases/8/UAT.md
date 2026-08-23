---
status: testing
phase: 8
fields_version: 1
started: 2026-08-23
updated: 2026-08-23
---

## Items

### 1. Cold start on a fresh data directory
expected: With an empty VERBATIM_DATA_DIR, a hook-driven ingest pass creates the store, brings forward every table including the phase-8 additions, and one primary query (verbatim sessions) returns real data with no errors.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Fresh VERBATIM_DATA_DIR + `verbatim ingest` created the store with every schema::TABLES table and session_meta ending in is_evicted/capture_mode; `verbatim sessions --project '*' --json` returned ok:true with one real session. Table set and append-at-the-end column order pinned by crates/verbatim-core/tests/schema.rs:30 and :457-549.

### 2. Retention unset deletes and evicts nothing
expected: With no [retention] block in verbatim.toml, an ingest pass over an aged corpus deletes zero sessions and evicts zero; verbatim sessions returns the same count before and after.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: Config::retention_selects_nothing short-circuits before any query (crates/verbatim-core/src/retention/mod.rs); tests/retention.rs:134 and tests/pass.rs:641 pass under --features testkit.

### 3. An evicted session survives listing, search, recall_get and reindex
expected: After a pass evicts a session, verbatim sessions still lists it, a search matching one of its turns still returns that turn, and recall_get on that turn returns body_evicted - and all three still hold after verbatim reindex. A deleted session appears in neither the listing nor the search.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim/tests/retention.rs:320 (listing, identical search hits, show --json body_evicted true / body null after a real eviction pass, deleted session absent from both) plus crates/verbatim-core/tests/reindex.rs:574 and :720 for the reindex half; body_evicted reaches the MCP tool through the same Record (crates/verbatim/src/cmd/mcp/tools.rs:556).

### 4. retention --dry-run names the set and changes nothing
expected: verbatim retention --dry-run --json names the session ids it would evict and delete and leaves every table row unchanged; the subsequent ingest pass acts on exactly that set of ids.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim/tests/retention.rs:237 and :289 pass under --features testkit; one shared evaluation with the instant passed in (crates/verbatim-core/src/retention/mod.rs).

### 5. compact shrinks the reported size and usage reconciles
expected: After a delete pass, verbatim compact leaves verbatim status's reported size_bytes (db + wal + shm) strictly smaller than before it ran, and verbatim usage --json reports archive bytes per project and per month summing to sum(length(sessions.blob)), plus a store-footprint table whose rows sum to the database file size.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim/tests/lifecycle.rs:185, :230, :260, :380, :492 all pass under --features testkit.

### 6. data move relocates the store without touching settings.json
expected: verbatim data move <path> relocates the database, LOCK and injection-state directory to a new path, after which a hook invocation and an MCP recall_search both read the new location with settings.json byte-identical to before the move.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim/tests/datamove.rs:232 and :293 pass under --features testkit, against the pointer arm in crates/verbatim-core/src/store/open.rs:630.

### 7. A snapshot taken during an active pass is a valid store
expected: A snapshot taken while an ingest pass holds the lock opens as a store, passes verbatim verify with no findings, and contains every session committed before that pass started.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim-core/tests/snapshot.rs:111 takes the snapshot with the ingest lock held and an uncommitted BEGIN IMMEDIATE row outstanding, then verifies zero findings and the uncommitted row's absence; :160 integrity_check ok.

### 8. Capture modes store strictly decreasing bytes and mark elisions
expected: Ingesting one transcript under full, lean and minimal stores strictly decreasing blob bytes; every elided record under lean and minimal carries the elision mark; the full store reproduces the transcript byte for byte.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim-core/tests/capture.rs:469, :499, :107, :171, :897 pass under --features testkit; mode threaded to the hook-driven pass at crates/verbatim-core/src/ingest/pass.rs:320.

## Summary

total: 8
passed: 8
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 0
