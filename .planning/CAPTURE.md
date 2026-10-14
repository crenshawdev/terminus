# Capture

## Todos

- [ ] (phase 8) The release CI must run `cargo test --workspace --all-features`.
      Without it `testkit` is off and the run still exits 0: `cargo test -p
      terminus-core` skips the 26 test files gated on
      `#![cfg(feature = "testkit")]`, and a bare `cargo test --workspace` skips
      `crates/terminus/tests/mcp.rs`, `recall_cli.rs` (49 tests) and
      `datamove.rs`, gated the same way on the `terminus` package.
- [ ] (phase 8) Change the license to MIT before release.
- [ ] (phase 8) The PR that merges cadence/v0.0.1 says "Fixes #13" in its description, so the issue closes on merge; 65ef2d1 fixed it.


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
