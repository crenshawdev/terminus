---
phase: 1
plan: 3
requirements: [PRIV-05]
files:
  - crates/terminus/src/cmd/search.rs
  - crates/terminus/src/cmd/mcp/tools.rs
  - crates/terminus/tests/mcp.rs
  - crates/terminus/tests/recall_cli.rs
  - crates/terminus-core/src/recall/search.rs
  - crates/terminus-core/src/recall/scope.rs
  - crates/terminus-core/src/recall/mod.rs
  - crates/terminus-core/tests/recall.rs
  - crates/terminus-core/tests/redacted_recall.rs
  - crates/terminus/src/cmd/observations.rs
  - crates/terminus/tests/observations.rs
  - crates/terminus-core/src/store/schema.rs
  - crates/terminus-core/src/store/open.rs
  - crates/terminus-core/tests/schema.rs
  - crates/terminus-core/tests/reindex.rs
---

# Phase 1: No Model Calls - Plan 3

## Goal

Terminus never calls a model, never handles an API key and opens no network connection, matching Baley's ADR 0039. The summary half of observations goes with everything that exists only for it, while the mechanical half and the shape rules `redact_recall` uses stay.

## Must be true when done

- `recall_search` with `kind: "observation"` comes back `isError`, naming the kind and saying session summaries were removed. `terminus search --kind observation` is misuse (exit 2) with the same wording.
- The `kind` description lists user, assistant, system and attachment and mentions no summaries, and there are still exactly three tools.
- `terminus observations --json` rows carry `session_key`, `session_id`, `generated_at` and `mechanical`, and none of the ten judgment keys.
- A store whose `observations` table still has `status`, `model`, `prompt_version`, `topic`, `outcome`, `decisions`, `learned`, `unresolved`, `raw` and `tokens` opens with none of them. After that open:
  - every row's `mechanical` is byte-identical to before;
  - the `observe.daily_tokens` meta key is gone;
  - `verify` is clean;
  - a fresh store's `observations` columns equal the upgraded store's.
- A session finished after the change still gets its mechanical observation, and `terminus observations` lists it. Its files, commands and errors stay searchable through the turn index, which this plan does not touch (D-03).
- The full suite passes and clippy with `-D warnings` is clean.

## Context

- Plan 3 of 4, after plan 2.
- Binding decisions:
  - D-01: drop the stored summaries, the `kind` branch, `CLAIMS`, `OBSERVATION_KIND` and `Reason::NoObservations`, and change the MCP `kind` description.
  - D-03: how criterion 4 is read.
- Planner's calls are recorded in Tasks 1 and 4: refusing the retired kind by name, and how the drop runs exactly once.
- Out of scope: re-pointing `kind: observation` at the mechanical facts. `export` is untouched because it never exported observations.

## Tasks

### Task 1: Both search front ends refuse `kind: observation` by name

- **Files:** crates/terminus/src/cmd/search.rs (anchor: `parse`), crates/terminus/src/cmd/mcp/tools.rs (anchor: the `recall_search` handler and its `kind` input-schema property), crates/terminus/tests/mcp.rs, crates/terminus/tests/recall_cli.rs
- **Action:**
  - **Why refuse.** This is the planner's call on CONTEXT's flagged assumption, within phase 7 D-17's rule that an accepted kind silently returning zero hits is the worst outcome. Once summaries are gone, `observation` would otherwise fall through to `t.record_type = 'observation'` and match nothing.
  - **The two refusals.**
    - `terminus search`'s `parse` refuses `--kind observation` as `Failure::Misuse`.
    - `recall_search`'s handler refuses `kind: "observation"` with `Refused::caller`, right where it reads `kind` and before it opens a reader.
    - Both messages name `observation` and say session summaries were removed from Terminus, in plain words.
    - The refusal matches the exact value, the same way `kind` matches everywhere else.
  - **Why the front ends.** Every other argument is validated there. `recall::search::run` has no refusal channel short of a new `Reason`, and D-01 removes the one it had.
  - **The `kind` description.** It names the four record types only, with no mention of `observation` or summaries. It no longer reads `terminus_core::recall::OBSERVATION_KIND`, which Task 2 deletes.
  - **Tests.**
    - In `tests/mcp.rs`, replace `recall_search_reaches_observations_through_the_kind_it_already_has` with a test that the call is `isError` with that message, that the tool list still has three tools, and that the `kind` description contains neither `observation` nor `summar`.
    - In `tests/recall_cli.rs`, replace `the_terminal_search_reaches_observations_through_the_same_kind` with a test that asserts exit 2 and the message on stderr.
    - Both of the replaced tests inserted judgment columns by hand, and Task 4 drops those columns.
