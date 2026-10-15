---
phase: 3
plan: 1
requirements: [INST-10, INST-11]
files:
  - crates/terminus-core/src/testkit.rs
  - crates/terminus/src/cmd/install/claude.rs
  - crates/terminus/src/cmd/install/mod.rs
  - crates/terminus/src/cmd/install/targets.rs
  - crates/terminus/src/cmd/install/json_file.rs
  - crates/terminus/src/cmd/install/binary.rs
  - crates/terminus/src/main.rs
  - crates/terminus/tests/install.rs
  - crates/terminus/tests/uninstall.rs
  - crates/terminus/tests/doctor.rs
---

# Phase 3: Install Asks Nothing - Plan 1

## Goal

`terminus install` places the MCP entry by running `claude mcp`, so Claude Code stays the only writer of `~/.claude.json`. When it cannot, it reports the install incomplete with the exact command to run. It asks no question along the way and takes no `--yes`.

## Must be true when done

- No test in `tests/install.rs`, `tests/uninstall.rs` or `tests/doctor.rs` can start a real `claude`. Every `terminus` they spawn gets a `PATH` holding only a fixture directory, which holds either a fake `claude` or nothing.
- Install never writes `~/.claude.json`.
  - With a fake `claude` that writes nothing, the file is byte-identical after an install and a rerun, and no `.claude.json.terminus-backup` appears.
  - The fake's log shows `mcp add-json --scope user terminus <JSON>`, where the JSON has `type` `"stdio"`, `command` set to the stable path and `args` set to `["mcp"]`.
  - Every `claude` process install starts has `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`.
- A rerun with the entry already correct starts no `claude` process. An entry in the wrong shape gets `mcp remove --scope user terminus` and then the add, and the add keeps any keys the user put beside the three Terminus owns, such as `env`.
- Install still places the binary, merges the four hook entries and starts the backfill in three cases: no `claude` on `PATH`, a `claude` that exits 1, and a `~/.claude.json` that does not parse. In each case it says the install is incomplete, gives the cause and the exact `claude mcp add-json` command (single-quoted), and exits 1.
- With stdin closed, install exits 0 and asks no question. It leaves the same `settings.json`, stable-path binary and `claude` call log as a run whose stdin is open and never written. `terminus install --yes` and `terminus install -y` exit 2 and change nothing. (human-verify at UAT: run `terminus install` from a terminal and see that it asks nothing there either.)
- The hook entries are still merged into `settings.json` in place with one backup, and a rerun leaves the file byte-identical with no new backup. A `settings.json` not in the two-space format is rewritten in full without asking. The backup keeps its original bytes, and the summary says the file was rewritten.

## Context

- Plan 1 of 3. The plans run in order: plan 2 builds on this plan's runner, entry logic and test fake, and all three plans touch the same test files.
- Binding decisions from `phases/3/CONTEXT.md`:
  - D-01: read `.mcpServers.terminus`, and never run `claude mcp get`.
  - D-02: any user-level `terminus` entry is Terminus's to replace, and keys beside the owned three are carried over.
  - D-03: find `claude` on `PATH` ourselves and run it by absolute path, with no shell, stdin at null and a deadline.
  - D-04: `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` on every `claude` process.
  - D-05: an incomplete install reports itself and exits non-zero.
  - D-06: `add-json`, or `remove` then `add-json` for an entry in the wrong shape.
  - D-07: a deadline above 2 s.
  - Also D-08, D-09, D-12, D-17 and D-18.
- Phase 3's D-03 reverses v0.1.0 phase 4 D-06, under which install merged `~/.claude.json` itself [recalled: v0.1.0/phases/4/CONTEXT.md, phase 4]. v0.1.0 phase 4 D-05 still holds, now as a rule about reading: the MCP entry lives in `~/.claude.json` under `.mcpServers` and never in `settings.json` [recalled: same file].
- Patterns to follow:
  - Doctor's `on_path` and `version_of`, for the `PATH` scan and the deadline loop.
  - `/code/baley/crates/baley/src/install/registration.rs` (`judge`, `interpret`, `hand_command`, and the 30 s `TIMEOUT`), for the shape of the decision, the cause text and the shell quoting.
