---
phase: 6
plan: 1
requirements: [INST-14]
files:
  - install.sh
  - crates/terminus/tests/install_script.rs
---

# Phase 6: Signed Install - Plan 1

## Goal

`install.sh` refuses a release it cannot verify. Before it reads the manifest, it downloads `terminus-<os>-<arch>.manifest.sig` and runs `ssh-keygen -Y verify` against the crenshawdev release key pinned in the script, under the namespace `terminus-release`. The SHA-256 check then runs as before. The key is never fetched and nothing overrides it, so an update, which is a rerun of the same command, gets the same check. This plan changes the script and its tests. Plan 2 changes the CHANGELOG, the design brief and `PROJECT.md`.

## Must be true when done

- **The key is pinned and the signature is fetched with the manifest.** `install.sh` holds `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q` exactly once, as a literal. Run with no argument, it requests these three URLs in this order and no others: `.../terminus-<os>-<arch>.manifest`, `.../terminus-<os>-<arch>.manifest.sig`, `.../terminus-<os>-<arch>`.
- **A bad or missing signature is refused before the manifest is read.** In each of these cases the run exits non-zero with the script's own reason on stderr: the signature is missing, it was made by another key, it was made under the namespace `baley-release`, or the manifest was changed after signing. The binary is never requested, the stable path and the versions folder are unchanged, and no `claude` starts. A missing signature says `signature download failed: <url>`.
- **A machine without `ssh-keygen` is refused before any download.** The message names `openssh-client`, `openssh-clients` and `openssh`, no URL is requested, and nothing under HOME changes.
- **A signed release installs as it did in Phase 5.** Every existing install test passes under `/bin/sh` and dash against a signed release, and nothing `ssh-keygen` prints appears in the run's output.
- **Nothing changes the pinned key.** The unmodified `install.sh` refuses a release signed by a throwaway key, even when `$HOME/.ssh/allowed_signers` lists that key under `crenshawdev-release`.
- **The header comment matches the code.** It describes the signature check, the pinned key and the need for `ssh-keygen`, says "No release is published yet.", and no longer says "unsigned development artifacts only" or that the check goes inside `verify_download` later.
- **The gates hold.** `sh -n install.sh` and `dash -n install.sh` exit 0, `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- Plan 1 of 2. Plan 2 shares no file with this plan and runs after it, so the docs it writes describe a script that already does what they say.
- Decisions D-01 to D-13 in `.planning/phases/6/CONTEXT.md` bind this plan. Read them all before starting. D-14 binds the header comment here. D-15 belongs to plan 2.
- Follow the harness patterns already in `crates/terminus/tests/install_script.rs`. `Fixture::uname_tools` and `Fixture::curl_tools` write a fake tool and swap it in through `Fixture::tools_folder`. `Fixture::assert_refused` checks the exit status, stderr, the snapshot and the `claude` calls. `both_shells!` runs each test as `::sh` and `::dash`.
- `ssh-keygen` (OpenSSH 10.6p1), dash and openssl are installed on this machine, so every check below runs here.
- Out of scope: release CI, the signing script, publishing a release, Baley's `install.sh`, the README, the workspace version (0.1.1), a version check on `ssh-keygen`, qualifying macOS, and `crates/terminus/src/cmd/install/binary.rs`.
- No new dependency, so `Cargo.toml` and `Cargo.lock` stay untouched. Run `rustfmt --edition 2021` on the Rust files you touch, and only on those. CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing the header comment or doc comments; its Code comments rule applies.

## Tasks

### Task 1: Verify the manifest's signature against the pinned key before reading it

- **Files:** install.sh (anchor: the header comment, `marker=`, the two `curl` lines after `trap 'exit 1' HUP INT TERM`, the manifest `read` block), crates/terminus/tests/install_script.rs (anchor: the module doc comment, `TOOLS`, `Fixture`, `fixture`, `script`, `Fixture::serve_manifest`, `Fixture::command_with`, `Fixture::piped`, `default_urls`, `a_failed_binary_download_is_refused`, `one_run_installs_and_wires_claude_code`)
- **Action:**
  - **Pin the key (D-04, D-10).** Assign the public line `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q` as a single-quoted literal at the top level, beside `marker=` and outside the brace group, so the file still ends in `\n}\n` as `a_cut_off_script_runs_nothing` requires. This is the only place the text appears in the file. The header comment names the key without repeating it. No argument, environment variable or file sets or overrides the key, and there is no unsigned mode.
  - **Download order (D-03).** The downloads become: manifest, then `$source/terminus-$os-$arch.manifest.sig` with the same curl flags into the scratch folder, then the signature check, then the binary. A refused release never downloads the binary. A failed signature download fails with `signature download failed: $source/terminus-$os-$arch.manifest.sig`, in the same form as the manifest and binary lines (D-08).
  - **The check (D-01, D-06, D-09).** It is its own step, after the signature download and before the binary download. The manifest `read` block stays where it is and as it is, so the check also runs before the manifest is read.
    - Write the allowed-signers line `crenshawdev-release <pinned public line>` from the script's own literal into a file in `$scratch`. Nothing is read from HOME, `~/.ssh` or any file the user controls.
    - Run `ssh-keygen -Y verify -f <that file> -I crenshawdev-release -n terminus-release -s <the downloaded .sig>` with the manifest redirected to its stdin. `-Y verify` reads the message only from stdin. The redirect also keeps it off the shell's own stdin, which is the script when it is piped.
    - Any non-zero exit fails through `fail`, with a reason saying the manifest does not verify against the pinned key. Do not check `ssh-keygen`'s version.
    - Put no `namespaces=` option on the allowed-signers line. This is the planner's call on the CONTEXT flagged assumption, explained in Notes.
  - **Output (D-07).** Send both `ssh-keygen` streams to `/dev/null`. On a success it prints `Good "terminus-release" signature for ...` on stdout. On a refusal it prints `Could not verify signature.` on stdout, and on a namespace mismatch it also prints `Couldn't verify signature: namespace does not match` on stderr (probe, 2026-10-14). The script's own reason through `fail` is the only refusal text.
  - **What stays (D-01).** `verify_download` stays the SHA-256 check on the binary, and nothing after the manifest `read` block changes.
  - **Header comment (D-14).** Rewrite lines 2 and 5-6. The header says the script verifies the manifest's signature (`terminus-<os>-<arch>.manifest.sig`, namespace `terminus-release`) against the crenshawdev release key pinned in the script before it reads the manifest, and that this needs `ssh-keygen`. Drop "unsigned development artifacts only" and the line saying the check goes inside `verify_download` in a later cycle. Add the sentence "No release is published yet." (AC6, see Notes). Keep lines 3-4 and 7-10 as they are.
  - **Harness: tools (D-12).** `TOOLS` gains `ssh-keygen`. It is found through `which` like the rest, so a machine without it fails the tests instead of skipping them.
  - **Harness: key and script copy (D-02).**
    - Each fixture makes a throwaway Ed25519 key in its root with the system `ssh-keygen`, with no passphrase and stdin at null.
    - Each fixture writes a copy of `install.sh` into its root, with the pinned public line replaced by the throwaway key's type and base64 (its `.pub` line without the comment).
    - Hold the pinned line once, as a constant in the test file. Task 4's source test reuses it.
    - `Fixture::command_with` and `Fixture::piped` run the copy. `script()` keeps naming the unmodified file, for Tasks 3 and 4. `a_cut_off_script_runs_nothing` keeps piping the unmodified bytes, because it tests the shipped file's shape.
  - **Harness: signing (D-13).** `Fixture::serve_manifest`, which `Fixture::serve` calls, signs the manifest it writes. It uses the throwaway key under `terminus-release` and writes `served/terminus-<platform>.manifest.sig`, replacing the earlier signature on every call.
    - Sign from stdin to stdout: the manifest on stdin, and stdout into the `.sig` file.
    - Never run `ssh-keygen -Y sign` on the file path. Once a `.sig` exists it stops at "Overwrite (y/n)?", and the CONTEXT measurement hung until its 120 s timeout.
    - The stdin form prints "Signing data on standard input" on stderr. That is expected.
  - **Harness: URLs (D-03).** `default_urls()` returns three URLs: manifest, `.manifest.sig`, binary. `a_failed_binary_download_is_refused` names the binary URL by its new position. The two no-argument tests then assert the three-URL order, which extends the Phase 5 UAT item for runs with no argument (`.planning/phases/5/UAT.md`, phase 5).
  - **Harness: no `ssh-keygen` output on success (D-07, AC4).** On the success path, at least in `one_run_installs_and_wires_claude_code`, assert that neither stdout nor stderr holds `Good "terminus-release" signature`.
  - **Doc comments.** Update the module doc comment and the `served` field's doc comment to match the new harness:
    - the served folder also holds the signature, and every real-server test serves all three files
    - the runs go through a key-swapped copy because the private half of the pinned key exists only on the owner's machine (D-02)
