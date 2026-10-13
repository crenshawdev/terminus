---
phase: 1
plan: 4
requirements: [PRIV-05]
files:
  - crates/terminus-core/src/store/schema.rs
  - crates/terminus-core/src/observe/mod.rs
  - crates/terminus-core/src/retention/apply.rs
  - crates/terminus-core/tests/retention.rs
  - crates/terminus-core/tests/schema.rs
  - crates/terminus-core/tests/reindex.rs
  - crates/terminus-core/tests/observe.rs
  - docs/json-shapes.md
  - DESIGN-BRIEF.md
  - CHANGELOG.md
  - .planning/CAPTURE.md
---

# Phase 1: No Model Calls - Plan 4

## Goal

Terminus never calls a model, never handles an API key and opens no network connection, matching Baley's ADR 0039. The summary half of observations goes with everything that exists only for it, while the mechanical half and the shape rules `redact_recall` uses stay.

## Must be true when done

- Every place that says why `observations` is archival gives D-02's reason. Retention's `runs.error` line about deleted observation rows no longer calls an observation a model call.
- `docs/json-shapes.md` shows the `observations` and `observations regenerate` shapes as they are now, and says `kind: observation` is refused.
- `DESIGN-BRIEF.md` describes Terminus as it now stands: no judgment, no provider, no shared credentials and no network connection.
- `CHANGELOG.md` `[Unreleased]` has a Removed entry, and CAPTURE's item about verifying remote observations is gone.
- The full suite passes.

## Context

- Plan 4 of 4, after plan 3.
- Binding decisions: D-02 (observations stay archival, for a new reason) and D-09 (docs).
- D-09's `Cargo.toml` half already landed in plan 2 Task 1, together with the `ureq` entry it described.
- The shipped PRIV-02 and OBS-02 to OBS-06 rows in REQUIREMENTS.md stay as written.

## Tasks

### Task 1: Observations stay archival, for the reason that is true now

- **Files:** crates/terminus-core/src/store/schema.rs (anchor: module doc / the `observations` comment in `CREATE_SQL` / `DERIVED_TABLES`), crates/terminus-core/src/observe/mod.rs (anchor: the "Archival - never derived" paragraph / `observe_new` / `write_one`), crates/terminus-core/src/retention/apply.rs (anchor: `Applied::observations_lost` / `Applied::lines`), crates/terminus-core/tests/retention.rs, crates/terminus-core/tests/schema.rs (anchor: `observations_is_not_a_derived_table`), crates/terminus-core/tests/reindex.rs (anchor: `a_reindex_leaves_every_observation_row_untouched`), crates/terminus-core/tests/observe.rs (anchor: the test doc saying a pass cannot discard a judgment half)
- **Action:**
  - **What stays the same (D-02).** `observations` stays out of `DERIVED_TABLES`, and `reindex` still never touches it. Only the reason changes.
  - **The new reason.** Wherever the reason is given as a paid model call, it becomes: the mechanical facts come off the session blob, and retention eviction empties that blob, so an evicted session's observation cannot be recomputed.
  - **Where to change it.**
    - `schema.rs`: the module doc, the opening paragraph of the `observations` comment in `CREATE_SQL`, and the doc on `DERIVED_TABLES`.
    - `observe/mod.rs`: the "Archival, never derived (D-02)" paragraph, and the docs on `observe_new` and `write_one` that say why a pass inserts and never overwrites.
    - `retention/apply.rs`: the `Applied::observations_lost` doc, and the `lines` text that tells `runs.error` a deleted observation "is a model call no rebuild reproduces". A deleted session's blob is gone with it, so the line says that instead.
    - The test docs named in Files, and the assertion message in `tests/retention.rs` that calls the loss "a paid model call".
  - **Behaviour does not change.**
- **Verify:** `rg -n -i 'paid|model call|judgment half|summaries a user' crates --glob '*.rs'` prints nothing. `cargo test --workspace --all-features` passes; the retention test still finds `observation` in the line.

### Task 2: `docs/json-shapes.md` shows the shapes as they are

