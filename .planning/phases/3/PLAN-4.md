---
phase: 3
plan: 4
requirements:
  - RCL-07
  - RCL-08
  - RCL-09
  - RCL-10
  - RCL-11
files:
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/mcp/mod.rs
  - crates/verbatim/src/cmd/mcp/rpc.rs
  - crates/verbatim/src/cmd/mcp/tools.rs
  - crates/verbatim/tests/mcp.rs
---

# Phase 3: Recall - Plan 4 of 4 (the MCP server)

**SEQUENTIAL: PLAN-2 and PLAN-3 must complete before this plan.** It calls the
`recall` module PLAN-2 and PLAN-3 built and shares
`crates/verbatim/src/main.rs` and `crates/verbatim/src/cmd/mod.rs` with PLAN-3.
Do not run them in parallel.

## Goal

The model can reach the archive from inside Claude Code: `verbatim mcp` is a
short-lived stdio process exposing exactly three read-only tools -
`recall_search`, `recall_context`, `recall_get` - auto-scoped to the project
the session is in, bounded server-side, and gone the moment the client
disconnects.

## Must be true when done

- A spawned `verbatim mcp` fed `initialize` and `tools/list` on stdin reports
  exactly three tools, each marked `readOnlyHint`, and exits 0 after stdin
  closes.
- The process opens no listening socket and holds no port for as long as it
  runs.
- `recall_search` for a path returns hits scoped to the current project by
  default and cross-project hits only when `project: "*"` is passed.
- `recall_context` returns the chronological turns around a hit, stopping at
  the session boundary; `recall_get` on an id whose body was evicted returns
  the record with the evicted flag rather than an error.
- A malformed request returns an empty result carrying a reason instead of
  throwing, and running against a nonexistent store path returns an empty
  result with a reason and creates no file.

## Context

- D-11 makes this a hand-written JSON-RPC 2.0 loop over stdin/stdout using
  `serde_json`, with no MCP SDK and no async runtime: `PROJECT.md:73` bars a
  runtime outright, and the SDK would drag one into the binary that also serves
  the hook path. D-26 makes `mcp` a subcommand of the same binary, dispatched
  from `main.rs` like every other command, so install has one artifact to
  place. D-27 bounds the process by its stdin - no socket, no listener, no
  background thread, and no ingest lock, because reads run beside an ingest
  pass under WAL exactly as `cmd::status` already does.
- Four MCP facts, pinned from the Claude Code 2.1.229 bundle at
  `~/.local/share/claude/versions/2.1.229` and from the MCP tool-annotation
  reference, so no task has to guess them:
  1. `readOnlyHint` sits inside a tool descriptor's `annotations` object,
     beside `destructiveHint`, `idempotentHint` and `openWorldHint`.
  2. That client accepts these `protocolVersion` strings: `2026-07-28`,
     `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`, `2024-10-07`. The
     client sends its own in `initialize` and the server answers with one it
     supports.
  3. Termination is stdin close first, a 2 s wait, then SIGTERM, another 2 s,
     then SIGKILL (measured in that bundle's stdio transport `close`). Exiting
     at EOF is therefore sufficient and no signal handling is needed.
  4. A tool-level failure is a normal result carrying `isError`, not a
     JSON-RPC error; JSON-RPC errors are for protocol-level failures - parse
     error, invalid request, method not found, invalid params.
- D-12's auto-scoping (longest-prefix match of the process `cwd` against the
  stored `project` / `project_pre_worktree` values) and D-10's read-only open
  are already built in PLAN-2; this plan calls them rather than reimplementing
  either.
- Out of scope: registering the server in `settings.json`, which is install's
  job in phase 4, and any fourth tool - `stats`, `usage`, `verify`, `reindex`
  and `export` are CLI-only because every tool description sits in every
  session's context forever.

## Tasks

### Task 1: Serve the MCP handshake over stdio

- **Files:** crates/verbatim/src/cmd/mcp/mod.rs,
  crates/verbatim/src/cmd/mcp/rpc.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs, crates/verbatim/tests/mcp.rs
