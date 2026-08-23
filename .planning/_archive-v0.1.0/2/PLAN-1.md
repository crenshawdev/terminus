---
phase: 2
plan: 1
requirements:
  - ING-03
  - ING-04
  - ING-08
  - ING-09
files:
  - Cargo.toml
  - Cargo.lock
  - crates/verbatim-core/Cargo.toml
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/error.rs
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/discover.rs
  - crates/verbatim-core/src/recover.rs
  - crates/verbatim-core/src/store/schema.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/src/reindex.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/config.rs
  - crates/verbatim-core/tests/discover.rs
  - crates/verbatim-core/tests/pass.rs
  - crates/verbatim-core/tests/recover.rs
  - crates/verbatim-core/tests/schema.rs
  - crates/verbatim-core/tests/ingest.rs
  - crates/verbatim-core/tests/fixtures.rs
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/ingest.rs
  - crates/verbatim/src/cmd/status.rs
  - crates/verbatim/tests/status.rs
  - crates/verbatim/tests/cli.rs
  - tests/fixtures/README.md
  - tests/fixtures/subagents/workflows/wf_demo/agent-deep.jsonl
---

# Phase 2: Ingest At Scale - Plan 1 of 3 (the tree pass)

**SEQUENTIAL: PLAN-1 must complete before PLAN-2, and PLAN-2 before PLAN-3.**
All three write `crates/verbatim-core/src/ingest/mod.rs`,
`crates/verbatim-core/src/ingest/pass.rs`, `crates/verbatim-core/src/lib.rs`,
`crates/verbatim-core/src/testkit.rs`, `crates/verbatim-core/tests/fixtures.rs`
and `tests/fixtures/README.md`. Do not run them in parallel.

## Goal

Ingest that runs against the real 1,896-file transcript tree: finding every
session including subagent sidecars, keying projects correctly, honoring
exclusions, and reporting what it did. This plan builds the spine that makes
that possible - the config that names the roots and the exclusions, the walk
that finds every transcript beneath them without opening an excluded one, the
pass that survives a damaged file, and the `runs` row plus `verbatim status`
that say what happened.

## Must be true when done

- `verbatim ingest` with no argument walks every configured transcript root and
  archives every `<uuid>.jsonl` at project depth and every `agent-*.jsonl` at
  any depth beneath it, including the ones under `subagents/workflows/wf_*/`,
  and archives no `journal.jsonl`, no `.meta.json` and nothing under a
  `tool-results/` directory.
- A tree containing one damaged transcript - short of its watermark, or
  archived with no `session_meta` row - still ingests every other file in the
  tree, and the pass exits reporting that file's error.
- With a project excluded in the config, a whole pass over a tree containing it
  records zero opens of any file beneath that project directory and adds no row
  to any table, while a sibling project directory whose encoded name extends the
  excluded one is ingested normally.
- Every run performs its recovery before it walks anything: a store carrying a
  watermark ahead of the bytes its committed blob holds is brought back to the
  blob, with no repair command and no external supervisor.
- One pass writes exactly one `runs` row whatever the file count, and a pass
  that dies still leaves one carrying its error.
- `verbatim status` prints store size, session and turn counts, watermark
  coverage, and the last run with its error, all read out of the store.

## Context

Locked and binding here: D-15 (verbatim's own config path, roots as a list,
`CLAUDE_CONFIG_DIR` as a single-directory override), D-16 (filename-pattern
discovery to unbounded depth), D-17 (canonicalize roots once, join walked
entries onto them), D-18 (sequential, single-threaded; no rayon), D-09 and D-22
(exclusion decided on the encoded project directory name before any file in it
is opened), D-10 (one `runs` row per pass), D-11 (per-file transactions; the
pass is not one transaction), D-12 (a per-file failure is recorded and skipped),
D-14 (a failed pass writes its `runs` row in a second transaction after the
rollback), D-24 (recovery is `reindex::open_up_to_date` plus a bounded
watermark-versus-blob sweep, exposed as one callable).

Out of scope for this plan: project identity from `cwd`, session lineage and
the read-path half of exclusion (PLAN-2); the compaction boundary, the
short-transcript flag on `verify`, the crash harness and the real-corpus pass
(PLAN-3). Out of scope for the phase: ING-07 capture mode (deferred to phase 8),
Rust-side text expansion and entity extraction (phase 3), hooks and bounded
parallelism (phase 4), retention (phase 8).

