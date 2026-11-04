# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub: on this repository, open the **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and what an attacker could do with it. You will get an answer, and credit in the fix if you want it.

## Scope

Reports about Terminus on the `main` branch and about any published release are in scope. Of most interest:

- `install.sh` installing a binary when the manifest does not verify against the pinned release key or the binary's SHA-256 does not match the manifest, or downloading the binary before the manifest's signature verifies;
- the Terminus binary opening a network connection;
- Terminus reading a project listed under `exclude` in `terminus.toml`, returning its sessions, or logging its prompts;
- with `[privacy] redact_recall` on, a secret of a shape the redaction filter is built to replace reaching Claude Code through a hook or an MCP tool;
- an archived session whose stored blocks no longer match their recorded hashes, which `terminus verify` still passes.

## How Terminus is checked

- **Every pull request and every push to `main`** runs the test suite through `.github/workflows/test.yml`: `cargo test --workspace --all-features --locked`, on the debug and the release build, on Linux and macOS, on x86_64 and aarch64. If any test fails in any of the eight jobs, the run fails. It skips three tests of a hook spawn's p99 on every runner and three time budgets on a single spawn on the macOS runners, because a hosted VM's timing measures the VM. It reads no secret, so the tests over the private transcript corpus, which need `TERMINUS_TEST_CORPUS`, skip themselves.
- **Fuzzing.** `fuzz/` holds a cargo-fuzz target for every parser that reads input from outside Terminus: transcripts, `terminus.toml`, the command line, hook input, MCP messages and Claude Code's settings files. `cargo +nightly fuzz build` builds them all, and `cargo +nightly fuzz run <target>` runs one. CI does not run them.
- **Miri.** Terminus has one `unsafe` block, the volatile read of the install marker in `crates/terminus/src/cmd/install/binary.rs`. The two tests that exercise it without touching the filesystem run under Miri:

  ```sh
  rustup +nightly component add miri
  cargo +nightly miri test -p terminus --lib -- a_marker_split_across_two_reads_is_still_found a_file_without_the_marker_is_foreign
  ```

## Fixed findings

Every finding that is fixed gets a regression test. The test goes into the test suite of the crate the finding is in, it fails before the fix and passes after, and it stays there so the same bug cannot come back unnoticed.