- **Action:** Add the `mcp` subcommand to `main.rs`'s dispatch table and give it
  the loop: read newline-delimited JSON-RPC 2.0 messages from stdin, one
  message per line, write one response line per request to stdout, and return
  0 at EOF. Hand-written over `serde_json`, no SDK, no async runtime, no
  thread (D-11). Answer `initialize` with the negotiated `protocolVersion`,
  `capabilities` declaring tools, and `serverInfo` naming the binary and its
  version - echo the client's requested version when it is one of the six
  listed in Context, otherwise answer with the server's own pinned default,
  because a client that cannot use the answer disconnects and one that can
  proceeds. Answer `tools/list` with exactly three tool descriptors -
  `recall_search`, `recall_context`, `recall_get` - each carrying a one-line
  description, an input schema, and an `annotations` object with
  `readOnlyHint` true. Descriptions state what the tool returns and never
  instruct the model how to behave. A message with no `id` is a notification,
  including `notifications/initialized`, and gets no response ever: replying to
  one is a protocol violation. An unparseable line, an unknown method or a
  malformed envelope gets a JSON-RPC error object with the standard code, and
  stdout carries nothing but JSON-RPC messages - every diagnostic goes to
  stderr, since stdout is the transport.
- **Verify:** `cargo test -p verbatim --features testkit --test mcp` spawns the
  built binary with `mcp`, writes `initialize` and `tools/list` on stdin,
  closes stdin, and shows: one response per request in order, exactly three
  tools, `annotations.readOnlyHint` true on each, no response emitted for
  `notifications/initialized`, a `-32601` error for an unknown method, and exit
  code 0.

### Task 2: Wire `recall_search`

- **Files:** crates/verbatim/src/cmd/mcp/tools.rs,
  crates/verbatim/src/cmd/mcp/mod.rs, crates/verbatim/tests/mcp.rs
- **Action:** Implement RCL-07 over PLAN-2's search: `tools/call` for
  `recall_search` accepts a query plus the filters the query layer already
  supports - project, paths, tool, kind, time window and a limit - and returns
  ranked hits each carrying id, session, timestamp, project and excerpt. The
  store is opened through PLAN-2's read-only constructor and no ingest lock is
  taken (D-27). Project defaults to the current one, resolved by PLAN-2's
  longest-prefix match of the process's working directory against the stored
  project keys - never a git subprocess, which costs 10-30 ms on a cold start
  and can disagree with what ingest wrote inside a worktree - and the literal
  `*` opts into every project. Subagent turns are in the result by default and
  ranked below top-level turns of equal score, which the query layer already
  does. Return the hits as the tool's content in a shape a model can read
  without prose framing.
- **Verify:** `cargo test -p verbatim --features testkit --test mcp` shows a
  `recall_search` call for a path, made with the process's working directory
  inside one fixture project, returning only that project's turns; the same
  call with `project: "*"` returning turns from more than one project; and an
  excluded project's turns absent from both.

### Task 3: Wire `recall_context`

- **Files:** crates/verbatim/src/cmd/mcp/tools.rs,
  crates/verbatim/tests/mcp.rs
- **Action:** Implement RCL-08 over PLAN-2's context window: given a turn id
  and how many turns before and after, return the surrounding turns of the same
  session in `turn_seq` order, with the boundary reported explicitly when the
  window ran out of session (D-06, D-22). It does not follow `continues_from`
  or `parent_session_key`; the caller is told it reached the boundary and can
  issue a second call, which is the only safe shape when continuation is a
  fan-out with zero, one or many successors. Counts before and after are
  bounded by the same server-side budget task 5 sets, so one call cannot return
  a whole session.
- **Verify:** `cargo test -p verbatim --features testkit --test mcp` shows a
  `recall_context` call around a middle turn returning the requested turns in
  `turn_seq` order, a call around the first turn of a session that continues
  from another returning nothing before it and reporting the start boundary,
  and a request for more turns than the budget returning the budget's worth.

### Task 4: Wire `recall_get`

- **Files:** crates/verbatim/src/cmd/mcp/tools.rs,
  crates/verbatim/tests/mcp.rs
