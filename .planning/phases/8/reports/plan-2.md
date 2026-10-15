PLAN COMPLETE
Plan: .planning/phases/8/PLAN-2.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Build the four platforms when a version tag is pushed | 4e74878 | release.yml build job; pin grep empty, features/test/cache grep empty, `1.97.1` count 1; clippy and check clean |
| 2 - Make one draft after all four builds pass | dadb8a2 | draft job, the only `contents: write`, `GH_TOKEN` from `github.token`; no `secrets`, no `--prerelease`, pins intact; the eight-file check script passed a complete scratch folder and failed one with a missing manifest and a stray file, naming both |
| 3 - Install the published release on the four runners | 2117553 | release-check.yml; no `uses:` so the pin grep is empty, the one-line command counted 1, runner-label diff empty, default shell `bash`, stand-in `claude` before install, version read from `$HOME/.local/bin/terminus` |
| 4 - Fail a test when a workflow names a secret or a signing step | 638739d | `release_workflows` passes; `secrets.X`, `secrets['X']`, `-Y sign` and `PRIVATE KEY` each failed it naming `release.yml:1`, a renamed folder failed it on the read, restored it passes; `gh api .../actions/secrets` total_count 0; all-features clippy clean |
Suite: `cargo test --workspace --all-features` (no `workflow.test_command` set; the plan names this one), exit 0, 65 test binaries, 843 passed, 0 failed, 0 ignored.
Deviations: none
Open items: none
