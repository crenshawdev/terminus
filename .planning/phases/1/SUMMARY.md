---
phase: 1
status: complete
completed: 2026-10-13
---

# Phase 1: No Model Calls - Summary

Terminus no longer calls a model, reads an API key or opens a network connection: the judgment, provider, network, cost and credentials code and `ureq` are gone, a leftover `[provider]` table refuses to load, and the ten summary columns are dropped from `observations` on the first writable open.

## What shipped

- Session judgment removed from the ingest pass and from `observations regenerate`, which now rebuilds the `mechanical` column only and refuses `--prompt-version` - `ingest/pass.rs`, `observe/mod.rs`, `cmd/observations.rs`
- `observe/judgment.rs`, `provider.rs`, `net.rs`, `cost.rs`, `credentials.rs` and their tests deleted; `observe/` holds `egress.rs`, `mechanical.rs`, `mod.rs`
- `ureq` and the test kit's HTTP stub removed; `cargo tree` names no HTTP or TLS crate; `tests/no_network.rs` fails on `std::net`, `TcpStream`, `TcpListener` or `UdpSocket` under `crates/*/src`
- `doctor` loses `credentials`, `config_mode`, `mode_checks` and `provider_local` - `cmd/doctor.rs`
- Egress keeps its shape rules for `redact_recall` and loses rule 1, `for_destination`, `scrub` and `Secret` - `observe/egress.rs`
- Any top-level `provider` key fails `Config::load_from` with `Error::ProviderRemoved`, naming the file and printing none of its values - `config.rs`, `error.rs`
- The planted-secret redaction proof moved from a stubbed provider request onto a recall context window - `tests/egress.rs`
- `kind: observation` refused by name on `terminus search` (exit 2) and `recall_search` (`isError`); recall's observation branch, `CLAIMS`, `OBSERVATION_KIND` and `Reason::NoObservations` gone - `cmd/search.rs`, `cmd/mcp/tools.rs`, `recall/search.rs`, `recall/scope.rs`
- `terminus observations` lists `session_key`, `session_id`, `generated_at` and `mechanical` only
- `drop_summary_columns` drops the ten summary columns and the `observe.daily_tokens` meta row inside one IMMEDIATE transaction on a writable open of a store that still has them; column presence is the only record - `store/open.rs`, `store/schema.rs`, AC4 test in `tests/schema.rs`
- D-02's reason that observations stay archival rewritten everywhere it is stated; `docs/json-shapes.md`, `DESIGN-BRIEF.md` and a `### Removed` entry in `CHANGELOG.md` updated; the phase 7 remote-observations item removed from `CAPTURE.md`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 50a644a | prove every planted secret is caught on a recall context window |
| 1 | 2 | e5d9614 | stop judging sessions at the end of an ingest pass |
| 1 | 3 | 4af568d | let observations regenerate rebuild the mechanical facts and nothing else |
| 1 | 4 | 7cf57d5 | delete the judgment, provider, network and cost code |
| 2 | 1 | 776d522 | drop the HTTP client and fail the build on any socket in source |
| 2 | 2 | efabb2d | take the key and provider checks out of doctor, and the credentials module with them |
| 2 | 3 | bf21fba | keep the shape rules for recall and drop the provider credential from egress |
| 2 | 4 | 4520c11 | refuse a config that still has a [provider] table |
| 3 | 1 | 4881f3c | refuse kind observation by name on both search front ends |
| 3 | 2 | d507858 | take the observation branch out of recall search |
| 3 | 3 | 04c7e39 | list the mechanical facts only, before the summary columns go |
| 3 | 4 | c5bd760 | drop the ten summary columns the first time an old store opens |
| 4 | 1 | bb198cc | keep observations archival because eviction empties the blob |
| 4 | 2 | 3ec0e97 | show the observation shapes without the summary half |
| 4 | 3 | be0b315 | say in the brief and changelog that Terminus calls no model |

