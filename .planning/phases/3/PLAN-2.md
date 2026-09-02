---
phase: 3
plan: 2
requirements:
  - PRIV-02
  - PRIV-04
files:
  - crates/verbatim-core/src/owner_only.rs
  - crates/verbatim-core/src/store/snapshot.rs
  - crates/verbatim-core/src/ingest/lock.rs
  - crates/verbatim-core/src/inject/state.rs
  - crates/verbatim-core/src/inject/decision.rs
  - crates/verbatim-core/tests/snapshot.rs
  - crates/verbatim-core/tests/pass.rs
  - crates/verbatim-core/tests/inject_state.rs
---

# Phase 3: Owner-Only On Disk - Plan 2

## Goal

Nothing verbatim writes is readable by group or world: the four creation sites
inside `verbatim-core` that are not the store itself - the snapshot directory
and its files, the `LOCK` file, and the `injection/` and `decisions/` scratch
directories with their files and temporaries - set an owner-only mode at
creation.

## Must be true when done

- Under umask 022 a snapshot lands in a 0700 `snapshots/` as a 0600 file, and
  the file was never wider than that at any instant.
- Under umask 022 `ingest::lock::try_acquire` on a machine with no data
  directory leaves the data directory 0700, its parent at the umask default,
  and `LOCK` 0600.
- Under umask 022 a saved injection state file sits in a 0700 `injection/` as a
  0600 file, and a saved decision sits in a 0700 `decisions/` as a 0600 file;
  every `.tmp` either writes through is 0600 from the moment it exists.
- `set_permissions` appears in none of the four source files.

## Context

Locked: D-01 (std only, `#[cfg(unix)]`), D-03 (the snapshot `.tmp` is
pre-created EMPTY at 0600 and `VACUUM INTO` writes into it; the module comment
claiming `VACUUM INTO` refuses any existing file is corrected in place - it
refuses a NON-EMPTY one), D-04 (leaf only), D-09 (`injection/` and
`decisions/` both in scope), D-10 (`LOCK` and every `.tmp` in scope). Every
site uses plan 1's `owner_only` primitive and may extend it. An existing wide
directory is not narrowed here - plan 1's repair does that on the next
`Store::open`, which `ingest::pass::roll` performs right before every
`snapshot::take`.

## Tasks

### Task 1: Snapshot directory and file owner-only at creation

- **Files:** crates/verbatim-core/src/store/snapshot.rs (`take`),
  crates/verbatim-core/src/owner_only.rs,
  crates/verbatim-core/tests/snapshot.rs
- **Action:** In `take`, create `snapshots/` through the leaf-0700 primitive
  instead of the bare `create_dir_all`. Keep the stale-temp unlink that
  precedes it, then pre-create `temp` as an EMPTY 0600 file through the
  primitive's options with `create_new` and close the handle before
  `VACUUM INTO` runs, so SQLite writes into a file that already has its mode
  (measured in CONTEXT D-03: `VACUUM INTO` onto a zero-length existing file
  succeeds and keeps 0600; onto a non-empty one it fails with `file is not a
  database`, which is why the unlink stays). The rename into place carries the
  mode with it. Rewrite the comment at the unlink so it says what is true:
  `VACUUM INTO` refuses a non-empty destination, and the module header's
  "refusing to overwrite an existing file" line is corrected the same way.
  No chmod after the copy (AC2). `prune` is unchanged.
- **Verify:** `sh -c 'umask 022; cargo test --manifest-path
  /code/verbatim/Cargo.toml -p verbatim-core --test snapshot'` passes,
  including a new `#[cfg(unix)]` test that takes one snapshot on a fresh bench
  and shows `snapshots/` is 0700 and the returned file is 0600, and the
  existing `a_snapshot_passes_sqlites_own_integrity_check` and
  `a_snapshot_taken_mid_pass_opens_verifies_and_excludes_the_uncommitted_session`
  pass unmodified, proving the pre-created destination is a database SQLite
  accepts. `grep -n "set_permissions\|refuses a file that already exists"
  crates/verbatim-core/src/store/snapshot.rs` returns nothing.

### Task 2: The data directory and LOCK owner-only from try_acquire

- **Files:** crates/verbatim-core/src/ingest/lock.rs (`try_acquire`),
  crates/verbatim-core/tests/pass.rs
- **Action:** `try_acquire` is the first creator of the data directory on a
  machine that has never ingested (`ingest` and `data move` both call it before
  `Store::open`), so its `create_dir_all(data_dir)` becomes the leaf-0700
  creation, and its `OpenOptions` chain - read, write, create-if-missing,
  never truncate, exactly as documented there - starts from the primitive's
  0600 options instead of `OpenOptions::new()`. Nothing else about the lock
  changes: the contents still carry no meaning and `try_lock` is still the
  whole implementation.
- **Verify:** `sh -c 'umask 022; cargo test --manifest-path
  /code/verbatim/Cargo.toml -p verbatim-core --test pass'` passes, including a
  new `#[cfg(unix)]` test that calls `try_acquire` on `tmp/parent/data` where
  neither exists, holds the returned lock, and shows `data` is 0700, `parent`
  is 0755 and `LOCK` is 0600; the existing
  `a_pass_reports_the_lock_rather_than_waiting_for_it` passes unmodified.

### Task 3: Injection state and decision files owner-only at creation

- **Files:** crates/verbatim-core/src/inject/state.rs (`write_atomically`,
  `create_temporary`), crates/verbatim-core/src/inject/decision.rs
  (`write_file`, `create_temporary`),
  crates/verbatim-core/tests/inject_state.rs
- **Action:** In `state::write_atomically`, the `create_dir_all(dir)` becomes
  two leaf-0700 creations - the data directory first, then `injection/` - so a
  hook that fires before any ingest leaves verbatim's own directory 0700 while
  `~/.local/share` above it keeps its default (D-04). In
  `decision::write_file`, the `create_dir_all(dir)` becomes the leaf-0700
  creation of `decisions/` alone; `Decision::save` already refuses a missing
  data directory and that rule stays. Both `create_temporary` functions and
  the reserved-target `create_new` in `decision::write_file` start from the
  primitive's 0600 options and keep their `create_new` (the `O_EXCL` reasoning
  in their comments is unchanged and still the point). Every failure still
  reads as `None`/`false` - these run on the hook path and fail open. No
  chmod anywhere (AC2).
- **Verify:** `sh -c 'umask 022; cargo test --manifest-path
  /code/verbatim/Cargo.toml -p verbatim-core --test inject_state
  inject::decision'` passes, including a new `#[cfg(unix)]` test in
  `tests/inject_state.rs` that saves a state into a data directory that does
  not yet exist and shows the data directory 0700, `injection/` 0700 and the
  `<session_id>.json` 0600, and a new `#[cfg(unix)]` test in `decision.rs`'s
  `mod tests` that saves one decision and shows `decisions/` 0700 and the
  record 0600. The existing
  `a_data_directory_that_does_not_exist_is_not_created_to_hold_a_record` and
  `a_hostile_session_id_reaches_no_filesystem_call` pass unmodified.
  `grep -n set_permissions crates/verbatim-core/src/inject/state.rs
  crates/verbatim-core/src/inject/decision.rs crates/verbatim-core/src/ingest/lock.rs`
  returns nothing.

## Notes

- Sequential after plan 1: every task here calls the primitive plan 1 creates.
  `owner_only.rs` is declared so a task may add a variant it needs (for
  example an "empty file, exclusive, 0600" convenience for the snapshot temp)
  without an undeclared-files refusal at commit.
- PRIV-02 and PRIV-04 are carried for the reason plan 1's Notes give.
