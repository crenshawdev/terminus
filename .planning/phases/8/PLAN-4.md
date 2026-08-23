---
phase: 8
plan: 4
requirements: [STOR-06, STOR-07]
files:
  - crates/verbatim-core/src/store/snapshot.rs
  - crates/verbatim-core/src/store/mod.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/tests/snapshot.rs
  - crates/verbatim-core/tests/store_open.rs
  - crates/verbatim-core/tests/config.rs
  - crates/verbatim/src/cmd/data.rs
  - crates/verbatim/src/cmd/ingest.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/main.rs
  - crates/verbatim/tests/datamove.rs
  - crates/verbatim/tests/hook.rs
---

# Phase 8: Retention And Lifecycle - Plan 4 (a store that copies itself, and one that can move)

## Goal

The store takes a consistent copy of itself on a rolling schedule without ever
stopping an ingest, and the whole data directory can be relocated to a path the
user names, after which every component - the hooks and the MCP server included -
reads the new location with no change to settings.json.

## Must be true when done

- A snapshot taken while an ingest pass holds the lock opens as a store, passes
  `verbatim verify` with no findings, and holds every session committed before
  that pass started and none of the uncommitted one.
- Snapshots run by default and at most once in the configured interval, and the
  configured number of most recent snapshots are kept while older ones are
  removed - so a machine that fires a hook on every prompt does not write an
  archive-sized file per prompt.
- A snapshot that fails is a note on stderr and never a failed ingest pass.
- `verbatim data move <path>` relocates `verbatim.db`, its sidecars, the `LOCK`
  file, the injection-state directory and the snapshots, writes the location
  pointer, and asks first - after warning about the SQLite-over-network-
  filesystem hazard.
- After that move, a hook invocation and an MCP `recall_search` both read the
  store at the new path, and `~/.claude/settings.json` is byte-identical to what
  it was before the move.
- The hook cold-start budget still holds: the p99 assertion in
  `crates/verbatim/tests/hook.rs` still passes with the pointer read on the path.

## Context

- D-07 fixes the mechanism: `VACUUM INTO`, never rusqlite's online backup API,
  which is behind a `backup` cargo feature this workspace does not enable.
  Measured against the live 1.10 GB store: 0.56 s, a 1.087 GB output opening
  with all 3,416 sessions and `PRAGMA integrity_check` = ok, succeeding while a
  second connection held an open `BEGIN IMMEDIATE` where plain `VACUUM` failed
  with `database is locked`.
- D-15 fixes the schedule: a timestamp row in `meta` gating the snapshot to at
  most once per interval, pruning to the last N, running OUTSIDE the ingest lock
  the way `observe::judge_new` does from `pass::run_with`; both the interval and
  the retained count are `verbatim.toml` keys.
- D-06 fixes the pointer: a plain-text file in verbatim's config directory, read
  by `store::data_dir()` BELOW `VERBATIM_DATA_DIR`, relocating the WHOLE data
  directory - which holds `LOCK`, the per-session injection state directory
  (`inject::decision::DIR_NAME`) and the WAL sidecars, not just `verbatim.db`.
  A move that copied only the database strands the undrained decision files.
- D-14: `store::open::data_dir` is the ONLY resolver, and neither the hook argv
  (`args: ["hook", <event>]`) nor the MCP registration (`args: ["mcp"]`) names a
  store path, so AC5's "no settings.json change" is already structurally true and
  nothing in `cmd/install/targets.rs` may be touched.
- D-16: `data move` WARNS about network filesystems and confirms rather than
  refusing them. Detection is Deferred - no `std` API detects a network mount on
  any of the three targets.
- D-17: `data move` is human-only like `install` and `uninstall`, and its
  two-word form follows the `observations regenerate` nested-dispatch precedent.

## Tasks

### Task 1: Take one consistent snapshot and keep only the newest few

- **Files:** crates/verbatim-core/src/store/snapshot.rs, crates/verbatim-core/src/store/mod.rs, crates/verbatim-core/tests/snapshot.rs
- **Action:** Add a `snapshot` module under `store`, declared and re-exported in
  `store/mod.rs` beside `open` and `schema`. It holds two functions and no
  schedule: one takes a snapshot of the store in a data directory, and one
  prunes a snapshot directory to the N newest. The snapshot is
  `VACUUM INTO '<path>'` issued on an ordinary connection, never
  `rusqlite::backup` - that module is gated behind a `backup` cargo feature this
  workspace does not enable, so a plan reaching for it does not compile, and
  enabling it is an unbudgeted change to the hook-path binary. It writes into a
  `snapshots` subdirectory of the data directory, so a relocation of the data
  directory carries the snapshots with it and `uninstall --purge` still removes
  everything it is asked to. The file name carries a UTC timestamp in the same
  fixed-width ISO-8601 shape every stored timestamp in this store uses, so
  ordering the files by name is ordering them by time and the prune needs no
  filesystem mtime. Write to a temporary name in that directory and rename into
  place, so a killed process leaves no half-written file for the prune to count
  as a snapshot. `VACUUM INTO` will not overwrite an existing file, which is one
  more reason the name carries the instant. The prune deletes the oldest files
  past the retained count and nothing else in the directory. Neither function
  takes the ingest lock: the whole point is a consistent copy without stopping
  ingest, and `VACUUM INTO` already reads a single consistent snapshot of the
  database through WAL.
