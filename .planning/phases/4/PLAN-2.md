---
phase: 4
plan: 2
requirements: [INST-12]
files:
  - install.sh
  - crates/terminus/src/cmd/install/binary.rs
  - crates/terminus/tests/install_script.rs
---

# Phase 4: One-Command Installer - Plan 2

## Goal

`sh install.sh https://<source>` downloads the manifest and binary for the machine and checks the SHA-256 in `verify_download` before placing anything. It stores the binary at `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, moves the link at `~/.local/bin/terminus` onto it, and ends by running `terminus install`. All of this is qualified against a local HTTPS server with unsigned development artifacts.

## Must be true when done

- **The script parses and holds the right marker.** `sh -n install.sh` exits 0. A unit test fails when the script's copy of the marker differs from `binary::MARKER` by one character.
- **One run against the local HTTPS server finishes the install.** The server serves `<os>-<arch>/manifest` and `<os>-<arch>/terminus`. One `sh install.sh` run with a fake `claude` that succeeds:
  - exits 0
  - leaves `~/.local/bin/terminus` a link to `~/.local/lib/crenshawdev/terminus/versions/<manifest version>/terminus`, which holds the served bytes
  - merges the four hook entries, each naming the stable path
  - leaves `claude mcp add-json --scope user terminus` in the fake's log, with the stable path as `command`

  With no `claude` on `PATH`, the run exits non-zero after the receipt's "install incomplete" line. Both outcomes hold with `sha256sum`, and with `shasum -a 256` when `sha256sum` is not on `PATH`.
- **Every refusal exits non-zero and changes nothing.** The stable path, the versions folder, `settings.json` and `.claude.json` are byte-identical afterwards, and no `claude` ran. The refusals are:
  - a manifest SHA-256 that does not match the binary
  - an unsupported system from `uname -s`
  - an unsupported architecture from `uname -m`
  - an `http://` source
  - `TERMINUS_BIN_DIR` set
  - a file without the marker at the stable path
  - a dangling link at the stable path that points outside the versions folder
- **Successive installs keep every version.** After installing version A and then version B:
  - the link points at B
  - A's file has the same bytes and inode as before
  - a `terminus mcp` started through the link before the second install still runs from A's file, per `/proc/<pid>/exe`

  A same-version download with different bytes replaces the version file, and the same bytes leave its inode unchanged. A plain file carrying the marker at the stable path is replaced by the link, and so is a dangling link into the versions folder.
