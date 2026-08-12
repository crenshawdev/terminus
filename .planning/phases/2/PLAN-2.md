---
phase: 2
plan: 2
requirements:
  - ING-04
  - ING-05
  - ING-08
files:
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/project.rs
  - crates/verbatim-core/src/lineage.rs
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/project.rs
  - crates/verbatim-core/tests/lineage.rs
  - crates/verbatim-core/tests/exclusion.rs
  - crates/verbatim-core/tests/fixtures.rs
  - crates/verbatim/src/cmd/status.rs
  - crates/verbatim/tests/status.rs
  - tests/fixtures/README.md
  - tests/fixtures/subagents/agent-alpha.meta.json
---

# Phase 2: Ingest At Scale - Plan 2 of 3 (identity and lineage)

**SEQUENTIAL: PLAN-1 must complete before this plan starts, and this plan
before PLAN-3.** All three write `crates/verbatim-core/src/ingest/mod.rs`,
`crates/verbatim-core/src/ingest/pass.rs`, `crates/verbatim-core/src/lib.rs`,
`crates/verbatim-core/src/testkit.rs`, `crates/verbatim-core/tests/fixtures.rs`
and `tests/fixtures/README.md`; this plan also shares
`crates/verbatim-core/src/config.rs`, `crates/verbatim/src/cmd/status.rs` and
`crates/verbatim/tests/status.rs` with PLAN-1. Do not run them in parallel.

## Goal

Every archived session knows what it belongs to and what it came from: its
project, resolved from the record's `cwd` and never from the encoded directory
name, with worktrees folded into their parent repo; the session it continues
from; and, for a sidecar, the parent session it ran under. Exclusion holds on
the read side as well as the ingest side.

## Must be true when done

- Two transcripts whose `cwd` values are a git repo and a worktree beneath its
  `.claude/worktrees/` resolve to one project key, and both the worktree key and
  the parent-repo key are recoverable from the store afterwards.
- Two transcripts whose `cwd` values are `/x/a.b` and `/x/a-b` resolve to two
  different project keys, even though both live in a project directory named
  `-x-a-b`.
- A session whose `cwd` directory no longer exists still gets a project key,
  and one already keyed keeps that key when the directory is later deleted.
- After a pass, no `session_meta` row has a `continues_from` equal to its own
  session id, and one predecessor named by several successors produces several
  links rather than one.
- Every sidecar transcript carries the `session_key` of the top-level transcript
  whose directory it sits under, and the bytes of its `agent-*.meta.json` when
  one exists.
- A session whose project is excluded is absent from the store's session-listing
  read path even when it was archived before the exclusion was configured.

## Context