- **Verify:** `cargo test -p verbatim-core --test snapshot` passes with new
  cases showing (a) with the ingest lock held by another handle AND a second
  connection holding an open `BEGIN IMMEDIATE` that has inserted an uncommitted
  session, the snapshot succeeds, opens as a store, returns no findings from
  `verify::verify`, and holds every session committed before that transaction
  began and not the uncommitted one, (b) `PRAGMA integrity_check` on the snapshot
  returns ok, (c) pruning to 3 over 5 snapshot files leaves the 3 newest by name
  and removes the other 2, and (d) an unrelated file in the snapshots directory
  is not removed. Do not build the comparison out of
  `crates/verbatim/tests/crash.rs`'s `Snapshot` struct: phase 2's summary records
  that it excludes `compaction_boundaries`, so a reused comparison is incomplete
  on exactly the table a boundary-carrying fixture would exercise.

### Task 2: Run snapshots by default, on a schedule, outside the lock

- **Files:** crates/verbatim-core/src/config.rs, crates/verbatim-core/src/ingest/pass.rs, crates/verbatim/src/cmd/ingest.rs, crates/verbatim-core/tests/config.rs, crates/verbatim-core/tests/snapshot.rs
- **Action:** Add a `[snapshot]` table to `FileConfig` in `config.rs` on the same
  `#[serde(default)]` terms as `[injection]` and `[provider]`, carrying an
  enabled flag defaulting to TRUE - STOR-06 says rolling snapshots run by default,
  and this is the one config block in this workspace whose default is on - an
  interval in hours defaulting to 24, and a retained count defaulting to 3.
  Resolve them onto `Config` with accessors, the way the injection budgets are.
  Then wire the step into `pass::run_with`, AFTER `locked` returns and beside the
  existing `summary.judgment = crate::observe::judge_new(...)` call, which is the
  shape D-15 names: the ingest guard and the store handle have both been dropped,
  the `runs` row has committed, and nothing below can fail the pass. The step
  reads a snapshot timestamp from the `meta` table through `Store::meta_int` or
  its text equivalent, does nothing when the interval has not elapsed, and
  otherwise takes a snapshot and prunes, then stamps the new timestamp. The gate
  is checked before any expensive work: a snapshot of the real store is 1.087 GB
  and 0.56 s, and the hook spawn is the scheduler, so an ungated step writes a
  gigabyte per prompt and the first user to notice is one whose disk fills.
  Carry the outcome on `Summary` as its own field beside `judgment`, with notes
  that go to STDERR and NOT into `runs.error` - that row has already committed at
  this point, which is exactly why `judgment`'s own doc comment says the same
  thing - and print them from `cmd::ingest::tree` where the judgment notes are
  already printed. A snapshot that fails is a note; the archive is the work.
- **Verify:** `cargo test -p verbatim-core --test config` and
  `cargo test -p verbatim-core --test snapshot` pass with new cases showing
  (a) a config directory with no `verbatim.toml` resolves to snapshots enabled,
  a 24-hour interval and 3 retained, (b) a first pass over a fixture tree leaves
  exactly one file in the snapshots directory and a timestamp in `meta`,
  (c) a second pass immediately after leaves that count and that timestamp
  unchanged, (d) a pass run with the stored timestamp backdated past the interval
  writes a second snapshot, and (e) with the snapshots directory made unwritable
  the pass still returns `PassOutcome::Ran`, still writes its `runs` row, and
  reports the failure as a note rather than an error.

### Task 3: One resolver, one location pointer

- **Files:** crates/verbatim-core/src/store/open.rs, crates/verbatim-core/tests/store_open.rs, crates/verbatim/tests/hook.rs
- **Action:** Extend `store::open::data_dir` - the ONLY resolver of the store
  path in either crate, which is what makes STOR-07 a change to this function
  alone - to consult a plain-text location pointer file in verbatim's config
  directory after `VERBATIM_DATA_DIR` and before `platform_data_dir`. The order
  is D-06 and it is not negotiable: the environment override outranks the
  pointer, so a test bench and a spawned child that set `VERBATIM_DATA_DIR`
  behave exactly as they do today. The file holds one absolute path as text,
  trimmed; an empty or whitespace-only file is treated as unset the way
  `non_empty_var` already treats an empty environment variable, and a missing
  file is not an error - it is the state every user starts in. A config
  directory that cannot be resolved falls through to `platform_data_dir` rather
  than failing, because a store path that stopped resolving because `HOME` moved
  would take every command down with it. TOML is deliberately not the format:
  the `toml` dependency in the root `Cargo.toml` is `default-features = false`
  with the serializer half absent, so writing a TOML pointer would cost a new
  cargo feature on the hook-path binary. This read lands on the hook path -
  `cmd::hook` resolves the data directory on every event - whose p99 is asserted
  at 10 ms against a measured 0.408 ms floor, so it must be one
  `fs::read_to_string` of a small file that usually is not there, reached only
  when the environment variable is unset, and nothing more.
