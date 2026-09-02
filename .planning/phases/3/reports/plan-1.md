PLAN COMPLETE
Plan: .planning/phases/3/PLAN-1.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - The owner-only creation primitive | 28fc4df | `crates/verbatim-core/src/owner_only.rs`: `create_dir_all` (leaf 0700, parents at umask, existing leaf untouched) and `options()` (an `OpenOptions` pre-loaded with mode 0600), both `#[cfg(unix)]` with a no-mode Windows arm and D-15 stated in the module doc. 5 unit tests pass under `umask 022`; workspace build ok, `Cargo.lock` unchanged, no `set_permissions` in the module. |
| 2 - Data directory and verbatim.db owner-only in Store::open | 6967f59 | `Store::open` uses the leaf-0700 creation and pre-creates `verbatim.db` empty at 0600 with `create_new` (`AlreadyExists` is the ordinary case). New `#[cfg(unix)] a_fresh_store_and_its_sidecars_are_owner_only` asserts data 0700, parent 0755, db/`-wal`/`-shm` 0600; 14 of 14 `store_open` tests pass under `umask 022`. |
| 3 - Tighten a wide store once, on Store::open only | 24c4e8d | `tighten`/`narrow` in `store/open.rs`, called before any SQLite connection so a `-shm` inherits the narrowed mode. Compare-then-chmod per known path, `symlink_metadata` plus a file-type test so a link is skipped, every failure ignored. New `a_wide_store_is_tightened_by_a_writable_open_and_by_nothing_else` covers AC3/AC4 at the library boundary; 15 of 15 pass. The only `set_permissions` outside `cmd/install/` and tests is the repair's. |
| 4 - Prove the repair and the read paths at the process boundary | aad0da4 | `crates/verbatim/tests/tighten.rs`: real ingest, hand-written `injection/` and `decisions/` files, everything widened to 755/644, then `status` repairs, a second `status` moves no regular file's ctime, `verify` exits 0, and `doctor`/`search`/`show` over a re-widened store change no mode and no ctime. Falsified as asked: with the repair's `set_permissions` commented out the test fails naming all nine paths, including the data directory and `verbatim.db`. |
Suite: `cargo test --workspace --all-features` under `umask 022` - exit 0, 776 tests across 62 targets, 0 failures.
Deviations: none
Open items: `verbatim doctor` is run without an exit-code assertion in `tighten.rs` - a bench with nothing installed into Claude Code legitimately reports a Problem and exits 1. The test asserts what AC4 is about (it changed no mode and no ctime) and leaves doctor's verdict to `tests/doctor.rs`.