- Out of scope for this plan: the receipt's `cleanupPeriodDays` lines (plan 2 task 1), uninstall and doctor behaviour (plan 2), the docs (plan 3), where install copies the binary from (phase 2 D-08 stands), and the project and local MCP scopes.
- Run `rustfmt --edition 2021` on the files you touch, and only those. `cargo fmt --all --check` already reports diffs in files outside this phase.

## Tasks

### Task 1: Tests run against a fake `claude` and never a real one

- **Files:** crates/terminus-core/src/testkit.rs, crates/terminus/tests/install.rs (anchor: `Fixture::command_at`), crates/terminus/tests/uninstall.rs (anchor: `Fixture::command`), crates/terminus/tests/doctor.rs (anchor: `Fixture::env`, `Fixture::shell`, `after_install_every_wiring_check_is_ok`)
- **Action:**
  - **The fake (D-18).** Add one fake-`claude` writer to `terminus_core::testkit`, so all three test binaries share it. The testkit module doc explains why shared test support lives there.
    - It writes a `#!/bin/sh` script, mode 0755, into a directory the test names.
    - Each time the script runs, it appends one record to a log file the test names. The record holds every argument and the value of `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`. Logging the whole environment through `export -p` also works.
    - It then prints a stderr line and exits with a status. The test chooses both.
    - Asked for `--version` alone, it prints a version line in the form `2.1.296 (Claude Code)`, so doctor's `claude_code` check can read it.
    - It uses only shell builtins, because the `PATH` it runs under holds nothing else.
  - **The fixtures (D-18).** In each of the three fixtures, every command that spawns `terminus` gets these settings.
    - `PATH` is set to a fixture directory that holds the fake and nothing else. Never add a system directory: one can hold a real `claude`, and on this machine `~/.local/bin` does. A test that wants no `claude` points `PATH` at an empty directory. Add no override variable to the binary.
    - `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` is removed from the spawned environment, so a logged `1` can only have been set by `terminus`.
    - `git` is not on this `PATH`. The ingests the tests spawn therefore degrade project identity, which `terminus_core::project` already allows for.
  - **Doctor's shell helper.** `Fixture::shell` in `tests/doctor.rs` runs `/bin/sh` by its absolute path. Rust looks up a bare `sh` on the child's `PATH`, which is now the fixture directory.
  - Install does not run `claude` yet, so this task changes no install or uninstall behaviour. The one visible change is that doctor's `claude_code` check now finds the fake.
- **Verify:**
  - `cargo test -p terminus --all-features --test install --test uninstall --test doctor` passes. `after_install_every_wiring_check_is_ok` also asserts that the `claude_code` finding names the fake's path inside the fixture.
  - `install.rs`'s `command_at`, `uninstall.rs`'s `Fixture::command` and `doctor.rs`'s `Fixture::env` each set `PATH` and remove `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 2: Install places the MCP entry through `claude mcp`

