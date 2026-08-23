---
phase: 7
plan: 2
requirements: [OBS-05, PRIV-01, PRIV-02, PRIV-03]
files:
  - Cargo.toml
  - Cargo.lock
  - crates/verbatim-core/Cargo.toml
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/credentials.rs
  - crates/verbatim-core/src/observe/mod.rs
  - crates/verbatim-core/src/observe/net.rs
  - crates/verbatim-core/src/observe/egress.rs
  - crates/verbatim-core/src/observe/provider.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/config.rs
  - crates/verbatim-core/tests/credentials.rs
  - crates/verbatim-core/tests/egress.rs
  - crates/verbatim-core/tests/provider.rs
  - crates/verbatim/src/cmd/doctor.rs
  - crates/verbatim/tests/doctor.rs
---

# Phase 7: Observations - Plan 2 (one provider path, one door to the network)

## Goal

One config block of base URL, model and key reaches ollama, llama.cpp,
OpenRouter or any other OpenAI-compatible endpoint through a single code path;
every connection this binary can make goes through one auditable constructor;
credentials load under an enforced permission rule and their values reach no
stream; and what leaves the machine is filtered on the declared destination.

## Must be true when done

- The workspace has exactly one HTTP client, named in exactly one source file,
  and a test fails if any other file in either crate names it.
- Every outbound connection is built by one constructor with an instrumented,
  test-visible attempt log, so "zero connections" is a number a test reads
  rather than a claim about the code.
- A credentials file readable by group or world is refused with a message
  naming the file and its mode, and no message, error or printed line anywhere
  carries a byte of the key's value.
- With the provider block absent, or present with `enabled` unset, nothing in
  this plan resolves a credential or builds a request.
- With `local = true` the request body goes out byte-identical to what was
  built; with `local` absent or false it goes out filtered, and a forgotten key
  therefore fails safe toward filtering.
- The phase-4 hook cold-start budget still passes with the HTTP client linked
  in.

## Context

- D-05 (OpenAI-compatible `chat/completions` only), D-06 (`ureq` 3.4 + rustls,
  blocking, no async runtime), D-08 (read `choices[0].message.content`
  specifically), D-09 (`response_format` json_schema strict), D-10 (config
  sub-table), D-13 (`local` is declared, never inferred), D-14 (loader lives
  here, precedence env then product config then shared file), D-15 (Unix mode
  bits; Windows ACL deferred with a doctor caveat), D-16 (the boundary covers
  error construction), D-20 (a hand-rolled `TcpListener` stub, never a mock),
  D-21 (an in-process seam plus a source-level assertion).
- Anthropic's own API shape and Anthropic subscription OAuth are out of this
  phase entirely (D-05); do not add a second request shape for them.
- Nothing here decides WHEN to call a provider or writes an `observations` row
  - that is PLAN-3. This plan builds the path and proves its boundary.

## Tasks

### Task 1: One HTTP client, behind one instrumented constructor

- **Files:** Cargo.toml, crates/verbatim-core/Cargo.toml, Cargo.lock, crates/verbatim-core/src/observe/net.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/tests/provider.rs
- **Action:** Add `ureq` 3.4 with rustls to the workspace dependencies and to
  `verbatim-core`, with default features trimmed to what a blocking
  `chat/completions` POST over TLS needs and nothing more. The root
  `Cargo.toml` justifies each of its dependencies in a comment against a
  measured 0.408 ms startup floor and states "no async runtime, no HTTP client,
  no thread pool" - that sentence is now false and must be rewritten to say
  what changed, why a blocking pure-Rust client is the one that keeps the
  constraint, and what it measured. Measure before writing it: the release
  binary size before and after, and the hook cold start before and after, and
  put both numbers in the comment. Then write `observe::net` as the ONE place a
  `ureq` type is ever named or constructed - every request in this workspace is
  built here and handed back as this module's own type, so the client is
  replaceable and, more importantly, countable. Give it a `testkit`-gated
  attempt log in the exact shape of `discover::opened`: it records the
  destination of every attempt BEFORE the connection is made, with `reset`,
  a listing accessor and `count`, and it compiles to nothing without the
  feature. Record the attempt for every call, including one that fails to
  connect: PRIV-03 is about what the binary reaches for, not what it succeeded
  in reaching.
- **Verify:** `cargo test -p verbatim-core --test provider` passes a
  source-level test that walks every `.rs` file under
  `crates/verbatim-core/src` and `crates/verbatim/src`, strips comment lines
  the way `inject_brief.rs`'s `nothing_in_the_injection_path_spawns_a_process_or_reads_observations`
  does, and asserts the string `ureq` appears in no file but
  `src/observe/net.rs`; and a test that makes one request against a
  `std::net::TcpListener` bound to a loopback port and finds exactly one entry
  in the attempt log. `cargo test -p verbatim --test hook
  every_event_exits_zero_with_an_empty_stdout_inside_the_budget` still passes.

### Task 2: The provider config block