- **Verify:**
  - `sh -n install.sh` and `dash -n install.sh` both exit 0.
  - `cargo test -p terminus --test install_script` passes, with every test run as `::sh` and `::dash`.
  - `grep -c 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q' install.sh` prints `1`.
  - `grep -n -e 'unsigned development' -e 'later cycle' install.sh` prints nothing.
  - `grep -c '^# .*No release is published yet\.' install.sh` prints `1`, and that line sits in the header comment above `set -eu`.
  - Read top to bottom, `install.sh` runs, in order: the manifest `curl`, the `.manifest.sig` `curl`, `ssh-keygen -Y verify`, the binary `curl`, then the manifest `read` block.
  - `git diff install.sh` changes no line inside `verify_download` or inside the manifest `read` block.

### Task 2: Refuse a machine without `ssh-keygen` before any download

- **Files:** install.sh (anchor: the `uname -s` and `uname -m` case statements), crates/terminus/tests/install_script.rs (anchor: `Fixture::curl_tools`, `Fixture::tools_folder`, `both_shells!`)
- **Action:**
  - **Script (D-05, D-09).** After the `uname` refusals and before `scratch=$(mktemp -d)`, check for `ssh-keygen` with `command -v ... >/dev/null 2>&1`, as `digest` checks for `sha256sum`. When it is missing, fail through `fail` with a message saying `ssh-keygen` is needed to verify the release signature. The message names each package with its distributions: `openssh-client` (Debian, Ubuntu), `openssh-clients` (Fedora) and `openssh` (Arch). Add no version check.
  - **Test, under both shells (AC3).**
    - Use a fresh fixture with nothing installed. Serve and sign 1.0.0.
    - Run with no argument through the logging fake `curl`, in a tools folder that also leaves out `ssh-keygen`. Extend the fake `curl` helper so a test can leave a tool out. `PATH` holds nothing else, so `ssh-keygen` is really absent.
    - Assert that `Fixture::assert_refused` holds, that the `curl` log is empty, and that HOME has no entries afterwards.
    - Assert that stderr names all three packages, each as a whole word. A substring check proves nothing here, because `openssh-clients` contains both `openssh-client` and `openssh`.
