# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub: on this repository, open the **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and what an attacker could do with it. You will get an answer, and credit in the fix if you want it.

## Scope

Reports about Terminus on the `main` branch and about any published release are in scope. Of most interest:

- `install.sh` installing a binary when the manifest does not verify against the pinned release key or the binary's SHA-256 does not match the manifest, or downloading the binary before the manifest's signature verifies;
- the Terminus binary opening a network connection;
- Terminus reading a project listed under `exclude` in `terminus.toml`, or returning its sessions;
- with `[privacy] redact_recall` on, a secret of a shape the redaction filter is built to replace reaching Claude Code through a hook or an MCP tool;
- an archived session whose stored blocks no longer match their recorded hashes, which `terminus verify` still passes.

## How Terminus is checked

Terminus is audited with [sentinel](https://github.com/crenshawdev/sentinel), which this repo carries as a git submodule at `.sentinel`. The audit works from [THREAT-MODEL.md](THREAT-MODEL.md), which lists what Terminus reads, writes and runs, and where untrusted data enters.

- **Every pull request** runs sentinel's mechanical checks through `.github/workflows/fast.yml`: `cargo audit`, `cargo deny` against this repo's `deny.toml`, `cargo clippy` and `semgrep`. If any of them fails, the pull request fails. When they pass, a second job, on GitHub only, runs [claude-code-security-review](https://github.com/anthropics/claude-code-security-review) on the pull request's diff and comments on the pull request with what it finds. It reads the `ANTHROPIC_API_KEY` secret, which GitHub does not pass to workflows run from a fork.
- **The full audit** runs by hand, not in CI. Each of sentinel's review prompts covers one concern and runs several times through more than one model. The results are merged, and each finding is then adjudicated. A finding counts as confirmed only once a failing test demonstrates it. The steps are in sentinel's README.
- **Fuzzing.** `fuzz/` holds a cargo-fuzz target for every parser that reads input from outside Terminus: transcripts, `terminus.toml`, the command line, hook input, MCP messages and Claude Code's settings files. `cargo +nightly fuzz build` builds them all, and `cargo +nightly fuzz run <target>` runs one.
- **Miri.** Terminus has one `unsafe` block, the volatile read of the install marker in `crates/terminus/src/cmd/install/binary.rs`. The two tests that exercise it without touching the filesystem run under Miri:

  ```sh
  rustup +nightly component add miri
  cargo +nightly miri test -p terminus --lib -- a_marker_split_across_two_reads_is_still_found a_file_without_the_marker_is_foreign
  ```

## Confirmed findings

Every confirmed finding becomes a regression test. The test goes into the test suite of the crate the finding is in, it fails before the fix and passes after, and it stays there so the same bug cannot come back unnoticed.