- **Files:** crates/terminus/src/cmd/install/claude.rs (new), crates/terminus/src/cmd/install/mod.rs (anchor: `run`, the `pub mod` list, module doc step 3), crates/terminus/src/cmd/install/targets.rs (anchor: `add_mcp_server`, the module doc sections `Two files, not one (D-05)`, `Merge, never replace` and `Idempotency and upgrade are the same rule (INST-05)`, and its closing D-06 paragraph), crates/terminus/src/cmd/install/json_file.rs (anchor: `Json::write_to`, `render`), crates/terminus/tests/install.rs (anchor: `two_installs_leave_one_mcp_entry_beside_the_servers_already_there`, `a_stale_mcp_registration_is_brought_up_to_date`, `an_accepted_confirmation_writes_all_three_after_showing_the_diff`, `two_installs_leave_one_hook_entry_per_event_and_change_nothing_else`, `the_summary_names_the_data_directory_and_the_backups_it_wrote`), crates/terminus/tests/uninstall.rs (anchor: `only_the_terminus_key_leaves_mcp_servers`), crates/terminus/tests/doctor.rs (anchor: `fixture`)
- **Action:**
  - **The runner, `install/claude.rs` (D-03, D-04, D-07).** Declare it `pub mod`, beside `binary`, `json_file` and `targets`, because plan 2's uninstall runs the same commands.
    - **Finding `claude`.** It scans `PATH` itself, the way doctor's `on_path` does, and runs `claude` by the absolute path it finds.
    - **How it runs.** No shell. Stdin is null. Stdout is null and never read. Stderr is captured for its first line.
    - **Environment.** It sets `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` on every process and otherwise inherits the environment. That way `CLAUDE_CONFIG_DIR` reaches `claude`, and `claude` writes the same file install read.
    - **Waiting.** It waits under a deadline, killing and reaping the child as `version_of` does. Drain stderr while it waits, so a child that writes more than a pipe holds is not left blocked until the deadline.
    - **The deadline (D-07).** 30 seconds. That is Baley's `TIMEOUT`, and well above doctor's 2 s `VERSION_BUDGET`; the measured runs took 197 to 252 ms. The deadline is a parameter of the run, so a unit test can pass a short one. Add no environment override.
    - **The outcome.** It tells apart five cases: done, not on `PATH`, timed out, could not start, and exited non-zero. A non-zero exit carries the first line of stderr, or the exit status when stderr is empty. Install reads only the exit status and that line, never stdout.
  - **`targets.rs` (D-17, D-01, D-02).**
    - **Delete `add_mcp_server` and its unit tests.** The tests that go are the MCP half of `a_second_application_changes_nothing` (keep its hooks half), `another_server_is_left_alone` and `a_stale_mcp_registration_is_brought_up_to_date`. `claude_json_path` and `MCP_SERVER_KEY` stay.
    - **Add the D-01 test of a found `.mcpServers.terminus` value.** The entry is correct when `type` is `"stdio"`, `command` decodes to the stable path and `args` is exactly `["mcp"]`. Any other key is ignored. This is the same test doctor's `mcp_server` check makes.
    - **Add the entry install places (D-02).** When the found value is an object, the entry is that object with `type`, `command` and `args` set in place and every other key carried over. Otherwise it is a new object holding just those three keys.
    - **Rewrite the module doc sections named in Files.** The doc should say four things.
      - Install writes one file, `settings.json`.
      - It reads `.claude.json`, and it places the MCP entry by running `claude mcp`, so Claude Code stays that file's only writer.
      - The old D-06 rule of "no shell and no subprocess" is gone.
      - Phase 3 D-03 replaces it: no shell, and a subprocess run by absolute path.
  - **`json_file.rs`.** The renderer writes only the two-space form. Add a compact form for the `add-json` argument, with no whitespace between tokens. It writes every scalar as its source text, the faithfulness rule the module doc states.
  - **`run` in `mod.rs` (D-01, D-06).**
    - **The reads.** `.claude.json` is no longer read among the checks that can refuse, and it is no longer previewed, diffed, shown, backed up or applied. The confirmation shows only the `settings.json` change.
    - **The registration step.** After `binary::place` and the hooks write, a new step reads `.claude.json` through `Document::read`, which only reads. It never runs `claude mcp get` (D-01). It then does one of three things.
      - The entry is already correct: it runs nothing.
      - There is no entry, meaning no file, no `mcpServers` or no `terminus` key: it runs `mcp add-json --scope user terminus <compact JSON>`.
      - An entry is there in any other shape: it runs `mcp remove --scope user terminus` and then that add. If the remove fails, the add does not run.
    - **The summary.** Its mcp line names `.claude.json`'s path and says which of the three happened.
    - **Failure, in this task.** A registration that does not complete (no `claude`, a `claude` that fails, or an unreadable `.claude.json`) fails the run as an operational failure with its cause. This happens after the binary and the hook entries are written. Task 3 turns it into D-05's incomplete receipt.
    - **Module doc.** Step 3 now says the entry is placed through `claude mcp`.
  - **Tests (D-18).**
    - **`tests/install.rs`, the MCP tests.** Rewrite the two MCP tests against the fake's log.
      - With the seeded `CLAUDE_JSON`, two installs leave `.claude.json` byte-identical. Each install logs exactly one `mcp add-json --scope user terminus <JSON>`. That JSON parses to `type` `"stdio"`, the stable path and `["mcp"]`, and the logged variable is `1`.
      - With a wrong entry seeded that carries `"env": {"KEEP": "me"}`, install logs the remove and then an add whose JSON keeps `env.KEEP`.
      - Add a test: with a correct entry seeded, install logs no call at all.
    - **`tests/install.rs`, the other tests.**
      - Every expectation of a `.claude.json.terminus-backup` goes. There is now one backup, `settings.json`'s.
      - `an_accepted_confirmation_writes_all_three_after_showing_the_diff` stops asserting an mcp diff and a written `.claude.json`, and asserts the logged add instead.
      - The two refusal tests, `a_foreign_binary_at_the_stable_path_stops_everything` and `a_settings_file_with_a_repeated_key_is_refused`, also assert that the log is empty.
    - **`tests/uninstall.rs`.** `only_the_terminus_key_leaves_mcp_servers` now seeds a correct `terminus` entry into `.claude.json` before install, because install no longer writes one. Replacing uninstall's own strip is plan 2's job.
    - **`tests/doctor.rs`.** The fixture's `.claude.json` now holds a correct `terminus` entry for the fixture's stable path. A fresh install then runs no `claude`, and `mcp_server` reads `ok`.
