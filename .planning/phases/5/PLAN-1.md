---
phase: 5
plan: 1
requirements: [INST-13]
files:
  - install.sh
  - crates/terminus/tests/install_script.rs
---

# Phase 5: Install from GitHub Releases - Plan 1

## Goal

`install.sh` downloads from the GitHub releases on crenshawdev/terminus by default, so the whole install is one curl line piped into `sh`. A release carries two flat files per platform, `terminus-<os>-<arch>` and `terminus-<os>-<arch>.manifest`. This plan changes the script and proves it under `/bin/sh`. Plan 2 runs the same tests under dash and updates the docs.

## Must be true when done

- **No argument means the GitHub releases.** Run with no argument, both as `sh install.sh` and with the script piped into `sh`, the script asks for exactly these two URLs, in this order, and the install finishes:
  - `https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<os>-<arch>.manifest`
  - `https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<os>-<arch>`

  "Finishes" means the run exits 0 and the stable path links to the manifest's version. A fake `curl` proves this, and no test reaches github.com.
- **One https argument replaces the default.** The script piped into `sh -s -- https://<local server>` installs from that server's flat names. An `http://` argument and a run with two arguments are each refused. A refusal exits non-zero, leaves the stable path and the versions folder unchanged, and starts no `claude`.
- **A failed download is refused by name.** When the manifest download fails, the script exits non-zero with `manifest download failed: <url>`. When the binary download fails, it exits non-zero with `binary download failed: <url>`. In both cases nothing changes.
- **A cut-off pipe runs nothing.** A copy of `install.sh` cut off at any line break before its last line, piped into `sh`, runs no command from the install and changes nothing under HOME.
- **Phase 4 still holds.** The Phase 4 tests pass against the flat names, so a SHA-256 mismatch is still refused before anything is placed. `sh -n install.sh` passes, `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- This is plan 1 of 2. Plan 2 runs after it, because both plans touch `install.sh` and `crates/terminus/tests/install_script.rs`.
- Decisions that bind this plan are D-01 through D-11 in `.planning/phases/5/CONTEXT.md`. Read them all before starting. D-12 (dash) and D-13 (docs) belong to plan 2.
- The test harness patterns to follow are all in `install_script.rs`. `Fixture::uname_tools` writes a fake tool and swaps it into a tools folder through `Fixture::tools_folder`. `Fixture::assert_refused` checks the exit status, the snapshot and the `claude` calls.
- dash is not installed on this machine (D-12), so this plan's tests run under `/bin/sh`, which is bash 5.3 here.
- Out of scope: publishing a release, release CI, signing and the signature check, the README (phase 6), Baley's `install.sh` (D-14), the workspace version (0.1.1) and qualifying macOS.
- No new dependency. `Cargo.toml` and `Cargo.lock` stay untouched. Run `rustfmt --edition 2021` on the Rust files you touch, and only on those.

## Tasks

### Task 1: Download the flat release asset names

- **Files:** install.sh (anchor: the two `curl` lines after `trap cleanup 0`), crates/terminus/tests/install_script.rs (anchor: `fixture`, `Fixture::serve`, `Fixture::serve_manifest`, `shasum_computes_the_digest_when_sha256sum_is_missing`, `a_digest_that_does_not_match_is_refused`)
- **Action:**
  - **The two URLs (D-01, D-06).** The manifest download becomes `$source/terminus-$os-$arch.manifest` and the binary download becomes `$source/terminus-$os-$arch`. Each `fail` message names the URL it tried, as it does today.
  - **Everything else in the script stays as Phase 4 built it (D-06).** That covers:
    - the `os`/`arch` mapping
    - the curl flags `-q --proto '=https' --tlsv1.2 -fsSL`, where `-L` follows GitHub's 302 from `releases/latest/download/<asset>`
    - the scratch names `$scratch/manifest` and `$scratch/terminus`
    - the manifest parsing, `verify_download`, staging, the link and the final `"$stable" install`
  - **The served layout in the tests (D-08).** `served/terminus-<platform>` and `served/terminus-<platform>.manifest` replace `served/<platform>/terminus` and `served/<platform>/manifest`. `platform()` is unchanged. This touches:
    - `fixture`, which no longer makes a platform subfolder
    - `serve` and `serve_manifest`
    - the two tests that build the binary path inline
    - the doc comment on the `served` field
  - **The Phase 4 tests otherwise stay as they are.** They still run real `curl` against `openssl s_server -WWW`.
  - **Why the existing tests are enough to prove the names.** `s_server -WWW` answers a missing file with `200 ok` and an error text (`.planning/phases/4/SUMMARY.md`, phase 4). A script still asking for the folder layout therefore gets that text as its manifest and is refused, so every install test fails.
- **Verify:**
  - `cargo test -p terminus --test install_script` passes all 14 tests.
  - `grep -n 'os-\$arch/' install.sh` prints nothing.
  - `grep -n 'terminus-\$os-\$arch' install.sh` shows the two `curl` lines and their two `fail` messages.
  - `sh -n install.sh` exits 0.

### Task 2: Make the source optional, defaulting to the GitHub releases

- **Files:** install.sh (anchor: the argument check before the `TERMINUS_BIN_DIR` check, and the header comment), crates/terminus/tests/install_script.rs (anchor: `Fixture::command`, `Fixture::command_with`, `Fixture::run`, `Fixture::run_with`, `Fixture::uname_tools`, `an_http_source_is_refused`)
- **Action:**
  - **Arguments (D-02).**
    - Check the argument count before `$1` is read, because `set -eu` aborts on an unset `$1`.
    - With no argument, the source is `https://github.com/crenshawdev/terminus/releases/latest/download`, written into `install.sh` once (D-01).
    - With exactly one argument, it must start with `https://`. The existing loop strips its trailing slashes, and it replaces the default.
    - Two or more arguments are refused with the usage line.
    - Add no environment-variable override. D-02 rules it out.
  - **Usage line (D-03).** Show the source as optional, for example in square brackets. Today's `usage: sh install.sh https://<source>` names a required argument that no longer exists. Leave every other message alone; the script uses no `$0`, so piping changes nothing else.
  - **Header comment.** Add that with no argument the script downloads from the GitHub releases on crenshawdev/terminus. Keep the existing lines about unsigned artifacts, the later signature check and the final `terminus install`.
  - **Harness.** The command builders take the script's arguments as a list, so a test can pass none, one or two. Existing tests keep passing the server URL as their one argument.
  - **Fake `curl` (D-10).** Follow the fake `uname` pattern: write a script into the fixture root and swap it in as `curl` through `tools_folder`'s swap list.
    - It appends every URL it is asked for, one per line, to a log in the fixture root.
    - It copies the file named by the URL's last path segment from the served folder to its `-o` path, then exits 0.
  - **Constraints on the fake, and why.**
    - It runs on the script's restricted `PATH`: the fake `claude` folder, then the tool links. Any program it needs beyond shell builtins is named by absolute path.
    - It must not read its stdin. In Task 4's piped runs, stdin is the rest of the script (D-05).
    - It never reaches the network. Every no-argument run in this file uses it, so no test reaches github.com.
  - **New test: no argument, run as a file.** Run `sh install.sh` with no argument and the fake `curl`. Assert:
    - the run exits 0
    - `assert_linked` holds for the served version
    - the fake's log is exactly two lines, in this order: `https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<platform>.manifest`, then `https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<platform>`

    Spell the default URL literally in the test rather than reading it from `install.sh`, so a typo on either side fails.
  - **New test: two arguments (D-09).** Pass the server URL twice. Refuse through `assert_refused`, after a first install, the way `an_http_source_is_refused` is set up. `an_http_source_is_refused` stays as the non-https case.
  - **Tests that pass an `https://` source** stay on real `curl` and `s_server` (D-10).
