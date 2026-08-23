---
phase: 2
status: complete
completed: 2026-08-12
---

# Phase 2: Ingest At Scale - Summary

A tree pass that walks the real 1,896-file transcript tree under a single lock,
keys every session by file identity and every project by resolved `cwd`, honors
exclusions before it opens anything, records compaction boundaries and
divergence, and writes one `runs` row per pass that `verbatim status` reads back.

## What shipped

- Verbatim's own config - `crates/verbatim-core/src/config.rs`: `verbatim.toml`
  under `VERBATIM_CONFIG_DIR`, transcript roots, and the two-entry-point
  exclusion predicate (pre-open on the encoded directory name, exact on a real
  path)
- Unbounded transcript discovery - `crates/verbatim-core/src/discover.rs`:
  `<uuid>.jsonl` at project depth and `agent-*.jsonl` at any depth, with a
  counted-open log AC5 reads off
- The tree pass - `crates/verbatim-core/src/ingest/pass.rs`: one lock, one
  store, per-file transactions, per-file failures recorded and skipped
- Recovery before discovery - `crates/verbatim-core/src/recover.rs`: derived
  rebuild plus a watermark sweep, under the lock, before anything is read
- Project identity - `crates/verbatim-core/src/project.rs`: `cwd` resolved
  through `git rev-parse --show-toplevel` with a 2 s budget and a per-`cwd`
  memo, worktrees folded into their parent repo with the pre-fold path kept
- Lineage - `crates/verbatim-core/src/lineage.rs`: `continues_from` with
  self-link rejection, and a sidecar's parent read from its path
- Compaction boundaries - `parse/record.rs` + `derive.rs`: `compactMetadata`
  lifted out of the line by a structural walk and stored verbatim (D-08), as a
  derived row a rebuild reproduces from the blob alone
- Divergence - a transcript that no longer matches its archive is skipped,
  flagged, and named by `verbatim verify`
- `verbatim status` - `crates/verbatim/src/cmd/status.rs`: store size, session
  and turn counts, watermark coverage, and the last run with its error in full
- The corpus test - `crates/verbatim/tests/corpus.rs`: one real pass, gated on
  `VERBATIM_TEST_CORPUS`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | a292f0c | Schema for the phase: three `session_meta` columns, two `runs` columns, `compaction_boundaries`, `DERIVED_SCHEMA` 1 -> 2, additive bring-forward |
| 1 | 2 | f389fcf | Config: roots, exclusions, `verbatim.toml` only |
| 1 | 3 | b15c117 | Discovery at any depth, root canonicalized once, counted opens |
| 1 | 4 | 28649a0 | The tree pass; one bad file does not stop it |
| 1 | 5 | 3984a7c | Recovery at the top of every run |
| 1 | 6 | 87af709 | One `runs` row per pass, including the pass that died |
| 1 | 7 | 3dc1f83 | `verbatim status` |
| 1 | gate | b93aa89 | Bound the rebuild to one session blob at a time |
| 2 | 1 | 6934dbe | A session's project from its first `cwd`, resolved once |
| 2 | 2 | fc9d432 | Worktrees fold into their parent repo, both keys kept |
| 2 | 3 | d21b911 | A session never continues from itself |
| 2 | 4 | 520ac71 | A sidecar's parent comes from its path |
| 2 | 5 | 6e83d10 | The sidecar's `agent-*.meta.json`, stored unparsed |
| 2 | 6 | 1cdc9fd | Exclusion holds on the read path too |
| 2 | gate | 02592b5 | A git fold keeps its pre-image, like the worktree fold |
| 2 | gate | 6632e7c | Resolve an ambiguous exclusion against the filesystem |
| 2 | gate | e4a0a38 | A live prefix with a dead leaf is unresolved, not a negative |
| 3 | 1 | 1757cef | The parser sees a subtype and keeps compaction metadata verbatim |
| 3 | 2 | 83c0a02 | A compaction boundary becomes a row through the derive seam |
| 3 | 3 | b294ec9 | A transcript shorter than its watermark is skipped and flagged |
| 3 | 4 | 04680b1 | Kill a tree pass mid-walk, on a first pass and on an append |
| 3 | 5 | 9fa9b8e | One real pass over the private corpus, measured |
| 3 | gate | 564b900 | Time the rebuild that actually rebuilds |
| 3 | gate | 9967f8c | Clear the divergence flag on the bytes, not on the length |

## Deviations

