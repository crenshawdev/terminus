PLAN COMPLETE
Plan: .planning/phases/1/PLAN-1.md
Tasks: 7 of 7
| Task | Commit | Note |
|---|---|---|
| 1 - Create the cargo workspace and its dependency floor | 88474a8 | Workspace builds; `--version` exits 0, `bogus` exits 2 with empty stdout; `cargo tree -p verbatim --edges normal` grep for async/HTTP/rayon prints 0. `fts5` cargo feature dropped (see deviation). |
| 2 - Commit the synthetic transcript fixture set | 111622c | 10 fixture-invariant tests pass. `tempfile` added as a dev-dependency so temp-dir tests need no cargo feature; `testkit` stays feature-gated for fixture access. |
| 3 - Open the store with the format version gate | 7d6253a | 7 store_open + 2 sqlite_capabilities tests pass. Gate reads over a read-only connection that closes before any writable one opens. The AC5 test was rebuilt against a hot-WAL copy after the first version proved unfalsifiable, then confirmed to fail with a read-write gate. |
| 4 - Create the store schema | 1d74f34 | 9 schema tests pass. Nine tables, `turns.id = (session_no << 24) \| turn_seq`, `turns_fts` contentless with `contentless_delete=1`. `meta`/`runs` moved out of open.rs into schema.rs as task 3 anticipated. |
| 5 - Write block-framed zstd blobs with a BLAKE3 checksum | 4eae97d | 6 blob tests pass: boundary lengths, incompressible input, pseudorandom lengths from a printed seed, three damaged headers refused. |
| 6 - Read a byte range by decompressing only the blocks it occupies | ab1604a | 13 blob tests pass. The block counter is checked against an independent windows-touched calculation over every boundary and 200 random ranges; an over-decompressing reader was confirmed to fail four of them. |
| 7 - Append to a blob without recompressing completed blocks | 61d24a3 | 19 blob tests pass. 200 compression calls to build 200 KB a kilobyte at a time, against ~416 for whole-stream recompression; whole-stream recompression was confirmed to fail five tests. Incremental blob is byte-identical to the one-shot write. |
Deviations:
- [deviation] D-19 asserts `rusqlite` needs features `["bundled", "fts5"]` because "FTS5 is not compiled in by `bundled` alone, it requires the `fts5` cargo feature". rusqlite 0.40.2 has no `fts5` feature and never did; cargo refuses to resolve the dependency at all. Ground truth: `libsqlite3-sys-0.38.1/build.rs:132` compiles the bundled amalgamation with `-DSQLITE_ENABLE_FTS5`, so `bundled` alone is what puts FTS5 in. Used `features = ["bundled"]`. What "done" means is unchanged: D-19's own mitigation is the runtime assertion, and `crates/verbatim-core/tests/sqlite_capabilities.rs` now proves SQLite >= 3.43 and a working `content=''` + `contentless_delete=1` table with insert, match, delete, re-match.
Open items:
- The block-size flagged assumption stands unresolved, as planned. 64 KB is a header field, not a constant in turn coordinates, so changing it costs a re-blob and no turn-row rewrite. D-08's 11.2% framing penalty was not re-measured in this plan.
- `Store::open` currently sets `journal_mode` and `synchronous` on every open, including opens that will report a rebuild. Harmless in phase 1 (the gate has already decided by then) but PLAN-2's lock work touches the same entry point and should keep the ordering explicit.
- The unused-code surface is real: `blob::append`, `split_turn_id`, `DERIVED_TABLES` and `Store::set_meta_int` have no caller until PLAN-2. They are covered by tests, not dead, but nothing in the binary reaches them yet.