- **Verify:**
  - `cargo test -p terminus --test install_script` passes, including both new tests.
  - Change one character of the default URL in `install.sh`, and the no-argument test fails and shows the URLs the fake logged. Revert the change.
  - From the repo root, `sh install.sh https://a.invalid https://b.invalid` exits 1. It prints the usage line with the source marked optional, and nothing else, because the count is checked first.
  - `sh -n install.sh` exits 0.

### Task 3: Test both download refusals on the default source

- **Files:** crates/terminus/tests/install_script.rs (anchor: the fake `curl` from Task 2, `Fixture::assert_refused`, the module doc comment)
- **Action:**
  - **No change to `install.sh` (D-11).** The two refusals, `manifest download failed: <url>` and `binary download failed: <url>`, already exist. Phase 4 left them untested because `s_server` answers a missing file with `200 ok` (`.planning/phases/4/SUMMARY.md`, phase 4).
  - **The fake `curl` can fail one named file.** For that file it exits 22, which is curl's status for an HTTP error under `-f`. It writes nothing to its `-o` path, and it still logs the URL.
  - **Two new tests (D-11).** Each starts by installing 1.0.0 from the real server, serves 2.0.0, and then runs with no argument and the failing fake.
    - **Manifest fails:** the run is refused, and stderr contains `manifest download failed: https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<platform>.manifest`.
    - **Binary fails:** the run is refused, and stderr contains `binary download failed: https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<platform>`.

    "Refused" means every check `assert_refused` makes: a non-zero exit, the stable path, the versions folder, `settings.json` and `.claude.json` unchanged, and no `claude` call. Let `assert_refused` hand back the output, or check stderr beside it; that is your call. A 404 on the default is what the one-line install hits today, before a release exists (D-07).
  - **Module doc.** Rewrite the paragraph about `s_server` answering a missing file with `200 ok`. It should say that every real-server test serves both files for that reason, and that the download refusals use the fake `curl` instead.
