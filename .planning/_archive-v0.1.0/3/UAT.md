---
status: testing
phase: 3
fields_version: 1
started: 2026-08-13
updated: 2026-08-13
---

## Items

### 1. Cold start from scratch
expected: With all processes stopped and ephemeral state cleared, a fresh run boots clean: the store opens, the DERIVED_SCHEMA bump drives a full reindex on the first ingest, and one primary query (verbatim search over a real term) returns real hits.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Fresh VERBATIM_DATA_DIR: `verbatim ingest` exit 0 over 185 real transcripts, `verbatim status` 185 sessions / 19077 turns, `verbatim search --json BlobReader` 10 hits. Aged store with derived_schema forced to 1 and turns_fts/entities emptied: the next `verbatim ingest` rebuilt to 19077 / 23367 and reset derived_schema to 3.

### 2. Case and separator expansion reaches a turn
expected: A turn containing `SearchManager` is returned by a search for `SearchManager` AND by a search for `manager`; a turn containing `src/worker/S.ts` is returned by a search for `worker`.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: One turn (rowid 50331681) matches turns_fts for blobreader, reader, blob and Reader alike; turn 16777221, whose paths row is `.../recall/scope.rs`, matches `scope` and `recall`. `a_component_query_finds_the_whole_token` passes.

### 3. JSON scaffolding is not indexed
expected: A search for `toolUseResult` - a JSON key present on 4,858 corpus records - returns zero hits.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: airlock holds 14 raw records with `"toolUseResult"` and 95 with `"parentUuid"`; a project-scoped search for each returns 0 hits. The verbatim project's 10 `toolUseResult` hits were read and are all prose/code that genuinely names the key.

### 4. Error normalization collapses variable parts
expected: Two turns in different sessions whose stderr differs only in line numbers, addresses, timestamps and UUIDs carry one identical `error` entity value.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: 12 distinct normalized `error` values are shared across more than one session in the live store; `one_failure_seen_twice_is_one_value_and_a_different_integer_is_two` passes.

### 5. Error normalization stops at bare integers
expected: Two turns whose errors differ only in a bare integer that is not a line number carry two DIFFERENT `error` entity values.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: `placeholder_at` (entity.rs:325-336) recognizes only timestamp, uuid, address and :line:col; live normalized values retain bare integers. Same named test carries the negative arm and passes.

### 6. All five entity kinds are emitted over the real corpus
expected: Running against the real archive emits entities of all five kinds; a `Read` tool_use naming a file emits a path entity while an assistant message naming that same path in prose emits none.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: command 3375, error 179, path 11156, symbol 3203, tool 5454 over 19,077 real turns; 423 turns carry a `/dev/null` paths row while 33 turns that name it in text carry none.

### 7. The per-turn entity cap holds
expected: No turn in the store carries more entities than the cap.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: MAX_ENTITIES_PER_TURN = 48 (entity.rs:62); the live per-turn maximum is exactly 48, so the cap binds and nothing exceeds it.

### 8. Reindex reproduces the derived rows
expected: Dropping `turns_fts` and the entity tables and running `verbatim reindex` reproduces the same rows.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: entities 23367 (486,367 value bytes) / paths 11156 / turns_fts 19077 -> deleted to 0/0/0 -> `verbatim reindex` -> byte-identical counts and value-length sum.

### 9. Search exit codes
expected: `verbatim search src/worker/S.ts` exits 0 with hits; a query matching nothing exits 0 with an empty result set; an unknown flag exits 2.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: path query exit 0 with hits; unmatched query exit 0 with an empty set and ok:true; `--nope`, an unknown subcommand and a malformed `--since` all exit 2.

### 10. --json on every data command
expected: Each of `search`, `show`, `sessions`, `status`, `verify` and `reindex` accepts `--json`, emits JSON on stdout validating against the shape documented in `docs/json-shapes.md`, and puts every diagnostic on stderr.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: search/show/sessions/status/verify/reindex each emit one stdout document keyed command,data,ok,reason with 0 bytes of stderr on success; fields match docs/json-shapes.md:71-160; diagnostics (out-of-scope show, bad --since) go to stderr. `every_data_command_emits_the_documented_shape` passes.

### 11. Default project scoping, and the star escape
expected: `verbatim search` and `recall_search` return only current-project turns by default, and cross-project turns when `project: "*"` is passed.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: Five cwd/scope cases run identically through `verbatim search` and through a spawned `verbatim mcp` recall_search: default returns only the standing project, `*` returns 3 projects, an unindexed cwd returns 0 hits with `no archived project contains /tmp` rather than widening. Sibling-prefix probe (`/data/code/verbatim-extra`, `/data/code/verbatimX`) does not resolve to /data/code/verbatim; a real worktree key on a second store folds to the repository's project.

### 12. An excluded project is invisible on both read paths
expected: With a project excluded, neither `verbatim search` nor `recall_search` returns any of its turns - and no reason string names it.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: 49 weathervane turns present, 0 returned by `verbatim search --project '*'` or by `recall_search` at either scope, and neither response sets a reason. The path is named only when the caller itself supplied it. Residual, already an open item: `recall_get`/`recall_context` at `project:"*"` still name the excluded absolute path in a reason; at the default scope they correctly answer `no turn N is archived`.

### 13. Sidechain turns rank below top-level
expected: A sidechain (subagent) turn ranks below a top-level turn of equal score.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: `a_sidechain_turn_sorts_below_an_identical_top_level_turn` passes on an equal-bm25 pair; live, six 50-hit cross-project searches over a store with 69 sidechain sessions produce zero equal-relevance inversions.

### 14. recall_context ordering and session boundary
expected: `recall_context` returns turns in `turn_seq` order around a hit and returns nothing past the session boundary, reporting the boundary explicitly.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: Live MCP windows over a 95-turn session: ascending turn_seq in every case, at_session_start true at seq 0, at_session_end true at seq 94, nothing past either end, and an over-large request lowered to the 25-per-side budget with that reason stated.

### 15. Evicted body is a flag, malformed call is a reason
expected: `recall_get` on an id whose session has `is_evicted` set returns the record with `body_evicted` rather than an error; a malformed request returns an empty result carrying a reason, never a throw.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: is_evicted=1 -> records returned with body_evicted true and body null, no error, exit 0. Seven malformed calls all return results with prose reasons; zero JSON-RPC errors, zero panics.

### 16. MCP handshake: three read-only tools, no socket, ends at EOF
expected: A spawned `verbatim mcp` fed `initialize` and `tools/list` on stdin reports exactly three tools each marked `readOnlyHint`, opens no listening socket under `ss`, and exits 0 after stdin closes.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: tools/list returns exactly 3 tools each with annotations.readOnlyHint true; `ss -ltnp` shows no listening socket for the live pid and /proc/<pid>/fd holds only stdio; exit code 0 after stdin close.

### 17. MCP against a nonexistent store
expected: The same binary run against a nonexistent store path returns an empty result with a reason and creates no file.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: Empty hits with `no verbatim store at <path>/verbatim.db; nothing has been archived yet`, exit 0, and the directory still does not exist afterwards.

## Summary

total: 17
passed: 17
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 0
