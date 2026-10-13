PLAN COMPLETE
Plan: .planning/phases/4/PLAN-2.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - install.sh in Baley's shape, with Terminus's ownership rule | 7a48983 | `sh -n install.sh` exit 0; `cargo test -p terminus --bin terminus` 54 passed, tie test fails with one marker character changed (restored); `http://` and `TERMINUS_BIN_DIR=/x` runs exit 1 with their messages, scratch HOME left empty; clippy and check clean |
| 2 - An HTTPS harness runs install.sh end to end | 29e5b4e | `cargo test -p terminus --test install_script` 3 passed (exit-0 run, no `claude`, `shasum` fallback); `pgrep -af "^openssl s_server -WWW"` found nothing afterwards; clippy and check clean |
| 3 - Every refusal leaves the machine untouched | ab5e816 | `cargo test -p terminus --test install_script` 10 passed (7 refusals, each its own test); with `verify_download` commented out the digest test failed, restored and `git diff install.sh` empty; clippy and check clean |
| 4 - Successive installs keep every version | b786f5a | `cargo test -p terminus --test install_script` 14 passed (A then B with `/proc/<pid>/exe` on A, same version rebuilt, marked plain file, dangling link into versions); `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean; `cargo test --workspace --all-features` 784 passed, 0 failed across 64 test binaries |
Deviations: [deviation] task 3: expected only the digest refusal test to fail with `verify_download` commented out; observed it and `shasum_computes_the_digest_when_sha256sum_is_missing` both failed, because the shasum test also asserts that a wrong digest is refused. A prediction miss, not a plan assertion; nothing changed.
Open items: `openssl s_server -WWW` answers a missing file with `200 ok` and an error text (measured 2026-10-13), so `curl -f` cannot fail on a missing file in this harness; every test serves both files, and the module doc says so. The plan's `pgrep -f "s_server -WWW"` matches its own shell's command line, so the check was run as `pgrep -af "^openssl s_server -WWW"`.