- **Files:** crates/verbatim-core/src/config.rs, crates/verbatim-core/tests/config.rs
- **Action:** Add a `[provider]` sub-table to `FileConfig` the way `[injection]`
  was added in phase 5: every key optional, unknown keys ignored under
  `FileConfig`'s documented rule that this file grows across phases, and a
  missing table yielding the defaults. The keys are `enabled` (false by
  default, so judgment is opt-in and off by default per OBS-02), `base_url`,
  `model`, `name` (the provider namespace all three credential tiers key off -
  see task 3), `api_key` (the product-config tier of D-14's precedence, holding
  a value rather than a reference), `local`, and `daily_token_budget` (D-11's
  money-facing knob, the one cost control that is a config key; the minimum
  turn count and the truncation budget stay compile-time constants). Expose
  them through accessors on `Config` beside `brief_chars` and `prompt_chars`.
  `local` is the destination declaration D-13 settles: absent or false means
  remote and therefore filtered, and its doc comment must say the key describes
  the DESTINATION and not the address - a user who sets it on a reverse proxy
  that forwards offsite sends unfiltered data. Parse no URL, inspect no host
  and resolve no name: a DNS lookup is itself a connection PRIV-03 bars, and
  there is no `url` crate in this workspace on purpose. `Config` derives
  `Debug`, and that derive must not render `api_key`: implement `Debug` by hand
  for whatever type holds the key, so the value cannot reach a stream through a
  formatted config.
- **Verify:** `cargo test -p verbatim-core --test config` passes: a
  `verbatim.toml` with no `[provider]` table yields a config that is not
  enabled and has no key; a table naming only `model` leaves the other keys at
  their defaults and `local` false; a table with an unrecognized key inside it
  still parses; a config carrying an `api_key` formats under `{:?}` with no
  byte of the value in the output.

### Task 3: The shared credentials loader and PRIV-02's refusal

- **Files:** crates/verbatim-core/src/credentials.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/tests/credentials.rs
- **Action:** Implement the loader in this repo rather than consuming a
  cross-product library (D-14: no such library exists on this machine, and
  blocking on building one would block every observation). Precedence is
  process env, then the product config's `api_key`, then the shared file at
  `~/.config/jcrenshaw/credentials.toml` on Unix and
  `%APPDATA%\jcrenshaw\credentials.toml` on Windows, namespaced by provider
  name - resolve the shared path the way `config::config_dir` resolves its own,
  through an environment override first so a test never reads the developer's
  real file. The env variable's spelling is derived from the provider name and
  documented in the module's own comment. An absent shared file is the common
  case and not an error: it does not exist on this machine. Enforce PRIV-02's
  permission rule on Unix through `std::os::unix::fs::PermissionsExt` - the
  only permission shape in this workspace, used at
  `crates/verbatim/src/cmd/install/json_file.rs` and in
  `crates/verbatim-core/tests/discover.rs` - refusing the load when any group
  or world bit is set, with an error naming the file and its octal mode and
  nothing else. The Windows arm accepts the file with a caveat rather than
  checking the ACL (D-15); task 6 surfaces that caveat. Return the secret in a
  type whose `Debug` and `Display` render a fixed marker and never the value,
  and whose only accessor is the one the request builder needs, so a value
  cannot reach a stream by being interpolated into a message. This module
  writes nothing, migrates nothing and reads no legacy file.
- **Verify:** `cargo test -p verbatim-core --test credentials` passes on Unix:
  a `0644` credentials file is refused, the error names the file and `644`, and
  the rendered error contains no substring of the key; a `0600` file loads and
  the loaded value formats under `{:?}` and `{}` with no substring of the key;
  an absent file with nothing in the environment yields no credential and no
  error; a value in the environment outranks one in the file, which outranks
  nothing when both are absent.

### Task 4: The destination-keyed egress filter and the error scrubber

- **Files:** crates/verbatim-core/src/observe/egress.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/tests/egress.rs
- **Action:** Two functions over one rule set. The first takes the declared
  `local` flag and the text about to be sent: `local = true` returns it
  unchanged, because a local provider is not egress at all and filtering it
  would destroy detail for nothing; anything else returns it filtered. The
  second scrubs a string that is about to become an error, and it runs
  regardless of destination - D-16 is the half that survives, because
  `Drained::discarded` and `Labeled::notes` travel into `runs.error` and
  `verbatim status` prints that column, so a 401 body carrying a key would be
  durably stored rather than merely scrolled past. Planner's choice of rule
  set, recorded here: both replace any credential value the loader resolved
  this run, plus assignment-shaped and header-shaped secrets in the text -
  `NAME=value` and `"name": "value"` where the name matches
  `token|password|passwd|secret|key|api|bearer`, `Authorization:` header lines,
  and PEM private-key blocks - each with a marker that says what was removed
  rather than deleting it silently, so a reader of a filtered payload can see
  that filtering happened. Redaction is at egress only and never at ingest:
  `.planning/PROJECT.md` bars ingest-time redaction outright and nothing in
  this module may be reachable from the ingest write path.
