# Roadmap: Verbatim

## Overview

The archive is the product, so it is built first and everything else is built on top of it: a store that can hold a session losslessly and prove it, then an ingest that survives a real transcript tree, then a query path that finds a turn from the terminal and from the model. Only once recall works is the thing wired into Claude Code, because hooks and install are worth nothing until there is something to recall. Injection follows, then the feedback loop that measures whether injection helped — deliberately after injection exists, since it labels injection's own decisions. Observations and their egress boundary come next as the one opt-in, one network-touching feature, and lifecycle management lands last because retention is meaningless until there is data worth aging out. Each phase leaves something exercisable: a store you can verify, an ingest you can run, a search you can type, hooks that fire, a brief that appears, stats that prove precision.

## Phases

- [ ] **Phase 1: Archive Core** - the store, the blob format, and single-transaction ingest of one transcript
- [ ] **Phase 2: Ingest At Scale** - discovery, identity, exclusion and crash resilience over a real transcript tree
- [ ] **Phase 3: Recall** - Rust-side expansion, entity extraction, terminal search, and the three MCP tools
- [ ] **Phase 4: Hooks And Install** - detached hook spawn, npm install, backfill, doctor and uninstall
- [ ] **Phase 5: Context Injection** - the SessionStart resume brief and precision-first prompt injection
- [ ] **Phase 6: Feedback Loop** - decision logging, outcome labels, offline replay, and stats that prove precision
- [ ] **Phase 7: Observations** - mechanical facts, opt-in LLM judgment, and the egress boundary
- [ ] **Phase 8: Retention And Lifecycle** - retention actions, compaction, usage, snapshots, export and relocation

## Phase Details

### Phase 1: Archive Core
**Goal:** A store that can hold a session's turns losslessly, read one turn cheaply, prove its own integrity, and rebuild everything derived from the blobs alone.
**Depends on:** Nothing (first phase)
**Requirements:** STOR-01, STOR-02, STOR-03, STOR-04, STOR-05, ING-01, ING-02
**Success Criteria:**
1. Ingesting a transcript and then reading one turn decompresses exactly one 64 KB block, shown by an instrumented read counting blocks touched.
2. Killing the process at randomized points during ingest and reopening the store leaves no session with turn, FTS or entity rows and no matching blob, and no watermark ahead of a committed blob.
3. `verbatim verify` on a store with one byte flipped inside a blob names that session id and no other.
4. `verbatim reindex` after dropping every derived table returns the same rows for a fixed query set as before the drop.
5. Opening a store whose `meta` format version is one higher than the binary knows exits non-zero with a message and writes nothing.
6. Running a second `verbatim ingest` while one holds the lock exits 0 in under 50 ms and writes no rows.

### Phase 2: Ingest At Scale
**Goal:** Ingest that runs against the real 1,896-file transcript tree: finding every session including subagent sidecars, keying projects correctly, honoring exclusions, and reporting what it did.
**Depends on:** Phase 1
**Requirements:** ING-03, ING-04, ING-05, ING-06, ING-07, ING-08, ING-09
**Success Criteria:**
1. A full pass over the real corpus parses every record with zero panics and zero unparseable lines, and the run's session count includes the 214 subagent sidecar directories a top-level glob misses.
2. Sessions continued across files are linked, and the count of `continues_from` links is non-zero on the real corpus (the measured rate is 1.2%).
3. Two transcripts whose `cwd` values are a repo and its worktree resolve to the same project key, and a project path containing a literal `-` is not confused with a path separator.
4. Appending a compaction to an already-ingested transcript and rerunning ingest adds the boundary record with its dropped-turn set, without rewriting any earlier turn.
5. A transcript under an excluded prefix produces no read of that file, shown by a syscall or instrumented-open trace, and no rows in any table.
6. Killing ingest mid-pass and rerunning converges to the same store contents as an uninterrupted pass, and `verbatim status` shows the failed run and its error.

### Phase 3: Recall
**Goal:** Finding a specific past turn, from the terminal and from inside Claude Code, by identifier, path, error or free text.
**Depends on:** Phase 2
**Requirements:** RCL-01, RCL-02, RCL-03, RCL-04, RCL-05, RCL-06, RCL-07, RCL-08, RCL-09, RCL-10, RCL-11
**Success Criteria:**
1. A turn containing `SearchManager` is returned by searches for both `SearchManager` and `manager`, and a turn containing `src/worker/S.ts` is returned by a search for `worker`.
2. The same error appearing in two sessions with different line numbers, addresses and timestamps normalizes to one entity value shared by both turns.
3. `verbatim search --json` output validates against its documented shape, and a query matching nothing exits 0 with an empty result set rather than non-zero.
4. `recall_search` for a path returns hits scoped to the current project by default and returns cross-project hits only when `project: "*"` is passed.
5. `recall_get` on an id whose body was evicted returns the record with `body_evicted` set rather than an error, and a malformed request returns an empty result with a reason instead of throwing.
6. The MCP server process exits after the client disconnects, holds no port, and its tools report `readOnlyHint`.

