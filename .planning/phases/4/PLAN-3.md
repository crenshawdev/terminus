---
phase: 4
plan: 3
requirements:
  - INST-06
  - INST-07
files:
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/doctor.rs
  - crates/verbatim/src/cmd/uninstall.rs
  - crates/verbatim/src/cmd/json.rs
  - crates/verbatim/src/cmd/install/targets.rs
  - crates/verbatim/tests/doctor.rs
  - crates/verbatim/tests/uninstall.rs
  - docs/json-shapes.md
---

# Phase 4: Hooks And Install - Plan 3 of 5 (doctor and uninstall)

**SEQUENTIAL: PLAN-1 -> PLAN-2 -> PLAN-3 -> PLAN-4.** This plan reads the stable
path, the marker, the target resolution and the JSON editor PLAN-2 writes, and
names the event constant PLAN-1 defines. It shares
`crates/verbatim/src/cmd/install/targets.rs` with PLAN-2 and
`crates/verbatim/src/main.rs` with all four. Do not run it in parallel with any
of them.

## Goal

A user who suspects verbatim is not wired in gets a read-only report naming
every problem and the exact command that fixes it, and a user who wants it gone
gets their settings back exactly as they were, with their archive still on disk
and its path printed.

## Must be true when done

- `verbatim doctor` against a data directory that does not exist creates no file
  and no directory, and says so as a state rather than an error (AC6).
- `verbatim doctor` against a hook entry pointing at a missing binary reports the
  problem and prints a command that, run, makes the same check pass (AC6).
- `verbatim doctor` never writes anything anywhere and never repairs (INST-06).
- `verbatim doctor --json` emits one document of named checks on stdout with
  every diagnostic on stderr (D-24).
- Uninstall on a `settings.json` otherwise unchanged since install restores it to
  its pre-install bytes, removes only the `verbatim` key from `.mcpServers`, and
  leaves the data directory present, printing its path (AC7).
- Uninstall on a `settings.json` the user has since edited through Claude Code's
  own UI removes verbatim's entries and keeps every later edit.

## Context

- D-12: doctor opens the store through `crates/verbatim/src/cmd/read.rs`, never
  `Store::open`. `crates/verbatim/src/cmd/status.rs:40` is `Store::open`, and
  `crates/verbatim-core/src/store/open.rs:101-119` shows that call doing
  `create_dir_all`, `initialize()`, `bring_forward` and
  `pragma_update(journal_mode=wal)` - so a doctor built on `status`'s shape would
  create the store its own check then reports as present.
  `read.rs`'s test `an_empty_data_directory_is_a_reason_and_creates_nothing`
  already asserts the behaviour this needs.
- D-13: uninstall removes exactly the entries whose `command` equals the stable
  path plus the `verbatim` key under `.mcpServers`. As measured 2026-08-13,
  `settings.json` carries 29 top-level keys including `theme`, `model`,
  `effortLevel`, `voice` and `statusLine`, all written by Claude Code's own UI
  between install and uninstall.
- D-16: the supported Claude Code floor for exec-form hooks is 2.1.227, which is
  the oldest version measured to carry `args` and not the release that introduced
  it. Word the check accordingly.
- D-24: `doctor` takes `--json`; `install` and `uninstall` stay human-only.
- Out of scope: repairing anything, `verbatim verify`'s blob walk, and the
  backfill `doctor` might otherwise suggest running (PLAN-4).

## Tasks

### Task 1: `verbatim doctor` and the wiring checks