- **Verify:**
  - `cargo test -p terminus --test install_script` passes, and the new test runs as `::sh` and `::dash`.
  - Temporarily moving the `ssh-keygen` check below the manifest `curl` makes the new test fail on the non-empty `curl` log. Moving it back makes the test pass again.
  - `sh -n install.sh` and `dash -n install.sh` exit 0.

### Task 3: Prove a missing or bad signature is refused before the binary is requested

- **Files:** crates/terminus/tests/install_script.rs (anchor: `assert_download_refused`, `a_failed_manifest_download_is_refused`, `Fixture::serve_manifest`, `Fixture::curl_tools`, `Fixture::assert_refused`, `both_shells!`)
- **Action:**
  - **Shared setup.** Each case runs under both shells and starts the same way:
    - Install 1.0.0 from a good signed release first, so "unchanged" covers an existing link and version folder.
    - Serve 2.0.0 with one defect, and run with no argument through the logging fake `curl`.
  - **Shared assertions.** Every case asserts all of these:
    - `Fixture::assert_refused` holds.
    - The `curl` log is exactly the manifest URL, then the signature URL. This shows the binary is never requested.
    - stdout is empty, and every stderr line starts with `terminus: `, so no `ssh-keygen` output got through (D-07).
  - **Missing signature (D-08, D-11).** The fake `curl` exits 22 for the `.manifest.sig` name, through `assert_download_refused`, beside the manifest and binary cases. The fake is needed because `s_server -WWW` answers a missing file with `200 ok` and error text, which would test a bad signature instead. stderr holds `signature download failed: <the default signature URL>`.
  - **Another key (AC5, D-02, D-10).** Run the unmodified `install.sh` from `script()`, not the copy. The release is signed with the fixture's throwaway key under `terminus-release`. Before the run, write `$HOME/.ssh/allowed_signers` holding `crenshawdev-release <throwaway public line>`. The run is refused.
  - **Another namespace.** Run the copy, so the key matches. Sign the 2.0.0 manifest with the throwaway key under `baley-release`.
  - **Manifest changed after signing.** Run the copy. After 2.0.0 is served and signed, rewrite the manifest without signing it again. The new manifest must be one that would otherwise install, for example a different valid version with the same digest, so that only the signature can refuse it.
  - **Messages.** The three bad-signature cases print the same refusal line, the one Task 1 wrote. It differs from the missing-signature line (D-08).