- `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- This is plan 2 of 3 and runs after plan 1. The tests here depend on plan 1 task 2: `terminus install` run from the link's own target touches nothing, whatever the version folder is named. The manifests name versions like 1.0.0 and 2.0.0, while every build reports 0.1.1.
- Decisions that bind this plan: D-01, D-03, D-04, D-06 (the script's shape), D-07 (ownership by the marker), D-08 (exit status), D-09 (`TERMINUS_BIN_DIR`), D-10 (testing). The base is `/code/baley/install.sh`. Read all of it before writing.
- **Measured while planning (2026-10-13), which changes how D-10's `PATH` is built:**
  - `/usr/bin/claude` exists. It is a wrapper from the `claude-code 2.1.296-1` package that execs `/opt/claude-code/bin/claude`, and `/bin` links to `usr/bin`.
  - So a `PATH` of the fake's folder "then the system folders" would put the real `claude` within reach. That breaks D-10's own reason ("so it stays unreachable") and testkit's rule that no test starts a real `claude`.
  - The test `PATH` is therefore the fake `claude`'s folder, then a tools folder the harness builds out of links to the system tools the script calls. The tools folder never holds `claude`. This also gives D-10's shaped `PATH`s for the `shasum` fallback and the fake `uname`.
  - Tool facts on this machine: `shasum` is at `/usr/bin/core_perl/shasum`, with shebang `/usr/bin/perl`. `openssl` is 3.6.5, `curl` is 8.22.0, and `/bin/sh` is bash 5.3.
  - Static checking is `sh -n` only (D-10).
- No new dependency. `Cargo.toml` and `Cargo.lock` stay untouched, and the tests compute digests with the system `sha256sum`.
- Out of scope: signing and the signature check, updates, pruning, publishing a release, and qualifying macOS.
- Run `rustfmt --edition 2021` on the Rust files you touch, and only on those.

## Tasks

### Task 1: install.sh in Baley's shape, with Terminus's ownership rule

- **Files:** install.sh, crates/terminus/src/cmd/install/binary.rs (anchor: `MARKER`, `mod tests`)
- **Action:**
  - **Start from Baley's script (D-06).** Copy `/code/baley/install.sh` to the repo root and swap `baley` for `terminus` everywhere:
    - the `fail` prefix
    - the versions folder: `$install_home/.local/lib/crenshawdev/terminus/versions`
    - the stable path: `$install_home/.local/bin/terminus`
    - the downloads: `$source/$os-$arch/manifest` and `$source/$os-$arch/terminus`
  - **Keep the rest as Baley has it:**
    - POSIX `sh`, `set -eu`, and `LC_ALL=C` exported
    - one argument, which must start with `https://`, with trailing slashes stripped
    - the `HOME` checks and the HOME spelling loop
    - the `uname` mapping and its refusal
    - the `curl -q --proto '=https' --tlsv1.2 -fsSL` flags
    - the manifest grammar and its exact-text check
    - `digest`, using `sha256sum` and falling back to `shasum -a 256`
    - `verify_download` before anything is staged
    - `umask 022`, and staging through `mktemp` and `mv`
    - the temporary link and its `readlink` check
    - the list of versions present
    - `"$stable" install` as the last line
  - **The header comment.** It says the script installs unsigned development artifacts only. It says the signature check goes inside `verify_download` with Baley's release design (#14, an issue in Baley's tracker, not Terminus's) in a later cycle, and that the script ends by running `terminus install` through the stable path. Baley's comment on the HOME loop gives "the updater's spelling" as its reason, and Terminus has no updater. Rewrite that comment so its reason is true here.
  - **`TERMINUS_BIN_DIR` (D-09).** Refuse when it is set to a non-empty value, before any download or write. Name the variable and say why: `terminus install` would point the hooks and the MCP entry into that folder while the script links `~/.local/bin/terminus`. Non-empty matches `binary.rs`'s `non_empty_var`. An empty value is the one `terminus install` treats as unset, so the script and the install it runs agree.
  - **Ownership by the marker (D-07).** This replaces Baley's `check_stable_path`, and it runs twice as Baley's does: before the downloads, and again just before `mv -f`.
    - Nothing at the stable path: go on.
    - A folder, or a link to a folder: refused. `mv -f` onto a folder would move the link into it (D-01).
    - A regular file, or a link to a file, that carries the marker: Terminus's, so it is replaced.
    - A dangling link whose `readlink` target has the shape `$versions/<version>/terminus`: Terminus's, so it is replaced. This is Baley's `check_stable_path` rule (`/code/baley/install.sh:74-81`), chosen by the owner on 2026-10-13. It covers a link left behind after the versions folder was deleted.
    - Anything else is refused with a non-zero exit and nothing changed. That covers an unmarked file, a link to an unmarked file, a dangling link pointing anywhere other than the versions folder, and any other kind of entry.
  - **The marker test.** It is `grep -q -F` under the script's `LC_ALL=C`, run on the stable path; grep follows a link. Hold the marker in one assignment on its own line, so a test can read it back. Measure under `sh` only, because the interactive shell's `grep` is a ugrep function that does not find it (D-07).
  - **Staging (D-03).** This replaces Baley's "staging conflict" on different bytes.
    - No version file yet: stage it as Baley does.
    - A regular file with the manifest's SHA-256 that is executable: left alone.
    - A regular file with another SHA-256: the verified download is renamed over it through a temporary in the same folder (`mktemp` then `mv -f`), as a fresh stage is.
    - A link at that path, an entry that is not a regular file, or the right bytes without the executable bit: Baley's staging-conflict refusal.
    - Nothing deletes an old version folder (D-04).
  - **Exit status (D-08).** Nothing follows `"$stable" install`, so its exit status is the script's.
  - **The tie test (D-07).** Add a unit test to `binary.rs`'s `mod tests`. It reads `install.sh` at the workspace root, found from `CARGO_MANIFEST_DIR`, and asserts that the script's marker assignment holds exactly `MARKER`.
