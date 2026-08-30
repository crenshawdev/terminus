# Phase 3: Owner-Only On Disk - Context

Gathered: 2026-08-30
Feeds: /cad-plan 3

## Scope boundary

In: Setting an owner-only mode AT CREATION on every file and directory
verbatim writes - the data dir, `verbatim.db` and its `-wal`/`-shm`, `LOCK`,
the snapshots dir and its files, the `injection/` and `decisions/` scratch
dirs and their files, every `.tmp`, the tree `verbatim data move` copies, and
the export dir and its `*.jsonl`; a one-shot tightening repair on `Store::open`
for a store an earlier build left wide; a mode refusal for a `verbatim.toml`
carrying `provider.api_key`, shaped like the shared credentials file's; and two
`verbatim doctor` additions - a runnable `chmod` per credential refusal with the
D-15 Windows deferral stated in words on every platform, and a non-refusing note
when `provider.local = true` meets a non-loopback `base_url`.

Out: Any change to the shared credentials file's read-and-refuse behaviour - it
is shared with every other product on the machine and this module refuses to
widen or narrow it silently; Windows ACL enforcement (D-15, deferred in v0.1.0
phase 7 and unchanged here); the egress rule set and the recall projections,
which are phases 2 and 4; any change to where bytes go, which D-13 fixes to the
`local` declaration and not to the address.

Deferred: None.

Plan shape: multiple plans, same phase - the mode-at-creation sites (seven
files), the tighten-on-open repair (`store/open.rs`), and the config refusal
plus the two doctor checks (`credentials.rs`, `config.rs`, `cmd/doctor.rs`) are
separable leases over different files, the same shape phases 1 and 2 used.

## Durable decisions

