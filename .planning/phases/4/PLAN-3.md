---
phase: 4
plan: 3
requirements:
  - PRIV-03
  - RCL-09
files:
  - crates/verbatim/src/cmd/search.rs
  - crates/verbatim/src/cmd/show.rs
  - crates/verbatim/tests/recall_cli.rs
---

# Phase 4: Redacted Recall - Plan 3

## Goal

The owner can still read their own archive unfiltered while the knob is on,
through an explicit per-invocation flag on the two terminal read commands that
print a projection - and turning the knob on changes nothing on disk.

## Must be true when done

- With the boolean on, `verbatim show <id>` prints the marker where a planted
  secret was, and the same command with the raw flag prints the secret's bytes.
- With the boolean on, `verbatim search` prints a filtered excerpt and the same
  search with the raw flag prints the sentinel; the flag works the same in
  `--json` mode and adds no key to either document.
- The raw flag exists only on `search` and `show`; `sessions`, `export` and
  `observations` reject it as misuse and exit 2.
- Over one fixture store ingested once, the stored blob bytes and `verbatim
  verify --json`'s document are identical with the boolean on and off, and
  `nothing_in_the_ingest_path_names_the_egress_filter` still passes.

## Context

Locked: D-07 (a per-invocation CLI flag parsed in each terminal read command's
own lexopt loop - never an environment variable, never a second config key; the
hook and the MCP server both load `Config` in-process, so an environment
variable would silently unfilter the model-facing paths, the exact thing AC4
must not enable), D-08 (no `--json` shape gains a field in either setting), D-12
(`verbatim export` stays entirely unfiltered and its manifest notice keeps its
wording), D-13 (`verbatim observations` prints what the column holds). The
forced-off config copy comes from PLAN-1 task 2; the filtering itself is
PLAN-1's and PLAN-2's and is not re-implemented here. `cmd/mod.rs`'s `JSON_FLAG`
note records why a shared-parser shortcut is the wrong shape: it is how
`verify --json` came to look supported before it was.

## Tasks

### Task 1: The raw flag on search and show

- **Files:** crates/verbatim/src/cmd/search.rs (`Args`, `parse`, `run`),
  crates/verbatim/src/cmd/show.rs (`Args`, `parse`, `run`),
  crates/verbatim/tests/recall_cli.rs
- **Action:** Give each of the two commands a `Long("raw")` arm in its OWN
  lexopt loop, beside the `Long(super::JSON_FLAG)` arm each already has, and a
  field on its own `Args` (D-07). When set, the command asks the query layer for
  unfiltered output by passing the forced-off config PLAN-1 task 2 added instead
  of `reader.config()` - `search::run`, `get::records` and `context::window` all
  take the `&Config` as their own argument, so nothing about opening the store
  changes. It governs the projection in both renderings: `--raw --json` emits
  the same keys with unfiltered values, because the flag changes a value and
  never a key (D-08). No environment variable is read for it anywhere, and it is
  not added to any other command: `sessions` prints no projection, `export` is
  out under D-12 and `observations` is out under D-13, and each of those still
  rejects an unknown flag as misuse. Update `cmd/show.rs`'s module doc, which
  today says without qualification that the bytes are the record's own, to name
  the knob and this flag as the way back to them.
- **Verify:** `cargo build --manifest-path /code/verbatim/Cargo.toml
  --workspace` succeeds and `cargo test --manifest-path /code/verbatim/Cargo.toml
  -p verbatim --features testkit --test recall_cli` passes with every existing
  test in that file unmodified, plus one new test showing `search --raw` and
  `show --raw <id>` exit 0 over a seeded store while `sessions --raw`, `export
  --raw` and `observations --raw` each exit 2. `grep -rn '"raw"'
  /code/verbatim/crates/verbatim/src/cmd/` names only the two parse loops, and
  `grep -rniE 'env::var.*raw|VERBATIM_RAW' /code/verbatim/crates/verbatim/src`
  returns nothing.

### Task 2: Prove the terminal round trip at the process boundary

- **Files:** crates/verbatim/tests/recall_cli.rs
- **Action:** Add tests that spawn the built binary through that file's existing
  `Bench` - `ingest_fixtures`, `config` (which writes `verbatim.toml` into the
  config directory the spawn points at) and `run_in` standing in a project
  directory. They cover AC4's terminal half over `session-secrets.jsonl`: with
  the knob written on, `show` of the `Authorization` turn prints the marker and
  none of the sentinel, the same `show` with the raw flag prints the sentinel's
  bytes, and the same pair holds for `search`'s excerpt and for both commands
  under `--json`. With no `verbatim.toml` at all, both commands print the
  sentinel, which is the default this phase must not move. Assert on the spawned
  process's stdout bytes rather than on a library call, for the reason that
  file's own module doc gives.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --features testkit --test recall_cli` passes, with the new tests failing if
  either direction is broken: an assertion that the filtered run carries no
  `VBEGRESS` fragment and one that the raw run carries the exact sentinel
  `sk-VBEGRESS-authz-9f2`. Every existing test in the file passes unmodified.

### Task 3: Prove the archive is untouched in both settings

- **Files:** crates/verbatim/tests/recall_cli.rs
- **Action:** Add one test that ingests the fixture corpus ONCE and then, over
  that one store, compares the two settings: read every `sessions.blob` straight
  out of SQLite and hash them, run `verbatim verify --json`, write the
  `verbatim.toml` that turns the knob on, and do both again. The blob hashes and
  the `verify --json` document must be equal across the two, and the document
  must report zero failures both times - the knob filters a projection on the
  way out and writes nothing (AC3). Keep the ingest before the first read so
  neither setting can be blamed for a different store rather than a different
  render, and do not re-ingest between the two halves.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --features testkit --test recall_cli` passes the new test, and `cargo test
  --manifest-path /code/verbatim/Cargo.toml -p verbatim-core --features testkit
  --test egress` still passes
  `nothing_in_the_ingest_path_names_the_egress_filter` with a non-zero test
  count reported.

## Notes

- Runs after PLAN-1 and PLAN-2, which is what gives it something to turn off.
  It shares no file with PLAN-4, so the two may run in parallel.
- The flag is spelled `--raw` on both commands. If PLAN-1 task 2's forced-off
  affordance was named differently, this plan calls whatever it is; the
  requirement is only that the command passes a config the query layer reads,
  never an environment variable and never a second config key.