- **Verify:** `cargo test -p terminus --all-features --test mcp --test recall_cli` passes. `cargo run -q -p terminus -- search --kind observation anything; echo $?` prints the refusal and exits 2, failing during argument parsing before anything is opened. `rg -n 'OBSERVATION_KIND' crates/terminus/src` prints nothing.

### Task 2: Recall's observation branch goes

- **Files:** crates/terminus-core/src/recall/search.rs (anchor: `OBSERVATION_KIND` / `observations` / `CLAIMS`), crates/terminus-core/src/recall/scope.rs (anchor: `Reason::NoObservations`), crates/terminus-core/src/recall/mod.rs, crates/terminus-core/tests/recall.rs, crates/terminus-core/tests/redacted_recall.rs
- **Action:**
  - **`search.rs` (D-01).** Remove `OBSERVATION_KIND`, the branch in `search::run` that switched to it, the `observations` function, `CLAIMS` and `CLAIMS_TAIL`. Remove the sentence in `Filters::kind`'s doc that names the observation kind.
  - **`scope.rs` and `mod.rs`.** `Reason::NoObservations` and its `Display` arm go from `scope.rs`, and `OBSERVATION_KIND` goes from `recall/mod.rs`'s re-export.
  - **`tests/recall.rs`.** Remove the whole OBS-08 section: its claim-row helper, its search helper, `the_observation_kind_returns_claims_anchored_to_real_turns`, `an_excluded_projects_observation_is_hidden_and_a_siblings_is_not`, `a_store_with_no_observations_table_says_so_rather_than_nothing`, any other test under that header, and the `OBSERVATION_KIND` import.
  - **`tests/redacted_recall.rs`.** Remove the "observation branch, filtered on read" section: `observe`, `a_claim_stored_before_the_knob_was_set_is_filtered_when_it_is_read` and `an_ordinary_claim_reads_back_the_same_under_both_settings`.
  - **What stays.** The turn search, its excerpt redaction and every other `Reason` stay as they are.
- **Verify:** `rg -n 'OBSERVATION_KIND|NoObservations|CLAIMS' crates --glob '*.rs'` prints nothing. `cargo test -p terminus-core --all-features --test recall --test redacted_recall` passes, and the full suite passes.

### Task 3: `terminus observations` lists the mechanical facts only

- **Files:** crates/terminus/src/cmd/observations.rs (anchor: `list` / `Row` / `nested`), crates/terminus/tests/observations.rs (anchor: `a_finalized_session_is_listed_with_every_obs_01_fact`)
- **Action:**
  - **The listing.** Its query selects `session_key`, `session_id`, `generated_at` and `mechanical`, and nothing else. `Row` and `Row::to_value` lose the ten judgment fields.
  - **Docs.** `Row`'s doc stops promising every column of a row that has a judgment half. `nested`'s doc speaks of the `mechanical` document alone.
  - **What does not change.** The human rendering never printed a judgment field, so it stays as it is.
  - **Test.** `a_finalized_session_is_listed_with_every_obs_01_fact` asserts that each of the ten keys is absent from the row, where it used to assert each was null.
  - **Order.** This lands before Task 4 so that no read names a column the store is about to lose.
- **Verify:** `cargo test -p terminus --all-features --test observations` passes. `rg -n 'prompt_version|o\.status|o\.topic|o\.decisions' crates/terminus/src` prints nothing.

### Task 4: The store drops the ten summary columns once

