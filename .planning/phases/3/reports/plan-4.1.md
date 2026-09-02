PLAN CHECKPOINT: suite-red
Plan: .planning/phases/3/PLAN-4.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Config remembers the file it was loaded from | 93c1fd2 | `Config::source` + `source_path()`; `load_from` fills it whether or not the file was there. `cargo test -p verbatim-core --test config`: 44 passed. `cargo clippy --all-targets -- -D warnings` clean (also the every-crate compile the Verify names). |
| 2 - Refuse a wide verbatim.toml where its key is consumed | 2dfa97c | New `credentials::Error::ConfigTooOpen`, raised at the `provider_api_key()` arm of `resolve` and nowhere else (D-07). `--test credentials`: 14 passed, 2 new. The bench now writes its `verbatim.toml` at 0600, because the tier and precedence tests would otherwise be refused for their file's mode rather than measuring which tier answered. New `crates/verbatim/tests/config_mode.rs`: 3 passed - a 0644 keyed config refused by name with `mode 644` on stderr and exit 0, the same file at 0600 silent, and the two D-07 arms (no key, key in the environment) loading; no fragment of the key in stdout, stderr or `runs.error` in any of them. Clippy clean. |
| 3 - Doctor prints the chmod for verbatim.toml and states the deferral | fece81a | New `config_mode` and `mode_checks` checks pushed after `credentials`, both through `credentials::permissions` so the report cannot disagree with the refusal. A wide config with no key is `ok`, not a problem. `--test doctor`: 23 passed, 5 new, including AC6's both-files case where the two printed `chmod` lines are run through `Fixture::shell` and the next run exits 0. `the_json_document_carries_every_check_by_name` unmodified and passing. Clippy clean. |
| 4 - Doctor notes a local provider that is not on loopback | 1c05044 | `provider_local` pushed from `archive`, `State::Note` so doctor still exits 0. Hand-written `host_of`/`is_loopback` (D-16): userinfo dropped at the LAST `@`, brackets for IPv6, a port cut only when it is all digits. Only the host is printed, never the configured `base_url`, since a `base_url` may carry userinfo. `--test doctor`: 26 passed, 3 new. `crates/verbatim-core/tests/provider.rs` and `Cargo.lock` both `git diff --quiet` clean. |

Suite: `cargo test --workspace --all-features` under `umask 022`, twice, both red in `crates/verbatim/tests/hook.rs` and nowhere else - and a DIFFERENT test each time:

- run 1: `every_event_exits_zero_with_an_empty_stdout_inside_the_budget` - `spawn the hook: Os { code: 26, kind: ExecutableFileBusy, message: "Text file busy" }` at `crates/verbatim/tests/hook.rs:144`.
- run 2: `the_ingest_survives_a_group_kill_and_a_descendant_sweep` - `the hook did not die of the group kill: ExitStatus(unix_wait_status(0))` at `crates/verbatim/tests/hook.rs:700`.

Both are the parallel-load flakiness plan 2's report, plan 3's report and phase 2's SUMMARY already record. Evidence it is not this plan's: `cargo test -p verbatim --all-features --test hook the_ingest_survives_a_group_kill` is green 3 runs of 3 in isolation; `--test hook` alone puts 6 of 7 green with only the loaded-machine timing assertion failing; and nothing in this plan's lease touches the hook path, the process spawn or the ingest lock - the four commits change a config field, the tier-2 credential arm and three `verbatim doctor` checks. Every other target in the run is green, including all four this plan's tasks name.

The repair would be in `crates/verbatim/tests/hook.rs`, which is outside this plan's `files:` lease, so it is the orchestrator's call rather than mine.

Deviations: none
Open items: Task 4's Verify names `-p verbatim-core --test provider` without `--features testkit`; run as written that target compiles and runs 1 test, and the test it names (`the_body_on_the_wire_is_filtered_for_remote_and_whole_for_local`) is behind that feature. Both were run: 1 passed as written, 17 passed with `--features testkit` including the named one, and `tests/provider.rs` is unmodified either way.
