---
status: testing
phase: 2
fields_version: 1
started: 2026-08-12
updated: 2026-08-12
---

## Items

### 1. Cold start from scratch
expected: With no existing data directory, a fresh ingest run boots clean, the schema is created at DERIVED_SCHEMA 2 with no migration error, and `verbatim status` afterwards prints real session/turn counts (not zeros).
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Bare `verbatim ingest` into an empty VERBATIM_DATA_DIR exits 0; meta shows derived_schema=2 with no migration error; `verbatim status` prints sessions 4, turns 17, watermarks 4 covering 9843 bytes, files 4 walked / 4 committed / 0 failed. All four discovered transcripts (two top-level, a depth-4 sidecar, a depth-6 sidecar) became sessions; the journal.jsonl beside them did not.

### 2. Full tree pass over the real corpus
expected: One pass over the real transcript root ingests every `<uuid>.jsonl` and `agent-*.jsonl` at any depth (including `subagents/workflows/wf_*/`), ingests no `journal.jsonl`, `.meta.json` or `tool-results/` file, reports zero unparseable lines and zero panics, and the session count exceeds the top-level file count by the sidecar count.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: `VERBATIM_TEST_CORPUS=/data/claude/.claude cargo test -p verbatim --test corpus -- --nocapture` -> ok, 2 passed, 126.58s: 2137 transcripts stable across the pass, 2137 sessions archived, 309904 lines / 972.7 MB decompressed with 0 unparseable and 0 untyped, 16 record types, pass 61.8s, exit 0. corpus.rs asserts both halves (every stable transcript archived; no journal / .meta.json / tool-results / non-.jsonl session key) using an AC1 rule written independently of discover.rs. `the_depth_six_sidecar_is_reached` pins the depth-6 case.

### 3. Session lineage links, none self-linked
expected: After the pass, the count of non-null `continues_from` values is non-zero and the count of rows whose `continues_from` equals their own session id is zero.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: Corpus run: `continues_from: 190 linked, 0 self-linked`, asserted in corpus.rs. tests/lineage.rs 11 ok. A CLI ingest wrote continues_from and parent_session_key into session_meta, so the link is produced by the shipped path, not only by a unit test.

### 4. Worktree folds in, look-alike paths stay apart
expected: Two transcripts whose `cwd` values are a git repo and a worktree beneath its `.claude/worktrees/` resolve to one project key, and two transcripts with `cwd` `/x/a.b` and `/x/a-b` resolve to two different project keys.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: tests/project.rs `a_repo_and_its_worktree_land_on_one_project_key` and `two_cwds_that_encode_identically_stay_two_keys` ok (11/11 in the file). Resolver is constructed once per pass in ingest/pass.rs:113 and a CLI ingest wrote session_meta.project as the git toplevel.

### 5. Appending a compact_boundary preserves earlier bytes
expected: Appending a `compact_boundary` record to an already-ingested transcript and rerunning ingest adds a boundary row carrying that record's `compactMetadata` verbatim, and every turn row and committed blob block from the earlier pass is byte-identical afterwards.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: tests/compaction.rs `appending_a_boundary_records_it_and_disturbs_nothing_already_archived` ok (completed blob blocks byte-identical, turn rows unchanged), plus a CLI ingest of session-compacted.jsonl producing compaction_boundaries.metadata byte-identical to the record's compactMetadata object.

### 6. Exclusion opens nothing, sibling still ingested
expected: With a project excluded, a pass over a tree containing it records zero opens of any file under that directory on the instrumented open counter and adds no row to any table, while a sibling directory sharing the excluded path's leading segments is ingested normally.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: tests/pass.rs `an_excluded_project_is_never_opened_and_adds_no_row` ok, reading the counted-open log (discover.rs open_transcript records the path before the open is attempted) for the excluded project and its worktree, then asserting zero rows in sessions/session_meta/turns/watermarks, with the hyphenated sibling committed. Confirmed live: a bare pass with the project excluded archived 0 sessions.

