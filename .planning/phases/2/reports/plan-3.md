PLAN CHECKPOINT: human-verify
Plan: .planning/phases/2/PLAN-3.md
Tasks: 5 of 5
| Task | Commit | Note |
|---|---|---|
| 1 - The parser sees a record's subtype and its compaction metadata | 1757cef | `Record` gains `subtype` and `compact_metadata: Option<Vec<u8>>`; `parse::COMPACT_BOUNDARY` and `Record::is_compact_boundary()`. Metadata bytes are lifted out of the line by a structural walk of the top-level object (`raw_field`), never a substring search and never a re-serialization. New fixture `tests/fixtures/session-compacted.jsonl` (real key order, `compactMetadata` deliberately not last, `uuids` a proper subset of `allUuids`), registered as `testkit::COMPACTED_FIXTURE` in `TRANSCRIPT_FIXTURES` with a `testkit::boundary_line()` helper, documented in the fixtures README. 3 new tests in `tests/parse.rs`, 2 in `tests/fixtures.rs`. |
| 2 - A compaction boundary becomes a row, through the derive seam | 83c0a02 | `TurnRow` carries `subtype` and `compact_metadata`; `derive_turn` deletes any boundary row at the turn id and inserts one when the subtype is `compact_boundary`, on the same delete-then-insert footing as `entities` and `paths`. Both construction sites pass the fields through (`ingest/mod.rs`, `reindex.rs`), so a rebuild reproduces every boundary row from the blob alone. New `tests/compaction.rs`: 6 tests including AC4 in full (append a boundary to an archived transcript; the stored bytes are a byte slice of the appended record, every pre-existing turn row is unchanged, every completed blob block is byte-identical), the whole-file path agreeing with the append path, and a double rebuild. 1 new test in `tests/derive.rs` for the clearing arm. `tests/reindex.rs` now drops `DERIVED_TABLES` in reverse and deletes `compaction_boundaries` before `turns` (checkpoint resolution). |
| 3 - A transcript shorter than its watermark is skipped and flagged | b294ec9 | `Error::TranscriptDiverged { path, len, watermark }` replaces the io error `read_tail` raised, and is the only thing the pass matches on. The pass sets `session_meta.transcript_diverged` in a transaction of its own; `verify::walk` reads it and reports a per-session failure saying the archive was left untouched. `Existing` carries the flag (off the query that already reads `session_meta`, so no extra round trip), and `ingest_locked` clears it the moment `read_tail` succeeds - before the `UpToDate` early return, which is the state a repaired file lands in - guarded on the flag being set, so a healthy walk writes nothing. 5 new tests in `tests/verify.rs` (report shape, both-failures, the clean-again control), 5 new in `tests/compaction.rs` (end to end: truncate, archive byte-identical including the watermark, flag set, `runs` names the path, `verify` names that session and no other; restore and the flag clears; regrowth clears it on the committing path; and both halves of the discrimination). Mutation-checked in both directions: removing the flag write fails 3 tests, flagging on every failure fails 1. |
| 4 - Killing a pass mid-tree converges, on a first pass and on an append | 04680b1 | `ingest::fault` gains `AFTER_FILES` + `AFTER_FILES_COUNT`: the walk stalls once N transcripts have committed, so a kill lands between two files rather than where a timed race falls. Kept out of `fault::POINTS` (it needs a count as well as a name) and inert without `testkit`. `crates/verbatim/tests/crash.rs` extended, not replaced: a four-transcript tree over two projects with a sidecar at real depth, and the same spread run twice - first pass and append pass - 12 kills each (3 file-aimed, 5 moment-aimed, 4 timed), every one resumed and required to reach the byte-identical reference store. An append iteration starts from a copy of the pre-append data directory, since the tree's files have already grown by then. `check_invariants` now asserts the stronger watermark property (the byte before it in the committed blob is a newline) and its doc names which of the five properties each check is; a second negative test moves a watermark off a boundary and requires the complaint to be about the boundary and not about containment. Every spawn now isolates `VERBATIM_CONFIG_DIR` and `CLAUDE_CONFIG_DIR` too, since a bare `ingest` walks the configured roots. Mutation-checked: disabling the boundary check fails exactly the new negative test. Ran green 4 times over. |
| 5 - A pass over the real corpus, measured | 9fa9b8e | New `crates/verbatim/tests/corpus.rs`, gated on `testkit::CORPUS_DIR_ENV` (`VERBATIM_TEST_CORPUS`, naming a Claude config directory) and skipping with a printed reason when unset. Isolates the data directory *and* the config directory, and asserts the config override resolves no `verbatim.toml`. AC1 as a set relation over the intersection of a walk before the pass and one after, with unarchived paths in that intersection excused only when the file holds no complete line, printed by name. The walk is the test's own reading of AC1's rule and deliberately not `discover::discover`. Negative half asserted over the session keys: no `journal.jsonl`, no `.meta.json`, no non-`.jsonl`, nothing under `tool-results/`, every key a transcript by the rule. Zero unparseable lines checked by decompressing every blob and handing every line to `serde_json` (`testkit::LineSurvey`); record types counted and printed, never asserted (D-25). AC2's two `continues_from` counts. Pass and forced-rebuild wall times printed (D-24). `testkit` gains `CORPUS_DIR_ENV`, `corpus_dir()` and `LineSurvey` - in the library because `crates/verbatim` has no `serde_json` dev-dependency and its manifest is outside this lease. Verified: skip path green; whole test dry-run green against a 133-file synthetic tree in the scratchpad that exercises every branch (both sidecar depths, a no-complete-line transcript, journals, a `.meta.json`, `tool-results/`, a `workflows/wf_*.json`); mutation-checked - counting journals as transcripts fails the archived-every-transcript assertion. **The real-corpus half of the Verify is the human-verify checkpoint below.** |

