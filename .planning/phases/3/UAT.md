---
status: testing
phase: 3
fields_version: 1
started: 2026-09-04
updated: 2026-09-04
---

## Items

### 1. Cold start from an empty data dir
expected: With a fresh VERBATIM data dir under umask 022, a first ingest creates the store, `verbatim status` succeeds and a search returns real turns.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Ran the shipped binary under `sh -c 'umask 022; ...'` against a fresh temp data dir: ingest exit 0, status --json sessions 1 / turns 8, search returned a hit, and the whole data dir landed 0700/0600 (data, snapshots, verbatim.db, -wal, -shm, LOCK).

### 2. Umask-022 process test covers all fourteen paths
expected: A #[cfg(unix)] test drives the real binary under `sh -c 'umask 022; ...'` through ingest, snapshot and export and asserts each path: data dir 0700; verbatim.db, -wal, -shm, LOCK 0600; snapshots/ 0700 with 0600 files; injection/ and decisions/ 0700 with 0600 files; export dir 0700 with 0600 files. Against the pre-fix build it fails naming at least the data dir, verbatim.db and the export files.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim/tests/owner_only.rs:105-131 sets the umask for the spawn (never in-process) and :455-469 asserts all fourteen paths in one collected message; test passes on HEAD. Falsified against the pre-fix build: 3e09d9a built in a throwaway worktree with this test copied in FAILED, naming all fourteen at 755/644 including the data dir, verbatim.db and both export files.

### 3. No widen-then-narrow: set_permissions only in the repair
expected: `set_permissions` appears in exactly one place in the build, the tighten-on-open repair in store/open.rs, and in none of the creation paths in store/open.rs, store/snapshot.rs, inject/state.rs, inject/decision.rs, ingest/lock.rs, cmd/data.rs, cmd/export.rs, each of which passes the mode at creation.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: `grep -rn set_permissions crates/*/src | grep -v cmd/install/` yields only store/open.rs:475 (the repair) and its doc comment at :457; all seven named creation sites pass the mode at creation through owner_only::create_dir_all / owner_only::options.

### 4. Wide store tightened once on Store::open
expected: A store chmod'd to 755/644 is opened once through Store::open; afterwards every AC1 path is owner-only, a second Store::open changes no file's ctime, and `verbatim verify` exits 0 over it.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: tighten() called only from Store::open (store/open.rs:139), compare-then-chmod with symlink_metadata at :466-475; tests/tighten.rs asserts every path owner-only after one `status`, every ctime (with nsec) unmoved on a second `status`, and `verbatim verify` exit 0. Passed.

### 5. Read-only commands repair nothing
expected: `verbatim doctor`, `verbatim search` and `verbatim show` over the same wide store leave every mode and every ctime unchanged.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: open_read_only calls no tighten; tests/tighten.rs:229-251 re-widens then runs doctor --json, search and show and asserts every mode still 755/644 and every ctime unmoved. Passed.

### 6. 0644 verbatim.toml with api_key refused at consumption
expected: With judgment on, a 0644 verbatim.toml carrying provider.api_key is refused with a message naming the path and the octal mode; the key's value appears in no byte of stdout, stderr or runs.error. At 0600 it is accepted. With the key absent or supplied via the environment, a 0644 verbatim.toml still loads and `verbatim status` succeeds.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: Refusal lives in credentials::resolve (credentials.rs:292-325) behind the env tier, not in Config::load_from; distinct Error::ConfigTooOpen wording names path and 3-digit octal mode; tests/config_mode.rs asserts the key value is absent from stdout, stderr and runs.error, that 0600 is accepted silently, and that a keyless or env-supplied 0644 file still loads and status succeeds. 3 tests passed.

### 7. Doctor prints a chmod per credential refusal and the Windows deferral
expected: `verbatim doctor` against a 0644 verbatim.toml and a 0644 shared credentials file prints a runnable chmod line for each, and prints the Windows ACL deferral in words on Linux, not only inside the Windows Unchecked arm.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: doctor.rs:142-145 wires credentials, config_mode and mode_checks; two distinct `chmod 600 '<path>'` fixes; mode_checks is an unconditional Note naming Windows/ACL/credentials/config_mode. tests/doctor.rs:1056-1135 runs both printed fixes through a shell and re-runs doctor to ok, and checks the deferral in report and --json. 26 doctor tests passed.

### 8. Doctor notes a local provider not on loopback
expected: With provider.local = true and a non-loopback base_url, `verbatim doctor` prints a note and exits 0; with a loopback base_url no such note. The existing D-13 provider test is unchanged and passes.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: provider_local emits State::Note (never Problem) so doctor still exits 0; host_of/is_loopback are hand-written, handle userinfo and bracketed IPv6, add no dependency. Loopback, non-loopback and not-declared-local cases all covered by passing tests; core provider suite 17 passed unchanged.

### 9. data move writes the destination tree owner-only
expected: `verbatim data move` to a new location leaves the destination dir 0700 and every copied file 0600 regardless of the source tree's modes.
status: pass
first_pass: pass
source: verifier
evidence: cmd/data.rs:157/306/334 create through owner_only; tests/owner_only.rs:346-372 widens the source to 755/644, moves, and asserts the whole destination tree 0700/0600. Passed, along with the 5 datamove tests.

## Summary

total: 9
passed: 9
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 0