- **Files:** crates/verbatim/src/cmd/doctor.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs
- **Action:** Add a `doctor` subcommand that runs a list of named checks, each
  producing a state, a one-line finding, and - when the state is a problem - the
  exact command that fixes it (INST-06). It never writes and never repairs: no
  file created, no file modified, no directory created, on any path.
  This task's checks are the wiring ones. A file exists at the stable path
  (PLAN-2's resolution, honouring `VERBATIM_BIN_DIR`); it carries PLAN-2's
  marker; its `--version` matches this build's. `settings.json` holds one hook
  entry per event, in exec form, whose `command` is that stable path, for exactly
  the four events PLAN-1's `cmd::hook` constant names - a missing entry, an entry
  pointing somewhere else, and a duplicate entry are three different findings.
  `.claude.json` holds a `verbatim` entry under `.mcpServers` pointing at the same
  path. And the installed Claude Code is at or above the 2.1.227 floor (D-16):
  resolve `claude` through PATH and read the leading dotted version out of
  `claude --version`, which prints `2.1.231 (Claude Code)` on this machine;
  spawning a helper is already how `crates/verbatim-core/src/project.rs` resolves
  git. A `claude` that cannot be found or whose output does not parse is reported
  as undetermined, not as a problem. Say in the finding that 2.1.227 is the
  oldest version verbatim has verified carries exec-form `args`, not a proven
  minimum.
  Every problem's fix command must be literally runnable - `verbatim install` for
  a missing or mismatched binary and for missing entries, and the overwrite
  command PLAN-2 prints for an unmarked incumbent - not a description of what to
  do. Add `doctor` to `USAGE` and to `dispatch` in `main.rs`.
- **Verify:** In `crates/verbatim/tests/doctor.rs`, against a temp
  `CLAUDE_CONFIG_DIR` and `VERBATIM_BIN_DIR`: after `verbatim install --yes`,
  `verbatim doctor` exits 0 and every wiring check reports ok; deleting the file
  at the stable path makes `doctor` report the problem, exit non-zero, and print a
  command whose execution makes a second `doctor` report ok again - the test runs
  the printed command and reruns doctor rather than asserting on its text;
  rewriting one hook entry's `command` to `/nonexistent/verbatim` produces a
  finding naming that event.

### Task 2: The store and config checks, which create nothing

- **Files:** crates/verbatim/src/cmd/doctor.rs
- **Action:** Add the checks about verbatim's own state, all of them through
  `crates/verbatim/src/cmd/read.rs`'s `open_in`, never `Store::open` (D-12): the
  store opens; its format version is one this build reads, reported through
  `Store::predates_this_build`; the last `runs` row and its error, which is the
  only account a detached ingest leaves (there is no log file, by design); which
  Claude config roots `Config::roots` resolved and how many, because more than one
  resolving is worth naming when only the first carries the hooks; and the data
  directory's state.
  A data directory that does not exist is a reported state and not a failure - it
  is what a machine looks like before the first ingest - and doctor must leave it
  not existing (AC6). Test writability by reading the directory's metadata
  permissions rather than by writing a probe file, which would create the thing
  the check is about. Do not walk blobs here: integrity over a 735 MB store is
  `verbatim verify`'s job and takes minutes, and doctor is a report a user runs
  when something looks wrong.
- **Verify:** In `crates/verbatim/tests/doctor.rs`, with `VERBATIM_DATA_DIR`
  pointed at a path inside a temp directory that does not exist: `verbatim doctor`
  exits 0, reports the store as not yet created, and the path still does not exist
  afterwards - asserted on the parent directory's entry list, so a created WAL or
  `LOCK` file would fail it too; after one ingest, doctor reports the session
  count and the last run's timestamp.

### Task 3: The settings doctor reads and never changes

- **Files:** crates/verbatim/src/cmd/doctor.rs
- **Action:** Report the effective `cleanupPeriodDays` and the effective
  `autoCompactEnabled`, naming which source won. For auto-compact that is the
  `DISABLE_AUTO_COMPACT` environment variable and the settings scopes in Claude
  Code's own precedence: the user file PLAN-2 writes, the project's
  `.claude/settings.json`, and the project's `.claude/settings.local.json`.
  Report only: doctor never repairs (INST-06) and verbatim never changes either
  setting (D-05's neighbour decision in the brief - compaction burns tokens
  summarizing context this store already holds losslessly, and a low
  `cleanupPeriodDays` costs the user their second recovery path). Where a value is
  not what verbatim recommends, the fix command is the `/config` or settings edit
  the user makes themselves, printed literally.
  A settings file that does not exist at a scope is that scope reporting nothing,
  never an error: a user with no project settings is the ordinary case.
- **Verify:** `verbatim doctor` against a temp `CLAUDE_CONFIG_DIR` whose
  `settings.json` carries `"cleanupPeriodDays": 7` and `"autoCompactEnabled": true`
  names both values and the file each came from, and `jq -c .` on that file is
  byte-identical before and after; with `DISABLE_AUTO_COMPACT=1` set, the
  auto-compact finding names the environment variable as the winning source.

### Task 4: `doctor --json`

- **Files:** crates/verbatim/src/cmd/doctor.rs, crates/verbatim/src/cmd/json.rs,
  docs/json-shapes.md
- **Action:** Accept `--json` on `doctor` and emit one `{command, ok, reason,
  data}` document whose `data` carries the checks by name, each with its state,
  its finding and its fix command, with every diagnostic on stderr and `ok` equal
  to the exit code's answer (D-24, RCL-06). Reuse
  `crates/verbatim/src/cmd/json.rs`'s `Document` - the envelope is the contract
  `cmd/mod.rs` documents, and a second one would be drift.
  `Document::emit` uses `println!` and so panics with exit 101 when stdout is a
  closed pipe (`.planning/CAPTURE.md`, phase 3), which would make
  `verbatim doctor --json | head` report a failure that is doctor's own. Add a
  fallible emission alongside `emit` and have `doctor` call it, mapping a broken
  pipe to a silent exit. Do not change what the six shipped commands call: that
  open item covers code this phase did not write, and widening the fix here turns
  one new command into six changed contracts.
  Document doctor's shape in `docs/json-shapes.md` beside the six already there,
  including which fields are null in which states - the sessions shape's silence
  about nullability is already a logged phase 3 open item and this one should not
  repeat it.
- **Verify:** `verbatim doctor --json | jq -e '.data | keys_unsorted | length > 0'`
  succeeds; `verbatim doctor --json | head -c 1` exits 0 rather than 101;
  `verbatim doctor --json` on a machine with a missing binary at the stable path
  emits `ok: false`, exits 1, and its document validates against the shape added
  to `docs/json-shapes.md`.

### Task 5: `verbatim uninstall`

- **Files:** crates/verbatim/src/cmd/uninstall.rs,
  crates/verbatim/src/cmd/install/targets.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs, crates/verbatim/tests/uninstall.rs
- **Action:** Remove exactly what install added and nothing else (INST-07, D-13),
  through PLAN-2's JSON editor so the untouched keys survive: every hook entry
  whose `command` equals the stable path, dropping a matcher group that becomes
  empty and an event key that becomes an empty array only if install created it,
  and the `verbatim` key under `.mcpServers`. Never a whole-object replacement -
  `settings.json` carries 29 keys and `.claude.json` carries 240 KB of the user's
  project history.
  Then the restore: after the surgical removal, if the resulting document is
  byte-identical to install's backup, write the backup's bytes back and remove the
  backup, so a settings file otherwise unchanged since install ends at its
  pre-install bytes exactly, formatting and key order included (AC7). If it
  differs - which is what a `/config` change to `theme` or `model` since install
  looks like - keep the surgically edited file, leave the backup in place, and say
  which of the two happened.
  Delete the copy at the stable path only when it carries PLAN-2's marker; an
  unmarked file there is somebody else's and is reported, not removed. Leave the
  data directory alone and print its path (INST-07). Work when the config is
  partly broken: a missing settings file, a missing backup, an entry already gone,
  each is one reported line and uninstall carries on to the rest rather than
  aborting. Human-only, no `--json` (D-24). Add to `USAGE` and `dispatch`.
- **Verify:** In `crates/verbatim/tests/uninstall.rs`: install then uninstall over
  a seeded `settings.json` leaves that file byte-identical to the seed
  (`cmp` on the bytes, not a `jq` comparison) and leaves
  `jq '.mcpServers|keys_unsorted'` back at `["context7"]` with the rest of
  `.claude.json` byte-equal to the seed; editing an unrelated key between install
  and uninstall leaves that edit present afterwards and leaves the four hook
  entries gone; the data directory still exists afterwards and uninstall's stdout
  contains its path; a `settings.json` deleted between install and uninstall
  produces exit 0 with a reported line.

### Task 6: `uninstall --purge`

- **Files:** crates/verbatim/src/cmd/uninstall.rs, crates/verbatim/tests/uninstall.rs
- **Action:** Under `--purge`, and only after everything in Task 5 has run, show
  the data directory's path and its size on disk - the database and its `-wal` and
  `-shm` sidecars, the same footprint `crates/verbatim/src/cmd/status.rs`'s
  `size_bytes` computes - and confirm once before deleting it, reusing PLAN-2's
  confirmation so `--yes` and the non-TTY rule behave identically here. This is the
  one irreversible thing this phase does: the archive is the product, so the size
  and the path are shown before the question and not after it. A declined
  confirmation exits 0 with the data intact and says so. Without `--purge` the
  data is never touched, which is Task 5's behaviour and the default.
- **Verify:** `verbatim uninstall --purge` with stdin closed and no `--yes` exits
  non-zero with the data directory intact; with `--yes` it prints the path and a
  byte count matching `verbatim status`'s `size_bytes` taken beforehand, then
  exits 0 with the directory gone; answering `n` at the prompt exits 0 with the
  directory intact.

## Notes

- The brief's doctor list also names a free-space check. It is planned nowhere
  here: `std` has no stable free-space API, and the only route is a `libc` or
  `windows-sys` dependency that D-04 refuses on this binary's behalf. No
  requirement and no acceptance criterion names it. Flagged for the human rather
  than silently dropped.
- Task 4 touches `crates/verbatim/src/cmd/json.rs`, a file phase 3 shipped. The
  change is additive - a new fallible emission beside `emit` - precisely so the
  six existing commands' behaviour is untouched. Their broken-pipe bug stays an
  open item.