Deviations:
- [deviation] Task 1's Verify asserts "parsing any line of `session-basic.jsonl` yields no subtype and no compaction metadata". The phase 1 fixture already carries `subtype: "local_command_output"` on a `system` record, as real transcripts do, so the subtype half of that clause is false against the fixture rather than against the change. Asserted the criterion's intent instead: no fixture but the compacted one yields compaction metadata or the `compact_boundary` subtype, a line carrying neither field parses to both `None`, and `session-basic.jsonl`'s one real subtype is read as itself.
- [deviation] Task 2's `files:` lease did not cover `crates/verbatim-core/tests/reindex.rs`, which task 1's fixture registration made necessary: with a boundary row present, that helper's creation-order `DROP TABLE` and its bare `DELETE FROM turns` both violate the foreign key the bundled SQLite enforces, failing three phase 1 tests. Raised as a structural checkpoint; the orchestrator added the file to this plan's lease and directed the fix (drop in reverse, delete the child rows first) over the alternative of unregistering the fixture. Applied, and the plan's frontmatter `files:` list edited to declare it - that edit is left **unstaged** for the orchestrator's docs commit, since the plan file is not itself in the lease.
- [deviation] Task 3's Verify asks for "`verbatim verify` exiting non-zero naming that session and no other" and then "exiting 0 with an empty stdout". Exit codes are process-level and `CARGO_BIN_EXE_verbatim` is defined only for integration tests of the package that declares the bin, which `verbatim-core` is not and does not depend on - `tests/verify.rs` says so in its own module doc, and `crates/verbatim/tests/cli.rs` is not in this plan's lease. Asserted the same fact one layer down, where `verify.rs` deliberately put it: `Report::render()` is documented as "the exact text `verbatim verify` writes to stdout", so the tests assert `!report.is_ok()` with the diverged session named and no other, then `report.is_ok()` with `render() == ""`. The `is_ok()`-to-exit-code mapping is already covered by the two phase 1 tests in `cli.rs`, which still pass untouched.