- **Verify:**
  - `cargo test -p terminus --test install_script` passes, and each new test runs as `::sh` and `::dash`.
  - Temporarily making the `ssh-keygen -Y verify` step always succeed makes the another-key, namespace and changed-manifest tests fail. Restoring it makes them pass.

### Task 4: Hold the pinned line in `install.sh` exactly once, as a literal

- **Files:** crates/terminus/tests/install_script.rs (anchor: `script`, the pinned-line constant from Task 1)
- **Action:**
  - **The test (D-10).** Write a plain `#[test]` that runs no shell, so it is not a `both_shells!` test. Model it on `install_sh_holds_the_same_marker` in `crates/terminus/src/cmd/install/binary.rs`.
    - It reads `install.sh` and asserts that the full pinned line appears exactly once.
    - It asserts that the line holding the key is not a comment.
    - It uses the same constant the key swap replaces. A swap that found nothing to replace would run the success tests against the wrong key, and this test catches that.
  - **Where it lives.** The test goes in `install_script.rs` rather than `binary.rs`. The swap it guards is in `install_script.rs`, and the key has no Rust counterpart the way `MARKER` does. `binary.rs` stays unchanged.
- **Verify:**
  - The new test passes.
  - Temporarily adding a second copy of the pinned line, in a comment in `install.sh`, makes it fail.
  - Temporarily changing one character of the pinned literal makes it fail, and the success tests fail with it. Restored, everything passes.
  - The phase gate holds:
    - `sh -n install.sh` and `dash -n install.sh` exit 0.
    - `cargo test --workspace --all-features` passes.
    - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **No `namespaces=` option (CONTEXT flagged assumption, planner's call).** D-06 states the allowed-signers line as `crenshawdev-release <pinned public line>`, and `-n terminus-release` alone does the refusing. The CONTEXT measurement and a probe on 2026-10-14 both refused a `baley-release` signature that way, and the probe's refusal was "namespace does not match", exit 255.
- **Header comment placement.** The header comment is in this plan rather than plan 2. That way the two plans share no file, and no commit leaves a header that contradicts the code beneath it. CONTEXT's plan-shape suggestion grouped it with the docs. This is a placement change only, and the plan count and order still follow the suggestion.
- **AC6 and "no release is published yet".** AC6 names the `install.sh` header among the docs that still say no release is published yet. The header only implied it, through "unsigned development artifacts only", which this plan removes, so the header gains the sentence "No release is published yet." Plan 2 keeps the same sentence in `CHANGELOG.md` and `DESIGN-BRIEF.md`. Phase 7, which publishes the first release, changes all three. Decided by the owner at the plan review, 2026-10-14.
- **Recalled prior art.** D-02 and D-01 come from `.planning/phases/6/CONTEXT.md` (phase 6). The three-URL order extends the Phase 5 UAT item for runs with no argument and Phase 5 D-01's flat asset names (`.planning/phases/5/UAT.md` and `.planning/phases/5/CONTEXT.md`, phase 5).