- **Verify:**
  - `cargo test -p terminus --all-features --bin terminus` passes, including new unit tests for each of these:
    - A `/bin/sh` script that loops forever is killed at a short deadline and reported as timed out.
    - A script that prints two stderr lines and exits 3 is reported with the first line only.
    - A `PATH` without `claude` is reported as not on `PATH`.
    - The D-01 test accepts an entry that has an extra `env` key. It rejects a wrong `type`, a wrong `command`, and `args` of `["mcp","x"]`.
    - The entry placed over a found object keeps that object's `env`.
  - `cargo test -p terminus --all-features --test install --test uninstall --test doctor` passes, including the rewritten and new tests named above.
  - `git grep -n add_mcp_server -- crates` prints nothing.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 3: A registration that cannot complete leaves an incomplete install, not an aborted one

- **Files:** crates/terminus/src/cmd/install/mod.rs (anchor: `run`, `start_backfill`), crates/terminus/src/cmd/install/targets.rs, crates/terminus/src/cmd/install/claude.rs, crates/terminus/tests/install.rs
- **Action:**
  - **What still happens (D-05).** This replaces task 2's operational failure. The cases are: `claude` is not on `PATH`, times out, cannot start or exits non-zero, or `.claude.json` cannot be read or parsed. In all of them, install still places the binary, merges the hook entries and starts the backfill. When `.claude.json` is unreadable, install starts no `claude` at all.
  - **What the summary says.** It says the install is incomplete rather than done. The mcp line gives the cause: the first stderr line, "not on `PATH`", the timeout, or the read failure in its own words.
  - **The command to finish it (D-05, D-06).** The summary also gives the exact command that finishes the registration.
    - **Where it lives.** Add the command builder to `targets.rs`, reachable from `cmd::doctor`, because plan 2's doctor task prints the same command. It builds the command from the entry as found under the key, or from no entry.
    - **Its form.** `claude mcp add-json --scope user terminus '<compact JSON>'`. The JSON is single-quoted, and every `'` inside it is written as `'\''`.
    - **When the remove comes first.** If an entry is still under the key, the command starts with `claude mcp remove --scope user terminus && `, because `add-json` refuses a name that already exists. That covers a wrong entry whose remove did not run or failed.
    - **When it is the add alone.** After a remove that succeeded and an add that failed, and for an unreadable file.
  - **The exit.** After the backfill has started, `run` returns `Failure::Silent`. The process exits 1 and prints nothing after the summary, as uninstall's `finish` does. An incomplete install must never exit 0, because a script, and phase 4's `install.sh`, read the exit status.
- **Verify:**
  - `cargo test -p terminus --all-features --test install` passes, including new tests for three cases:
    - `PATH` points at an empty directory.
    - The fake exits 1 with stderr `first\nsecond`. The output contains `first` and does not contain `second`.
    - `.claude.json` holds just `{`, which does not parse. The fake's log is empty.

    In each case, all of these hold:
    - Install exits 1.
    - The stable binary exists, and `settings.json` has the four entries.
    - The output contains `incomplete`, `terminus status` (which shows the backfill ran), and a line holding `claude mcp add-json --scope user terminus '`.
    - `.claude.json` is byte-identical to the seed.
  - `cargo test -p terminus --all-features --bin terminus` passes, including two new unit tests:
    - For a stable path containing `'`, the quoted argument, run through `/bin/sh -c 'printf %s <quoted>'`, gives back the exact compact JSON.
    - With a wrong entry found, the command starts with `claude mcp remove --scope user terminus && `.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 4: Install asks nothing and takes no `--yes`

