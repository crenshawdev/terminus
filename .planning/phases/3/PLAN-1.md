---
phase: 3
plan: 1
requirements:
  - PRIV-02
  - PRIV-04
files:
  - crates/verbatim-core/src/owner_only.rs
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/tests/store_open.rs
  - crates/verbatim/tests/tighten.rs
---

# Phase 3: Owner-Only On Disk - Plan 1

## Goal

Nothing verbatim writes is readable by group or world. This plan is the one
creation primitive every later site uses, the store's own creation (data
directory 0700, `verbatim.db` 0600 so its `-wal` and `-shm` inherit), and the
tighten-on-open repair for a store an earlier build left wide.

## Must be true when done

- Under umask 022 a fresh `Store::open` leaves the data directory 0700 and
  `verbatim.db` 0600, and a parent directory it had to create on the way keeps
  the umask's own default rather than 0700.
- While that store is open and has been written, `verbatim.db-wal` and
  `verbatim.db-shm` are 0600.
- A store whose every path a test has widened to 755/644 is owner-only on every
  AC1 path after one `Store::open`; a second `Store::open` moves no regular
  file's ctime; `verbatim verify` exits 0 over it.
- `verbatim doctor`, `verbatim search` and `verbatim show` over a widened store
  leave every mode and every regular file's ctime exactly as they found them.
- A chmod that fails does not stop the store from opening.
- `set_permissions` is called by the repair and by nothing else outside
  `cmd/install/` and tests.

## Context

Locked: D-01 (std alone, `OpenOptionsExt::mode` and `DirBuilderExt::mode`
behind `#[cfg(unix)]`, the Windows arm a no-op stated in words, no `libc`),
D-02 (`verbatim.db` is created by verbatim at 0600 before rusqlite opens it;
`-wal`/`-shm` are left to SQLite), D-04 (mode on the leaf directory only), D-05
(repair in `Store::open` only, never `open_read_only`), D-06
(compare-then-chmod per known path, no `meta` marker), D-12 (a failed chmod is
not fatal). SQLite gives a fresh sidecar the main database's mode by `fchmod`
after open, bypassing umask - libsqlite3-sys 0.38.1 `sqlite3/sqlite3.c`
`robust_open` at :41021-41051 and the `-shm` open at :45219-45225 - which is
why 0600 on `verbatim.db` is what covers both sidecars. Plans 2 and 3 consume
the primitive this plan creates and may extend it, so all three declare it and
the phase runs in numeric order.

## Tasks

### Task 1: The owner-only creation primitive

- **Files:** crates/verbatim-core/src/owner_only.rs,
  crates/verbatim-core/src/lib.rs
- **Action:** Add a small public module to `verbatim-core`, declared in
  `lib.rs` beside the other `pub mod` lines, that offers exactly two things
  every creation site in this phase needs. First, directory creation whose LEAF
  is 0700: the parent chain goes through `std::fs::create_dir_all` at the
  process default, and the leaf alone is made with a non-recursive
  `std::fs::DirBuilder` carrying `DirBuilderExt::mode(0o700)`, where a leaf
  that already exists is success and is left at whatever mode it has (the
  repair in Task 3 is what narrows an existing one). Never a recursive
  `DirBuilder` with a mode: std applies that builder's mode to every component
  it creates, which is exactly what D-04 forbids (`~/.local/share` must not come
  out 0700). Second, an `OpenOptions` already carrying
  `OpenOptionsExt::mode(0o600)` that a caller finishes with its own
  `create`/`create_new`/`truncate`/`read`/`write` flags, so `ingest::lock`'s
  never-truncate open and `inject::state`'s exclusive create both fit. Both
  halves have a `#[cfg(not(unix))]` arm that does the same work with no mode,
  and the module doc states D-15 in words: on Windows the equivalent of
  owner-only is an ACL this build does not set. No `set_permissions` anywhere
  in this module's creation paths (AC2). No new dependency and no lockfile
  change (D-01). Unit tests live in a `#[cfg(test)] mod tests` inside the file,
  the pattern this workspace uses, and are `#[cfg(unix)]`.
