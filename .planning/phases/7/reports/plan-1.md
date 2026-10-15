PLAN COMPLETE
Plan: .planning/phases/7/PLAN-1.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Route the four range readers through one archive module | 5d0e8de | `store::archive::ranges` holds the one `SELECT blob`; excerpt, get, context, brief and `testkit::read_turn` call it. Both Verify greps empty; targeted core and binary tests green; full suite deferred to the plan's single suite run. |
| 2 - Route the whole-session readers and the archive sizes through the same module | 767fd42 | `archive::stream` (outer error = database, inner = bytes will not read) and `archive::stored_bytes`; reindex, verify (keys and meta first, then one stream at a time), mechanical, export (evicted arm reads nothing) and usage moved. Verify grep empty; targeted tests green. |
| 3 - Make the core crate's tests reach the archive only through testkit | 74da35a | testkit gains `stored`, `stored_len`, `stream`, `damage`, `rewrite_stream`, `evict`, `plant_session`, each on its own SQL and the blob format. 43 grep lines to 0; the eleven files pass. `damage` zeroes every compressed byte behind an intact header, replacing verify's one-byte flip, recall's 64-zero blob and reindex's zeroed 150 bytes. |
| 4 - Make the binary crate's tests reach the archive only through testkit | f2223ca | testkit gains `stored_total` for lifecycle's independent sum. Snapshots in crash and backfill and `blobs` in recall_cli compare `testkit::stored`; corpus, lifecycle export and crash properties 3-5 read `testkit::stream`; cli's three damage sites use `testkit::damage`. Grep empty; six files pass, both crash negative tests still caught. Product-wide grep names only archive.rs, ingest/mod.rs, retention/apply.rs, testkit.rs. |
Deviations:
- [deviation] Must-be-true says no test names `sessions.blob`. `tests/index.rs:203,756` and `tests/snapshot.rs:125` plant sessions with `INSERT INTO sessions (session_key, session_no, blob)` and are outside this plan's file list. They plant rows and do not read, size, damage or empty the archive, so the operative clause holds; left as they are. Plan 2 must change them when the column goes (`testkit::plant_session` is the seam).
Open items:
- `tests/schema.rs:110` still asserts `sessions` has a `blob` column. It tests the schema itself, so it stays until plan 2 changes the schema.
- `tests/corpus.rs` returns early without the private corpus (0.00s here), so its move to `testkit::stream` compiles but was not exercised.
- The tasks' Verify names the full workspace suite; per the executor contract it ran once, after Task 4: `cargo test --workspace --all-features`, exit 0, 64 test binaries, 827 passed, 0 failed. Each task was verified with its targeted test files.