Existing code this plan builds on: `ingest::run` and `ingest::ingest_locked` in
`crates/verbatim-core/src/ingest/mod.rs`, `ingest::lock::try_acquire`,
`reindex::open_up_to_date`, `Store::open` and `data_dir()` in
`crates/verbatim-core/src/store/open.rs`, `schema::CREATE_SQL`,
`schema::TABLES` and `schema::DERIVED_TABLES`, and the `cmd::Failure` exit-code
split in `crates/verbatim/src/cmd/mod.rs`.

Note on test invocation: the phase 1 open item stands - `cargo test -p
verbatim-core` runs none of the `#![cfg(feature = "testkit")]` test files and
exits 0. Every Verify below uses `cargo test --workspace` for that reason.

## Tasks

### Task 1: Give the store the shape the rest of the phase writes into

- **Files:** crates/verbatim-core/src/store/schema.rs, crates/verbatim-core/src/store/open.rs, crates/verbatim-core/src/reindex.rs, crates/verbatim-core/tests/schema.rs
- **Action:** Extend `schema::CREATE_SQL` with everything phase 2 fills, so the
  store shape moves once rather than three times. On `session_meta`: a column
  holding the bytes of a sidecar's `agent-*.meta.json` opaquely and unparsed
  (D-04), a column marking a session whose transcript on disk is shorter than
  its stored watermark so `verbatim verify` can report the divergence (D-13),
  and a column holding the project key a session had before worktree mapping,
  beside the existing `project` column, so ING-05's "both keys stored" is
  satisfiable. On `runs`: columns letting one row distinguish files walked from
  files that committed and files that failed, since `files_seen` alone cannot
  carry that once a row covers a whole pass (D-10). And a new derived table
  holding one compaction boundary per turn - the turn id and that record's
  `compactMetadata` bytes stored verbatim, no dropped-turn set (D-08, D-21) -
  added to `schema::TABLES` and to `schema::DERIVED_TABLES` after `turns`, since
  `reindex` drops that list in reverse and the boundary rows reference turn ids.
  Nothing writes any of this in this plan; PLAN-2 and PLAN-3 do. Bump
  `DERIVED_SCHEMA` in `crates/verbatim-core/src/store/open.rs` and leave
  `ARCHIVE_FORMAT` at 1: the bytes in `sessions` do not change meaning, and the
  archive table never migrates. Every statement in `CREATE_SQL` is
  `IF NOT EXISTS`, so on an existing store it creates the new table and adds no
  column to `session_meta` or `runs` - add an additive bring-forward that reads
  each table's current column list and issues `ALTER TABLE ... ADD COLUMN` only
  for the ones missing. Run it in `Store::open`, in its own transaction, on
  every open of an existing store - NOT gated on `Store::rebuild_required` and
  NOT inside `reindex`. That gating is the obvious placement and it is wrong:
  `Store::open` executes `CREATE_SQL` only when the store is `StoreState::Fresh`
  (`crates/verbatim-core/src/store/open.rs:99`), and `reindex::open_up_to_date`
  is the only caller that acts on `rebuild_required`, while `verbatim status`,
  `verbatim verify` and every phase 3 reader open through `Store::open` alone.
  A user who upgrades the binary and runs `status` before any ingest would hit
  `no such column` on a healthy store, and `status` cannot fix that by calling
  `open_up_to_date` instead - that runs a destructive derived-table rebuild
  outside the ingest lock, the exact defect gate fix `3e5d9ff` closed. Adding a
  column is safe where rebuilding derived tables is not, which is why the two
  split here: `ALTER TABLE ADD COLUMN` on every open, the derived rebuild left
  exactly where it is. The bring-forward is idempotent by construction (it adds
  only missing columns), reads the column list from `PRAGMA table_info`, and
  must run before `Store` is handed to any caller. Never
  drop, recreate or rewrite `sessions` or `session_meta`; adding a column is the
  only change either is allowed. `crates/verbatim-core/src/reindex.rs` is in
  this task's lease because `DERIVED_TABLES` gains the boundary table and
  `reindex`'s drop order (reverse of that list, `reindex.rs:50-118`) must stay
  correct; the bring-forward itself does not live there. Update
  `a_fresh_store_carries_exactly_the_phase_one_tables` in
  `crates/verbatim-core/tests/schema.rs` to the phase 2 table set and rename it
  to match.