- **Verify:** `sh -c 'umask 022; cargo test --manifest-path
  /code/verbatim/Cargo.toml -p verbatim-core owner_only'` passes tests that
  create `tmp/a/b/leaf` from nothing and show `leaf` is 0700 while `a` and `b`
  are 0755, a second call on the existing leaf succeeds and leaves it at a
  mode the test had set to 0755, and a file opened through the options with
  `create_new` is 0600. `cargo build --manifest-path /code/verbatim/Cargo.toml
  --workspace` succeeds, `git -C /code/verbatim diff --quiet Cargo.lock` exits
  0, and `grep -n set_permissions crates/verbatim-core/src/owner_only.rs`
  returns nothing.

### Task 2: Create the data directory and verbatim.db owner-only in Store::open

- **Files:** crates/verbatim-core/src/store/open.rs (`Store::open`),
  crates/verbatim-core/tests/store_open.rs
- **Action:** In `Store::open`, replace the bare `create_dir_all(data_dir)` with
  the Task 1 leaf-0700 creation, and before `inspect` runs pre-create
  `verbatim.db` as an EMPTY 0600 file through the Task 1 options with
  `create_new`, treating `AlreadyExists` as the ordinary case of an existing
  store and any other error as `Error::io` on that path. A zero-length file is
  what `inspect` already reads as `StoreState::Fresh` (the `m.len() == 0` arm),
  so `initialize` runs exactly as it does today and SQLite's own `journal_mode`
  stamp lands in a file that is already 0600 - measured in CONTEXT D-02 with
  this build. Do nothing for `-wal` and `-shm`: SQLite `fchmod`s a fresh
  sidecar to the main database's mode (see Context), and a chmod after the fact
  is the widen-then-narrow window AC2 bars. `Store::open_read_only` is not
  touched: it creates nothing (D-10) and must keep creating nothing.
- **Verify:** `sh -c 'umask 022; cargo test --manifest-path
  /code/verbatim/Cargo.toml -p verbatim-core --test store_open'` passes,
  including a new `#[cfg(unix)]` test that opens a store at `tmp/parent/data`
  where neither directory exists and shows `data` is 0700, `parent` is 0755,
  `verbatim.db` is 0600, and - after one `set_meta_int` on the open store -
  `verbatim.db-wal` and `verbatim.db-shm` both exist and are 0600. The existing
  `fresh_store_opens_in_wal_with_both_version_integers`,
  `an_initialize_that_never_committed_reopens_instead_of_bricking` and
  `a_read_only_open_of_a_missing_store_creates_nothing` still pass unmodified.

### Task 3: Tighten a wide store once, on Store::open only

- **Files:** crates/verbatim-core/src/store/open.rs (`Store::open`),
  crates/verbatim-core/src/owner_only.rs,
  crates/verbatim-core/tests/store_open.rs