### 7. A killed pass converges on rerun
expected: Killing a pass at 10 or more randomized points and rerunning to completion yields the same blob checksums, turn rows and watermarks as one uninterrupted pass over the same tree.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim/tests/crash.rs `killing_a_tree_pass_converges_on_a_first_pass_and_on_an_append` ok: >=10 kills against a first pass and >=10 against an append pass, each resumed and compared to the uninterrupted reference snapshot of blobs, turn rows and watermarks.

### 8. One damaged transcript does not stop the pass
expected: A tree containing one damaged transcript (short of its watermark, or archived with no `session_meta`) completes ingest of every other file, and `verbatim status` prints the run with that file's error alongside store sizes, counts and watermarks.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: Live CLI: one transcript truncated below its watermark, its sibling appended to; the pass exits 0, skips the damaged file with a named error, commits the sibling, and `verbatim status` prints sizes, counts, watermarks, '2 walked, 1 committed, 1 failed' and the error text. `verbatim verify` then names that session and exits 1. tests/pass.rs and tests/status.rs cover the same paths.

### 9. Exclusion is bypassed by `verbatim ingest <path>` and by an exclusion string with a trailing separator
expected: behavior wrong - ING-08 requires exclusion honored on the ingest path, never read-then-filtered; two entry conditions read and archive an excluded project
origin: verifier
status: pass
first_pass: fail
source: model
evidence: Live CLI against a556897, both reproductions now refuse. (A) exclude=["/data/code/demo/"] with a trailing separator: bare tree pass leaves 'sessions 0' and 'excluded 1 project(s): /data/code/demo' - the pre-open test now sees the same spelling excludes_path does. (B) exclude=["/data/code/demo"] then 'verbatim ingest <that transcript>': prints 'skipped <path>: inside the excluded project <dir>', exits 0, 'sessions 0'. (C) control, nothing excluded: the same call gives 'sessions 1'. Tests: config.rs 15 ok, discover.rs 12 ok, pass.rs 10 ok, workspace 214 passing / 26 binaries / 0 failed, clippy and fmt clean. VERBATIM_TEST_CORPUS=/data/claude/.claude cargo test -p verbatim --test corpus -> ok, 2 passed, 123.23s, 2147 sessions, 311154 lines, 0 unparseable.
reported: behavior wrong - ING-08 requires exclusion honored on the ingest path, never read-then-filtered; two entry conditions read and archive an excluded project
severity: major
cause: ingest::run (crates/verbatim-core/src/ingest/mod.rs:60) never loads Config, so the exclusion predicate exists only on the tree-walk path (discover.rs:96). Separately, Config::from_parts (config.rs:133) encodes each exclusion string as configured, with no normalization, so a trailing separator encodes to a name no project directory can equal and excludes_encoded_dir never matches, while excludes_path still hides the rows - read-then-filter, which ING-08 forbids.
fix: a556897, retest

### 10. No dropped-turn set on the compaction boundary (ROADMAP phase 2 criterion 4)
expected: missing - the boundary carries compactMetadata verbatim but no set of turns that fell out
origin: verifier
status: pass
first_pass: fail
source: model
evidence: Docs corrected in 9e078d0, verified by reading both back. REQUIREMENTS.md ING-06 now reads 'a boundary carrying the record's compaction metadata verbatim' with the dropped set derived at query time by INJ-05 in phase 5, citing D-08. ROADMAP phase 2 criterion 4 now reads 'the boundary record carrying its compactMetadata verbatim ... The dropped-turn set is DERIVED from that metadata at query time in Phase 5 (INJ-05), not stored here'. The behavior itself was never the gap: tests/compaction.rs already stores compactMetadata byte-identically (item 5, AC4).
reported: missing - the boundary carries compactMetadata verbatim but no set of turns that fell out
severity: minor
cause: Not a code gap: D-08 deliberately defers the dropped-turn complement to phase 5 (INJ-05) on a measurement that contradicts DESIGN-BRIEF.md:140. The gap is documentary - ROADMAP phase 2 criterion 4 and REQUIREMENTS ING-06 still read as though phase 2 derives the dropped set.
fix: 9e078d0, retest

## Summary

total: 10
passed: 10
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 2
