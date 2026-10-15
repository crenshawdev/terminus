PLAN COMPLETE
Plan: .planning/phases/13/PLAN-1.md
Tasks: 2 of 2
| Task | Commit | Note |
|---|---|---|
| 1 - Declare the 27 whole-file testkit test targets with `required-features` | a4f9e8a | All 27 named runs exit non-zero with "requires the features: `testkit`"; discover and hook `--no-run` exit 0; grep counts 24 and 3; `--all-features --no-run` builds 63 test executables, one per file under crates/*/tests. The suite clause of Verify is covered by the end-of-plan suite run. |
| 2 - Run the whole suite on every push and pull request on the four release runners, and stop the docs saying no workflow does | 6e8e0f5 | `release_workflows` 2 passed. test.yml holds all four runners, the locked all-features test command, RUST_VERSION 1.97.1, persist-credentials false, contents read, push and pull_request; its one `uses:` is SHA-pinned; no `--release`. Tool-check script copied to scratch: exit 0 on the normal PATH; exit 1 printing `::error::not on PATH: dash` on a PATH of links without dash; an extra probe with a stand-in openssl lacking `-WWW` exits 1 naming it. The doc grep prints nothing; README keeps `cargo test --workspace --all-features`. The human-verify (all four jobs green on GitHub, tool check before cargo test, timing budgets met on hosted runners) is still outstanding. |
| Suite | - | `workflow.test_command` is null; ran `cargo test --workspace --all-features --locked` once. GREEN: 67 result groups, 878 passed, 0 failed, 0 ignored. Static analysis (`cargo clippy --all-targets -- -D warnings`, `cargo check --all-targets`) clean before both commits. |
Deviations: none
Open items:
- human-verify for Task 2 is outstanding: push the branch and confirm the test workflow runs on all four runners for the push and the pull request, that the tool check runs before `cargo test`, and that all four jobs pass. A timing-budget failure on a hosted runner goes to the owner with the test name and its measured number (D-04).
- No Linux install step was added. The Ubuntu images are expected to carry every listed tool (dash, shasum via perl, ssh-keygen, openssl 3), and the check step names any that is missing; add an apt step only if the first run shows a gap, most likely on the slimmer ubuntu-24.04-arm image.
- On macOS a missing `sha256sum` is supplied as a single link to Homebrew's `gsha256sum` in a folder of its own, not by putting coreutils' gnubin first on PATH, so install.sh's tests still run against the BSD tools a Mac ships.
- README.md and SECURITY.md are written and were opened in RustRover for the owner to review.