- **Action:** After the directory and database creation of Task 2 and BEFORE
  `Connection::open`, so a narrowed `verbatim.db` is what any sidecar SQLite
  creates next inherits from, walk a fixed list of known paths and for each one
  `symlink_metadata` it, compare `mode & 0o777` to the target, and call
  `set_permissions` ONLY when a bit differs (D-06: a chmod to the same mode
  still moves ctime, and AC3 asserts a second open moves none). The list is
  exactly AC1's: the data directory itself (0700); `DB_FILE_NAME`, the same
  name with `-wal` and with `-shm` appended, and `ingest::lock::LOCK_FILE_NAME`
  (0600); `store::snapshot::DIR_NAME`, `inject::state::DIR_NAME` and
  `inject::decision::DIR_NAME` (0700) plus every regular file directly inside
  each of those three (0600). Skip anything that is not a regular file or
  directory - a symlink left in `snapshots/` would otherwise have its TARGET
  chmod'd, which may be a file outside the data directory - and skip a path
  that is not there. Every metadata or chmod failure is ignored and the open
  proceeds (D-12; `inject::state`'s `write_atomically` is the crate's pattern
  for best-effort filesystem work). No marker key, no `predates_this_build`
  gate: `reindex::open_up_to_date` opens the same store twice in one process
  and both opens run this. `Store::open_read_only` gets none of it (D-05) -
  `read.rs`'s `an_empty_data_directory_is_a_reason_and_creates_nothing`
  and doctor's read-only guarantee stay true. The primitives (stat, compare,
  chmod one path) may sit in `owner_only.rs`; the list and the walk belong in
  `open.rs` beside the thing they repair.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --test store_open` passes a new `#[cfg(unix)]` test that builds
  a store, creates `snapshots/`, `injection/` and `decisions/` each holding one
  regular file plus a symlink in `snapshots/` pointing at a 0644 file outside
  the data directory, widens every directory to 0755 and every file to 0644
  with `set_permissions`, calls `Store::open` once, and shows every listed path
  owner-only while the symlink's target is still 0644; then records
  `MetadataExt::ctime` and `ctime_nsec` of every regular file, calls
  `Store::open` again, and shows none moved; then re-widens and calls
  `Store::open_read_only`, showing every mode unchanged. `grep -rn
  set_permissions /code/verbatim/crates/verbatim-core/src
  /code/verbatim/crates/verbatim/src | grep -v cmd/install/` lists only the
  repair's call site(s) in `store/open.rs` or `owner_only.rs` and no test-gated
  line outside `#[cfg(test)]`.

### Task 4: Prove the repair and the read paths at the process boundary

- **Files:** crates/verbatim/tests/tighten.rs
- **Action:** A new binary test file, benched like `tests/lifecycle.rs`
  (tempdir; `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR`, `CLAUDE_CONFIG_DIR`
  pinned; `env!("CARGO_BIN_EXE_verbatim")`). It ingests
  `session-basic.jsonl` through the binary (the first pass also takes a
  snapshot, STOR-06), hand-writes one `.json` under `injection/` and one under
  `decisions/`, widens every directory to 0755 and every file to 0644, then
  runs `verbatim status` (which opens through `Store::open`, `status.rs`),
  asserts every AC1 path owner-only, records every regular file's ctime, runs
  `verbatim status` again and asserts no ctime moved, and runs `verbatim
  verify` asserting exit 0 (AC3). It then re-widens and runs `verbatim doctor`,
  `verbatim search` with a word from the fixture, and `verbatim show` with a
  turn id read from the store (`cmd/show.rs`'s `parse` says what it takes),
  asserting every mode and every regular file's ctime unchanged after all three
  (AC4). Regular files only for the ctime comparison: SQLite creates and
  removes `-wal`/`-shm` on every read-only open, which moves the data
  directory's own ctime without any chmod, and that is not what AC3 and AC4
  are about. The test is `#[cfg(unix)]`.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --test tighten` passes; with the repair's chmod call temporarily commented
  out, the same command fails on the AC3 mode assertions and names the data
  directory and `verbatim.db` in its message, then passes again with the call
  restored.

## Notes

- Deviation from the CONTEXT plan shape, recorded per the planner contract: the
  directive asked for the seven creation sites in one plan and the repair in
  another, but both live in `Store::open` (`store/open.rs`), so this plan holds
  the store's creation AND the repair, and the remaining six creation sites are
  plans 2 and 3. All three declare `owner_only.rs`, so the phase executes in
  numeric order rather than in parallel.
- PRIV-02 and PRIV-04 are the phase's IDs; the roadmap files success criteria 1,
  2 and 6 (the on-disk modes) under them, so they are carried here.
- Flagged assumption from CONTEXT: sidecar mode inheritance is asserted directly
  by Task 2's test, so a SQLite that stops inheriting fails loudly on whatever
  platform runs the test.