- **Verify:** `cargo test -p verbatim-core --test store_open` passes with new
  cases showing (a) with `VERBATIM_DATA_DIR` set and a pointer file present, the
  environment variable wins, (b) with it unset and a pointer file naming a path,
  that path is resolved, (c) with an empty or whitespace-only pointer file the
  platform location is resolved, and (d) with no pointer file the resolution is
  byte-identical to today's. `cargo test -p verbatim --test hook` still passes,
  including `every_event_exits_zero_with_an_empty_stdout_inside_the_budget`.

### Task 4: `verbatim data move <path>`

- **Files:** crates/verbatim/src/cmd/data.rs, crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs, crates/verbatim/tests/datamove.rs
- **Action:** Add a `data` subcommand whose only verb is `move`, dispatched from
  `main.rs` with a `USAGE` line naming both words, and parsed the way
  `cmd::observations::parse` handles the workspace's first two-word subcommand:
  the verb comes first or not at all, and any other second word is misuse rather
  than a silently ignored word. It is human-only, exactly like `install` and
  `uninstall` and for the reason `main.rs` already gives about them - it shows
  what it is about to do and asks once, and a single JSON document on stdout
  cannot be both of those things - so it takes no `--json`, joins no
  `DATA_COMMANDS` and gets no `docs/json-shapes.md` section; it takes `--yes` the
  way `uninstall --purge` does. It resolves the current data directory through
  `cmd::data_dir`, refuses a destination that already exists and is non-empty,
  prints what is about to move and its size, prints the
  SQLite-over-network-filesystem warning unconditionally - no `std` API detects a
  network mount on any of the three targets and this command asks regardless of
  destination, so the warning costs nothing - and confirms through
  `install::confirm` the way `uninstall`'s `answered` does, refusing when there
  is no answer to read rather than assuming one. Then it takes the ingest lock,
  so a pass mid-transaction is never left writing into a directory that moved,
  and copies the WHOLE directory: `verbatim.db` and its `-wal` and `-shm`
  sidecars, the `LOCK` file, the `decisions` injection-state directory named by
  `inject::decision::DIR_NAME`, and the snapshots directory. A copy and then a
  delete, never `fs::rename`, because a rename across filesystems fails and
  relocating onto another disk is the whole point. It writes the location
  pointer task 3 reads, atomically through a temporary file and a rename, and
  only after the pointer is in place does it release the lock and remove the
  source. It touches no settings file: neither the hook entries nor the MCP
  registration names a store path, so there is nothing there to rewrite and
  rewriting it would break INST-03.
- **Verify:** `cargo test -p verbatim --test datamove` passes with new cases
  showing, in a bench that sets `VERBATIM_CONFIG_DIR` and the platform data-dir
  variable but NOT `VERBATIM_DATA_DIR` - the environment override outranks the
  pointer, so a test that sets it cannot fail - (a) after `verbatim data move
  <new> --yes` the database, `LOCK`, injection-state directory and snapshots are
  at the new path and the old directory is gone, (b) `verbatim status` then
  reports the new path and the same session count, (c) a `verbatim hook`
  invocation and an MCP `recall_search` both answer off the new location, (d)
  the settings file the bench wrote before the move is byte-identical after it,
  (e) the command prints the network-filesystem warning and, without `--yes` and
  with no answer to read, exits non-zero having moved nothing, and (f) a
  destination that already holds files is refused with nothing copied.

## Notes

- D-17 says `data move` follows the `observations regenerate` nested-dispatch
  precedent "with its own envelope `command` string". The dominant clause of the
  same decision, and its cited evidence, is that `data move` is human-only like
  `install` and `uninstall` - which have no envelope at all. This plan takes the
  human-only reading and the nested-dispatch half, and drops the envelope phrase
  as vestigial from the `observations` precedent it was drawn from. Recorded
  rather than done silently.
- `DESIGN-BRIEF.md:407`'s free-space guard before writing is not planned: no
  phase-8 decision names it and there is no `std` API for free space, so it would
  be a new platform-specific dependency on the hook-path binary, which is the
  same reason D-16 defers network-filesystem detection.
- PLAN-4 shares `crates/verbatim-core/src/config.rs` with PLAN-1 and PLAN-5,
  `crates/verbatim-core/src/ingest/pass.rs` with PLAN-1, and
  `crates/verbatim/src/main.rs` and `crates/verbatim/src/cmd/mod.rs` with PLAN-2
  and PLAN-3. It is SEQUENTIAL with all of them.