- D-01 (Mechanism): Modes are set with `std` alone -
  `std::os::unix::fs::OpenOptionsExt::mode` and `DirBuilderExt::mode` behind
  `#[cfg(unix)]` - and no `libc`, `nix` or `windows-sys` dependency is added;
  the Windows arm is a no-op that D-15 states in words. Carries forward v0.1.0
  phase 4 D-04. Evidence: `.planning/_archive-v0.1.0/4/CONTEXT.md` D-04,
  `crates/verbatim/src/cmd/doctor.rs:836-838` ("the mode's write bits, which is
  what `std` offers without a `libc` dependency (D-04)"), `Cargo.toml:17-70`
  (every dependency carries a startup-cost justification),
  `crates/verbatim/src/cmd/install/binary.rs:231`. If wrong: a `libc` crate
  lands on the hook path against a measured 0.408 ms startup floor, and
  `crates/verbatim-core/src/lib.rs`'s own "links no async runtime, no HTTP
  client and no thread pool" claim gains a caveat it did not have.

- D-02 (Mechanism): `verbatim.db` gets 0600 by CREATING THE FILE ITSELF at 0600
  before rusqlite opens it; `-wal` and `-shm` are left to SQLite, which gives
  them the main database's mode. Measured 2026-08-30 on this machine: with
  `umask 022`, `install -m 600 /dev/null pre.db` then sqlite3 3.53.4
  `PRAGMA journal_mode=wal` plus an insert left `pre.db`, `pre.db-wal` and
  `pre.db-shm` all `-rw-------`, while a database SQLite created itself came out
  `-rw-r--r--`; with the real build (`target/debug/verbatim`, bundled
  libsqlite3-sys 0.38.2) a pre-created zero-length 0600 `verbatim.db` in a 0700
  data dir was accepted as `StoreState::Fresh`, initialized, and left at 0600,
  with `status` and `verify` both passing over it. Baseline for the same build
  under umask 022: data dir 755, `verbatim.db` 644, `LOCK` 644, `snapshots/`
  755, snapshot file 644. Evidence:
  `crates/verbatim-core/src/store/open.rs:111-126` (`create_dir_all`, then
  `Connection::open`), the zero-length-is-Fresh rule at `:192-196`. If wrong:
  `-wal` and `-shm` land 644 while `verbatim.db` is 0600, and AC1 fails on
  exactly the two files that hold uncommitted session text.

- D-03 (Mechanism): Snapshot files get 0600 the same way - the `.tmp`
  destination is pre-created EMPTY at 0600 and `VACUUM INTO` writes into it,
  never chmod'd after the copy. Measured 2026-08-30, sqlite3 3.53.4:
  `VACUUM INTO` onto a pre-created zero-length 0600 file succeeded, produced a
  readable database and kept mode 0600; onto a NON-EMPTY file it failed with
  `file is not a database`; onto a path it created itself it produced 0644 under
  umask 022 regardless of the source database's mode. This partly contradicts
  the module's own comment at `crates/verbatim-core/src/store/snapshot.rs:73-77`
  ("`VACUUM INTO` refuses a file that already exists"), which is true only for a
  non-empty file - and the code already unlinks a stale temp at `:75-77`, so
  that comment is corrected in place by this phase. If wrong: snapshots are
  copies of the whole archive sitting at 644 - the largest single leak in the
  phase - and the only fallback is the chmod-after-copy window AC2 bars.

- D-04 (Mechanism): The directory mode is applied to the LEAF directory only,
  never to every component `create_dir_all` happens to create. Evidence: six
  `create_dir_all` sites, all of which may create parents -
  `crates/verbatim-core/src/store/open.rs:112`,
  `crates/verbatim-core/src/ingest/lock.rs:67`,
  `crates/verbatim-core/src/store/snapshot.rs:68`,
  `crates/verbatim-core/src/inject/state.rs:271`,
  `crates/verbatim-core/src/inject/decision.rs:278`,
  `crates/verbatim/src/cmd/export.rs:160`. If wrong: a user whose
  `~/.local/share` did not exist finds it 0700 after the first ingest, and every
  other tool's data under it becomes unreadable to their own group-shared
  workflows.

- D-05 (Repair): The tightening runs ONLY in `Store::open`, never in
  `Store::open_read_only`, so `doctor`, `search`, `show` and `replay` still
  repair nothing. Carries forward v0.1.0 phase 4 D-12 and INST-06. Evidence:
  `.planning/_archive-v0.1.0/4/CONTEXT.md` D-12,
  `crates/verbatim/src/cmd/read.rs:7-9`,
  `crates/verbatim-core/src/store/open.rs:159-186`,
  `crates/verbatim/src/cmd/doctor.rs:654-663`; the writable openers are
  `status.rs:40`, `verify.rs:11`, `compact.rs:84`, `observations.rs:193`,
  `reindex.rs:262,270`, `ingest/pass.rs:190`. If wrong: `verbatim doctor` chmods
  the store it was asked to report on, and INST-06's read-only guarantee -
  already asserted by `read.rs`'s
  `an_empty_data_directory_is_a_reason_and_creates_nothing` - becomes false.

- D-06 (Repair): "Exactly once" means COMPARE-THEN-CHMOD - a stat per known
  path, a chmod only when a bit is wrong - and NOT a `meta` marker key
  recording the repair as done. Chosen by the user this pass over the marker and
  over gating on `predates_this_build()`. It is self-healing: a store widened
  again later is tightened again, and a `data move` or a restore from backup is
  covered where a version gate would miss it. Accepted cost: a handful of `stat`
  calls on every open, including the ingest path. Evidence:
  `crates/verbatim-core/src/reindex.rs:262` and `:270` open the same store twice
  inside one process, so a once-per-process flag would have to survive that;
  `meta_int`/`set_meta_int` at `crates/verbatim-core/src/store/open.rs:334-352`
  with `META_ARCHIVE_FORMAT`/`META_DERIVED_SCHEMA` are the rejected precedent.
  If wrong: the marker path would leave a re-widened store never re-tightened,
  and would itself be a write on a path that already refuses to write during a
  refused open (`open.rs:101-110`).

- D-07 (Refusal): The `verbatim.toml` mode refusal fires where tier 2 is
  CONSUMED - `credentials::resolve` - and not in `Config::load_from`, so a
  wide-mode `verbatim.toml` with judgment off, or with the key supplied through
  the environment, still loads. Evidence:
  `crates/verbatim-core/src/credentials.rs:236-278` states exactly this rule for
  the shared file ("The file is only opened when the two tiers above it came up
  empty ... A `0644` file therefore yields `None` and not `Error::TooOpen` while
  judgment is off"); `crates/verbatim-core/src/config.rs:587-603` `load_from` is
  called by every command including `doctor`
  (`crates/verbatim/src/cmd/doctor.rs:666`). If wrong: a 644 `verbatim.toml`
  carrying a stale `api_key` makes `verbatim status`, `verbatim search`, the
  hook and `verbatim doctor` itself all fail to read their config - doctor's
  `config_roots` check goes Problem and it can no longer report the mode it is
  refusing over.

## Decisions

- D-08 (Mechanism): The umask-022 test drives the real binary through
  `sh -c 'umask 022; ...'` under `#[cfg(unix)]` rather than calling `umask()`
  in-process. Evidence: `crates/verbatim/tests/doctor.rs:189-207` already has a
  `#[cfg(unix)] fn shell(&self, line: &str)` that runs a printed command through
  `sh`; every other binary test spawns `env!("CARGO_BIN_EXE_verbatim")`
  (`tests/cli.rs:56`, `tests/lifecycle.rs:60`); no `libc` dev-dependency exists
  in `crates/verbatim/Cargo.toml`. If wrong: the test sets no umask at all and
  passes on a developer machine whose umask is already 077 - the exact false
  green AC1 was written against.

- D-09 (Scope): AC1's "scratch dir" is `injection/`
  (`inject::state::DIR_NAME`), and the sibling `decisions/` directory is in
  scope on the same grounds even though the roadmap does not name it. Both hold
  quoted prompt text and session ids, and both are created by a bare
  `create_dir_all` and written through `OpenOptions::create_new` with no mode.
  Evidence: `crates/verbatim-core/src/inject/state.rs:42` ("one session's
  disposable scratch", `:22`, `:264`, `:271`, `:308`),
  `crates/verbatim-core/src/inject/decision.rs:49`, `:278`, `:298`, `:350`. If
  wrong: the injected-brief text and per-prompt decision records ship at 644 in
  a 0700 parent - not world-reachable, but AC1's own list is unmet and a
  `data move` to a wider location re-exposes them.

- D-10 (Scope): Three more writers the roadmap goal does not name are in scope
  because they are files verbatim creates in its own data directory: `LOCK`,
  every `.tmp` temporary (inject state, decision, snapshot), and the tree
  `verbatim data move` copies. Measured 2026-08-30: `LOCK` is 644 after a real
  ingest. Evidence: `crates/verbatim-core/src/ingest/lock.rs:73-79` (no mode),
  `crates/verbatim/src/cmd/data.rs:152` (`File::create` for the destination
  lock), `:254`, `:289` (`copy_tree` `create_dir_all`, then `std::fs::copy`,
  which reproduces the source mode). If wrong: `data move` is the one command
  that reads every mode from the source and writes it to a new location, so a
  store tightened on open is re-widened by the copy path, and the destination
  directory itself lands 755.

- D-11 (Scope): The export destination is treated as verbatim's own output and
  forced to 0700/0600 even though the user names the path and may name a
  removable or network filesystem. Evidence:
  `crates/verbatim/src/cmd/export.rs:160-165` (`create_dir_all`, no mode),
  `:196` (`std::fs::write` per session, no mode), `:145-152` (a non-empty
  destination is refused, so the directory is always one this command created).
  If wrong: a mode-setting failure on a filesystem with no mode bits (exFAT
  backup drive, SMB mount) turns a successful export into an error - the one
  command whose whole purpose is writing somewhere else.

- D-12 (Repair): A chmod that fails is NOT fatal - the open still succeeds,
  because the alternative is an ingest that stops on a store whose files another
  account owns. Evidence: `crates/verbatim-core/src/store/open.rs:186-206`
  treats every non-NotFound failure as an inspectable variant rather than a
  panic; `crates/verbatim-core/src/inject/state.rs:270-286` is this codebase's
  pattern for best-effort filesystem work on the hook path. If wrong: a store on
  a read-only mount, or one whose files are owned by root after a `sudo` run,
  stops opening at all - and AC3's "`verbatim verify` exits 0 over it" cannot be
  reached.

- D-13 (Refusal): `Config` gains the path it was loaded from, because it carries
  no source path today and the refusal must name the file. Evidence:
  `crates/verbatim-core/src/config.rs:527-540` (the struct: no path field),
  `:587-588` (`load_from` is the only place the path exists), `:611-620`
  (`from_parts` and `Config::default()` build configs with no file at all). If
  wrong: the refusal has to re-resolve `config_dir()` at refusal time and can
  name a different file than the one the values came from - precisely wrong for
  `VERBATIM_CONFIG_DIR` tests and for a config loaded via `load_from`.

- D-14 (Refusal): The refusal reuses the shared file's shape - the same
  `mode & 0o077` test, the same path-and-octal-mode-only payload - but is a
  DISTINCT error case, because the existing `Error::TooOpen` message says "a
  credentials file readable by group or world is refused" and this file is
  verbatim's own config. Evidence:
  `crates/verbatim-core/src/credentials.rs:99-113` (the message text), `:196-219`
  (the mask and its comment on write bits),
  `crates/verbatim-core/src/config.rs:1202-1240` (`withhold_secret_excerpt`:
  `verbatim.toml` already has a weaker redaction rule than the shared file
  because only one key is secret); v0.1.0 phase 7 UAT asserted that no assertion
  over stdout, stderr, `runs.error` or the observations table finds the key's
  value. If wrong: a user with a 644 `verbatim.toml` is told a "credentials
  file" is at fault and goes looking for
  `~/.config/jcrenshaw/credentials.toml`, which may not exist.

- D-15 (Doctor): Doctor's two new findings are new checks with their own stable
  `--json` names, and the loopback mismatch is `State::Note`, not
  `State::Problem`, so `verbatim doctor` still exits 0. Evidence:
  `crates/verbatim/src/cmd/doctor.rs:139-166` (any `Problem` makes the exit
  non-zero via `Failure::Silent`), `:112-137` (`Check` + `with_fix`), `:722-776`
  (the credentials check is the template, including the D-15 `Unchecked`
  wording); `crates/verbatim/tests/doctor.rs:695-727` asserts name parity
  between the human report and `--json` but enumerates no fixed name list, so
  new checks do not break it. If wrong: a user running `provider.local = true`
  against a remote URL gets a non-zero `doctor`, which in a CI health check
  reads as a broken install rather than a likely mistake, and D-13's "the
  declaration decides, not the address" is contradicted by the exit code.

- D-16 (Doctor): Loopback detection is a hand-written host extraction over the
  configured `base_url` STRING, not a new URL-parsing dependency. Evidence:
  `Cargo.toml:62-69` (ureq's `json` feature was dropped because "serde_json is
  already linked and ureq's wrapper would only hide where the parse happens"),
  no `url` crate in `Cargo.lock` (`http` 1.5.0 is present only as a ureq
  transitive), `crates/verbatim-core/src/observe/provider.rs:339-344`
  (`endpoint` already treats `base_url` as a string with one trailing-slash
  rule), and this cycle's phase 2 precedent of hand-written byte scanners rather
  than a regex crate. If wrong: `http://127.0.0.1:11434@evil.example/` or an
  IPv6 `http://[::1]:11434/` is classified the wrong way, and a warning that
  exists to catch a mistake either misses it or fires on a correct local setup.

- D-17 (Doctor): AC6's "each refusal" means the TWO CREDENTIAL refusals - the
  shared file and `verbatim.toml` - and not verbatim's own data files, which are
  repaired on open rather than refused. Doctor states the D-15 Windows deferral
  once, in words that cover both, on every platform. Evidence:
  `crates/verbatim/src/cmd/doctor.rs:759-768` (today the deferral is stated only
  inside the `Unchecked` arm of the credentials check, so it is invisible on
  Unix and on Windows when the file is absent). If wrong: doctor prints a
  `chmod` for `verbatim.db` that the next `Store::open` would have applied
  anyway, telling the user to fix something that fixes itself; or the Windows
  user is left inferring that "ok" on a mode check means an ACL was examined.

## Acceptance criteria

- [ ] AC1: A `#[cfg(unix)]` test drives the real binary under
      `sh -c 'umask 022; ...'` through a fresh ingest, a snapshot and an export,
      then asserts each path separately: data dir 0700; `verbatim.db`, `-wal`,
      `-shm` and `LOCK` 0600; `snapshots/` 0700 with 0600 files; `injection/`
      and `decisions/` 0700 with 0600 files; export dir 0700 with 0600 files.
      The same test against the pre-fix build fails, naming at least the data
      dir, `verbatim.db` and the export files.
- [ ] AC2: No path widens then narrows. `set_permissions` appears in exactly one
      place in the build - the tighten-on-open repair - and in none of the
      creation paths in `store/open.rs`, `store/snapshot.rs`, `inject/state.rs`,
      `inject/decision.rs`, `ingest/lock.rs`, `cmd/data.rs` or `cmd/export.rs`,
      each of which passes the mode at creation.
- [ ] AC3: A store whose paths the test has chmod'd to 755/644 is opened once
      through `Store::open`; afterwards every path in AC1's list is owner-only,
      a second `Store::open` changes no file's ctime, and `verbatim verify`
      exits 0 over it.
- [ ] AC4: `verbatim doctor`, `verbatim search` and `verbatim show` over that
      same wide store leave every mode and every ctime unchanged.
- [ ] AC5: With judgment on so tier 2 is consumed, a `verbatim.toml` at 0644
      carrying `provider.api_key` is refused with a message naming the path and
      the octal mode, and the key's value appears in no byte of stdout, stderr
      or `runs.error`. The same file at 0600 is accepted. With the key absent,
      or supplied through the environment, a 0644 `verbatim.toml` still loads
      and `verbatim status` succeeds.
- [ ] AC6: `verbatim doctor` against a 0644 `verbatim.toml` and a 0644 shared
      credentials file prints a runnable `chmod` line for each, and prints the
      Windows ACL deferral in words on this Linux machine - not only inside the
      Windows `Unchecked` arm.
- [ ] AC7: With `provider.local = true` and a non-loopback `base_url`,
      `verbatim doctor` prints a note and exits 0; with a loopback `base_url` it
      prints no such note. The existing D-13 provider test is unchanged and
      still passes, so no bytes move differently.

## Flagged assumptions

- SQLite gives `-wal` and `-shm` the main database's mode, so pre-creating
  `verbatim.db` at 0600 covers all three (D-02) - Likely; measured only here, on
  sqlite3 3.53.4 and bundled libsqlite3-sys 0.38.2. Whether it holds on macOS
  and across every version the `bundled` feature may resolve to is a question
  about SQLite's source, not this codebase. The user's call this pass: leave it
  flagged and let AC1 assert the two modes directly, so a SQLite that stops
  inheriting fails loudly on whatever platform runs the test. If wrong: the
  phase passes on Linux and ships 644 WAL files on another platform.
- The Windows equivalent of owner-only for these same files is an ACL, not a
  mode bit, and stays deferred under D-15 - Confident; nothing in the repo can
  say what the minimum correct ACL is. If wrong: a Windows user reads doctor's
  "ok" as a statement about their ACLs when no ACL was examined - which is why
  D-17 makes doctor say so in words.