- **Action:** Implement RCL-09 over PLAN-3's by-id read: a list of turn ids in,
  each turn's full verbatim text out, with its session, timestamp and project,
  and each session's blob read at most once per request (D-20). A turn whose
  session carries `session_meta.is_evicted` comes back flagged as evicted with
  no body rather than as an error (D-08) - retention is phase 8, so the test
  sets that column directly on a fixture session. An unknown id and a turn in
  an excluded project are absent from the result with a reason, never a throw.
  Bound the number of ids one call may ask for.
- **Verify:** `cargo test -p verbatim --features testkit --test mcp` shows
  `recall_get` on ids from two sessions returning both records with bytes equal
  to the fixtures' own lines, an id whose session has `is_evicted` set
  returning the evicted flag and no body with the call still succeeding, and an
  unknown id returning a reason.

### Task 5: Bound every tool and answer malformed requests with a reason

- **Files:** crates/verbatim/src/cmd/mcp/tools.rs,
  crates/verbatim/src/cmd/mcp/mod.rs, crates/verbatim/tests/mcp.rs
- **Action:** Close RCL-10's remaining half. A server-side result budget caps
  what any tool returns whatever the caller asks for, so a client cannot pull
  the archive into its context one call at a time; a caller's own limit may
  only lower it. A malformed `tools/call` - missing arguments, an argument of
  the wrong JSON type, a limit that is not a number, an id that is not an
  integer, an unparseable date - returns a normal tool result carrying an empty
  result and a reason naming what was wrong, not a JSON-RPC error and never a
  panic: RCL-10 requires the empty-result-with-reason contract because a throw
  is what the client surfaces as a broken server. Protocol-level failures keep
  the JSON-RPC error codes task 1 established; the line between them is that a
  request the server understood and could not satisfy is a result, and a
  message the server could not understand as a request is an error. No unwrap
  on caller-supplied data anywhere on this path - a panic writes a Rust
  backtrace onto the transport stream and takes the session's tool with it.
- **Verify:** `cargo test -p verbatim --features testkit --test mcp` shows each
  of the three tools called with a missing argument, a wrong-typed argument and
  an over-large limit returning a successful response whose content carries an
  empty result and a reason, the process still answering the next request
  afterwards, and no response containing a Rust panic message.

### Task 6: Prove the process is short-lived and holds nothing

- **Files:** crates/verbatim/tests/mcp.rs
- **Action:** Assert RCL-11 and ROADMAP success criterion 6 at the process
  level, which is where they are claims about a process rather than about
  code. Spawn the built binary with `mcp`; while it is alive, show it holds no
  listening socket by inspecting the system's socket table for its pid; close
  stdin and show it exits 0 within a short deadline without any signal being
  sent, which is the property that makes Claude Code's stdin-close-then-SIGTERM
  shutdown clean rather than a kill. Show that a run pointed at a data
  directory holding no store answers a `recall_search` call with an empty
  result and a reason, exits 0, and leaves that directory with no file in it -
  the read-only open PLAN-2 built is what makes that true, and a server
  advertising `readOnlyHint` that created a database on connect would be the
  contradiction D-10 exists to prevent. Probe for the socket-inspection tool
  before using it and skip that one assertion loudly when it is absent, the way
  `testkit::corpus_dir` skips loudly, so the suite stays green on a machine
  without it rather than passing silently.
- **Verify:** `cargo test -p verbatim --features testkit --test mcp` passes on
  this machine, where `ss` is present at `/usr/bin/ss`, and prints the skip
  line instead of the socket assertion when `ss` is unavailable; the empty-data-
  directory run leaves the directory empty, asserted by reading it back.

## Notes

- The four MCP ecosystem facts CONTEXT flagged as unpinnable from the codebase
  were measured from the installed Claude Code 2.1.229 bundle rather than
  assumed; they are listed in this plan's Context so no task has to rediscover
  them. The one that most changes the code is the third: because the client
  closes stdin first and only then escalates, exit-at-EOF is the whole
  lifecycle and no signal handler is needed on any platform.
- The other flagged assumption - whether the official `rmcp` SDK has a
  runtime-free stdio transport - is not resolved and does not need to be: D-11
  chose the hand-written loop on the no-async-runtime constraint alone, and
  task 1's loop is a few hundred lines against a dependency that would be
  linked into the hook path too.