- [deviation] PLAN-3 task 1's Verify asserts `session-basic.jsonl` yields no
  subtype; that phase 1 fixture already carries `subtype:
  "local_command_output"` on a `system` record, as real transcripts do. The
  criterion's intent was asserted instead: no fixture but the compacted one
  yields compaction metadata or the `compact_boundary` subtype. (1757cef)
- [deviation] PLAN-3 task 2 needed `crates/verbatim-core/tests/reindex.rs`,
  outside its lease: with a boundary row present, that helper's creation-order
  `DROP TABLE` and bare `DELETE FROM turns` both violate the foreign key the
  bundled SQLite enforces. Raised as a structural checkpoint; the file was added
  to the lease and the fix directed (drop in reverse, delete child rows first)
  over unregistering the fixture. (83c0a02)
- [deviation] PLAN-3 task 3's Verify asks for exit codes, but
  `CARGO_BIN_EXE_verbatim` is defined only for integration tests of the package
  declaring the bin, which `verbatim-core` is not. Asserted one layer down on
  `Report::render()`, documented as the exact text `verbatim verify` writes.
  (b294ec9)

## Open items

Four risk-surface gates fired; every blocker and high was fixed and confirmed
closed. What follows is what those reviews raised below that bar, plus what the
executors carried.

- `bring_forward` (`store/open.rs`) reads `PRAGMA table_info` outside the
  transaction and runs `ALTER TABLE ADD COLUMN` inside it, on every
  `Store::open` including the lock-free read commands. Two processes upgrading
  one phase-1 store can race; the loser exits 1, including an ingest whose whole
  pass is then lost.
- `record_run`'s single-file (`RunRow::PerFile`) path omits `files_committed` /
  `files_failed`, so a single-file ingest reports `0 committed` in `verbatim
  status`.
- A session archived before 02592b5 keeps `project_pre_worktree = NULL`;
  `session_meta` is not derived, so no rebuild backfills it. D-23's retroactive
  promise does not hold for existing stores without a re-ingest.
- `verify`'s divergence message is length-only and now also fires for a
  content-rewritten transcript, telling the user a file is "shorter than the
  bytes the archive holds" when it is longer. The flag is one boolean; the
  variant that distinguishes them does not reach `verify`. This state never
  clears on its own.
- `prefix_matches` runs after `read_tail`, so a file swapped between the two
  reads could have its prefix validated and a different file's tail appended.
  Narrow, and closable by reading head and tail from one handle.
- `parse/record.rs`: `raw_field` walks bytes while `contains_key` reads serde's
  decoded view, so a key written with a JSON escape stores `metadata` NULL
  silently, and a duplicated `compactMetadata` key stores the first value while
  every other JSON reader sees the last. A record past serde_json's 128-level
  nesting limit is demoted to a fieldless `Record` with no diagnostic.
- The boundary signal is taken from the untrusted `subtype` with no cross-check
  against `type`, so a `user` record carrying that subtype gets a boundary row,
  and a real boundary missing `uuid` or `timestamp` gets none. The corpus holds
  1,494 occurrences of the literal in message text against one real boundary.
- The crash harness's `Snapshot` excludes `compaction_boundaries`,
  `session_meta`, `entities` and `paths`, and no killed fixture carries a
  boundary - so AC6 proves nothing about the row this phase adds.
- The forced rebuild measures 50.4 s over the real corpus, inside the ingest
  lock, on the first hook-spawned pass after a `DERIVED_SCHEMA` bump.
- `verbatim status` now loads `Config`, so a `verbatim.toml` that does not parse
  fails a read command (exit 1) where it previously succeeded.
- `crates/verbatim/Cargo.toml` has no `rusqlite`, so `cmd/status.rs` reaches
  SQLite through `Store::conn` with a local error helper. A later phase wanting
  richer read commands needs that dependency.
- Three copies of the same four-line reverse-drop loop exist (`tests/reindex.rs`,
  `crates/verbatim/tests/cli.rs`, `tests/compaction.rs`); a
  `testkit::drop_derived` would leave one.
- `.planning/ROADMAP.md` still lists ING-07 under phase 2 after its deferral to
  phase 8, needing a `/cad-phase` edit.

Both phase 1 open items this phase carried are closed by PLAN-3 task 4: the
crash harness now kills an append/resume pass, and the watermark invariant
asserts the record-boundary property rather than only containment.

## UAT (2026-08-12)

`/cad-verify 2` passed all 10 items. The deep verifier auto-verified the eight
acceptance items and raised two gaps, both fixed and retested.

