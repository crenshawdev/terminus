---
phase: 5
status: complete
completed: 2026-08-20
---

# Phase 5: Context Injection - Summary

SessionStart resume brief (index pointer + last-session block, budgeted, deterministic) and UserPromptSubmit precision-first injection (candidate spellings -> structural threshold -> INJ-04 suppressions -> INJ-05 compaction debt), all through the phase-4 hook with silence as every failure mode.

## What shipped

- `[injection]` config table (`brief_chars` 6000 / `prompt_chars` 4000, 10,000-char hard clamps) - `crates/verbatim-core/src/config.rs`
- Hook payload parsing (five optional fields off `serde_json::Value`) and one JSON object on stdout for the two events that have one - `crates/verbatim/src/cmd/hook.rs`, `crates/verbatim-core/src/inject/`
- Failure-is-silence inside a 50 ms watchdog, 3 ms busy timeout, schema version gate - `crates/verbatim/src/cmd/hook.rs`
- Live-capture proof that `/compact` fires `SessionStart` with `source: "compact"` (D-08's affirmative arm holds; fallback not needed) - `tests/fixtures/hooks/`, `tests/fixtures/README.md`
- Resume brief: last non-subagent session, branch, last user/assistant turns cut from one blob read; byte-for-byte deterministic; measured p50 1.27 ms / p99 2.35 ms against the 10 ms budget - `crates/verbatim-core/src/inject/brief.rs`
- Retrieval: `Hit::entity_match`/`entity_count`, relative paths resolved against payload `cwd`, candidate spellings (capped 8) disjoined across / conjoined within, structural threshold (entity match AND rank<3, OR >=2 distinct entities), cap 3, excerpts attached only after the threshold - `crates/verbatim-core/src/inject/prompt.rs`, `crates/verbatim-core/src/recall/search.rs`
- INJ-04 suppressions: per-session state file (allow-listed `session_id`, tmp+rename, fail-open reads), already-injected / visible-in-session / carried-by-brief refusals recorded - `crates/verbatim-core/src/inject/state.rs`
- INJ-05: compaction-owed flag set by `source: "compact"`, spent by the next prompt; dropped set = complement of `preservedMessages.uuids`, infallible reads - `crates/verbatim-core/src/inject/compaction.rs`
- Final workspace state: 448 tests passed, 0 failed; clippy `-D warnings` clean.

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 24bf33b | `[injection]` config table with defaults and clamps |
| 1 | 2 | 624b47a | Hook keeps its payload instead of dropping it |
| 1 | 3 | d102635 | One JSON object on stdout on the two events that have one |
| 1 | 4 | 5489773 | Every injection failure is silence, inside a deadline |
| 1 | 5 | d1fe832 | Compaction fires SessionStart with `source: "compact"` (live capture + fixture) |
| 2 | 1 | a6e423c | Brief opens with where the last session left off |
| 2 | 2 | 116dae2 | Brief fits its budget by quoting less, not saying less |
| 2 | 3 | ef45d2b | A brief that changes while the archive does not is the bug |
| 2 | 4 | fa4fbde | A hundred briefs, timed, inside the configured budget |
| 3 | 1 | 426a205 | A hit says how its entities matched, and how many |
| 3 | 2 | ef376a9 | Fixture storing a path the way real transcripts do |
| 3 | 3 | c716ecd | A relative path in a prompt finds the absolute one |
| 3 | 4 | 31e89f5 | Structural threshold, and the excerpt nobody pays for |
| 3 | 5 | 4176a6b | The injected turns, and the prompt that gets nothing |
| 4 | 1 | 83e196f | A file that remembers what this session was already given |
| 4 | 2 | 755df5f | Nothing injected twice, nothing already on screen at all |
| 4 | 3 | 312e64a | The turns that fell out of context, from the bytes ingest kept |
| 4 | 4 | 3743415 | The first prompt after a compaction asks what fell out |

## Deviations

- [deviation] Plan 1 task 4: lowering the read-only busy timeout does not bound the open itself - `Store::open_read_only` applies its own 5 s timeout and queries `sqlite_master` before returning. Same-process exclusive holder waits the full 5,011 ms; another process (the real case) reports back in ~6 ms. Both mechanisms built as planned; docs and tests state what each actually bounds. AC6 met; bounding the open needs `store/open.rs`, outside the lease. (5489773)
- [deviation] Plan 1 task 5: five identity fields replaced in the captured fixture, not three - `prompt_id` (a real session's UUID) and `model` (a build-specific unpublished name, and this is a public repo) also synthesized. The two fields anything reads (`hook_event_name`, `source`) are exactly as captured. (d1fe832)
- [deviation] Plan 1 task 5: the capture contradicts `tests/fixtures/README.md`'s account - `prompt_id` is not an unconditional base field, and `permission_mode`/`agent_type`/`session_title` were absent from both live lines. Nothing reads those fields; README and `Payload::source` doc record the contradiction. (d1fe832)
- [deviation] Plan 3 task 1: `src/worker/S.ts` is prose-only in the fixture, so it cannot be the stored-value exact-match case (RCL-02 never extracts from sentences); `docs/RETRY.md` (a real `Read` record) serves that case and `src/worker/S.ts` became the free-text control. (426a205)
- [deviation] Plan 3 task 1: the covered case is unobservable through a conjunctive query as written (`who edited docs/RETRY.md yesterday` matches nothing); asserted with `Read docs/RETRY.md` instead. (426a205)
- [deviation] Plan 3 task 4: a search over the prompt's own conjunctive `Query` is vacuous for injection - it fires only when a turn repeats the sentence. `Request::candidates` now carries the spellings the prompt names (resolved paths, relative spelling, identifier tokens, capped 8), disjoined across and conjoined within; prose stays in `Request::query` for weighting and excerpts. Threshold, limit, cap and deferral exactly as planned; both existing search front ends byte-identical. All candidate terms pass through `Query::match_expression`, so untrusted input never reaches `MATCH` raw. (31e89f5)
- [deviation] Plan 4 task 2: AC4's dedupe falsifies two PLAN-3 assertions in `crates/verbatim-core/tests/inject_prompt.rs` that re-invoke the arm under one `session_id`. Raised at a structural checkpoint, approved: lease extended to that file, each repeated call takes its own `session_id`; neither assertion weakened. PLAN-4's `files:` list carries the added path. (755df5f)

## Open items

Filed in `.planning/CAPTURE.md` under phase 5:

- AC6's watchdog timeout arm has no forcing test (needs a testkit fault point in the injection path).
- Pre-existing ETXTBSY flake in `crates/verbatim/tests/hook.rs` spawn sites (reproduced on baseline 3fd5b19).
- Pre-existing rustfmt diffs in `ingest/backfill.rs:250` and `tests/hook.rs:252,681`.
- `inject/brief.rs` keeps a private `chars`/`clip` pair duplicating `inject/mod.rs`; one pending deletion.
- `entity_score` dwarfs bm25 on common terms - phase 6's auto-tuner gates should be told.
- The `error` entity kind has no candidate spelling of its own; pure-prose failure text opens no store.
- `inject::state::MAX_SUPPRESSED` caps the suppression list at 100 oldest-dropped; FEED-01 owns the durable record.
- `cmd/hook.rs`'s inject doc has one stale clause: the abandoned thread now writes the D-06 scratch file.

Risk-surface reviews: three blocking fires (plans 1, 3, 4), 5 findings raised, 0 survived - 2 refuted as staged-delivery reads of a mid-sequence phase, 3 downgraded as documented precision-first bounds (`MAX_CANDIDATES`, `RANKED`, the `MAX_TURNS` cut). Records at `ADJUDICATION-risk_surface-plan-{1,3,4}.json`.

## Goal check

The phase goal has both halves on disk with evidence. "Starts a session already knowing where it left off": the brief opens with the last session's date, branch and final exchange (a6e423c, `inject/brief.rs`), renders byte-for-byte identically against an unchanged store (ef45d2b, `crates/verbatim/tests/brief.rs`), and 100 spawned SessionStart runs measured p50 1.27 ms / p99 2.35 ms inside the 10 ms budget (fa4fbde). "The one past turn that matters when a prompt names something": a relative path resolves against `cwd` to the stored absolute entity (c716ecd), injection requires a structural match - entity + rank<3 or two co-occurring entities, never free text alone (31e89f5, `inject/prompt.rs::eligible`) - and AC3's both halves pass in `crates/verbatim/tests/prompt.rs` (4176a6b). "Silence the rest of the time": every failure mode emits nothing inside the 50 ms watchdog (5489773), a repeat prompt injects once (755df5f, `tests/suppression.rs` 3 controls), and the compaction debt carries across the boundary-row race (3743415, D-08 race test). The whole workspace closed at 448 passed, 0 failed. Honest gaps: the watchdog's timeout arm is asserted by no test that forces it, INJ-04's `visible_in_session` over-suppresses dropped turns on prompts after the one that spent the INJ-05 flag (documented, scoped by design), and the injection quality claims rest on fixture-scale stores - phase 6's decision log is what measures precision on the real corpus.
