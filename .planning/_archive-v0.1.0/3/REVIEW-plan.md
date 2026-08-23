{ "findings": [
  { "file": ".planning/phases/3/PLAN-2.md",
    "line": 242,
    "severity": "high",
    "claim": "Task 6 matches a query TOKEN against a whole entities.value_norm, so the IDF boost can never fire for the two entity kinds whose values are multi-token - path and error - which are the phase's headline recall cases.",
    "failure_scenario": "Task 2 splits the user's string on the index's unicode61 separators, so `src/worker/S.ts` becomes the tokens [src, worker, S, ts]. PLAN-1 task 4 stores a path entity as the whole value `src/worker/S.ts` (quotes and :line:col trimmed only), and PLAN-1 task 5 stores an error entity as a whole normalized single-line stderr. No query token ever equals either value_norm, so document frequency is never computed and no entity contribution is added. Task 6's own verify - 'for a query naming a path, the turn whose tool_use carried that path as an entity outranks a turn that merely mentions it in text' - is unachievable as specified, and RCL-04's query-time half is dead for path and error while still passing tests written around single-token `command`/`tool`/`symbol` values." },

  { "file": ".planning/phases/3/PLAN-2.md",
    "line": 99,
    "severity": "high",
    "claim": "Task 1's read-only constructor drops `bring_forward`, so a read command against a store written before phase 2 dies on `no such column` instead of taking D-18's degraded-read path, and PLAN-3's test for that path cannot detect it.",
    "failure_scenario": "`store/schema.rs:255-262` lists `session_meta.project_pre_worktree` (plus `agent_meta`, `transcript_diverged`) in BRING_FORWARD_COLUMNS, i.e. they are absent from any store initialized by a phase-1 binary and are added only by `Store::open`'s `bring_forward` (`store/open.rs:104-117`, whose comment names exactly this hazard). PLAN-2 task 1 removes that call from the read path and a SQLITE_OPEN_READ_ONLY connection cannot ALTER TABLE anyway. `verbatim search`/`sessions`/`mcp` then run PLAN-2 task 4's scoping query and `config::visible::sessions` (`config.rs:530-540`), both of which select `m.project_pre_worktree`, and rusqlite returns `no such column` -> Failure::Operational, exit 1, on the exact upgrade-then-search sequence D-18 exists to make graceful. PLAN-3 task 2's verify only rolls `meta.derived_schema` back by one, which leaves every column present, so the test is green while the real upgrade path errors." },

  { "file": ".planning/phases/3/PLAN-1.md",
    "line": 99,
    "severity": "high",
    "claim": "Task 1 specifies the new fixtures' contents but not their `cwd`, so they inherit the existing fixtures' hardcoded `/data/code/verbatim`, and PLAN-3 task 4's headline verify only passes on a checkout that literally lives at that path.",
    "failure_scenario": "Every existing fixture carries `\"cwd\": \"/data/code/verbatim\"` and nothing else, so `session_meta.project` for all fixture sessions is that one string. PLAN-2 task 4 makes the default scope a longest-prefix match of the process cwd against stored keys, and `crates/verbatim/tests/cli.rs:56-60` spawns the binary without `.current_dir`, so the child inherits the test's cwd (`<checkout>/crates/verbatim`). On this machine `/data/code/verbatim` is a prefix and the test passes; on CI or any clone at `/home/runner/work/verbatim` no stored key is a prefix, the default scope 'matches nothing and the reason says so', and PLAN-3 task 4's `verbatim search src/worker/S.ts` exits 0 with an empty result instead of 'at least one hit naming the fixture turn'. The same silent-empty applies to PLAN-3 tasks 5 and 6 and PLAN-4 task 2." },

  { "file": ".planning/phases/3/PLAN-2.md",
    "line": 207,
    "severity": "medium",
    "claim": "No task puts a second project into the fixture corpus, so the cross-project half of AC5 has nothing to prove itself against in the fixture-based tests PLAN-2 task 4 and PLAN-4 task 2 name.",
    "failure_scenario": "All seven existing fixtures and the four PLAN-1 task 1 adds resolve to the single project key `/data/code/verbatim`. PLAN-2 task 4's verify ('`*` returns turns from several projects') and PLAN-4 task 2's verify ('the same call with `project: \"*\"` returning turns from more than one project', 'an excluded project's turns absent from both') are unsatisfiable over the fixture store. `crates/verbatim-core/tests/exclusion.rs:25-50` builds its own multi-project tree, but PLAN-4's file list is only `crates/verbatim/tests/mcp.rs` and no task says the MCP test must synthesize transcripts with distinct `cwd` values whose directories actually exist so the spawned server can chdir into one - so the executed test degrades to a single-project assertion that passes with the project filter wired backwards." },

  { "file": ".planning/phases/3/PLAN-2.md",
    "line": 189,
    "severity": "medium",
    "claim": "Longest-prefix matching of the cwd against both stored keys selects the pre-worktree key inside a worktree, which re-splits the project phase 2 deliberately folded and hides the repo's own sessions.",
    "failure_scenario": "Phase 2 D-06/ING-05 stores `project = /repo` and `project_pre_worktree = /repo/.claude/worktrees/wt-a` for a worktree session (asserted at `crates/verbatim-core/tests/exclusion.rs:150-167`), so that a repo and its worktree are ONE project key (phase 2 success criterion 3). With cwd `/repo/.claude/worktrees/wt-a`, both keys are prefixes and the longest is the worktree key; scoping the search to that single key returns only the sessions run from the worktree and drops every session whose `project` is `/repo` with a null pre-worktree column. Standing in a worktree, `verbatim search` and `recall_search` therefore see a fraction of the project's history - the identity fragmentation D-12's own failure clause says the rule exists to prevent - and no task specifies that a matched pre-worktree key must be resolved back to its folded `project`." },

  { "file": ".planning/phases/3/PLAN-2.md",
    "line": 219,
    "severity": "medium",
    "claim": "Task 5 adds a path filter over the `paths` table without specifying the match rule, and the only rule consistent with PLAN-1's 'kept exactly as written, never canonicalized' storage - string equality - misses the same file under any other spelling.",
    "failure_scenario": "PLAN-1 task 4 writes `paths` rows verbatim: a `Read` tool_use contributes the absolute `/home/u/proj/src/worker/S.ts` while a `Bash` argv word in the same session contributes the relative `src/worker/S.ts`. A user running `verbatim search --path src/worker/S.ts` (the spelling `search` printed nowhere, and the one a human types) matches only the Bash-derived rows and silently loses every Read/Edit/Write hit for that same file, while a user pasting the absolute path loses the Bash ones. RCL-07 names paths as a first-class filter and nothing in the plan picks equality, suffix, or basename matching, so the filter's recall is decided accidentally by whoever writes the SQL." },

  { "file": ".planning/phases/3/PLAN-2.md",
    "line": 287,
    "severity": "medium",
    "claim": "Task 7's primary verification instrument, `BlobReader::blocks_decompressed`, structurally cannot detect the failure the task is bounding - PLAN-3 task 3 says so in its own body.",
    "failure_scenario": "`testkit::read_turn` (`crates/verbatim-core/src/testkit.rs:285-310`) selects the whole blob, opens a fresh `BlobReader`, resets its counter, and reads one range; the counter is per-reader. An excerpt implementation that materializes the 10.1 MB blob once per hit therefore reports 1 block for each of five hits from one session and passes 'reading that session's blob once'. PLAN-3 task 3 states this explicitly ('a naive per-id read materializes a whole 10 MB blob per turn while the block counter at `blob/reader.rs:44-53` still reports one block decompressed'). Offering it as the first of two instruments means an implementer can satisfy the verify while D-20 is violated on the hottest read path." },

  { "file": ".planning/phases/3/PLAN-2.md",
    "line": 149,
    "severity": "medium",
    "claim": "Task 3 says the search is `turns_fts MATCH` 'joined to `turns` and `session_meta`' without requiring an outer join, so a session with no `session_meta` row loses every turn from search - the case the read path is explicitly required to keep visible.",
    "failure_scenario": "`config::visible::sessions` uses `LEFT JOIN session_meta` with the comment 'a session archived without a `session_meta` row is still listed rather than silently dropped: `verbatim verify` is what reports that damage, and a read path that hid it would hide the evidence' (`config.rs:528-533`), and `crates/verbatim-core/tests/exclusion.rs:214-226` pins it. An inner join in `recall::search` makes those turns unfindable by any query with no error and no reason string, so the store looks smaller than it is and the damage `verify` would name is concealed by the search path instead." },

  { "file": ".planning/phases/3/PLAN-4.md",
    "line": 107,
    "severity": "medium",
    "claim": "Task 1 enumerates `initialize`, `tools/list` and notifications and sends every other method to a `-32601` error, but `ping` is a base-protocol MCP method the bundled client implements.",
    "failure_scenario": "The Claude Code 2.1.229 bundle this plan pins its four MCP facts from carries `async ping(){return this.request({method:\"ping\"},...)}` on both client and server sides plus 13 references to `PingRequestSchema`, so a `{\"method\":\"ping\",\"id\":N}` request is reachable. Under task 1's rule the server answers `-32601 method not found` to a request the base protocol says must be answered promptly; a client using ping as a liveness probe treats the error as an unhealthy server and tears the connection down mid-session, and task 1's verify - which only sends `initialize`, `tools/list`, a notification and one deliberately unknown method - never exercises it." }
] }