Open items:
- Both phase 1 open items this phase was asked to carry are closed by task 4: the crash harness now kills an append/resume pass, and the watermark invariant asserts the record-boundary property rather than only containment.
- `crates/verbatim-core/src/store/schema.rs` already documents that `DERIVED_TABLES` is in creation order "because `reindex` drops in reverse". `tests/reindex.rs` was the one reader that did not, and `crates/verbatim/tests/cli.rs` and `tests/compaction.rs` each carry their own copy of the reverse-drop loop. Three copies of the same four lines; a `testkit::drop_derived(&conn)` would leave one. Out of scope here (`testkit.rs` is shared with PLAN-1 and PLAN-2, both landed).
- `ingest::run`, the single-file entry point, still returns `Error::TranscriptDiverged` to its caller and sets no flag - which is the documented split (`ingest/pass.rs`: "a caller that named one file deserves the error it asked about") and matches task 3's Action, which puts the signal in the pass. Worth naming because it means a user who runs `verbatim ingest <file>` on a shortened transcript sees the error but leaves no mark `verify` can report; only a tree pass flags it.
- `flag_divergence` can only mark a session that has a `session_meta` row and a UTF-8 path. Both hold for every real divergence (the flag is raised after `path_key` succeeded and after a prior pass committed), so this is not a gap, but it does mean the variant match in `pass.rs` is defence in depth rather than the only thing standing between a wrong failure and a wrong flag - `another_per_file_failure_flags_no_session` is the test that actually holds it.

## Checkpoint

Task 5 is implemented, committed and verified as far as this machine's reach
goes. The plan marks its Verify human-verify, and the half that needs the
private tree has not been run: `VERBATIM_TEST_CORPUS` was never set against
`/data/claude/.claude`, so no measurement was taken against John's real
transcripts.

**What was verified without it.** `cargo test --workspace` is green (26 test
binaries), `cargo clippy --all-targets -- -D warnings` is clean, `cargo fmt
--check` is clean. The skip path prints its reason and passes. The whole test
body was dry-run against a synthetic 133-transcript tree built in the
scratchpad, which exercises every branch it has: sidecars at both real depths,
a transcript holding no complete line (excused by name, as designed), a
`journal.jsonl` at project depth and another beside a sidecar, an
`agent-*.meta.json`, a `tool-results/` directory and a `workflows/wf_*.json` -
none of which became sessions - plus a non-zero `continues_from` count with
zero self-links, a 15-type histogram, zero unparseable lines and a timed
forced rebuild. The archived-every-transcript assertion was mutation-checked:
counting journals as transcripts fails it with the 11 paths named.

**What the human run adds that nothing else can.** Scale. 2,077 `.jsonl` files
against 7 fixtures; the 41 sidecars at depth 6 that a synthetic tree only has
because this test put them there; the ~988 MB rebuild time D-24 asks to be
measured; and any record shape the parser has never seen. It also costs a
temporary store of roughly the corpus's compressed size and several minutes of
wall time, which is the other reason it is not mine to start.

**The one command.**

```
VERBATIM_TEST_CORPUS=/data/claude/.claude cargo test --workspace --test corpus -- --nocapture
```

Expected: exit 0, with the session count, the before/after intersection size,
any excused path, the per-record-type histogram, the two `continues_from`
counts and the pass and rebuild wall times printed. Task 5's Verify also
records the numbers to sanity-check against - CONTEXT measured 2,075 `.jsonl`
files, 1,253 at project depth and 818 sidecars, and the tree has grown since,
so the printed counts should be at or above those and are deliberately pinned
to no literal.

**Working tree.** Everything is committed (1757cef..9fa9b8e). Two files are
left unstaged on purpose: `.planning/phases/1/CONTEXT.md`, which holds an
unrelated edit of John's, and `.planning/phases/2/PLAN-3.md`, whose `files:`
list now declares `crates/verbatim-core/tests/reindex.rs` - the orchestrator's
docs commit owns that one, since the plan file is not in its own lease.
