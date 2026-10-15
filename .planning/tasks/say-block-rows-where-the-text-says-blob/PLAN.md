# Task: say block rows where the text still says blob

Phase 7 replaced the whole-session blob with block rows, but the word outlived
it. Users see it in `terminus --help`, in two error messages and in a watermark
repair line `status` prints. `DESIGN-BRIEF.md:432` says format bumps rebuild
derived tables only, which is false for the archive format. The `terminusElided`
figure is documented as the bytes that stood there, but it is the dropped
value's size re-encoded as compact JSON, and it is written into every lean-mode
archive for good. The owner ruled this a fix before release (2026-10-14).

Scope: text and comments, never identifiers. The `blob` module, the `Blob*`
error variants and the `after-blob` fault point keep their names; renaming them
changes nothing a user sees. Comments outside the list below are left for the
"Comments back to invariants" seed.

## Task 1: what a user reads

Files: `crates/terminus/src/main.rs`, `crates/terminus-core/src/error.rs`,
`crates/terminus-core/src/recover.rs`, `crates/terminus-core/src/capture.rs`,
`DESIGN-BRIEF.md`, `docs/json-shapes.md`, `CHANGELOG.md`.

Action: the usage lines for `verify` and `reindex`, the `Display` text of
`BlobFormat` and `BlobChecksumMismatch`, the watermark repair line, the elision
size's doc and its changelog line, `DESIGN-BRIEF.md:60,65,70,72,93,341,432`,
and `docs/json-shapes.md:96,120,295-296` say block rows, the archive or the
archived session instead of blob, and line 432 says what an upgrade actually
does to each kind of table.

Verify: `grep -n -i blob` over `DESIGN-BRIEF.md`, `docs/json-shapes.md`, the
usage text and the three messages prints nothing, and `cargo test --workspace
--all-features` passes.

## Task 2: the comments the phase 7 summary lists

Files: `crates/terminus-core/src/lib.rs`, `reindex.rs`, `observe/mod.rs`,
`recall/search.rs`, `parse/mod.rs`, `crates/terminus/src/cmd/reindex.rs`,
`crates/terminus-core/tests/retention.rs`, `tests/reindex.rs`,
`tests/backfill.rs`.

Action: each listed comment describes block rows. `parse/mod.rs` stops saying
the archived bytes are always identical to the source: they are in capture mode
`full`, and lean and minimal elide named subtrees.

Verify: `cargo build --workspace --all-features --tests` succeeds and the listed
lines no longer say blob.

## Outcome

- Task 1 shipped in fcc19f7: the usage text, both archive error messages, the watermark repair line, the elision size's doc and changelog line, DESIGN-BRIEF and json-shapes. Also `tests/recover.rs:122`, an assert message quoting the repair line, which the plan did not list. The full suite passed, 842 tests across 64 binaries.
- Task 2 shipped in the commit after it: the listed comments, and `parse/mod.rs` now says the archive is byte-identical to the source in capture mode `full` only. The workspace builds with its tests.
- About a hundred other comments still say blob, and the `blob` module and `Blob*` variants keep their names, as scoped.
