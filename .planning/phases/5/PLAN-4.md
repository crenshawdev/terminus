---
phase: 5
plan: 4
requirements:
  - INJ-04
  - INJ-05
files:
  - crates/verbatim-core/src/inject/state.rs
  - crates/verbatim-core/src/inject/compaction.rs
  - crates/verbatim-core/src/inject/mod.rs
  - crates/verbatim-core/src/inject/prompt.rs
  - crates/verbatim-core/src/inject/brief.rs
  - crates/verbatim-core/tests/inject_state.rs
  # Lease extended at the task-2 checkpoint (approved): two PLAN-3 assertions
  # re-invoke `prompt::user_prompt_submit` under ONE `session_id`, which AC4's
  # dedupe answers with silence. Each repeated call gets a `session_id` of its
  # own so they keep asserting rendering and budget.
  - crates/verbatim-core/tests/inject_prompt.rs
  - crates/verbatim-core/tests/inject_compaction.rs
  - crates/verbatim/tests/suppression.rs
---

# Phase 5: Context Injection - Plan 4 of 4 (what a session remembers)

**ORDER: last. After PLAN-2 and PLAN-3 both, and never beside either** - this
plan edits `inject/brief.rs` (PLAN-2's file) and `inject/prompt.rs` (PLAN-3's).

## Goal

Injection stops repeating itself: a turn the session has already been given -
by an earlier prompt, by the resume brief, or by being in the session's own
transcript - is never given again, and the one prompt after a compaction draws
from the turns that just fell out of the model's context instead of from the
whole archive.

## Must be true when done

- The same entity-bearing prompt fed twice under one `session_id` injects on the
  first and emits nothing on the second, and the per-session state file names
  the suppressed turn with its reason (AC4).
- A candidate turn belonging to the current session is never injected.
- A candidate turn the resume brief already carried is never injected.
- After a `SessionStart` whose `source` is `compact`, the next
  `UserPromptSubmit` draws candidates only from the turns absent from the
  boundary's `preservedMessages.uuids`, at most three; a later prompt in the
  same session does not (AC5).
- Nothing on the prompt path writes to SQLite.

## Context

- D-06 binds where the state lives: a per-session file in the data directory,
  keyed on `session_id`, outside SQLite entirely. A write on the prompt path is
  the one thing INJ-06 cannot tolerate - a backfill holds the store for a
  measured 49 s and a blocked writer is strictly worse than a blocked reader -
  and this state is per-session and disposable, a different lifetime from phase
  6's durable decisions log.
- D-14 binds why it cannot be a table: `Store::open` runs `CREATE_SQL` only on a
  fresh store and `bring_forward` covers columns, not tables, so a new table
  would reach an existing user's store only through a `DERIVED_SCHEMA` bump that
  forces a full rebuild - measured at ~49 s - on the first hook-spawned pass
  after upgrade.
- D-15 binds "already visible": the archive's own turns for the current session,
  found by `turns.session_key` equal to the canonicalized `transcript_path` from
  the payload. Never a live read of the transcript, which costs the measured p90
  of 1.03 MB and p99 of 2.6 MB per prompt. All four hooks spawn an ingest, so
  the session is archived incrementally by byte watermark; this accepts a small
  duplication window at the head of a session.
- D-07 binds the dropped set: the complement of `preservedMessages.uuids` over
  `turns.uuid` for the session, read from `compaction_boundaries.metadata`. NOT
  `allUuids`, NOT `preservedSegment`, NOT "everything before `headUuid`".
- D-08 binds the trigger: a `SessionStart` whose `source` is `"compact"` writes
  an injection-owed flag that the next `UserPromptSubmit` consumes, and the flag
  persists until it fires rather than being lost to the race against the
  boundary row being committed.
- Out of scope: the decision log, outcome labels, replay and `verbatim stats` -
  all of FEED-01..FEED-04, phase 6. This state file records only what INJ-04
  needs to suppress within a session; it is not the durable decision record.

## Tasks

### Task 1: A per-session file that remembers what this session was given

- **Files:** crates/verbatim-core/src/inject/state.rs,
  crates/verbatim-core/src/inject/mod.rs,
  crates/verbatim-core/tests/inject_state.rs