- **Verify:**
  - `cargo test -p terminus --test install_script` passes, including both new tests.
  - In `install.sh`, make the manifest `fail` message name the binary URL, and the manifest test fails. Revert the change.

### Task 4: Wrap the script body in one function, and test the piped forms

- **Files:** install.sh (anchor: everything from the argument check through `"$stable" install`), crates/terminus/tests/install_script.rs (anchor: the command builders from Task 2)
- **Action:**
  - **One function (D-04).** Move the body into one function: everything from the argument check through `"$stable" install`. Call it with `"$@"` on the last line. Name it as you see fit. Only these stay above it:
    - the shebang and the header comment
    - `set -eu`, and `LC_ALL=C` exported
    - the `marker=` assignment
    - the helper functions `fail`, `valid_utf8`, `valid_version`, `digest`, `verify_download`, `check_stable_path` and `stage`
  - **Constraints, and why.**
    - **The `marker=` line stays at column 0, outside the function.** `install_sh_holds_the_same_marker` in `crates/terminus/src/cmd/install/binary.rs` reads lines that start with `marker=`. An indented copy would fail it.
    - **The last line is the call as a plain command.** Never put it inside `if`, after `!`, or in an `&&`/`||` list. POSIX `sh` ignores `set -e` for the whole function when it is called in such a context, and failures inside would stop aborting the script.
    - **No `local`.** It is not POSIX, and `verify_download`, `stage` and `cleanup` read `scratch`, `sha256`, `versions`, `version`, `staged`, `staging_temporary` and `link_temporary` as globals the body sets.
    - **Both traps are still set before the downloads.** That means `trap cleanup 0` and `trap 'exit 1' HUP INT TERM`. The exit trap is shell-wide, so it still fires when the script exits.
    - **The body moves without other changes.** Re-indenting it is fine.
  - **Header comment.** Say why the body is a function. Without it, a download cut off at a line break runs the part that arrived: cut after the link moves, the new version is linked, `terminus install` never runs, and `sh` exits 0.
  - **Stdin (D-05).** Nothing in the run reads stdin, so the piped form needs no stdin handling.
  - **Harness.** The harness can feed the script to the shell through a pipe, as `curl | sh` does: it writes the script's bytes into the child's stdin pipe and then closes it. Do not open the file on stdin instead, because a shell reads a pipe differently from a seekable file.
  - **New test: no argument, piped.** Feed the script through a pipe to `sh` with no arguments, using the fake `curl`. Assert:
    - the run exits 0
    - `assert_linked` holds
    - the fake's log is exactly the two default URLs, in order

    This is the piped half of AC1.
  - **New test: one argument, piped (AC2).** Feed the script through a pipe to `sh -s -- <server url>`, using real `curl` and `s_server`. Assert that the run exits 0 and `assert_linked` holds for the served version.
  - **AC4 has no committed test.** D-09 lists the new tests, and a cut-off test is not among them. The cut-off check below is run once.
- **Verify:**
  - `cargo test -p terminus --test install_script` passes, including both new tests.
  - `cargo test -p terminus install_sh_holds_the_same_marker` passes.
  - `tail -n 1 install.sh` shows the function call with `"$@"` and nothing else.
  - **The cut-off check (AC4).** Make an empty scratch folder with `h=$(mktemp -d)`, so `h` is an absolute path. A relative `h` would let a broken wrapper stop at the relative-HOME refusal before it reaches `uname`, and the check would pass. For every n from 1 to one less than `wc -l < install.sh`, run `head -n "$n" install.sh | env -i HOME="$h" PATH=/usr/bin:/bin /bin/sh -x -s -- https://127.0.0.1:9`, saving stderr. When the loop finishes:
    - `h` is still empty.
    - No `+` trace line in any run names `uname`, `mktemp` or `curl`.

    The trace check is what makes this falsifiable. Without the function, a copy cut after the argument lines traces `uname -s`. The closed local port keeps a broken wrapper off the network.
  - `sh -n install.sh` exits 0.
  - `cargo test --workspace --all-features` passes.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **A departure from the Plan shape directive.** CONTEXT asks for one plan. The phase has six single-concern tasks, and the ceiling is four per plan. This plan holds the four script tasks, and plan 2 holds dash and the docs. Both plans touch `install.sh` and `install_script.rs`, so they run in order and not in parallel, as phase 4's did.
- **AC4 is checked once, not by a committed test.** That follows D-09's list of new tests. If the owner wants the cut-off check kept as a regression test, it is a small addition, and it is his call.
- **Prior art.** `.planning/phases/4/UAT.md` (phase 4) records the four refusals the Phase 4 tests guard: bad checksum, unsupported system, unsupported architecture and an `http://` source. Task 1 keeps those tests running against the flat names. `.planning/phases/4/SUMMARY.md` (phase 4) records why `s_server` cannot fail a download, which is why Tasks 2 and 3 use a fake `curl`.
