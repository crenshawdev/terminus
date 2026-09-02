---
phase: 3
status: complete
completed: 2026-09-02
---

# Phase 3: Owner-Only On Disk - Summary

Every path verbatim creates lands 0700/0600 under umask 022 through one `owner_only` primitive, a wide store is tightened once on a writable open, and a group- or world-readable `verbatim.toml` is refused where its `api_key` is consumed.

## What shipped

- `owner_only::create_dir_all` (leaf 0700) and `owner_only::options()` (0600 `OpenOptions`), Unix-only with a documented Windows no-op - `crates/verbatim-core/src/owner_only.rs`
- Data directory and `verbatim.db` created owner-only, sidecars inheriting - `crates/verbatim-core/src/store/open.rs`
- Compare-then-chmod tightening of a wide store on `Store::open` only, symlinks skipped, failures ignored - `crates/verbatim-core/src/store/open.rs`
- Snapshots, `LOCK`, `injection/` scratch and `decisions/` records owner-only at creation - `snapshot.rs`, `ingest/lock.rs`, `inject/state.rs`, `inject/decision.rs`
- Export directory, its `.jsonl` files and `manifest.json` owner-only - `crates/verbatim/src/cmd/export.rs`
- `data move` writes the destination tree owner-only regardless of the source's modes - `crates/verbatim/src/cmd/data.rs`
- `Config::source_path()` carries the file a config was loaded from - `crates/verbatim-core/src/config.rs`
- Refusal of a group/world-readable `verbatim.toml` at tier-2 key consumption, same shape as the credentials refusal - `crates/verbatim-core/src/credentials.rs`
- Doctor reports `verbatim.toml`'s mode with the chmod, states the Windows deferral, and notes a provider declared local whose address is not loopback - `crates/verbatim/src/cmd/doctor.rs`
- Process-boundary tests: `tighten.rs` (repair and read paths), `owner_only.rs` (fourteen paths under umask 022), `config_mode.rs`, plus per-site unit tests
- `tests/hook.rs` held still under a fully parallel suite: an `RwLock` separates the binary copy from sibling forks (ETXTBSY), and the group-kill test aims its kill as soon as the ingest is named

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 28fc4df | Owner-only creation primitive, 5 unit tests |
| 1 | 2 | 6967f59 | Data dir and `verbatim.db` created owner-only in `Store::open` |
| 1 | 3 | 24c4e8d | Tighten a wide store once, on a writable open only |
| 1 | 4 | aad0da4 | `tighten.rs`: repair and read paths proven at the process boundary |
| 2 | 1 | 9ce40c0 | Snapshot directory and file owner-only at creation |
| 2 | 2 | 9abe7d9 | Data directory and `LOCK` owner-only from `try_acquire` |
| 2 | 3 | 5fbaf5d | Injection scratch and decision log owner-only |
| 3 | 1 | 9e58ad1 | Export directory and files owner-only |
| 3 | 2 | 57b367f | `data move` writes the moved tree owner-only |
| 3 | 3 | 671b54f | Umask-022 test over the whole AC1 list, fourteen paths |
| 4 | 1 | 93c1fd2 | `Config` carries the file it was loaded from |
| 4 | 2 | 2dfa97c | Refuse a group-readable `verbatim.toml` where its key is consumed |
| 4 | 3 | fece81a | Doctor reports `verbatim.toml`'s mode and what the checks cover |
| 4 | 4 | 1c05044 | Doctor notes a local provider not on loopback |
| 4 | repair | b43e5c0 | `tests/hook.rs` held still under a fully parallel suite |

## Deviations

- [deviation] Plan 2: every task's `Verify:` ran zero of the tests it named (tasks 1 and 2 omitted `--features testkit` behind `#![cfg(feature = "testkit")]`; task 3 passed `inject::decision` as a name filter to the wrong binary). Confirmed the vacuous green, then verified with the feature added and `--lib inject::decision` run separately. No code change.
- [deviation] Plan 3 task 2: `--test owner_only data_move` selected nothing; the test was renamed `a_data_move_writes_an_owner_only_tree_however_wide_the_source_was` so the command as written selects it (57b367f). `--test datamove` also needed `--features testkit`.
- [deviation] Plan 3 task 3: the two acceptance clauses (each path asserted separately / the pre-fix run names at least three paths) conflicted under fail-fast; the end-to-end test now collects every path's complaint and fails once naming all fourteen (671b54f).
- [deviation] Plan 4: the executor checkpointed `suite-red` on `tests/hook.rs`, outside its lease. The coordinator widened PLAN-4's `files:` to that file and dispatched a continuation, which diagnosed both flakes as timing (3232 ETXTBSY over 84,263 execs with copy and fork overlapping, 0 with the lock) and fixed them without loosening any budget (b43e5c0).

## Open items

- `tighten.rs` runs `verbatim doctor` without an exit-code assertion: a bench with nothing installed into Claude Code reports a Problem and exits 1. The test asserts what AC4 is about (no mode or ctime change); doctor's verdict stays with `tests/doctor.rs`.
- Plan 4 task 4's `Verify:` names `-p verbatim-core --test provider` without `--features testkit`; the named test is behind that feature. Run both ways: 1 passed as written, 17 with the feature.
- The `risk_surface` fire on plan 4 raised one `high` (refuted: `Config::load_from` stores the joined `verbatim.toml` path, `config.rs:598` and `:616`). `issue-filing.mjs unfixed` refused with `no-forge` again (`git.forge_provider` and `git.forge_repo` unset), as in phase 2; the ruling is in `ADJUDICATION-risk_surface-plan-4.json`.
- `.planning/reads.jsonl` and `ADJUDICATION-plan-cad-plan-3e09d9a.json` were left modified/untracked by the planning session before this run; the adjudication record is staged with this summary, `reads.jsonl` is not.

## Goal check

The fifteen commits plausibly deliver the goal. The one creation primitive (28fc4df) is used by every writer the roadmap names and the three D-10 adds: store (6967f59), snapshots, `LOCK`, injection and decisions (9ce40c0, 9abe7d9, 5fbaf5d), export and `data move` (9e58ad1, 57b367f). Plan 3's report shows `grep -rn set_permissions` over both `src` trees, minus `cmd/install/` and tests, lists only `store/open.rs:475`, the tighten-on-open repair (24c4e8d), and the umask-022 process-boundary test (671b54f) checks all fourteen paths and failed on every one against `3e09d9a`. The `verbatim.toml` refusal matches the credentials file's shape (2dfa97c) and doctor says what it covers (fece81a). Suites: plan 1 776 passed, plan 3 783 passed, plan 4 64 targets green including `hook.rs` after b43e5c0. Not covered by design: Windows modes (D-15, stated in words), and a reviewer's `high` was refuted rather than fixed, so nothing in the range was changed by the review.