- **Action:** D-06 and D-14. Add a state module holding one JSON file per
  session under the data directory, in a subdirectory of its own so a data
  directory listing still reads as one store plus its injection scratch. It
  carries the turn ids injected so far this session, the turn ids the resume
  brief carried, the suppressions as turn id plus reason, and the
  compaction-owed flag task 4 sets. The `session_id` comes off an untrusted
  payload and becomes a file name, so accept only a name of ASCII hex digits and
  dashes at a plausible uuid length and refuse everything else outright rather
  than joining it onto a path - a `session_id` carrying a separator or `..`
  must reach no filesystem call at all. Write through a temporary in the same
  directory and rename over the target, the way
  `crates/verbatim/src/cmd/install/json_file.rs` writes settings, so a kill
  mid-write cannot leave a half-written file where a read expects one. Read
  fails open in every direction: a missing file, an unreadable one, a
  file that is not JSON and a file whose shape this build does not recognize all
  read as empty state, because this is disposable per-session scratch and a
  failure to read it must never become a failure to answer a prompt. Nothing
  here is phase 6's decision log (FEED-01), which is durable, joined against
  later transcripts and out of scope in this phase.
- **Verify:** `cargo test -p verbatim-core --test inject_state` passes with
  cases showing a round trip through the file, a file of garbage bytes reading
  as empty state rather than an error, a missing directory reading as empty
  state and creating nothing, a `session_id` containing a path separator or
  `..` refused with no file created anywhere, and a write landing atomically
  (the target either holds the previous content or the new one).

### Task 2: Nothing gets injected twice, and nothing already on screen gets injected at all

- **Files:** crates/verbatim-core/src/inject/prompt.rs,
  crates/verbatim-core/src/inject/brief.rs,
  crates/verbatim-core/src/inject/state.rs,
  crates/verbatim/tests/suppression.rs
- **Action:** INJ-04's three suppressions, applied to the candidate turns before
  the cap of three so a suppressed turn does not consume a slot, each recorded
  in the state file with its reason. First, a turn already injected this
  session: read from the state file, and every turn this prompt injects is
  written back to it. Second, a turn already visible in this session: any
  candidate whose `turns.session_key` equals the canonicalized
  `transcript_path` from the payload (D-15) - canonicalize the payload's path
  the way ingest keys a session, and when the path cannot be canonicalized,
  because the file is not there yet, suppress nothing rather than failing.
  Never read the live transcript. Third, a turn the resume brief already
  carried, which means the brief must write the turn ids it used into the same
  state file when it renders - the one edit this plan makes to
  `inject/brief.rs`. The brief writes state only after it has actually rendered
  something, so a machine with no archive still acquires no files.
- **Verify:** AC4. `cargo test -p verbatim --test suppression` passes with a
  temp data directory seeded from the rooted fixtures and ingested to
  completion, showing: the same entity-bearing prompt fed twice under one
  `session_id` writes one JSON object the first time and nothing the second, and
  the state file for that `session_id` names the suppressed turn id with a
  readable reason; a prompt whose only candidate is a turn of the session named
  by the payload's own `transcript_path` writes nothing; and a prompt whose only
  candidate is a turn the brief carried in the same session writes nothing.

### Task 3: The turns that fell out of context

- **Files:** crates/verbatim-core/src/inject/compaction.rs,
  crates/verbatim-core/src/inject/mod.rs,
  crates/verbatim-core/tests/inject_compaction.rs
- **Action:** D-07. Derive the dropped-turn set for a session from the bytes
  ingest stored verbatim in `compaction_boundaries.metadata` - phase 2 D-08 and
  `crates/verbatim-core/src/derive.rs` deliberately left this to be derived
  here, where a wrong reading costs a query and not a reingest. Join
  `compaction_boundaries` to `turns` on `turn_id` for the boundary's
  `session_key` and `turn_seq`, take the session's most recent boundary, parse
  the metadata with `serde_json`, and return the turns of that session whose
  `turn_seq` is below the boundary's and whose `uuid` is not listed in
  `preservedMessages.uuids`. It is NOT the complement of `allUuids` - two of the
  eight in the one real boundary are absent from the file entirely - NOT
  `preservedSegment`, whose `anchorUuid` resolves to a record AFTER the
  boundary, and NOT everything before `headUuid`, which silently loses the class
  of turn that sits before the boundary and is not preserved. `idx_turns_uuid`
  covers the lookup. A boundary whose metadata is absent, unparseable, or
  carries no `preservedMessages.uuids` yields an empty set rather than an error,
  and a session with no boundary at all yields nothing - the format has exactly
  one real example across 300,556 measured records and the read must degrade
  rather than throw.
