PLAN COMPLETE
Plan: .planning/phases/4/PLAN-3.md
Tasks: 6 of 6
| Task | Commit | Note |
|---|---|---|
| 1 - `verbatim doctor` and the wiring checks | 569818e | Checks `binary`, `settings_file`, `hook_<event>` x4, `mcp_server`, `claude_code`. Four states (ok / note / unknown / problem) and only `problem` reaches the exit code. Missing, pointing elsewhere and duplicated are three separate findings per event. The marker is checked before the version, and a file that fails it is never executed. The report is built as one string and written once, so `doctor | head` drops the pipe error and still returns the verdict the checks earned. `a_missing_binary_is_a_problem_whose_printed_command_fixes_it` runs the printed string through `sh -c` and reruns doctor. |
| 2 - The store and config checks, which create nothing | 9b76dff | `config_roots`, `data_directory`, `store`, `last_run`, all through `read::open_in` and a permissions read rather than a probe file. Verified: after `install --yes` and with `VERBATIM_DATA_DIR` absent, doctor exits 0, reports the store as not yet created, and the parent directory's entry list is byte-equal before and after, so a created WAL or `LOCK` would fail it. After one ingest the store check carries `1 session(s)` and `last_run` carries the timestamp. |
| 3 - The settings doctor reads and never changes | b18ed95 | `cleanup_period_days` and `auto_compact`, resolved across the user file, the project `.claude/settings.json` and `.claude/settings.local.json` in Claude Code's own precedence, with `DISABLE_AUTO_COMPACT` outranking all three and named when it decides. Both are `note`, so neither reaches the exit code. Verified the settings file is byte-identical before and after doctor, and that a project scope outranks the user one. Booleans are read off `Json::Scalar`'s source text, so no new API on `json_file` was needed for this task. |
| 4 - `doctor --json` | eccb89c | `data` keyed by check name, 14 keys on every run whatever it found - the `wiring` early return now reports every dependent check as `unknown`, so the key set is stable. `Document::try_emit` added beside `emit` in `cmd/json.rs`; doctor maps `BrokenPipe` to the verdict the checks earned rather than to exit 0 or 101, so a closed reader never forges "no problems". Verified `verbatim doctor --json \| jq -e '.data \| keys_unsorted \| length > 0'` (exit 0), `set -o pipefail; ... \| head -c 1` (exit 0), and a missing binary emitting `ok:false` with the problem named in `reason`. Shape and nullability documented in `docs/json-shapes.md`. |
| 5 - `verbatim uninstall` | 6756a42 | Removes every hook entry whose `command` is the stable path under any event key the file has (not only `hook::EVENTS`), the group it emptied, an event key install's backup proves install created, and the `verbatim` key under `.mcpServers`. Restore decided by comparing renders, so formatting alone never decides it, then the backup's raw bytes go back and the backup is removed; a file edited since install keeps the edit, keeps the backup and is told so. `json_file` gained `Json::remove`, `Document::exists`, `Document::backup_path`, `Document::overwrite`, and `Json::keys` lost its `#[cfg(test)]`; array surgery needed no new API because `Json::Array(Vec<Json>)` is a public variant. 8 tests: byte-identical restore of both files against the seed, `mcpServers` back to `["context7"]` with `projects` and `oauthAccount` untouched, a `theme` edit surviving with the entries gone and the backup kept, the data directory present with its path on stdout, a deleted settings file as one line with the rest of the work still done, a second uninstall changing nothing, an unmarked file at the stable path kept, and `--json` rejected with exit 2. |

| 6 - `uninstall --purge` | 1ad6320 | Runs only after everything in task 5, prints the path and the on-disk size before the question, and asks through `install::confirm` (widened to `pub(super)`; that is the only edit to `install/mod.rs`). `--yes` means the delete rather than "take the default", since the default for something irreversible is no. Refused with no store present, refused while the ingest lock is held, and the delete itself is taken under that lock. 6 more tests: stdin closed and no `--yes` exits 1 with the store intact, `n` exits 0 with it intact, the size is printed before the question (offset comparison, not a substring), `--yes` prints path and count then leaves no directory, the lock case, and the no-store case. |