- **Files:** crates/terminus-core/src/store/schema.rs (anchor: `CREATE_SQL`'s `observations` table / `BRING_FORWARD_COLUMNS`), crates/terminus-core/src/store/open.rs (anchor: `Store::open` / `bring_forward`), crates/terminus-core/tests/schema.rs (anchor: `column_layout`), crates/terminus-core/tests/reindex.rs (anchor: `a_reindex_leaves_every_observation_row_untouched`)
- **Action:**
  - **Fresh stores (D-01).** `CREATE_SQL`'s `observations` table declares `session_key`, `session_id`, `generated_at` and `mechanical`, in that order and nothing else. Remove the comments on the removed columns, the paragraph about claim lists having no reference to `turns`, and the paragraph about `observations.decisions` versus the `decisions` table. The block's opening paragraph on the reason belongs to plan 4 Task 1 (D-02).
  - **Existing stores: the drop.**
    - It runs in `Store::open`, on every writable open, after `bring_forward`. `bring_forward` stays additive-only, as its doc promises.
    - It drops whichever of the ten columns `PRAGMA table_info(observations)` still lists, using `ALTER TABLE observations DROP COLUMN`, and deletes the `observe.daily_tokens` meta row.
    - All of that happens in one transaction that takes SQLite's write lock before it reads the column list (`TransactionBehavior::Immediate`, already imported in `open.rs`). That way two processes opening at once cannot both attempt the drop and have the second fail on a missing column.
    - Keep the ten names in one place in `schema.rs`, beside `BRING_FORWARD_COLUMNS`.
  - **How it runs exactly once (planner's call on the flagged assumption).** The record that it ran is the columns being gone. There is no marker key, and no `ARCHIVE_FORMAT` or `DERIVED_SCHEMA` bump, which would rebuild every derived table of a 2.4 GB store for a change to one table that is not derived. When none of the ten is present, nothing is written: the check is one `PRAGMA` read, the same per-table cost `bring_forward` already pays.
  - **Read-only opens.** `open_read_only` drops nothing, because a read never migrates. It does not need to, since after Tasks 1-3 no query names a dropped column.
  - **The row stuck at `judging`.** It keeps its row and its `mechanical`. Only its status column goes.
  - **SQLite support.** The bundled SQLite 3.53.2 supports `DROP COLUMN`, and none of the ten columns is a key, indexed, or referenced. Checked during planning against system sqlite 3.54: `DROP COLUMN` inside a transaction succeeds with `foreign_keys` on and an orphan child row present, so an orphaned observation row cannot wedge the open.
  - **AC4 test in `tests/schema.rs`.**
    - Set-up: rebuild the `observations` table with the pre-change DDL (all fourteen columns, spelled out in the test). Give it rows for real archived sessions, because the foreign key is enforced, including one `ok` row with claims and one at `judging <timestamp>`. Add an `observe.daily_tokens` meta row.
    - After reopening with `Store::open`:
      - `PRAGMA table_info(observations)` lists none of the ten;
      - every row's `mechanical` is byte-identical to before;
      - the meta row is gone;
      - `terminus_core::verify::verify` reports no failure;
      - the column layout equals a fresh store's, compared with `column_layout`;
      - a second open succeeds and leaves the same columns and rows.
  - **`tests/reindex.rs`.** `a_reindex_leaves_every_observation_row_untouched` inserts and compares the four remaining columns. Its doc comment belongs to plan 4 Task 1.
- **Verify:**
  - `cargo test -p terminus-core --all-features --test schema --test reindex --test store_open` passes.
  - `cargo test --workspace --all-features` passes and clippy with `-D warnings` is clean.
  - Human check on real data (human-verify: needs the owner's store):
    - Copy one rolling snapshot `terminus-<time>.db` from the data directory into an empty directory as `terminus.db`.
    - Run this branch's `terminus status` with `TERMINUS_DATA_DIR` pointed at that directory.
    - Then `sqlite3 <dir>/terminus.db 'PRAGMA table_info(observations)'` lists four columns, and `SELECT count(*) FROM observations` equals the count before the upgrade (2,530 at the 2026-10-12 read).

## Notes

- The front-end refusal is the planner's call on the flagged assumption about the retired kind. It is refused, not answered with nothing.
- The run-once mechanism is the planner's call on the flagged assumption about recording the drop. Column presence is the record. There is no marker and no format bump.