## Deviations

- [deviation] Plan 1 Task 2: its Verify search matched two doc comments in `observe/mod.rs` that Task 4 owned, so the D-07 paragraph was removed and `rejudge`'s doc reworded early (e5d9614).
- [deviation] Plan 1 Task 4 stopped at a structural checkpoint: three doc comments outside the plan's lease (`cmd/doctor.rs`, `testkit.rs`, `config.rs`) still named deleted modules. The owner chose to exclude them from plan 1's search and carry the full search into plan 2 Task 4's Verify, where it came back empty (7cf57d5, 4520c11).
- [deviation] Plan 2 Task 2: its Verify search also matched lines in `config.rs`, `tests/config.rs` and `tests/egress.rs` that Tasks 3 and 4 removed; after Task 4 it printed only the absence test (efabb2d).
- [deviation] Plan 4 Task 1: its Verify search for `paid|model call|judgment half|summaries a user` still prints 13 hits outside the lease - D-04's own "Model calls were removed" refusal text, "paid" meaning cost elsewhere, and the stale `ingest/pass.rs` comment under Open items. None gives a reason for observations being archival (bb198cc).

## Open items

- `crates/terminus-core/src/ingest/pass.rs:319-321` still says the judgment half runs outside the lock because a slow provider would block hook passes. Stale since plan 1; outside every plan's lease.
- The owner's installed `terminus` 0.1.1 (`~/.local/bin`) keeps archiving against a store this branch has upgraded, but its `terminus observations` listing and `kind: observation` search fail with `no such column` until the new build is installed.
- Left out of D-09 by the planner, for the owner to rule on: `tests/fixtures/README.md` still says the secrets fixture is caught "on the remote provider path"; `.planning/PROJECT.md` Constraints still allows a network connection to a model provider; `npm/terminus/README.md` mentions a model provider (phase 2 deletes it); CAPTURE items 131, 132, 135, 137, 138 and 141 describe removed code; `Config::source_path` has only test callers.
- `cargo fmt --all --check` lists about ten test files with formatting drift that predates this phase.
- Plan 2's executor ran `git stash` / `git stash pop` once over the main working tree while checking formatting; it reports `git status` unchanged afterwards and the older `stash@{0}` untouched.
- Filed from the risk reviews: crenshawdev/terminus#4 (the planted-secret test checks generic markers over the whole window, not per turn) and #5 (no shape rule catches a bare `sk-` key in prose).

## Goal check

The commits deliver the goal. `cargo tree --workspace -e all --target all` names none of `ureq`, `reqwest`, `hyper` or `rustls`, and `rg 'std::net|TcpStream|TcpListener|UdpSocket' crates/*/src` and `rg '_API_KEY|credentials\.toml' crates/*/src` both come back empty (AC1, AC2). `crates/terminus-core/src/observe` holds only `egress.rs`, `mechanical.rs` and `mod.rs`, and the unexcluded search for the four deleted modules prints nothing. The `[provider]` refusal is `Error::ProviderRemoved` (`error.rs:159`), proven at the process boundary in `tests/status.rs` with a sentinel key that appears in no output (AC3). The column drop ran on a copy of the owner's 2026-10-12 snapshot: 14 columns to 4, 6,440 rows before and after, every `mechanical` byte-identical, the meta key gone, `terminus verify` 6,442 sessions and 0 failed (AC4). Mechanical observations are still written and listed (`a_finalized_session_is_listed_with_every_obs_01_fact`), and turn search over paths and entities is unchanged (AC5). Redaction on recall is proven per planted shape on the context window and by the existing search, get and brief tests (AC6), though #4 notes the marker check is weaker than its name for the generic-marker rows. Doctor's four checks are gone with an absence test, and the last full suite run passed 748 tests with 0 failed (AC7). What remains is the stale `ingest/pass.rs` comment and the planner's out-of-scope leftovers above.