- **Verify:** `cargo test --workspace --test schema --test reindex --test
  store_open --test ingest` passes, and two new tests in
  `crates/verbatim-core/tests/schema.rs`: one that opens a store, ingests a
  fixture, drops each phase 2 column back off `session_meta` and `runs`, sets
  `meta.derived_schema` back to 1, reopens through `reindex::open_up_to_date`
  and finds every phase 2 column present again with `sessions.session_key`,
  `sessions.blob` and `session_meta.checksum` byte-identical to before the drop;
  and one that does the same drop and then reopens through plain `Store::open`
  WITHOUT going near `reindex`, finds every phase 2 column present again, and
  finds `meta.derived_schema` still at 1 - the columns come back, the derived
  rebuild does not fire, and a `SELECT` naming every phase 2 column succeeds.

### Task 2: Verbatim's own config: transcript roots and project exclusions

- **Files:** Cargo.toml, Cargo.lock, crates/verbatim-core/Cargo.toml, crates/verbatim-core/src/config.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/src/error.rs, crates/verbatim-core/tests/config.rs
- **Action:** Add a config loader as a new module, declared in
  `crates/verbatim-core/src/lib.rs` beside the existing `store` and `ingest`
  modules. Use the `toml` crate as a workspace dependency with default features
  off and only its parsing and serde integration enabled - that is the choice
  this plan makes rather than hand-rolling a config format, because serde is
  already in the tree and a bespoke parser is a correctness liability, and the
  writer half is dead weight on a cold-start-sensitive binary. Resolve the
  config directory the way `data_dir()` in
  `crates/verbatim-core/src/store/open.rs` resolves the data directory: an
  environment override first, then the per-platform location
  (`$XDG_CONFIG_HOME/verbatim` on Linux, the same convention on macOS,
  `%APPDATA%\verbatim` on Windows). The config file inside it is
  `verbatim.toml`, and the loader must never read `config.toml` in that
  directory even when present - D-15 is binding and that exact file exists on
  the development machine holding the legacy tool's `base_dir =
  "/data/verbatim"`, whose data import PROJECT.md defers; adopting it would
  point the new store at the deferred legacy directory. A missing config file is
  not an error: it yields the defaults. The config carries transcript roots as a
  list of Claude config directories, defaulting to `~/.claude`, with
  `CLAUDE_CONFIG_DIR` when set and non-empty replacing that default with the
  single directory it names; the tree walked for each root is its `projects`
  subdirectory. It also carries a list of excluded project paths. Expose the
  exclusion test as two entry points over one configured list, because the two
  callers have different information: one takes an encoded project directory
  name and answers before anything is opened, and one takes a real filesystem
  path and answers on the read side. The encoded test applies the
  `[^A-Za-z0-9] -> '-'` rule to the configured path and requires the encoded
  directory name either to equal it, or to equal it followed by the fixed
  literal `--claude-worktrees-` and any further text. D-09 requires
  `-data-projects-cadence` not to match `-data-projects-cadence-research`, and
  D-07 proved the encoding is lossy in exactly that spot, so any
  extension-tolerant rule cannot tell a child directory from a hyphenated
  sibling and would silently exclude a project the user never named. The
  worktree clause is the one exception that is safe, and it is not optional:
  D-06 folds a `cwd` of the form `<repo>/.claude/worktrees/<name>` into
  `<repo>`, and that path shape encodes literally, so `/data/code/cadence`'s
  worktrees sit in directories named `-data-code-cadence--claude-worktrees-*`.
  Without the clause, excluding `/data/code/cadence` walks and archives every
  one of its worktree sessions - 15 of 69 real project directories have this
  shape - and PLAN-2 task 6 then hides them at read time, which is read-then-
  filter and is exactly the ING-08 failure PROJECT.md cites claude-mem for. The
  clause reintroduces no ambiguity: `--claude-worktrees-` is a fixed literal
  segment, not an open-ended prefix extension. The path
  test is an ordinary prefix match on `/` segment boundaries against a
  canonicalized path, which is unambiguous and is where subtree exclusion
  actually works. Case-fold the comparison on Windows and macOS. Add an error
  variant for a config file that exists and does not parse, naming the file and
  the parse position, rather than silently falling back to defaults.
- **Verify:** `cargo test --workspace --test config` passes, covering: a missing
  config file yields the default single root; `CLAUDE_CONFIG_DIR` set to a
  temporary directory replaces that default and an empty value does not; a
  config file naming two roots yields both in order; the encoded test says yes
  to `-data-projects-cadence` for an exclusion of `/data/projects/cadence`, yes
  to `-data-projects-cadence--claude-worktrees-agent-a33a`, and no to
  `-data-projects-cadence-research`; the path test says yes to
  `/data/projects/cadence/sub` for the same exclusion and no to
  `/data/projects/cadence-research`; and a loader pointed at a directory
  containing only a `config.toml` with `base_dir = "/data/verbatim"` returns the
  defaults and reports no root under `/data/verbatim`.

### Task 3: Discover every transcript beneath the roots, at any depth

- **Files:** crates/verbatim-core/src/discover.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/src/testkit.rs, crates/verbatim-core/tests/discover.rs, crates/verbatim-core/tests/fixtures.rs, tests/fixtures/README.md, tests/fixtures/subagents/workflows/wf_demo/agent-deep.jsonl
- **Action:** Add a discovery module that takes the roots a loaded config
  resolved and yields transcript paths. Canonicalize each root once and join
  every walked entry onto the canonical root rather than calling `canonicalize`
  per file (D-17): there are zero symlinks inside the tree and the root itself
  is a two-hop symlink chain on the development machine, so root canonicalization
  alone yields the same session keys `ingest::run` produces today, at one
  syscall instead of two thousand. Recurse with `std::fs::read_dir` and no new
  dependency - D-18 keeps rayon, walkdir and glob out of this phase and the walk
  is sequential and single-threaded. Descend into dot-directories. Filter by
  filename and never by record shape (D-16): accept a file whose name is a UUID
  with a `.jsonl` extension when it sits directly inside a project directory,
  and accept a file whose name begins with `agent-` and ends with `.jsonl` at
  any depth below that. Everything else is not a transcript, which is what keeps
  the four `journal.jsonl` files, the 816 `agent-*.meta.json` files, the
  `workflows/wf_*.json` files and the `tool-results/` directories of `.txt` and
  `.md` out without opening any of them. Sidecars sit at depth 4 and also at
  depth 6 under `subagents/workflows/wf_*/`, so the recursion must be unbounded;
  a depth limit silently drops 41 real files. Apply the config's encoded
  exclusion test to each project directory name before descending into it, so an
  excluded project is never even listed (D-22): all 822 real sidecar files sit
  beneath `<project>/<sessionId>/subagents/`, so a directory-level skip covers
  them too. Yield results in a deterministic order - sort each directory's
  entries - so two passes over one tree walk it identically. An unreadable
  directory is reported and skipped, never fatal. Also add to this module the
  one counted way to open a transcript file for reading, recording each opened
  path into a testkit-gated log that tests can read and reset; the shipped
  binary takes no branch for it. It records PATHS and not a bare count, because
  a scalar cannot answer the question AC5 asks: "zero opens of any file under
  that directory" is satisfied by a total that a pass could reach by opening the
  excluded files and skipping an equal number elsewhere. Tests query the log by
  path prefix, so AC5's assertion is attributable rather than aggregate; a count
  is derivable from the log where one is wanted. Add a fixture at
  `tests/fixtures/subagents/workflows/wf_demo/agent-deep.jsonl` for the depth-6
  sidecar case, register it in `testkit::TRANSCRIPT_FIXTURES`, document it in
  `tests/fixtures/README.md`, and keep `testkit::UNIQUE_TOKEN` out of its bytes -
  `unique_token_appears_exactly_once_across_the_corpus` in
  `crates/verbatim-core/tests/fixtures.rs` asserts that token appears exactly
  once across the whole fixture set, and the file must end with a newline like
  every fixture but `session-truncated.jsonl`.
- **Verify:** `cargo test --workspace --test discover --test fixtures` passes,
  over a temporary tree built to the real layout: a project directory holding
  two `<uuid>.jsonl` files, a `<sessionId>/subagents/agent-a.jsonl`, a
  `<sessionId>/subagents/workflows/wf_x/agent-deep.jsonl`, a
  `<sessionId>/subagents/workflows/wf_x/journal.jsonl`, an
  `agent-a.meta.json`, a `wf_x.json`, a `tool-results/out.txt` and a
  `not-a-uuid.jsonl` at project depth. Discovery returns exactly the two
  top-level transcripts and the two `agent-*.jsonl` files, in a stable order
  across two calls, every returned path beginning with the canonical root even
  when the root is reached through a symlink; and with the project directory's
  encoded name excluded in the config it returns nothing from that directory
  while a sibling directory named by extending the excluded name with a further
  segment is still returned in full.

### Task 4: One run walks the tree, and one bad file does not stop it

- **Files:** crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/lib.rs, crates/verbatim/src/cmd/ingest.rs, crates/verbatim/src/main.rs, crates/verbatim-core/tests/pass.rs, crates/verbatim-core/tests/ingest.rs, crates/verbatim/tests/cli.rs
- **Action:** Add a pass module under `ingest` that loads the config, takes the
  ingest lock through the existing `ingest::lock::try_acquire` exactly once for
  the whole pass, opens the store once, walks the discovered transcripts and
  calls the existing `ingest::ingest_locked` per file. The pass is not one
  transaction (D-11): each file keeps its own transaction and its own watermark,
  which is the granularity phase 1's crash harness already proved atomic and the
  granularity that bounds in-flight loss to one file (real files are p50 290 KB,
  p99 3.1 MB). Do not hold a write transaction across the walk - that stops MCP
  readers, which is the reason redb was rejected. Every per-file failure is
  recorded against that file's path and skipped, never propagated (D-12): both
  of phase 1's hard-error paths reach this loop - `Existing::read` returning
  `Error::NotAStore` for a session archived without a `session_meta` row, and
  `read_tail` returning an IO error when the file is shorter than its watermark -
  and either one stopping the pass would let a single damaged session wedge
  every future ingest of every other transcript, the exact failure gate fix
  `3e5d9ff` closed for `reindex`. Keep `ingest::run`'s single-file behaviour and
  its refusals unchanged, so the phase 1 tests that assert them still hold; the
  skip lives in the pass, not in `ingest_locked`. Return a summary carrying files
  walked, files committed, bytes read, turns added and the per-file failures with
  their reasons. Route the transcript open in `read_tail` through the counted
  open helper task 3 added, so the pass's opens are the number AC5 measures.
  Wire the CLI: `verbatim ingest` with no argument runs the pass over the
  configured roots, `verbatim ingest <path.jsonl>` keeps ingesting that one file
  through the existing `ingest::run`, and `crates/verbatim/src/main.rs`'s
  `USAGE` describes both. A pass that skipped a file still exits 0 with the
  skipped files named on stderr - the tree was ingested - while a pass that could
  not run at all keeps `Failure::Operational`.

  Two things in `crates/verbatim/tests/cli.rs` must change with it, and neither
  is optional. First, `misuse_exits_two_with_an_empty_stdout` (cli.rs:282)
  asserts that bare `ingest` exits 2 - `cmd::ingest::parse` returns
  `Failure::Misuse` today. That is the behaviour this task deliberately
  replaces, so drop `vec!["ingest"]` from that test's misuse list (leaving
  `verify --json` and `no-such-command`, which stay misuse) and add a positive
  assertion that bare `ingest` against an isolated config exits 0. Do not leave
  it: the test is in this task's own Verify and would fail. Second, and more
  serious, the spawn helper `verbatim()` at cli.rs:58 sets only
  `VERBATIM_DATA_DIR`. Once bare `ingest` walks configured roots, that helper
  makes every CLI test spawn resolve roots from the developer's real config and
  walk the live `~/.claude` tree - 2,000+ private transcripts and ~988 MB
  ingested into a temp dir on every `cargo test` run. Extend the helper (and
  `bench()`, and the equivalent spawn helpers in
  `crates/verbatim/tests/status.rs`) to also set the config-directory override
  task 2 defines, pointing at a temporary directory that holds no
  `verbatim.toml`, and to set the transcript root to a temporary tree. No test
  process may resolve a real transcript root except PLAN-3 task 5's explicitly
  env-gated corpus test.
- **Verify:** `cargo test --workspace --test pass --test ingest --test cli
  --test lock_race` passes, including a test that builds a temporary tree of
  five transcripts, corrupts one into each of the two damaged states (its
  `session_meta` row deleted after a first ingest, and its file truncated below
  its stored watermark), runs one pass, and finds the other four sessions
  archived with their turns while the summary names exactly the two damaged
  paths and their reasons; and a test that runs the pass over a tree with one
  project excluded and finds the counted-open log holds no path beneath that
  directory and no path beneath its `--claude-worktrees-` sibling directory, and
  that no `sessions`, `session_meta`, `turns`, `turns_fts` or `watermarks` row
  exists for any of them, while the sibling project whose encoded name merely
  extends the excluded one has its rows present. Plus, in
  `crates/verbatim/tests/cli.rs`: bare `verbatim ingest` against an isolated
  config directory and a temporary transcript root exits 0, and the misuse test
  still exits 2 for `verify --json` and `no-such-command`. Confirm the
  isolation holds by asserting that a CLI test run leaves the process's resolved
  root inside the temporary tree - no test spawn may reach a real root.

### Task 5: Recovery at the top of every run

- **Files:** crates/verbatim-core/src/recover.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/tests/recover.rs
- **Action:** Expose recovery as one callable function that every ingest run
  invokes before it walks anything, with no repair command and no external
  supervisor (ING-03, D-24). It is two things and only two. First,
  `reindex::open_up_to_date`, which already runs on the ingest path and brings a
  store older than this build forward. Second, a bounded consistency sweep over
  `watermarks` against what each session's committed blob actually holds: read
  `session_meta.uncompressed_len` rather than decompressing anything, which is
  what makes the sweep a single indexed SQL pass over ~2,000 rows instead of a
  988 MB decompression, and reconcile the two states phase 1's crash invariants
  enumerate - a watermark ahead of the bytes the committed blob holds is lowered
  to those bytes so the next pass re-reads the gap, and a watermark with a
  non-zero offset naming no archived session is removed because it claims bytes
  nothing archived. Both repairs commit in one transaction and are reported in
  the return value so the pass can put them in its `runs` row. There is no
  stale-lock handling here and there must not be: the lock is an OS lock that
  dies with the process (`ingest::lock`), so there is no stale lock to recover
  from. Call it from BOTH ingest entry points, under the lock, before any
  transcript is read: from the pass module before discovery, and from
  `ingest::run` (`crates/verbatim-core/src/ingest/mod.rs:76`) before it reaches
  `ingest_locked`. ING-03 says every run, and `ingest::run` is a run - it calls
  `reindex::open_up_to_date` today and so gets the rebuild half but no watermark
  sweep. Wiring only the pass leaves `verbatim ingest <path.jsonl>` proceeding
  against a watermark ahead of what that file's committed blob holds, so
  `read_tail` reads from an offset the archive never reached and the gap is
  silently lost from the blob. One callable, two callers, and the sweep is
  cheap enough to run twice (one indexed pass over ~2,000 rows, no
  decompression).
- **Verify:** `cargo test --workspace --test recover --test pass --test ingest`
  passes, including a test that ingests a fixture, then writes the watermark for
  that session to a byte offset past its `session_meta.uncompressed_len` and
  inserts a watermark row for a path with no `sessions` row, then runs a pass and
  finds the first watermark equal to the committed length, the second row gone,
  both repairs named in the returned report, and the session's blob and checksum
  unchanged; the same test repeated against single-file `verbatim ingest
  <path.jsonl>` rather than a pass, with the same outcome; plus a test that
  either entry point over a store needing no repair changes no `watermarks` row.

### Task 6: One runs row per pass, including the pass that died

- **Files:** crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/tests/pass.rs, crates/verbatim-core/tests/ingest.rs
- **Action:** Move the `runs` row from per-file to per-pass (D-10).
  `ingest::record_run` today inserts a literal `1` for `files_seen` from inside
  the per-file transaction; reused unchanged by a tree walk it would write
  roughly 2,071 rows per hook-triggered pass and ING-09's "the last ingest run"
  would name one file rather than the pass. Keep `record_run` writing the row
  for the single-file `ingest::run` path so phase 1's assertion in
  `crates/verbatim-core/tests/ingest.rs` still holds, and have the pass write
  its own single row after the walk, in a transaction of its own, carrying files
  walked, files committed, files failed, bytes read, turns added, the duration,
  and the per-file failures in `runs.error` with each failing path and its
  reason. A pass that dies before finishing the walk writes its row in a second
  transaction after the rollback (D-14) - `record_run`'s own doc comment already
  says this is phase 2's - so that with no log file by design the failure is
  still visible. A pass that walked a tree and found nothing new still writes a
  row, because "the last ingest run" must move; only the single-file
  `Outcome::UpToDate` path keeps writing nothing.
- **Verify:** `cargo test --workspace --test pass --test ingest` passes,
  including a test that runs one pass over a tree of five transcripts and finds
  `runs` grew by exactly one row whose files-walked count is 5; a test that runs
  a pass that fails partway (the store made unwritable mid-pass, or a fault
  point) and finds one `runs` row whose `error` is non-null and names the
  failure; and a test that a pass over a tree with one damaged file exits with a
  `runs` row whose `error` names that path and whose files-committed count is
  one less than files walked.

### Task 7: `verbatim status`

- **Files:** crates/verbatim/src/cmd/status.rs, crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs, crates/verbatim/tests/status.rs
- **Action:** Add a `status` subcommand alongside the existing `ingest`,
  `verify` and `reindex` in `crates/verbatim/src/cmd/`, dispatched from
  `crates/verbatim/src/main.rs` and listed in its `USAGE`. It reports, sourced
  from the store and never from a log file (ING-09): the store path and its
  on-disk size, the session count, the turn count, how many transcripts have a
  watermark and the total bytes those watermarks cover, and the last `runs` row
  with its start time, duration, file counts, bytes and turns, and its `error`
  printed in full when non-null. It opens the store read-only through
  `Store::open` and takes no ingest lock - reading while a pass runs must work,
  which is why WAL is on. Data goes to stdout and errors to stderr, exit 0 on
  success, 1 on operational failure, 2 on misuse, and an empty store is exit 0
  with zeroes rather than an error, following the contract
  `crates/verbatim/src/cmd/mod.rs` already encodes in `Failure`. No `--json`
  flag: RCL-06's stable shapes are phase 3, and an unadvertised flag accepted
  now becomes a shape to keep. Reject any argument as misuse the way
  `no_more_arguments` in `main.rs` already does for `verify` and `reindex`.
- **Verify:** `cargo test --workspace --test status` passes, including: `status`
  on an empty data directory exits 0 and prints zero sessions; after a pass over
  a tree of five transcripts it exits 0 and prints the session count, turn count
  and a last-run line whose numbers match the `runs` row; after a pass over a
  tree containing one damaged transcript it prints that file's path and error in
  its last-run section; and `status --json` exits 2 printing nothing to stdout.

## Notes

- ING-07 (capture mode `full`/`lean`/`minimal`) is **deferred to phase 8** by
  `.planning/phases/2/CONTEXT.md`'s Scope boundary and is deliberately absent
  from all three plans. Elision at ingest contradicts D-13 (the blob holds the
  transcript bytes verbatim) and would break phase 1's
  `the_blob_reproduces_the_transcript_byte_for_byte` invariant in the very phase
  that first ingests the real corpus. `.planning/ROADMAP.md` still lists ING-07
  under phase 2 and needs a `/cad-phase` edit this workflow does not perform.
- Task 2 narrows `DESIGN-BRIEF.md:392`'s "prefix match on canonicalized paths"
  on the pre-open side only. The encoded directory name is provably lossy
  (D-07), so the zero-open test is exact equality plus the one fixed-literal
  `--claude-worktrees-` clause; a user excluding `/data/clients` therefore
  excludes the project directory for `/data/clients` and its worktrees, and not,
  before any open, the separate project directory for `/data/clients/acme`. The
  path-side test keeps full subtree semantics everywhere a real path is
  available. If the human wants subtree exclusion enforced pre-open too, the
  config needs one entry per project directory, or the ambiguity has to be
  accepted and `-data-projects-cadence-research` swept up with
  `-data-projects-cadence`. Flagged rather than chosen silently.
- Task 1's bring-forward runs in `Store::open`, not in `reindex`. The split is
  deliberate and is the one place this plan lets a read path write: adding a
  missing column is safe on any opener, rebuilding derived tables is not, and
  gating both on `rebuild_required` would leave `status` and `verify` selecting
  columns that do not exist on an upgraded store.
- Task 4 changes a shipped CLI contract: bare `verbatim ingest` stops being
  misuse (exit 2) and becomes the tree pass (exit 0). That is an intended
  behaviour change, and it obliges the test-isolation work in the same task -
  once bare `ingest` resolves configured roots, any test spawn without a config
  override walks the developer's real `~/.claude`.
- Task 1 bumps `DERIVED_SCHEMA`. D-24's scale consequence follows directly: the
  next ingest against John's existing store rebuilds every archived session from
  its blob inside the ingest lock before the walk begins. PLAN-3 measures that
  cost on the real corpus.
- The `toml` dependency is the first addition to the dependency floor since
  phase 1. It must not pull an async runtime, an HTTP client or rayon; if the
  chosen version does, hand-roll the loader instead and record the deviation.