- **Verify:**
  - `sh -n install.sh` exits 0.
  - `cargo test -p terminus --bin terminus` passes the tie test. Changing one character of the script's marker makes it fail; change it back afterwards.
  - With `HOME` set to a fresh, empty scratch folder, and never the real home:
    - `sh install.sh http://example.invalid` exits 1 with "the source must start with https://".
    - `TERMINUS_BIN_DIR=/x sh install.sh https://example.invalid` exits 1 with a message naming `TERMINUS_BIN_DIR`.
    - Afterwards the scratch folder is still empty.

### Task 2: An HTTPS harness runs install.sh end to end

- **Files:** crates/terminus/tests/install_script.rs
- **Action:**
  - **The file.** A new integration test file. Its module doc says what it covers and why the harness is shaped the way it is (D-10, plus the `/usr/bin/claude` measurement above).
  - **Per test.**
    - A temporary root, with `HOME` a folder inside it.
    - `CLAUDE_CONFIG_DIR`, `TERMINUS_DATA_DIR` and `TERMINUS_CONFIG_DIR` inside the root. Seed `settings.json` and `.claude.json` the way `tests/install.rs` does. Integration test files share no module, so this file keeps its own copies of the fixture text.
    - `TERMINUS_BIN_DIR` and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` removed from the environment.
    - The fake `claude` from `terminus_core::testkit::fake_claude`.
  - **`PATH`.**
    - First the fake `claude`'s folder. Then a tools folder the harness fills with links to every system tool `install.sh` calls; read the script for the list. The tools folder never holds `claude`.
    - The test process's own `PATH` is never passed on.
    - A helper builds the tools folder without a named tool, or with a named tool swapped for a fake. Task 3's fake `uname` and this task's `shasum` run use it.
  - **The server.** The port and how a test waits for it are the planner's call (CONTEXT, flagged assumptions), and this is the choice:
    - Make a throwaway certificate and key with `openssl req` at test time, for the host the URL uses. Give it a `subjectAltName` matching that host, because curl checks it.
    - Run `openssl s_server -WWW` with its working folder set to the served folder.
    - For the port, bind `127.0.0.1:0`, read the port, release it, and start `s_server` on it.
    - Wait by polling a TCP connect until it is accepted. Fail with the server's stderr after a 10-second deadline.
    - Kill the server when the fixture drops.
    - `CURL_CA_BUNDLE` names the certificate, so the script stays https-only.
  - **What is served.**
    - The script's own mapping of this machine to `<os>-<arch>`: `linux` or `macos`, then `x86_64` or `aarch64`.
    - A binary made from a copy of `CARGO_BIN_EXE_terminus`, with optional bytes appended. Appended bytes give a different digest, and the binary still runs and carries the marker.
    - A manifest that is exactly `version X.Y.Z\nsha256 <hex>\n`, with the hex from the system `sha256sum`.
    - A helper that serves a given version and given bytes, so tasks 3 and 4 can install several.
  - **How the script runs.** `/bin/sh <workspace root>/install.sh https://<host>:<port>`, with stdin null.
  - **Tests (AC3, D-08, D-10).**
    - **A fake `claude` that exits 0.**
      - The run exits 0.
      - The stable path is a link to `versions/<manifest version>/terminus`, and that file holds the served bytes.
      - `settings.json` holds four hook entries whose `command` is the stable path.
      - The fake's log holds `mcp add-json --scope user terminus` with `command` set to the stable path and `args` `["mcp"]`, as phase 3's UAT recorded (phases/3/UAT.md, phase 3).
    - **No `claude` in either folder.**
      - The run exits non-zero.
      - stdout holds "install incomplete: the mcp server is not registered".
      - The link is still placed.
    - **The `shasum` fallback.**
      - With a tools folder that lacks `sha256sum` and holds `shasum`, the exit-0 run still succeeds.
      - The same tools folder with a wrong digest in the manifest fails, which proves `shasum` computed the digest.
      - Find `shasum` on the test process's `PATH` or at `/usr/bin/core_perl/shasum`. When it cannot be found, the test fails with a message naming it. It never skips, because a skipped test would read as a pass.