- **Files:** crates/terminus/src/cmd/install/mod.rs (anchor: `parse`, `Options`, `run`, `advise`, `pending`, `show`, `confirm`, `offer`, `unanswerable`, module doc sections `Nothing is written before the answer (INST-03)` and `--yes, and what happens with nothing to read (INST-08, D-20)`), crates/terminus/src/cmd/install/json_file.rs (anchor: `diff`, `line_into`, `align`, `Document::source`, `Document::would_write`, `Document::reformats`, `Document::apply`, module doc sections `Why not serde_json::Value`, `Writing` and `What the user approves is what lands (INST-03)`, test `the_diff_shows_only_the_changed_block`), crates/terminus/src/cmd/install/binary.rs (anchor: `place`), crates/terminus/src/main.rs (anchor: the comments on the `install` and `uninstall` match arms), crates/terminus/tests/install.rs, crates/terminus/tests/uninstall.rs (anchor: `Fixture::install`), crates/terminus/tests/doctor.rs (anchor: `Fixture::install`, `Fixture::shell`)
- **Action:**
  - **Removed from install (D-08).**
    - The `confirm` call in `run`, and its "nothing was changed" branch.
    - `Options.yes`, and the `Short('y') | Long("yes")` arm. `--yes` and `-y` then reach `crate::unexpected` and exit 2 as misuse, as `--json` does.
    - `offer`.
    - The branch of `advise` that can write `cleanupPeriodDays`. The rest of `advise` stays until plan 2's receipt task replaces it.
    - `pending` and `show`, and the listing they printed before the question.
    - Install still has no `--json` (D-24).
  - **`confirm` stays (D-09).** It is the shared question for `uninstall --purge` and `data move`. Keep its signature, because the doc on `data.rs`'s `answered` relies on how its `yes` argument behaves. `unanswerable`'s install wording ("install needs an answer", "rerun with --yes") goes. Both callers already replace `confirm`'s error with their own, so its no-answer failure needs no install wording. Its doc names its two callers.
  - **D-17 in `json_file.rs`.** Delete `diff`, `align`, `line_into` and the test `the_diff_shows_only_the_changed_block`. Two methods stay:
    - `Document::would_write` stays. It still decides whether `settings.json` is backed up and written, since a backup is taken only for a file that will change.
    - `Document::reformats` stays. It supplies the summary's D-12 line.

    Rewrite the module doc sections and method docs named in Files so they describe a write that is not shown first. `source` stays for uninstall's settings restore, and `overwrite` stays (D-17).
  - **D-12.** A `settings.json` this renderer did not format is rewritten in full, without a question, and the one-time backup holds its original bytes. The summary's `settings.json` line says three things: the file was rewritten in full in two-space form, its keys and values are unchanged, and where the backup is.
  - **`place` in `binary.rs`.** Its doc and inline comment describe the check in `run` as made "before the confirmation" or "before the question". Reword both to say the check `run` made before anything was written. The re-check before the rename stays exactly as it is.
  - **`main.rs`.**
    - The comment on the `install` arm gives D-24's reason as "it shows a diff and asks once". Replace it: install prints a receipt for a person to read.
    - Give the `uninstall` arm its own reason: it asks under `--purge`.
    - USAGE does not change. It never named `--yes` (D-19).
  - **`mod.rs`'s module doc.** Remove or rewrite the two sections named in Files. The doc now says install applies its defaults and asks nothing.
  - **Tests (D-18).**
    - **`install --yes` everywhere.** Change every `install --yes` to `install`. That is 19 calls in `tests/install.rs`, `Fixture::install` in `tests/uninstall.rs`, and `Fixture::install` in `tests/doctor.rs`.
    - **Doctor's shell helper.** `shell` gives its child a null stdin instead of writing `y\n`, and its doc stops naming the confirmation.
    - **Removed from `tests/install.rs`.**
      - The helpers `answer`, `at_the_prompt` and `Paused`.
      - The tests `without_an_answer_and_without_yes_install_refuses_and_writes_nothing`, `a_declined_confirmation_changes_nothing_and_exits_zero` and `an_accepted_confirmation_writes_all_three_after_showing_the_diff`, which the AC1 tests below replace.
      - `a_binary_that_appears_during_the_confirmation_is_still_refused`. The window it held open was the confirmation, which no longer exists. `binary::place`'s re-check is unchanged.
    - **The reformat test.** Rewrite `a_differently_formatted_file_shows_the_rewrite_it_will_get`. There is no diff and no question, the file lands in two-space form, the backup is `four_space(SETTINGS)`, and the summary names the rewrite.
    - **The stale-temporary test.** Keep the property `a_stale_temporary_name_is_stepped_over_rather_than_followed` tests, with a new way to pause.
      - Spawn `/bin/sh` with a script that reads one line from stdin and then `exec`s the terminus binary with `install`. The child pid the test sees is then the pid install runs under.
      - Plant the two symlinks, then write the line and close stdin.
    - **AC1: `--yes` is misuse.** `install --yes` and `install -y` each exit 2, place nothing, leave `settings.json` byte-identical and log no `claude` call.
    - **AC1: nothing is read from stdin.** Run install in two fresh fixtures against a fake that succeeds: one with stdin null, and one with stdin a pipe held open and never written. Wait under a deadline, so an install that reads stdin fails the test instead of hanging it. Then check all of these:
      - Both runs exit 0.
      - Neither output contains `[Y/n]`, `[y/N]`, or a line ending in `?`.
      - `settings.json`, the stable-path binary and the fake's log match between the two fixtures, once each fixture's root is replaced by a placeholder.
