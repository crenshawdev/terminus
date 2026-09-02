---
phase: 3
plan: 3
requirements:
  - PRIV-04
files:
  - crates/verbatim-core/src/owner_only.rs
  - crates/verbatim/src/cmd/export.rs
  - crates/verbatim/src/cmd/data.rs
  - crates/verbatim/tests/owner_only.rs
---

# Phase 3: Owner-Only On Disk - Plan 3

## Goal

Nothing verbatim writes is readable by group or world: the export directory
and every file in it, and the tree `verbatim data move` copies, are owner-only
at creation - and one test under a umask of 022 drives the real binary through
a fresh ingest, both hooks and an export and reads every mode AC1 names.

## Must be true when done

- Under umask 022 `verbatim export <dir>` creates `<dir>` 0700 and every
  `.jsonl` and `manifest.json` inside it 0600 (PRIV-04: the output is still
  portable and still labelled; now it is also not readable by anyone else).
- Under umask 022 `verbatim data move <dir>` creates `<dir>` 0700, every
  copied subdirectory 0700 and every copied file 0600 even when the SOURCE
  tree is 755/644, and the fresh `LOCK` it writes at the destination is 0600.
- A `#[cfg(unix)]` test that sets umask 022 itself observes, after a fresh
  ingest, a `SessionStart`, a `UserPromptSubmit` and an export: data dir 0700;
  `verbatim.db`, `-wal`, `-shm` and `LOCK` 0600; `snapshots/` 0700 with 0600
  files; `injection/` and `decisions/` 0700 with 0600 files; export dir 0700
  with 0600 files - each asserted separately, each failure naming its path.
- The same test against the tree as it stood before this phase fails naming at
  least the data directory, `verbatim.db` and the export files.
- `set_permissions` appears in neither `cmd/export.rs` nor `cmd/data.rs`.

## Context