- **Verify:** `cargo test -p terminus --test install_script` passes, with the three cases above as separate tests. After the run, `pgrep -f "s_server -WWW"` finds no server left behind.

### Task 3: Every refusal leaves the machine untouched

- **Files:** crates/terminus/tests/install_script.rs
- **Action:**
  - **One snapshot helper.** It records:
    - the stable path's kind, with the link's target or the file's bytes
    - every entry under the versions folder, with each file's bytes and inode
    - the bytes of `settings.json` and `.claude.json`
  - **Each refusal test.** Take the snapshot, run the script, and assert a non-zero exit, a non-empty stderr, the same snapshot afterwards, and no new call in the fake `claude`'s log.
  - **The cases.** Each one starts from a completed install of version A, except the unmarked-file case.
    - (AC1) The manifest names a SHA-256 the binary does not have.
    - (AC1) A fake `uname` in the tools folder answers `-s` with `FreeBSD`.
    - (AC1) The fake `uname` answers `-s` with `Linux` and `-m` with `riscv64`.
    - (AC1) The source is `http://`.
    - (D-09) `TERMINUS_BIN_DIR` is set to a folder inside the root.
    - (AC4) The stable path holds a regular file without the marker, and nothing else is installed.
    - (AC4) The stable path is a dangling link to a path outside the versions folder, and nothing else is installed. The link and its target text are unchanged.
- **Verify:** `cargo test -p terminus --test install_script` passes, with each case above as a separate test. Temporarily commenting out the `verify_download` call in `install.sh` makes the SHA-256 case fail, which proves that test can catch a missing check. Restore the line afterwards and confirm `git diff install.sh` shows nothing left over from the check.

### Task 4: Successive installs keep every version

- **Files:** crates/terminus/tests/install_script.rs
- **Action:**
  - **AC2: version A, then version B.**
    - Install A: version 1.0.0, a test-binary copy with one appended suffix.
    - Start `<stable path> mcp` through the link, with stdin piped and held open.
    - Install B: version 2.0.0, with a different suffix.
    - Assert that the link points at `versions/2.0.0/terminus`.
    - Assert that `versions/1.0.0/terminus` has the same bytes and inode as before.
    - Assert that `read_link("/proc/<pid>/exe")` for the `mcp` process equals the canonical path of `versions/1.0.0/terminus`. That assertion is gated to Linux, because macOS is not qualified in this phase.
    - Then close the process's stdin and wait for it to exit.
  - **D-03: the same version twice.**
    - Install version C.
    - Serve version C again with different bytes. Assert that the version file holds the new bytes and the link still points at it.
    - Serve the same bytes once more. Assert that the version file's inode is unchanged.
  - **AC4: a marked plain file.** With a regular-file copy of the test binary at the stable path, like an install from before this phase, the run exits 0 and the stable path ends up as the link to the version file.
  - **A dangling link into the versions folder.** Install version A, delete the versions folder, then install version B. The run exits 0 and the stable path ends up as the link to B's version file.
- **Verify:** `cargo test -p terminus --test install_script` passes with these tests. `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **A deviation from D-10's wording.** D-10's `PATH` of "the fake `claude`'s folder first and then the system folders" is built as the fake's folder followed by a folder of links to the system tools. The reason is the `/usr/bin/claude` measurement in Context. D-10's two stated goals are both kept: coreutils run, and the real `claude` stays unreachable.
- **A dangling link at the stable path.** The owner chose Baley's rule on 2026-10-13. `install.sh` replaces a dangling link that points into the versions folder and refuses one that points anywhere else. `terminus install` still treats any dangling link as `Vacant` (D-05), so the two differ only for a dangling link that points outside the versions folder.
- **macOS stays unqualified.** These assumptions are open, as CONTEXT flags them: `current_exe()` through a link on macOS, BSD `grep -q -F` on a binary file, the BSD behaviour of the tools the script uses, and whether an unsigned download runs.
