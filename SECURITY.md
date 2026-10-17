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

Terminus is audited with [sentinel](https://github.com/jcrenshaw-dev/sentinel), which this repo carries as a git submodule at `.sentinel`. The audit works from [THREAT-MODEL.md](THREAT-MODEL.md), which lists what Terminus reads, writes and runs, and where untrusted data enters.

- **Every pull request** runs sentinel's mechanical checks through `.github/workflows/fast.yml`: `cargo audit`, `cargo deny` against this repo's `deny.toml`, `cargo clippy` and `semgrep`. If any of them fails, the pull request fails. When they pass, a second job, on GitHub only, has Claude Code review the pull request's diff against THREAT-MODEL.md, reading only the repository's files, and posts the review as a comment on the pull request. The job fails if the review did not run. Findings don't fail it; they are posted for the author to read. It reads the `ANTHROPIC_API_KEY` secret, which GitHub does not pass to workflows run from a fork.
- **Every push and pull request** runs the whole test suite through `.github/workflows/test.yml`: `cargo test --workspace --all-features --locked`, on the debug and the release build, on Linux and macOS, on x86_64 and aarch64. If any test fails in any of the eight jobs, the run fails. It reads no secret.
- **The full audit** runs by hand, not in CI. The next section says what it runs.
- **Fuzzing.** `fuzz/` holds a cargo-fuzz target for every parser that reads input from outside Terminus: transcripts, `terminus.toml`, the command line, hook input, MCP messages and Claude Code's settings files. `cargo +nightly fuzz build` builds them all, and `cargo +nightly fuzz run <target>` runs one.
- **Miri.** Terminus has one `unsafe` block, the volatile read of the install marker in `crates/terminus/src/cmd/install/binary.rs`. The two tests that exercise it without touching the filesystem run under Miri:

  ```sh
  rustup +nightly component add miri
  cargo +nightly miri test -p terminus --lib -- a_marker_split_across_two_reads_is_still_found a_file_without_the_marker_is_foreign
  ```

## The full audit

The full audit runs [sentinel](https://github.com/jcrenshaw-dev/sentinel)'s pipeline in eight steps. The run counts are sentinel's defaults.

1. **Mechanical checks.** The same checks every pull request runs.
2. **Context map.** Trail of Bits' `audit-context-building` skill maps the code. The map goes at the top of every review prompt in step 4.
3. **Claude Security scan.** One scan of the whole repository.
4. **Review prompts.** Eight prompts, which sentinel calls lenses, each limited to one concern: untrusted input, path handling, hook execution, permissions, data exposure, crypto and state, resource limits, and supply chain. Each prompt runs three times on Claude, each run in a fresh context, and once on OpenAI's Codex, 32 runs in all. Every run reads the same snapshot of the repository and cannot write to it. The step 3 findings are added as one more run of each prompt. Findings from the same prompt that cite the same file and overlapping lines are merged into one, with a count of how many runs reported it, and a finding only one run reported is tagged low-agreement. The merge removes which run and which model wrote each finding.
5. **Adjudication.** The merged findings are split into batches of ten, and a fresh Claude agent judges each batch in turn without knowing where the findings came from, marking each confirmed, unproven or dropped. A finding is confirmed only when the agent has written a test that fails because of it and reaches the code through a way in that THREAT-MODEL.md names. Unproven and dropped findings each carry the reason. The audit's own Claude Code session then reruns every confirmed test and checks that it fails for the reason the finding gives.
6. **Refute.** A fresh Claude agent per batch sees only the confirmed and dropped rulings and tries to prove each one wrong: it reruns every confirmed test and checks every drop's reason against the code. Every ruling that does not hold goes to the maintainer to decide.
7. **Variant analysis.** For each confirmed finding, Trail of Bits' `variant-analysis` skill looks for the same pattern elsewhere in the repository. New hits are adjudicated and refuted the same way.
8. **Findings log.** For each step, how many findings were reported, confirmed, unproven and dropped, and how many rulings did not hold.

## Confirmed findings

Every confirmed finding becomes a regression test. The test goes into the test suite of the crate the finding is in, it fails before the fix and passes after, and it stays there so the same bug cannot come back unnoticed.