- **Verify:**
  - `cargo test -p terminus --all-features --test install --test uninstall --test doctor` passes, including the AC1 tests, the rewritten reformat test and the stale-temporary test.
  - `cargo test -p terminus --all-features --bin terminus` passes.
  - `git grep -n -e 'fn diff' -e 'fn align' -e 'fn line_into' -e 'fn offer' -e 'fn pending' -e 'fn unanswerable' -e 'Long("yes")' -- crates/terminus/src/cmd/install` prints nothing.
  - `git grep -n -e '"--yes"' -e '"-y"' -- crates/terminus/tests/install.rs crates/terminus/tests/doctor.rs crates/terminus/tests/uninstall.rs` prints only two kinds of line: the AC1 exit-2 test's, and uninstall's own `--purge --yes` tests.
  - `git grep -n -e 'confirm(' -e is_terminal -- crates/terminus/src/cmd/install` prints exactly two lines: `confirm`'s own definition and the `is_terminal` line inside its body. Nothing install runs calls `confirm` or checks for a terminal, so a run with a terminal attached takes the same path as the AC1 runs, which have none. (Plan review, 2026-10-13: the AC1 tests cannot attach a terminal, so this check stands in for one.)
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **Shape.** CONTEXT suggests three plans: the install rework, then uninstall and doctor, then the docs. This plan holds the install rework up to the 4-task ceiling. The receipt's backfill and `cleanupPeriodDays` lines (D-10, D-11) are plan 2's first task, ahead of uninstall and doctor, so the order is unchanged.
- **D-17 versus D-12.** D-17 lists `Document::would_write` and `Document::reformats` among the code that loses its only caller. Neither does.
  - `would_write` still decides whether `settings.json` is backed up and written.
  - `reformats` supplies the receipt line D-12 requires.

  Both stay, and only `diff`, `align` and `line_into` go. Confirmed by the owner at planning, 2026-10-13.
- **D-04's reach.** D-04 says "every `claude` process Terminus starts". This plan applies it to the runner that install, and in plan 2 uninstall, use. Doctor's existing `claude --version` call (`version_of`) is not touched here. The owner ruled at planning, 2026-10-13, that D-04 covers it too, and plan 2 task 3 sets the variable there.
- **The command for a wrong entry.** The finishing command adds `claude mcp remove --scope user terminus && ` when an entry is under the key. Without it, the command would fail with "already exists", by D-06's own measurement. Plan 2's doctor fix uses the same command. Confirmed by the owner at planning, 2026-10-13.
- **Files outside D-19's list.** `binary.rs` (`place`) and `main.rs` carry comments that describe the confirmation and the diff being removed. Task 4 rewords them, and only them.
- **Known flake.** The fake is a freshly written script that is run while other test threads fork, which is the ETXTBSY flake class already known from `tests/hook.rs`. A failure with ETXTBSY at the fake's exec belongs to that class. Report it, and do not chase it inside this plan.