- `a556897` closed the two exclusion bypasses this list carried: the single-file
  `ingest::run` now loads the config and refuses an excluded transcript before
  the lock, and exclusion strings are normalized once so the pre-open and
  read-side predicates cannot disagree about a spelling. The risk-surface gate
  fired twice on that fix and its two rounds found four `high` findings, all
  fixed: a symlinked transcript reaching into an excluded project (twice, once
  through the encoded directory name and once through the project's own path),
  `..` and `~` spellings that matched nothing, and plain relative exclusions.
- `9e078d0` corrected ING-06 and ROADMAP criterion 4 to D-08's split. The
  behaviour was never the gap.

What those two review rounds raised below the blocking bar, still open:

- `ingest::run` now calls `Config::load()`, which errors when
  `VERBATIM_CONFIG_DIR`, `XDG_CONFIG_HOME` and `HOME` are all unset - so
  `verbatim ingest <path>`, the shape phase 4's hooks call, exits non-zero in a
  stripped environment (systemd unit, cron, scrubbed container) where it
  previously succeeded. Every test reaches the new `run_with` seam, which loads
  no config, so nothing covers it.
- `discover::open_transcript` records the path it was handed, not the file it
  resolves to, so the counted-open log AC5 reads cannot see a read that reached
  its target through a link. `path.canonicalize().unwrap_or(path)` would make
  the instrument answer the question AC5 asks.
- `config::is_filesystem_root` requires `RootDir` as the FIRST component, which
  is right for `/` and wrong for a Windows drive or UNC root: `C:\` is
  `[Prefix, RootDir]`, so it answers false, and the encoded extension rule
  cannot reach it either - the two predicates disagree there, on a stated
  target platform.
- A `~`-prefixed exclusion is silently dropped when `HOME` is unset, which
  contradicts the module's own "unresolvable means excluded" rule.
  `Config::load_from` returns `Result` and could refuse instead.
- `Config::exclusions()` is documented as "exactly as configured" and now
  returns the normalized rewrite, omitting entries `normalize` dropped, so
  `verbatim status` reports a list that is not what the user typed. That is
  also the only channel that could tell a user an exclusion line was malformed,
  and it says nothing.
- `..` is resolved lexically (deliberately - the excluded directory is
  frequently deleted, D-06), so it names a different subtree than the kernel
  whenever a popped component is a symlink, with no report when the result
  names a path that exists nowhere.

## Goal check

The phase delivers its goal, and the real-corpus run is the evidence rather than
an inference: `VERBATIM_TEST_CORPUS=/data/claude/.claude cargo test --workspace
--test corpus` exits 0 having archived 2,129 transcripts and 309,060 records
across 16 record types with zero unparseable lines, which is AC1's "every record
parses, zero panics" measured against the tree the product exists for.
Subagent sidecars are reached at both real depths - `discover.rs`'s explicit
stack is unbounded and `the_depth_six_sidecar_is_reached` pins the depth-6 case
that a bounded walk drops. AC2's linkage is non-zero at 188 `continues_from`
links with 0 self-linked, though that is 8.8% of sessions against CONTEXT's
measured 1.2% of files, and nobody has reconciled the two denominators - worth
one look in phase 3 before anything depends on the rate. AC3, AC4 and AC6 each
have a test that fails against the pre-change code: worktree folding and the
literal-`-` project in `tests/project.rs`, the append-a-boundary case in
`tests/compaction.rs` asserting every completed blob block byte-identical, and
24 SIGKILLs in `crates/verbatim/tests/crash.rs` each resumed to a byte-identical
reference store.

AC5 is where the honest qualification sits. Zero opens under an excluded prefix
holds, and `tests/pass.rs` reads it off the counted-open log rather than
inferring it from row counts. But D-09 as written asked for two things that
cannot both hold - a segment-boundary match on the encoded name, and sparing a
hyphenated sibling - because a child and a sibling encode identically. That is
now resolved against the filesystem, at the cost that an unresolvable case
(a deleted or unreadable excluded directory) excludes rather than reads, so a
project can go unarchived without being named. The direction was chosen
deliberately and the decision needs amending in CONTEXT.

The largest gap is not in what was built but in what is warranted about it: the
open items list two silent-data-loss paths in `parse/record.rs` that no test
covers, and a crash-convergence snapshot that does not include the derived row
this phase added. None of them breaks an acceptance criterion, and all of them
are the kind of thing `/cad-verify` should probe rather than take on trust.