- **Files:** docs/json-shapes.md (anchor: `search`'s `--kind observation` bullet / `### observations` / `### observations regenerate`)
- **Action:**
  - **Current state only.** No change log. Per `~/.claude/john-voice.md` "Design docs", these docs describe the system as it stands now.
  - **`search`.** The `--kind observation` bullet becomes this: `kind` takes `user`, `assistant`, `system` or `attachment`. `observation` is refused, as misuse (exit 2) from `terminus search` and as `isError` from `recall_search`, because Terminus stores no session summaries.
  - **`observations`.** The example drops the ten judgment keys. The bullets on the judgment columns, the `status` shapes and reservations go. The bullet saying no model is called stays.
  - **`observations regenerate`.**
    - The example and the selector bullet lose `prompt_version` and `--prompt-version`.
    - The bullets on re-asking the model and on the judgment half's notes go, and so does the HTTP clause in the lock bullet.
    - The "only rebuild path" bullet keeps its claim, with D-02's reason.
  - **Accuracy.** Every key, flag and exit code the doc names must match the code as plans 1-3 left it.
- **Verify:** `rg -n -i 'judgment|provider|prompt.version|paid|reservation' docs/json-shapes.md` prints nothing. (`judg` alone would also match the replay section's unrelated "judge".) The example in `### observations` has exactly the keys `Row::to_value` emits. The example in `### observations regenerate` has exactly the keys `tests/cli.rs` lists for that envelope.

### Task 3: The design brief, changelog and capture list say no model calls

- **Files:** DESIGN-BRIEF.md (anchor: §9 Observations / "Providers" / "Shared credentials" / §10 Privacy / the concurrency paragraph / the SessionStart block table / the command table), CHANGELOG.md (anchor: `## [Unreleased]`), .planning/CAPTURE.md (anchor: the phase 7 item "Verify AC4's remote half for observations")
- **Action:**
  - **`DESIGN-BRIEF.md` (D-09).** Describe the current state only.
    - §9 keeps the mechanical facts, which are always on, and corrects its "Opt-in, off by default" head.
    - §9 loses the Judgment block, the strict-JSON and parse-failure paragraph, the cost-control line, "Providers" and "Shared credentials".
    - §10's quoted contract, its "key redaction on destination" paragraph, and "no network connections except to the model provider" become what is true now: recall output is filtered by the shape rules when `redact_recall` is on, and Terminus calls no model, handles no key and opens no network connection.
    - The concurrency paragraph's "blocking HTTP scoped to the observation command only" goes.
    - The SessionStart table's "Summary / open threads" row goes.
    - The command table's `observations regenerate` loses `[--prompt-version]`.
    - Lines contrasting claude-mem's model summaries stay.
  - **`CHANGELOG.md`.** Add a `### Removed` section under `[Unreleased]`, written for a user. It says that:
    - Terminus no longer calls a model, reads an API key or opens a network connection;
    - a `terminus.toml` with a `[provider]` table now fails to load, with a message naming the file;
    - `credentials.toml` is no longer read;
    - stored session summaries (the ten columns) are dropped from an existing store on its first open, and the mechanical facts are kept;
    - `kind: observation` is refused;
    - `observations regenerate --prompt-version` is gone;
    - `terminus doctor` no longer reports `credentials`, `config_mode`, `mode_checks` or `provider_local`.

    This is text John puts his name on, so read `~/.claude/john-voice.md` first.
  - **`.planning/CAPTURE.md`.** Remove the phase 7 item asking to verify AC4's remote half for observations, because the feature it verifies no longer exists (recalled: `CAPTURE.md`, phase 7).
- **Verify:**
  - `rg -n -i 'judgment|provider|credentials\.toml|api.key|prompt.version|blocking HTTP' DESIGN-BRIEF.md` prints nothing that describes Terminus calling a model or holding a key.
  - `CHANGELOG.md` `[Unreleased]` has a `### Removed` section covering each item above.
  - `rg -n 'remote half for observations' .planning/CAPTURE.md` prints nothing.
  - `cargo test --workspace --all-features` still passes.

## Notes

- These docs are stale after this phase but outside D-09's list. They are left for the human to decide:
  - `tests/fixtures/README.md`'s `session-secrets.jsonl` row says the shapes are caught "on the remote provider path".
  - `.planning/PROJECT.md` Constraints says "no network connections except the configured model provider" and "credentials `0600`-enforced".
  - `npm/terminus/README.md` mentions a model provider. Phase 2 deletes that file.
- CAPTURE items 131, 132, 135, 137 and 138 (cost, provider, reservation, and regenerate's judged fields) and 141 (the brief reading observations) describe removed code too. D-09 names only the remote-verification item, so they are left for the human.