Deviations:

- [deviation] Task 6's `Verify:` asks for "a byte count matching `verbatim
  status`'s `size_bytes` taken beforehand". Those two numbers cannot be equal.
  `cmd::status` computes `size_bytes` while its own store connection is open, so
  it counts the `-wal` and `-shm` SQLite creates for the life of a connection;
  `uninstall --purge` opens nothing and counts what is on disk. Measured on a
  one-session fixture: `status --json` reports 159,744 and the directory holds
  126,976, the 32,768 difference being the `-shm`. `--purge` uses the same three
  file names and the same rule (the plan's actual instruction), and the test
  asserts the printed count against a footprint the test measures itself, plus
  `footprint <= status_size` as the relationship that does hold. The
  discrepancy and its cause are stated in `size_bytes`'s doc comment in
  `cmd/uninstall.rs`.

Open items:

- `cargo fmt --check` still reports `crates/verbatim/tests/hook.rs` (PLAN-1's
  file, outside this lease), unchanged from PLAN-2's report. `cargo fmt -p
  verbatim` reformats it, so this dispatch reverted that file rather than carry
  an out-of-lease change. Every file this plan touched is `rustfmt` clean.
- `doctor` is not in `crates/verbatim/tests/cli.rs`'s `DATA_COMMANDS` sweep,
  which is where the other six commands are held to the envelope contract.
  `cli.rs` is outside this plan's lease, so the doctor document's envelope,
  streams and exit-code properties are asserted in `tests/doctor.rs` instead.
- `LOW_CLEANUP_DAYS`, `CLAUDE_DEFAULT_CLEANUP_DAYS` and `KEEP_CLEANUP_DAYS` are
  stated in both `cmd/install/mod.rs` (private) and `cmd/doctor.rs`. Same
  policy, two homes; comments in doctor say so.
- The command that removes a file (`rm -f '<path>'` / `del "<path>"`) is built in
  both `install::occupied` and `doctor::removal`. Same reason.
- The six shipped `--json` commands still call `Document::emit` and still panic
  with exit 101 on a closed pipe. `try_emit` is additive and only doctor uses it,
  per the plan's own instruction not to widen the fix here.
- Doctor spawns two `--version` processes (the stable binary, and `claude` off
  PATH) with a 2 s budget each. That is fine for a diagnostic a human runs and is
  not on any hook path, but doctor is the only command in this binary that
  spawns anything.
- The brief's free-space check is still unimplemented and unplanned: `std` has no
  stable free-space API and the only route is the `libc`/`windows-sys` dependency
  D-04 refuses. Flagged in PLAN-3's own notes; recorded here so it is not lost.
- `crates/verbatim/tests/hook.rs`'s
  `the_ingest_survives_a_group_kill_and_a_descendant_sweep` failed once under a
  full `cargo test` (the hook exited 0 before the test's group kill landed) and
  passed on the two full-suite runs after it and on every isolated run. A timing
  race in PLAN-1's test under parallel load, outside this lease, unrelated to
  anything this plan touched - recorded so it is not discovered as new.
- `--purge` deletes the whole data directory with `remove_dir_all`, guarded by
  the presence of `verbatim.db` in it. A data directory the user shares with
  something else would lose that too. The guard is what keeps a mistyped
  `VERBATIM_DATA_DIR` from being a catastrophe, but it is a guard rather than a
  proof of ownership.
- `size_bytes` and the `human` byte formatter are now stated in both
  `cmd/status.rs` (private) and `cmd/uninstall.rs`. `status.rs` is outside this
  plan's lease; one `pub(super)` on each would fold them together.
- Uninstall does not restore a settings file that was deleted between install
  and uninstall, even though install's backup is sitting beside it. Reported as
  "not there: nothing to remove" and left alone: a file the user deleted is not
  a file to recreate. Naming it because the backup then stays on disk with
  nothing that will ever remove it.
