PLAN COMPLETE
Plan: .planning/phases/7/PLAN-1.md
Tasks: 5 of 5
| Task | Commit | Note |
|---|---|---|
| 1 - Declare the `observations` table | 571393b | Added to `CREATE_SQL` and `schema::TABLES`, absent from `DERIVED_TABLES`, no `DERIVED_SCHEMA` bump, no reference to `turns`. Three new tests in `tests/schema.rs`, one in `tests/reindex.rs`; 15 + 8 pass. (First dispatch; see plan-1.1.md.) |
| 2 - Compute the mechanical facts for one session | 837c0ab | `observe::mechanical::observe` - SQL over `paths`/`entities`/`session_meta`/`turns`/`compaction_boundaries` plus one blob read for commands-with-arguments and commit subjects. Lists capped at 64, values at 512 bytes, cuts named in the document. `--test observe` 4 pass, 7 new unit tests pass. (First dispatch.) |
| 3 - Ingest writes an observation per newly finalized session | d936601 | `observe::observe_new` behind `config::visible::sessions`, insert-only (`ON CONFLICT DO NOTHING`), bounded at 100 sessions per pass, notes folded into `runs.error`. Called in `pass::run_with` after `feedback::outcomes` and before `record_pass`. `--test observe` 7 pass. (First dispatch.) |
| - checkpoint resolution | e562684 | `tests/backfill.rs` (approved lease extension): the row-count comparison now excludes the post-walk set as a SET - `observations`, `decisions`, `labels` - with an assertion holding each excluded name to `store::TABLES`. Module doc's stale test count corrected 2 -> 3. `--features testkit --test backfill` 3 pass. |
| 4 - `verbatim observations` lists them | 8dffc43 | `cmd/observations.rs` through `cmd::read::open`, `--project` + `--json`; the visible listing is the outer loop so exclusion holds; `Store::missing_tables` asked before the query; whole row under `data` with the judgment columns present and null; new `### observations` section in `docs/json-shapes.md`. `--test observations` 3 pass, `--bins` 46 pass. |
| 5 - `verbatim observations regenerate` rebuilds a selected set | a9f0fc6 | Nested dispatch inside the command's own parser (D-18); `--since` via `cmd::time_bound`, `--prompt-version`, conjunctive, neither = every visible session. `observe::regenerate` rewrites `mechanical` only, on the selected rows only, through `config::visible::sessions`, under the ingest lock, unbounded. Both verbs added to `DATA_COMMANDS`; `sweep_args` splits a two-word name. `--test observations` 7 pass, `--test cli` 17 pass. |

Verification beyond the per-task commands: `cargo test --workspace --all-features`
green (0 failures, every target), `cargo clippy --all-targets --all-features -D
warnings` clean.

Deviations: none. Nothing the plan asserted turned out false. The first
dispatch's checkpoint was a lease question, resolved by the user with an
approved one-file extension.

Open items:
- `crates/verbatim-core/tests/backfill.rs` was not in PLAN-1's declared
  `files:`. The user approved a one-file lease extension for it; the path was
  added to PLAN-1.md's `files:` list (an unstaged working-tree edit the
  orchestrator's docs commit picks up) so `lease-check` records the grant rather
  than being overridden. Declared count 15 -> 16.
- `regenerate` runs each row's `UPDATE` on its own rather than one transaction
  over the selected set, so a row whose blob will not decompress is a note and
  the rest still rebuild. The scoping claim is unaffected - every rewritten row
  was selected - but a caller wanting all-or-nothing does not have it.
- `observations regenerate --json` emits `command: "observations regenerate"`,
  the two-word name. That is a new spelling in the envelope's `command` field
  (every other value is one word); `docs/json-shapes.md` heads its section the
  same way and `cli.rs`'s sweep asserts the two agree.
- `cargo fmt --all -- --check` reports pre-existing drift in
  `crates/verbatim/tests/hook.rs` (two sites) and
  `crates/verbatim-core/src/ingest/backfill.rs:250`. Untouched by this plan;
  every file this plan wrote is formatted.