### Phase 4: Hooks And Install
**Goal:** Verbatim wired into Claude Code by a single install command, keeping itself current with no user action and no measurable cost at the hook.
**Depends on:** Phase 3
**Requirements:** ING-10, ING-11, INST-01, INST-02, INST-03, INST-04, INST-05, INST-06, INST-07, INST-08
**Success Criteria:**
1. Feeding each hook the exact JSON Claude Code sends returns exit 0 within the asserted cold-start budget, with clean stdout, and the spawned child survives the parent's exit with no inherited handles and no console window on Windows.
2. Running `verbatim install` twice leaves exactly one hook entry per hook and one MCP registration, with a settings.json backup written on the first run.
3. Replacing the binary and rerunning install changes no byte of the hooks object in settings.json.
4. `verbatim doctor` against a store whose hook path points at a missing binary reports the problem and prints a command that, when run, makes the same check pass.
5. Uninstall on a settings.json unchanged since install restores it to its pre-install bytes and leaves the data directory present, printing its path.
6. Backfill of the real corpus prints an estimate, returns the shell immediately, and resumes from where it stopped when killed and rerun.

### Phase 5: Context Injection
**Goal:** The model starts a session already knowing where it left off, and gets the one past turn that matters when a prompt names something the archive has seen — and silence the rest of the time.
**Depends on:** Phase 4
**Requirements:** INJ-01, INJ-02, INJ-03, INJ-04, INJ-05, INJ-06
**Success Criteria:**
1. SessionStart in a project with indexed history emits a brief within the configured token budget and within the single-digit-millisecond wall budget, measured over repeated runs.
2. Two SessionStart runs against an unchanged store produce byte-identical output, and no timestamp finer than a day appears anywhere in the brief.
3. A prompt naming a file path recently edited in a past session injects that turn, and a prompt of generic prose with no entity match injects nothing.
4. Repeating the same entity-bearing prompt twice in one session injects the turn once and suppresses it the second time with a logged reason.
5. The first prompt after a compaction draws candidates only from the turns the `compact_boundary` marked as dropped, capped at 3.
6. Deleting the store file and submitting a prompt still exits 0 within the deadline and emits nothing.

### Phase 6: Feedback Loop
**Goal:** Whether injection helps becomes a measurement rather than an opinion, and a retrieval change can be tested against history before it ships.
**Depends on:** Phase 5
**Requirements:** FEED-01, FEED-02, FEED-03, FEED-04
**Success Criteria:**
1. Every prompt produces a decision record including the ones that injected nothing, with entities, candidates, suppressions and reasons, thresholds and tokens.
2. Ingest labels a decision `hit` when an injected turn's path or symbol is referenced downstream in the same session, and `false positive` when it never is, on a fixture session with both cases.
3. Changing an extraction rule and running the replay harness reports the label diff across the logged history without touching the live store.
4. `verbatim stats` reports precision, miss count, and tokens injected versus referenced, and the numbers change in the expected direction on a fixture with a known outcome mix.

### Phase 7: Observations
**Goal:** Optional, auditable session summaries whose every claim points back at a real turn, generated through one provider path, with a redaction boundary that only exists when data actually leaves the machine.
**Depends on:** Phase 4
**Requirements:** OBS-01, OBS-02, OBS-03, OBS-04, OBS-05, OBS-06, OBS-07, OBS-08, PRIV-01, PRIV-02, PRIV-03
**Success Criteria:**
1. Mechanical observations for a finalized session list its files, tools, commands, errors, branch, commits, turn count and duration with no model call made.
2. With judgment enabled, a finalized session produces one call and JSON validating against the schema, and every decision, learned and unresolved entry carries a `turn_id` that resolves to a real turn.
3. A provider response that fails to parse is retried once and then stored with `status = parse_failed`, and ingest of the same run still completes.
4. Pointing the provider block at a local endpoint and at OpenRouter exercises the same code path with only base URL, model and key changing.
5. A credentials file readable by group or world is refused with a message, and no test asserting on stdout, stderr or the runs table ever finds a key value in them.
6. A full run with judgment disabled opens no network connection, shown by a socket-level trace; with judgment enabled against a remote provider, the payload is filtered at egress while the local-provider path filters nothing.

### Phase 8: Retention And Lifecycle
**Goal:** A store that can be aged, shrunk, measured, snapshotted, exported and relocated, so it stays healthy over years without manual surgery.
**Depends on:** Phase 2
**Requirements:** RET-01, RET-02, RET-03, RET-04, RET-05, STOR-06, STOR-07, PRIV-04
**Success Criteria:**
1. With retention unset, an ingest pass over a corpus older than any plausible default deletes and evicts nothing.
2. An evicted session still appears in `verbatim sessions` and still matches a search, and `recall_get` on its turns returns `body_evicted`; a deleted session appears in neither.
3. `--dry-run` reports the sessions a retention pass would evict or delete and changes no row, and the subsequent real pass acts on exactly that set.
4. `verbatim compact` after a delete pass reduces the on-disk store size, and `verbatim usage` reports per-project and per-month byte totals that sum to it.
5. `verbatim data move` relocates the store to a new path, after which the hooks and MCP server read the new location with no settings.json change.
6. A snapshot taken during an active ingest opens as a valid store and passes `verbatim verify`.