Locked and binding here: D-20 (a session's project is its first `cwd` record;
a transcript whose `cwd` changes mid-session keeps the first), D-05 (git runs
only when the `cwd` directory still exists; otherwise degrade to the canonical
`cwd` string), D-06 (worktree-to-parent is a path-shaped rule, not
`git rev-parse --git-common-dir`, and the resolved mapping is persisted), D-07
(the encoded project directory name is never decoded into a path), D-01
(`continues_from` rejects a `session_id` equal to the file's own), D-02
(continuation is a fan-out, not a chain), D-19 (`continues_from` may name a
session absent from the store; no foreign key, no ingest-order requirement),
D-03 (a sidecar's parent comes from its path, never from its records), D-04
(the `agent-*.meta.json` bytes are stored opaquely in a `session_meta` column,
not parsed into typed columns), D-23 (the read-path half of ING-08 stores no
per-session flag; read paths re-apply the same predicate against
`session_meta.project`).

Out of scope: the compaction boundary, the short-transcript flag and the crash
and corpus harnesses (PLAN-3). Interpreting `compactMetadata` into a
dropped-turn set is phase 5. Search and the MCP tools are phase 3; the read
path this plan hardens is the session listing `verbatim status` uses.

Existing code this plan changes: `write_session_meta` in
`crates/verbatim-core/src/ingest/mod.rs`, which today writes `session_id`,
`cwd` and `branch` from the first record carrying each via `find_map` and
coalesces every session-wide column onto what is already stored, and leaves
`project` and `parent_session_key` null; and `continues_from` in the same file,
which takes the first `foreign_session_id` in the scan with no comparison and
falls back to resolving `parentUuid` through `turns`. The columns this plan
fills - `session_meta.project`, `session_meta.parent_session_key`, the
pre-worktree project key, and the opaque agent-meta bytes - all exist after
PLAN-1 task 1.

Test invocation note: `cargo test -p verbatim-core` runs none of the
`#![cfg(feature = "testkit")]` test files and exits 0 (phase 1 open item), so
every Verify below uses `cargo test --workspace`.

## Tasks

### Task 1: A session's project comes from its first `cwd`, resolved once

- **Files:** crates/verbatim-core/src/project.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/tests/project.rs
- **Action:** Add a project-resolution module, declared in
  `crates/verbatim-core/src/lib.rs`. It turns a `cwd` string into a project key
  and nothing else - no store access, no ingest knowledge - so every rule in it
  is a unit-testable function. The rule (D-05): if the `cwd` directory still
  exists on disk, run `git rev-parse --show-toplevel` in it, by absolute
  program lookup with no shell, and take the toplevel it prints; if the
  directory does not exist, or is not a git repository, or git is not installed,
  or git fails or is slow, degrade to the canonicalized-by-string `cwd` itself.
  Degrading is the common case and must not look like an error: 33 of the 63
  distinct `cwd` values in the real corpus no longer exist, 13 of the surviving
  30 are not repositories, and git answers for only 17 of 63. A null project for
  the other 46 would drop half the archive out of every project-scoped search
  and every resume brief. Memoize the answer per `cwd` string for the lifetime
  of a pass, because 1,253 transcripts carry only 63 distinct values and an
  unmemoized resolver shells out to git a thousand times for answers it already
  has. Never decode the encoded project directory name into a path (D-07):
  applying `[^A-Za-z0-9] -> '-'` to a `cwd` reproduces its containing directory
  name for 1,252 of 1,252 real files, so the encoding is exact and provably
  lossy - `/data/code/jcrenshaw.dev` and `/data/code/jcrenshaw-dev` encode
  identically, and a decoder would merge two unrelated projects under one key
  unrecoverably. The directory name is an input to the exclusion test and to
  nothing else.
- **Verify:** `cargo test --workspace --test project` passes, including: a `cwd`
  pointing at a temporary directory that is a real `git init` repository
  resolves to that repository's toplevel; a `cwd` two directories deep inside it
  resolves to the same toplevel; a `cwd` naming a path that does not exist
  resolves to that path string rather than to `None`; a `cwd` naming an existing
  non-repository directory resolves to that directory; and resolving the same
  `cwd` twice runs git at most once, asserted by a counter on the resolver. The
  git-backed cases skip with a printed reason when `command -v git` finds
  nothing, so the suite stays runnable without it.

### Task 2: Worktrees fold into their parent repo, and both keys are kept

- **Files:** crates/verbatim-core/src/project.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/tests/project.rs
- **Action:** Add the worktree rule to the resolver and wire the resolver into
  ingest. The rule is path-shaped (D-06): a `cwd` of the form
  `<repo>/.claude/worktrees/<name>`, with or without further path below it,
  maps to `<repo>`. It is not `git rev-parse --git-common-dir`, and that is
  measured rather than preferred - every worktree `cwd` in the real corpus has
  exactly that nested form and all 13 of them are deleted, so git returns
  nothing for any of them. The rule must apply before the on-disk git branch,
  since the worktree directory may still exist while its parent is what the
  session belongs to. Store both keys (ING-05): the resolved parent-repo key in
  `session_meta.project` and the pre-mapping key - the worktree path - in the
  column PLAN-1 added beside it, so nothing has to re-derive the mapping later
  and a deleted directory cannot un-key an already-archived session. Wire it
  into `write_session_meta` in `crates/verbatim-core/src/ingest/mod.rs`, which
  already selects the first record carrying a `cwd` with `find_map` and
  coalesces `cwd` onto what is stored: resolve from that same first `cwd` and
  coalesce `project` the same way, so a tail pass never revises a project the
  first pass established (D-20) and the 19 real transcripts whose `cwd` changes
  mid-session keep their first. Pass the per-pass memo down from the pass module
  so one walk resolves each distinct `cwd` once. Getting this wrong splits six
  `cadence`, six `hindsight` and six `assistant` worktrees into seven keys each -
  the identity fragmentation PROJECT.md cites claude-mem for.
- **Verify:** `cargo test --workspace --test project --test pass` passes,
  including AC3 in full: a constructed fixture with a real `git init` repository
  at a temporary path and a directory at `<repo>/.claude/worktrees/wt-a`, two
  transcripts whose first records carry those two `cwd` values, ingested in one
  pass, whose `session_meta.project` values are equal to the repository toplevel
  and whose stored pre-mapping keys differ; and two transcripts whose first
  records carry `/x/a.b` and `/x/a-b`, both placed inside one project directory
  named `-x-a-b`, whose `session_meta.project` values are different strings.
  Deleting the worktree directory and re-running the pass leaves both
  `session_meta.project` values unchanged.

### Task 3: A session never continues from itself, and a predecessor may fan out

- **Files:** crates/verbatim-core/src/lineage.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/tests/lineage.rs
- **Action:** Fix `continues_from` in `crates/verbatim-core/src/ingest/mod.rs`
  and move its rules into a lineage module so PLAN-3 and phase 3 have one place
  to read them. Today it takes the first `foreign_session_id` in the scan with
  no comparison against the file's own `sessionId`. Measured over all 1,253
  top-level transcripts, 433 carry a `session_id` field, of which 253 carry only
  their own id, 169 only another session's and 11 both - so the current rule
  makes one in five archived sessions claim to continue from itself, and every
  phase 3 and phase 5 thread walk then either cycles or special-cases the
  self-edge at each read site. Reject any candidate equal to the record's own
  `sessionId` and take the first surviving foreign id (D-01); when a file
  carries two distinct foreign ids, byte order decides. Keep the existing
  `parentUuid` fallback for a transcript's first turn exactly as it is - D-11
  from phase 1 makes it the fallback and only that. Change no constraint that
  would make continuation single-successor: one predecessor may be named by
  several successors and thread reconstruction is a tree walk (D-02) - session
  `ebd78c2b` is named by four separate later transcripts in one project. Leave
  `continues_from` a bare indexed `TEXT` with no foreign key and no
  ingest-order requirement (D-19): of the 173 files naming a foreign session,
  2 name a predecessor that has been aged out by `cleanupPeriodDays` and is not
  on disk at all, so a link to an absent session is normal and must not error.
  Note the phase 1 open item this closes: `session_meta.continues_from` holds a
  value in the session-id namespace while `sessions` is keyed on file identity,
  so record in the module's documentation which namespace the column is in and
  that a resolver crossing to `session_key` must expect zero, one or many rows.
- **Verify:** `cargo test --workspace --test lineage --test ingest` passes,
  including: a transcript whose only `session_id` field equals its own
  `sessionId` produces a null `continues_from`; a transcript carrying both its
  own id and a foreign one records the foreign one; three transcripts naming one
  common predecessor all record it, producing three rows with the same
  `continues_from`; a transcript naming a predecessor that was never ingested
  records the id anyway; and, over a tree fixture, the count of rows whose
  `continues_from` equals their own `session_id` is zero while the count of
  non-null `continues_from` values is greater than zero.

### Task 4: A sidecar's parent comes from its path

- **Files:** crates/verbatim-core/src/lineage.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/tests/lineage.rs
- **Action:** Fill `session_meta.parent_session_key`, which phase 1 created and
  left null. The parent is derived from the sidecar's path and never from its
  records (D-03): a sidecar sits at `<project>/<sessionId>/subagents/...`, at
  depth 4 or at depth 6 under `subagents/workflows/wf_*/`, so walk up to the
  nearest `subagents` ancestor, take the directory above it, and the parent is
  the top-level transcript named by that directory inside the project directory.
  The stored value is the parent's `session_key` - the canonical transcript path
  string, which is what `sessions` is keyed on - built by joining onto the same
  canonical root the walk used, so it matches the key the parent's own ingest
  produced. Records cannot answer this: all 822 real sidecar files sit under a
  directory named for an existing top-level transcript's session id, and 818 of
  818 `agent-*.jsonl` files report that same id as their own `sessionId`, so the
  record-level id distinguishes nothing and keying on it links a sidecar to
  itself or to whichever session shares the id. Store the key whether or not the
  parent has been ingested, for the same reason `continues_from` carries no
  foreign key (D-19). A top-level transcript's `parent_session_key` stays null.
  Without this column phase 3 has no way to filter subagent turns out of a
  brief.
- **Verify:** `cargo test --workspace --test lineage --test pass` passes,
  including: a pass over a tree holding `<project>/<uuid>.jsonl`,
  `<project>/<uuid>/subagents/agent-a.jsonl` and
  `<project>/<uuid>/subagents/workflows/wf_x/agent-deep.jsonl` gives both
  sidecars a `parent_session_key` equal to the top-level transcript's
  `session_key` and gives the top-level transcript a null one; ingesting the
  sidecars before the parent produces the same values; and the sidecar's
  `session_id` still equals its parent's, so the link cannot have come from the
  records.

### Task 5: The sidecar's `agent-*.meta.json` is stored, unparsed

- **Files:** crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/src/testkit.rs, crates/verbatim-core/tests/lineage.rs, crates/verbatim-core/tests/fixtures.rs, tests/fixtures/README.md, tests/fixtures/subagents/agent-alpha.meta.json
- **Action:** When ingesting a sidecar, read the `agent-*.meta.json` sitting
  beside it - same directory, same stem - and store its bytes verbatim in the
  `session_meta` column PLAN-1 added, with no parsing into typed columns (D-04).
  816 such files exist carrying `agentType`, `description`, `toolUseId`,
  `spawnDepth` and `model`, the format is undocumented and may drift, and
  storing bytes keeps D-13 intact (the blob stays transcript bytes only) while
  letting phase 3 extract `description` for ranking without a reingest. Do not
  add the meta file to discovery - D-16's filename filter excludes it and must
  keep excluding it, since it is not a transcript and must never become a
  session. Coverage is 816 meta files against 818 sidecars, so a missing one is
  normal: leave the column null and record nothing. Route this read through the
  counted open helper as well, so an excluded project's meta files are counted
  the same way its transcripts are. Add the fixture
  `tests/fixtures/subagents/agent-alpha.meta.json` beside the existing
  `subagents/agent-alpha.jsonl`, shaped with those five real fields, document it
  in `tests/fixtures/README.md`, and register it in `testkit.rs` as a non-
  transcript fixture rather than in `TRANSCRIPT_FIXTURES` - the assertions in
  `crates/verbatim-core/tests/fixtures.rs` that every transcript fixture parses
  as JSONL and ends with a newline do not apply to it.
- **Verify:** `cargo test --workspace --test lineage --test discover --test
  fixtures` passes, including: ingesting `subagents/agent-alpha.jsonl` with its
  meta file beside it stores the meta file's bytes byte-identically in
  `session_meta` and creates no `sessions` row for the meta file itself;
  ingesting a sidecar with no meta file beside it leaves the column null and
  produces no error; and the archived blob for the sidecar still reproduces only
  its transcript bytes.

### Task 6: Exclusion holds on the read path too

- **Files:** crates/verbatim-core/src/config.rs, crates/verbatim-core/src/lib.rs, crates/verbatim/src/cmd/status.rs, crates/verbatim-core/tests/exclusion.rs, crates/verbatim/tests/status.rs
- **Action:** Close the second half of ING-08. An excluded project must be
  invisible on read as well as never read on ingest, and the failure mode is
  named in PROJECT.md: claude-mem honors exclusion on write and ignores it on
  read. Store no per-session flag (D-23) - a flag written at ingest cannot
  retroactively hide a session archived before its project was excluded, which
  is precisely the case that matters. Instead expose one session-listing read
  entry point that applies the configured exclusions against
  `session_meta.project` AND against the pre-mapping key task 2 stored beside
  it, using the path-side predicate task 2 of PLAN-1 built. Both columns,
  because either one alone leaks: a worktree session archived before its parent
  repo was excluded carries the folded parent in `project` and the worktree path
  in the pre-mapping column, and a user may reasonably exclude either path,
  and make `verbatim status` count sessions, turns and watermarks through it, so
  the phase's only read command already goes through the boundary phase 3's
  search will reuse. A session whose `project` is null - one real transcript
  carries no `cwd` at all - is visible, since nothing can say it is excluded.
  Document at the entry point that every future read path is required to go
  through it rather than querying `session_meta` directly.
- **Verify:** `cargo test --workspace --test exclusion --test status` passes,
  including: ingest a tree of three projects with no exclusions configured, then
  add one project's path to the config and re-run `verbatim status` without
  re-ingesting - the session and turn counts drop by exactly that project's
  share and its sessions are absent from the listing entry point, while the rows
  are still present in `session_meta` when queried directly; a project whose
  path merely shares leading segments with the excluded one stays visible; a
  session whose `project` folded to a repo but whose pre-mapping key is a
  worktree beneath it is hidden when either of those two paths is excluded; and
  a session with a null `project` stays visible under any exclusion list.

## Notes

- ING-07 stays deferred to phase 8 and appears in no task here; see PLAN-1's
  Notes for the reason and for the `.planning/ROADMAP.md` edit still owed.
- AC3's worktree half cannot be demonstrated against live corpus data: all 13
  real worktree `cwd` values are deleted directories, so git answers for none of
  them. Task 2 therefore constructs a repository and a worktree directory in a
  temporary tree, which is what D-06 anticipated.
- Task 1's git-backed assertions skip when `git` is absent from `PATH`. On the
  development machine `git` is at `/usr/bin/git`, so they run there; a CI image
  without git degrades to the non-git cases rather than failing.