- **Verify:** `cargo test -p verbatim-core --features testkit --test inject_compaction`
  passes over a store ingested from `session-compacted.jsonl`, whose four
  records make all three readings distinguishable: `preservedMessages.uuids`
  lists the second and third uuids, `allUuids` lists the first three, and
  `preservedSegment.headUuid` is the first. The dropped set must be exactly the
  first uuid's turn - not empty, which is what both wrong readings return - and
  must exclude the boundary record's own turn. A boundary row whose metadata is
  replaced with garbage yields an empty set and no error.

### Task 4: The first prompt after a compaction

- **Files:** crates/verbatim-core/src/inject/mod.rs,
  crates/verbatim-core/src/inject/prompt.rs,
  crates/verbatim-core/src/inject/brief.rs,
  crates/verbatim-core/src/inject/state.rs,
  crates/verbatim/tests/suppression.rs
- **Action:** D-08 and INJ-05. A `SessionStart` whose `source` is `"compact"`
  renders its brief exactly as any other SessionStart does and additionally sets
  the injection-owed flag in that session's state file. The next
  `UserPromptSubmit` under that `session_id` consumes it: it derives the dropped
  set for the session the payload's canonicalized `transcript_path` names, runs
  the prompt as the query over a wider bounded window than the ordinary path
  uses - the excerpt step is already skipped until after the threshold, so a
  wider window costs only small columns - keeps only the candidates in the
  dropped set, applies the same structural threshold and the same suppressions,
  injects at most three, and clears the flag. The threshold is not relaxed
  inside the pool and this is the choice being recorded: one definition of
  relevance for both paths, and INJ-03's "never on a free-text-only match" is
  not conditioned on a compaction having happened. The flag clears when the
  dropped set was derivable at all - when a boundary row for this session
  existed - whether or not anything passed the threshold; it stays set when no
  boundary row is there yet, so the debt carries to the following prompt instead
  of being lost to the race against the ingest that is still committing it. A
  later prompt, with the flag clear, is the ordinary path again. If PLAN-1
  task 5 recorded that a compaction fires no `SessionStart` at all, take D-08's
  named fallback instead: read the store's most recent boundary row for this
  session at `UserPromptSubmit` and treat a boundary newer than the last prompt
  this session injected against as the trigger, accepting the ingest race, and
  say in the module doc that this is the fallback and why.
- **Verify:** AC5. `cargo test -p verbatim --test suppression` passes with a
  store holding a compacted session, showing: after `verbatim hook SessionStart`
  with a payload whose `source` is `compact`, the next
  `verbatim hook UserPromptSubmit` under that `session_id` injects only turns
  absent from `preservedMessages.uuids`, at most three, and never a preserved
  one even when a preserved turn matches the prompt better; a second prompt in
  the same session is answered from the ordinary candidate pool; and with the
  boundary row absent from the store, the first prompt injects nothing from a
  dropped set, leaves the flag set in the state file, and the following prompt
  still draws from the dropped set once the row is there.

## Notes

- Task 4's first sentence depends on PLAN-1 task 5, which is a human-verify
  against a live compaction. If that fixture is still uncaptured when this plan
  runs, the fallback arm is the one to build, and the capture becomes the thing
  that later simplifies it rather than the thing that unblocks it.
- The state file's suppression reasons exist because AC4 asks to read one, not
  because suppression needs them. Keep them to what INJ-04 distinguishes -
  already injected, visible in this session, carried by the brief - and leave
  entities, candidates, scores, thresholds and token counts to FEED-01, which
  owns them and lands in phase 6.