- **Verify:** `cargo test -p verbatim-core --test egress` passes: a body
  containing an `OPENAI_API_KEY=sk-...` assignment and an `Authorization:
  Bearer ...` line comes back byte-identical under `local = true` and comes
  back with neither secret's value present under `local` false and under
  `local` absent; the error scrubber applied to a synthetic 401 body carrying a
  loaded credential returns a string containing no substring of that value and
  a marker saying something was removed.

### Task 5: The `chat/completions` request and response

- **Files:** crates/verbatim-core/src/observe/provider.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/src/testkit.rs, crates/verbatim-core/tests/provider.rs
- **Action:** One code path serving every OpenAI-compatible endpoint (OBS-05,
  D-05): a POST to the configured base URL's `chat/completions`, a bearer
  authorization header carrying the resolved credential, a body naming the
  configured model and the messages, and `response_format` set to
  `{"type": "json_schema", "strict": true}` (D-09, honored by the 2026-08-21
  ollama probe with `finish_reason` `stop`). Base URL, model and key are the
  only things that change between local and remote - no second branch, no
  Anthropic `x-api-key` or `anthropic-version` header, no Anthropic body shape.
  The request goes out through `observe::net` and through nothing else, and the
  body passes through `observe::egress` on the declared-destination rule before
  it is handed over. Parse the response by reading
  `choices[0].message.content` SPECIFICALLY and treating every other key on the
  message object as ignorable (D-08): the same probe found `message` carrying a
  non-standard `reasoning` key holding roughly 1 KB of chain-of-thought beside
  `content`, and a parser that stringified the message object would feed that
  prose into the JSON parse and burn two calls per session on every thinking
  model. Read the `usage` object's token counts back for the caller, since the
  probe returned one and PLAN-3's budget accumulates it. Every failure - a
  connect error, a non-2xx status, an unreadable body - is returned as this
  module's own error type with its text already through the scrubber, so no
  caller can reach an unscrubbed one. Put the test endpoint in
  `crates/verbatim-core/src/testkit.rs` behind the existing `testkit` feature:
  a hand-rolled `std::net::TcpListener` on a loopback port serving one canned
  HTTP response per connection, with the request it received handed back for
  assertions, and an arm that serves a deliberately unparseable body for
  PLAN-3's OBS-04 test. The project bars mocks - real sockets, the way it uses
  real SQLite temp databases.
- **Verify:** `cargo test -p verbatim-core --test provider` passes: one call
  against the stub produces exactly one HTTP request whose path ends
  `chat/completions`, whose body names the configured model and carries
  `response_format`, and whose authorization header carries the resolved
  credential; a canned response whose `message` carries both `content` and a
  `reasoning` key parses the `content` and ignores the rest; a canned 401 whose
  body echoes the key returns an error containing no substring of it; the
  reported token counts equal the stub's `usage`. AC4's remote half is
  human-verify: with an OpenRouter (or other remote OpenAI-compatible) API key
  in the shared credentials file, change only `base_url`, `model` and the key
  in `verbatim.toml`, run the same call, and observe a parsed response - the
  machine this was planned on has no remote key to prove it with.

### Task 6: `doctor` reports the credentials state

- **Files:** crates/verbatim/src/cmd/doctor.rs, crates/verbatim/tests/doctor.rs
- **Action:** One check, in the shape of the checks already in that file: it
  reports where the credentials file was looked for, whether it is there, and
  what its permissions say. On Unix a group- or world-readable file is a
  problem state with the exact `chmod` command that fixes it, matching
  `doctor`'s rule that it never repairs and always prints the command that
  does. On Windows it is the `Unknown` state carrying D-15's caveat in words:
  the ACL is not checked by this build. An absent file is not a problem - it is
  the ordinary state, and the shared file does not exist on this machine. The
  check never prints, and never puts into the `--json` document, any part of a
  credential value; `doctor` is read-only and creates nothing, which this check
  must not change.
- **Verify:** `cargo test -p verbatim --test doctor` passes: with a `0600`
  credentials file in a test-owned config location the check reports the good
  state and exit stays 0; with `0644` on Unix it reports a problem, prints a
  fix command, and `verbatim doctor` exits 1; with no file at all it reports
  neither a problem nor a failure; in every one of the three cases neither
  stdout nor stderr nor the `--json` document contains a substring of the key
  written into the file.

## Notes

- Flagged from CONTEXT and unresolved here: OpenRouter's support for
  `response_format: {"type":"json_schema"}` and its exact auth header shape are
  unverified - there is no remote key on this machine. If task 5's human-verify
  fails against a remote endpoint, D-09's stated fallback is tool-calling, and
  that is a second request-shaping branch inside the single code path OBS-05
  promises. Report it rather than adding the branch speculatively.
- Also flagged: `ureq` 3.4 + rustls meeting the root `Cargo.toml` dependency
  bar is likely but unmeasured. Task 1 measures it. If the transitive tree
  breaches the bar, that is a finding for the human, not a licence to pick a
  different client mid-task.
- OBS-05's Anthropic subscription auth half is deferred out of this phase and
  the requirement row needs a note saying phase 7 delivered the
  OpenAI-compatible half only.
