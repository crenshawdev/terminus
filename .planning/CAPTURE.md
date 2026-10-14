# Capture

## Todos

- [ ] (phase 8) The release CI must run `cargo test --workspace --all-features`.
      Without it `testkit` is off and the run still exits 0: `cargo test -p
      terminus-core` skips the 26 test files gated on
      `#![cfg(feature = "testkit")]`, and a bare `cargo test --workspace` skips
      `crates/terminus/tests/mcp.rs`, `recall_cli.rs` (49 tests) and
      `datamove.rs`, gated the same way on the `terminus` package.
- [ ] (phase 7) `observe::cost::spend` is a read-then-write against the `meta` row, so two concurrent increments store one and the day's token accumulator permanently under-reports. Distinct from the bounded one-call overshoot `cost::admits` documents and accepts; an atomic add closes it. Raised by the phase 7 plan 3 risk_surface review and adjudicated down to medium.
- [ ] (phase 7) `observe::cost::admits` runs the minimum-turn and daily-budget gates before the reservation check, so a concurrent caller on an in-flight session reports `BudgetSpent` rather than the new `InFlight` skip. Affects the skip string only, not whether a request is made or what is charged.
- [ ] (phase 7) `crates/verbatim/tests/hook.rs` fails nondeterministically under full-workspace parallel load - a different subset each run, all passing in isolation. Pre-existing process-spawn contention, not a regression (hook.rs is untouched by phase 7). Needs either serialization or a resource guard so the suite is trustworthy under `cargo test --workspace`.
- [ ] (phase 7) `observe::provider::complete` sends `response_format: {"type":"json_schema","strict":true}` from a private constant with no `json_schema` object beside it; the actual schema travels in the system message. A strict remote OpenAI-compatible endpoint may reject that shape - a one-line change in `provider.rs` if AC4's remote verification hits it.
- [ ] (phase 7) OBS-05's Anthropic subscription-auth half was deferred out of phase 7 (CONTEXT's deferral list). The REQUIREMENTS row needs the note saying phase 7 delivered the OpenAI-compatible half only.
- [ ] (phase 7) A judgment reservation abandoned by a killed process is retaken once its 900s lease lapses; where the dead run was an `observations regenerate` over an already-judged row, retaking it buys a second answer for a session that already had one. Bounded by `JUDGED_PER_PASS`. Carrying the replaced status inside the reservation token would let the stealer restore it instead.
- [ ] (phase 7) Promote `judged` and `unasked` from `notes` text to real `data` fields on `observations regenerate --json`. They are already typed on `Regenerated`; promoting them breaks `crates/verbatim/tests/cli.rs`'s exact key-set assertion, so it needs a lease that includes `cli.rs`.
- [ ] (phase 7) `observe::regenerate` runs each row's UPDATE on its own rather than one transaction over the selected set, so a row whose blob will not decompress becomes a note and the rest still rebuild. A caller wanting all-or-nothing does not have it.
- [ ] (phase 7) The SessionStart resume brief still does not read observations, and nothing schedules the work. INJ-01's own text says the brief covers "the index pointer and observations when enabled", and DESIGN-BRIEF.md:233 lists observations as a brief block, but D-19 deferred that line out of phase 7 and `crates/verbatim-core/tests/inject_brief.rs:282` actively fails if any file under `src/inject/` names the string. Phase 8 is retention only - INJ-01 is not in its requirement list and observations appear in none of its seven success criteria - and there is no phase 9. Meanwhile INJ-01 reads Complete against phase 5 in the traceability table, so /cad-audit finds a phase, a plan and a verification for it and passes: the gap is invisible to the one check meant to catch it. Closing it means either wiring observations into the brief (which retires the inject_brief.rs guard) or amending INJ-01's text to say the brief is complete without them, as phase 5 signed off. Distinct from the OBS-05 and PRIV-02 deferrals, which now carry notes on their REQUIREMENTS rows in c9f7f1c; this one has neither a note nor a phase.
- [ ] (phase 8) `verbatim reindex` does not say how many sessions it preserved: the count is on `Rebuilt::preserved` and documented, but `crates/verbatim/src/cmd/reindex.rs` was in no phase-8 plan's lease, so the stderr line, the `--json` `data` object, `DATA_COMMANDS` and `docs/json-shapes.md` are untouched.
- [ ] (phase 8) `crates/verbatim-core/src/parse/mod.rs:3-5` still says the ingest hands the blob bytes identical to the source without qualification; phase 8's D-05 narrows that to `full` capture mode. One sentence, one file, no behaviour.
- [ ] (phase 8) `verify` skips an evicted session entirely rather than asserting the one thing eviction guarantees about it - that its blob is empty - so it has a class of row it never checks. From the phase-8 plan-2 risk_surface fire, downgraded from medium: `evict_one` is the only writer of the flag and it empties the blob in the same transaction.
- [ ] (phase 8) `retention::delete_one` does not re-check `transcript_is_gone` before removing a session; the precondition is evaluated once, in `evaluate`. Archived bytes are recoverable (the watermark row goes in the same transaction so `discover` re-ingests from offset 0) but the `observations` rows are not. From the phase-8 plan-1 risk_surface fire, downgraded from high.
- [ ] (phase 8) The `verbatimElided` figure is the canonical reserialized size of the dropped subtree, not the source bytes it occupied on the line, and the doc comment does not say so. Nothing in production reads it today. From the phase-8 plan-5 risk_surface fire, downgraded from medium.
- [ ] (phase 8) `cmd::status` and `cmd::uninstall` still hold private copies of the three-file footprint rule that `cmd::mod::footprint`/`human` now also expresses.
- [ ] (phase 8) Several test files are `#![cfg(feature = "testkit")]`, so `cargo test --test <name>` without `--features testkit` compiles an empty binary and reports green. Some plans' `Verify:` commands are written in that vacuous form.
- [ ] (phase 8) `verbatim data move` skips the `LOCK` file in the copy and creates an empty one at the destination: the file's contents carry no meaning, only the OS lock attached to it, and copying a locked file is a read Windows refuses.
- [ ] Verbatim's MCP server does not speak MCP 2 (protocol 2026-07-28). Probed 2026-10-05 against verbatim 0.1.1 on Claude Code 2.1.289: `server/discover` answers -32601 method not found, so the server stays connected only because Claude Code falls back to the old `initialize` handshake. `initialize` also agrees to `2026-07-28` when asked (`crates/verbatim/src/cmd/mcp/mod.rs:64`) while `tools/list` carries no `ttlMs` or `cacheScope`, the shape Claude Code rejected in Baley's 2026-10-02 probe (fixed there in baley #175). Fix: answer `server/discover`, add the two fields at 2026-07-28, and stop offering 2026-07-28 from `initialize` until both land. Tool descriptions (73-126 chars) and the absent server instructions are under the 2,048-char cap. Scheduled with the Baley-conformance work once Baley ships, not before.
- [ ] (phase 8) Change the license to MIT before release.


## Seeds

### Archive and injection fixes (second review, 2026-10-12, the cycle after the install work)

Findings from an outside review, each checked against the code on 2026-10-12
by reading, not by rerunning the reviewer's synthetic probes. All six held. The
first, Blocks as rows, is now phase 7 of v0.0.1, because nothing ships until it
is done. The five below are planned as the cycle after the install work.

- [ ] **Tighten the injection gate.** `eligible` (`inject/prompt.rs:511`) fires
  on any entity match in the top three or on two distinct matches at any rank.
  The candidates come from a prompt's paths and identifier-shaped tokens, but
  each hit is scored against every token of the whole prompt (`query_of`), so
  the word "read" matches the tool entity `Read` and makes a path match count
  twice. Zero-IDF matches still count (`recall/search.rs:774`), and `Covered`
  counts where the brief says exact. Drop `tool` from eligibility, add a
  query-time IDF floor, and prefer the result turn or the call-and-result pair
  over a bare `tool_use`. Tune with `terminus replay` over the real decision
  log. The 0.82 precision `terminus stats` reported on 2026-10-12 likely
  overstates it: an injected "Read <path>" turn counts as a hit whenever the
  path is touched later.
- [ ] **Evict to a minimal blob.** Eviction empties the blob and leaves the
  derived rows as the only copy, frozen under the rules that wrote them
  (`reindex.rs:19-46`). Re-blob an evicted session under minimal capture
  instead, so it stays rebuildable and partly readable and `reindex` has one
  path again. Retention is opt-in and nothing has been evicted yet.
- [ ] **Version the labels.** `labels` carries no rule version, so a change to
  the labelling rules leaves `terminus stats` averaging old and new rules.
  Recompute labels in `reindex` or stamp a rule version on each row.
- [ ] **Targeted pass per hook.** Every hook spawns the same full tree walk
  (`cmd/hook.rs:200`) and holds the ingest lock for it. The payload carries
  `transcript_path`, so `UserPromptSubmit`, `SessionEnd` and `PostCompact` can
  pass that one file, keeping the full walk for `SessionStart`.
- [ ] **Comments back to invariants.** 8,643 of 21,964 lines in
  `terminus-core/src` are comments, with 1,207 phase-local `D-NN` references
  across `crates/`, three files citing `.planning/phases/` paths that no longer
  exist, and drift already visible: `store/schema.rs:56` says foreign keys stay
  off, `reindex.rs:127` says they are enforced. Keep invariants and their
  reasons at module level, short and plain, and move the history into one
  decisions file with globally unique ids.

### Security disclosure

- [ ] **SECURITY.md and a private disclosure channel.** Still undecided, and no
  phase owns it.

## Notes

## Debt markers

- None.