Locked: D-01, D-04 (leaf only: the parents of a user-named export or move
destination keep their default), D-08 (the umask test drives the binary
through `sh -c 'umask 022; ...'`, never `umask()` in-process), D-10 (`data
move` writes every mode explicitly rather than reproducing the source's), D-11
(the export destination is verbatim's own output and is forced to 0700/0600).
`doctor.rs`'s test `shell` helper is the precedent for running a line through
`sh`; `tests/datamove.rs` and `tests/lifecycle.rs` are the benches to mirror.
Both source files are in the `verbatim` binary crate and reach the primitive
as `verbatim_core::owner_only`.

## Tasks

### Task 1: Export directory and files owner-only at creation

- **Files:** crates/verbatim/src/cmd/export.rs (`write_export`),
  crates/verbatim-core/src/owner_only.rs,
  crates/verbatim/tests/owner_only.rs
- **Action:** In `write_export`, the `create_dir_all(destination)` becomes the
  leaf-0700 creation, and both `std::fs::write` calls - one per session file,
  one for `MANIFEST` - become an open through the primitive's 0600 options
  with `create_new` (the directory was just verified empty or created, so an
  existing name is a bug worth failing on rather than truncating) followed by
  writing the bytes. The `Failure::Operational` messages stay as they are.
  `refuse_if_occupied`, `NOTICE`, the manifest's shape and the `--json`
  document are untouched (PRIV-04's labelling is not this task's concern and
  must not move). No chmod (AC2); an existing EMPTY destination the user made
  keeps the user's directory mode, see Notes. This task also creates
  `tests/owner_only.rs` with its bench and the umask harness: a helper that
  runs the binary through `sh -c` with `umask 022` set, passing the binary path
  and its arguments as positional parameters rather than interpolating them
  into shell text, with the same environment pinning `tests/lifecycle.rs`
  uses. Constraint on the whole file: it must compile against the tree as it
  stood at commit `3e09d9a` (only std, `tempfile`, `rusqlite`, `serde_json`,
  `verbatim_core::testkit` and `verbatim_core::ingest::lock`), because Task 3
  runs it there to prove it fails on the pre-fix build.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --test owner_only export` passes a `#[cfg(unix)]` test that ingests one
  fixture, exports through the umask-022 harness into `<root>/export`, and
  shows the directory 0700 and every `.jsonl` plus `manifest.json` 0600; the
  existing `--test lifecycle` export tests pass unmodified. `grep -n
  set_permissions crates/verbatim/src/cmd/export.rs` returns nothing.

### Task 2: The moved tree owner-only at creation

- **Files:** crates/verbatim/src/cmd/data.rs (`run`, `copy_tree`),
  crates/verbatim-core/src/owner_only.rs,
  crates/verbatim/tests/owner_only.rs
- **Action:** In `copy_tree`, the `create_dir_all(to)` becomes the leaf-0700
  creation (which on the recursive call makes every copied subdirectory 0700
  too), and the `std::fs::copy` per file - which reproduces the source's mode,
  the exact thing D-10 names - becomes an open of the destination through the
  primitive's 0600 options with `create_new`, an `std::io::Copy` of the source
  into it, and a close. The error strings stay in the `failed` closure's shape.
  In `run`, the destination `LOCK` at the `File::create(&lock_file)` site is
  created through the same 0600 options. Everything else - the lock held over
  the copy, the pointer written before the source is removed, the network
  warning, the question - is unchanged.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --test owner_only data_move` passes a `#[cfg(unix)]` test benched like
  `tests/datamove.rs` (no `VERBATIM_DATA_DIR`, `XDG_DATA_HOME` pinned) that
  ingests once, deliberately widens every directory in the source to 0755 and
  every file to 0644 with `set_permissions`, runs `data move --yes` through
  the umask-022 harness, and shows the destination 0700, `snapshots/` 0700,
  every file including the snapshot and `LOCK` 0600, while the existing
  `--test datamove` tests pass unmodified. `grep -n "set_permissions\|fs::copy"
  crates/verbatim/src/cmd/data.rs` returns nothing.

### Task 3: The umask-022 test over the whole AC1 list

- **Files:** crates/verbatim/tests/owner_only.rs
- **Action:** The end-to-end AC1 test, `#[cfg(unix)]`, in the file the first
  two tasks started. Sequence: place `session-basic.jsonl` under the bench's
  Claude projects tree and run `ingest` through the umask-022 harness (data
  directory, `verbatim.db`, `LOCK`, `snapshots/` and its first file). Then, in
  the test process, take `verbatim_core::ingest::lock::try_acquire` on the
  data directory and hold it for the rest of the test - every hook spawns a
  detached ingest (`hook.rs`), and one that got the lock would drain the
  `decisions/` file before it is stat'd; with the lock held, `pass.rs`'s
  `a_pass_reports_the_lock_rather_than_waiting_for_it` says it exits having
  written nothing. Also open a `rusqlite::Connection` to `verbatim.db` and hold
  a read transaction across the hook runs: SQLite deletes `-wal` and `-shm`
  when the last connection closes, which is why CONTEXT D-02's umask-022
  baseline lists neither, and a held reader keeps them on disk. Their mode is
  set by SQLite from `verbatim.db`'s own whichever process created them (plan
  1 Context), so the two sidecar assertions test the flagged D-02 assumption
  directly and not the test's umask. Then run `hook SessionStart` through the
  harness with a payload shaped like `tests/brief.rs`'s but with `source` set
  to `compact`, which `inject::brief::owe_compaction` answers by saving the
  state file unconditionally, and `hook UserPromptSubmit` with a payload
  shaped like `tests/prompt.rs`'s, which writes a decision on every prompt
  (D-11). Then run `export` through the harness. Then assert, one assertion
  per path with the path in the message: data dir 0700; `verbatim.db`,
  `verbatim.db-wal`, `verbatim.db-shm`, `LOCK` 0600; `snapshots/` 0700 and
  each file in it 0600; `injection/` 0700 and each file in it 0600;
  `decisions/` 0700 and each file in it 0600; export dir 0700 and each file in
  it 0600. Every directory listed must be non-empty for the test to pass, so
  a hook that wrote nothing is a failure and not a vacuous green.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --test owner_only` passes. The falsifying half: `git -C /code/verbatim
  worktree add /tmp/claude-1000/-code-verbatim/73a34844-316e-4185-aa11-09d3e4b787a2/scratchpad/prefix
  3e09d9a`, copy `crates/verbatim/tests/owner_only.rs` into that worktree's
  `crates/verbatim/tests/`, run `cargo test --manifest-path
  <worktree>/Cargo.toml -p verbatim --test owner_only` there, and the run
  FAILS with messages naming at least the data directory, `verbatim.db` and an
  export file; then `git -C /code/verbatim worktree remove --force
  <worktree>`. Finally `grep -rn set_permissions
  /code/verbatim/crates/verbatim-core/src /code/verbatim/crates/verbatim/src |
  grep -v cmd/install/ | grep -v "cfg(test)"` lists only the tighten-on-open
  repair (AC2, whole tree).

## Notes

- Sequential after plans 1 and 2: Task 3 reads every mode the earlier plans
  set, and `owner_only.rs` is declared here for the same extension reason plan
  2 gives (a "copy into a 0600 destination" variant, if the executor wants it
  in the primitive rather than inline).
- An export destination that already exists and is empty is accepted by
  `refuse_if_occupied` and keeps the mode its creator gave it: AC2 bars a
  `set_permissions` in `export.rs`, so only a directory export itself creates
  is 0700. The files inside are 0600 either way.
- Out of scope, reported for the human rather than planned: `data move`'s
  pointer file (`write_pointer`, `std::fs::write` then rename) holds only a
  path and is not in CONTEXT's list; `cmd/install/json_file.rs` creates its
  temporary at the umask default and then `set_permissions` it to the
  original's mode (:652, :674), a widen-then-narrow window on a copy of
  `settings.json`, which the roadmap goal names as the install exception.
