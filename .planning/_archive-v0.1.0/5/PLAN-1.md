---
phase: 5
plan: 1
requirements:
  - INJ-01
  - INJ-06
files:
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/tests/config.rs
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/inject/mod.rs
  - crates/verbatim-core/src/inject/brief.rs
  - crates/verbatim-core/src/inject/prompt.rs
  - crates/verbatim/src/cmd/hook.rs
  - crates/verbatim/tests/inject.rs
  - tests/fixtures/hooks/session-start-compact.json
  - tests/fixtures/README.md
---

# Phase 5: Context Injection - Plan 1 of 4 (the seam)

**ORDER: PLAN-1 first, then PLAN-2 and PLAN-3 (which share no file and may run
in parallel), then PLAN-4.** This plan writes the config keys, the payload
parse, the stdout object and the deadline that all three of the others sit on,
and it creates `crates/verbatim-core/src/inject/brief.rs` and
`crates/verbatim-core/src/inject/prompt.rs` so PLAN-2 and PLAN-3 each deepen one
of them without touching the other or the seam.

## Goal

A hook event that could carry context reaches a real read of the archive and
back out to Claude Code's stdout in the shape the harness accepts, and every way
that read can fail - no store, a locked store, a corrupt store, a slow one -
comes out as silence and exit 0 inside a deadline.

## Must be true when done

- `verbatim hook SessionStart`, against a data directory holding indexed history
  for the project the payload's `cwd` names, writes exactly one line to stdout
  that parses as JSON, whose `hookSpecificOutput.hookEventName` is `SessionStart`
  and whose `hookSpecificOutput.additionalContext` names how many sessions and
  turns are indexed for that project.
- `verbatim hook SessionEnd` and `verbatim hook PostCompact` write nothing to
  stdout on every path, against every store.
- Deleting the store file, holding the store with an exclusive writer past the
  deadline, or pointing the data directory at a file that is not a database each
  leaves `verbatim hook UserPromptSubmit` exiting 0 with empty stdout inside the
  deadline (AC6).
- `verbatim.toml` carries an injection block whose budgets the injection paths
  read, and a config file without one still loads to documented defaults.
- The three `assert!(output.stdout.is_empty())` sites in
  `crates/verbatim/tests/hook.rs` still pass, unedited (D-11).
- The repository holds the payload Claude Code actually sends when a compaction
  happens, and the `source` it carries is written down where PLAN-4 reads it.

## Context

- D-01 binds the output: one JSON object, `hookSpecificOutput.hookEventName`
  equal to the event that fired, the text in
  `hookSpecificOutput.additionalContext`. Only `SessionStart` and
  `UserPromptSubmit` have such a variant; the dispatcher throws
  `Hook returned incorrect event name` on a mismatch, and a stdout starting with
  `{` that fails schema validation becomes a `hook_non_blocking_error` banner
  shown to the user.
- D-02 binds the input: everything arrives through the stdin payload,
  `verbatim.toml` and the store. The hook argv stays `["hook", "<event>"]` and
  gains no flag, because `crates/verbatim/src/cmd/install/targets.rs` writes
  those entries once and never rewrites them (INST-05).
- D-03 binds the failure path: a short busy timeout on an injection-specific
  read-only open, plus a watchdog that abandons the work and exits 0 at the
  deadline. Backfill was measured holding the store for 49 s.
- D-12 binds the scope: the payload's `cwd`, never `std::env::current_dir()`.
- D-09 binds the counts: the project-only projection plus a targeted count,
  never `config::visible::sessions`, which costs 6.2-6.8 ms against 0.56-0.58 ms
  on a store shaped like the real one.
- The binary crate depends on `serde_json` and not on `rusqlite`
  (`crates/verbatim/Cargo.toml`), so every store query in this phase lives in
  `verbatim-core` and the binary keeps stdin, stdout, threads and exit codes.
- Out of scope here: the last-session block and the budget (PLAN-2), any
  retrieval at all (PLAN-3), the per-session state file and the compaction
  complement (PLAN-4).

## Tasks

### Task 1: Injection settings in `verbatim.toml`

- **Files:** crates/verbatim-core/src/config.rs (`FileConfig`, `Config`,
  `Config::resolve`, `Config::from_parts`), crates/verbatim-core/tests/config.rs
- **Action:** D-18: the injection budgets are new keys in `verbatim.toml`, not
  hardcoded constants, because INJ-01 says "configured". Add an optional
  `[injection]` table to `FileConfig` and expose its resolved values on `Config`
  as two budgets: `brief_chars` for the resume brief and `prompt_chars` for the
  prompt injection. Characters, not tokens (D-16): the workspace has eight
  dependencies, each justified in the root `Cargo.toml` against a measured
  0.408 ms startup floor, and none is a tokenizer; the existing budgets are
  already byte-shaped (`recall::EXCERPT_CHARS`, `index::MAX_BODY_BYTES`).
  Defaults when the table or a key is absent: 6000 for the brief and 4000 for
  the prompt, both chosen as roughly 1-2k tokens at four characters a token and
  both under the 10,000-character ceiling past which bundle 2.1.237 persists a
  hook's stdout to disk and replaces it with a reference. Keep the documented
  rule that unknown keys are ignored rather than rejected - this file grows
  every phase - and keep `Config::from_parts`'s existing signature, filling the
  defaults, so no test anywhere else in the workspace has to change.
- **Verify:** `cargo test -p verbatim-core --test config` passes with new cases
  showing a `verbatim.toml` carrying `[injection]` resolving to the numbers it
  names, a file with no `[injection]` table resolving to 6000 and 4000, a file
  with only one of the two keys resolving the other to its default, and an
  unrecognized key inside `[injection]` still parsing; `cargo test --workspace`
  shows no other test needing an edit.

### Task 2: The hook reads its payload instead of dropping it

- **Files:** crates/verbatim/src/cmd/hook.rs (`run`, `drain`, `MAX_PAYLOAD`,
  `DRAIN_DEADLINE`)
- **Action:** the drained line stops being discarded and becomes the five fields
  injection needs: `session_id`, `transcript_path`, `cwd`, `prompt` and
  `source`. Read them off a `serde_json::Value` rather than a derived struct -
  the binary crate depends on `serde_json` and not on `serde`'s derive, and the
  hook path is the startup floor the whole architecture is shaped around. Every
  field is optional at the type level: a payload that omits one is a payload
  that gets less injection, never an error. Nothing else about the read changes:
  the ingest still spawns before stdin is touched, the read still happens on a
  thread that is never joined, `MAX_PAYLOAD` still bounds the bytes and
  `DRAIN_DEADLINE` still bounds the wait, and a stdin that is not JSON, not an
  object, or never closed is still one line on stderr and still exit 0 with
  empty stdout. The argv stays `["hook", "<event>"]` and gains no flag (D-02):
  a new argument would mean rewriting every user's `settings.json` on upgrade,
  which is the version-skew class INST-03 and phase 4 AC5 exist to eliminate.
- **Verify:** `cargo test -p verbatim --test hook` passes unedited, and a
  `#[cfg(test)]` module in `cmd/hook.rs` reading each of the four
  `tests/fixtures/hooks/*.json` payloads through
  `verbatim_core::testkit::fixture_bytes` shows the extracted fields matching
  the fixture's own values - including `source` `"resume"` on the SessionStart
  payload and the `prompt` text on the UserPromptSubmit one - and shows a
  payload that is not JSON yielding a value with every field absent rather than
  a panic.

### Task 3: One JSON object on stdout, on the two events that have one

- **Files:** crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/src/inject/mod.rs,
  crates/verbatim-core/src/inject/brief.rs,
  crates/verbatim-core/src/inject/prompt.rs,
  crates/verbatim/src/cmd/hook.rs, crates/verbatim/tests/inject.rs
- **Action:** add an `inject` module to `verbatim-core` holding every store
  query and every rendered string this phase produces, with one entry point per
  injecting event: the `SessionStart` one in `brief.rs` and the
  `UserPromptSubmit` one in `prompt.rs`. Each takes the data directory, the
  loaded `Config` and all five payload fields - even the ones this task's
  implementation ignores - and returns the text to inject or nothing, so PLAN-2
  and PLAN-3 deepen one arm each without touching this seam or the binary. The
  module opens the store with `Store::open_read_only` and never `Store::open`,
  which would `create_dir_all`, initialize a fresh database, run the column
  bring-forward and set two pragmas: a hook must not leave a store behind as the
  side effect of a question. Scope resolves from the payload's `cwd` through
  `Scope::Directory` and `recall::scope::resolve` (D-12), never
  `Scope::current_directory`, because the working directory Claude Code chose
  for a hook may be a plugin root or a worktree and would scope to the wrong
  project or to `Reason::UnknownProject`. In the binary, when an arm returns
  text, `cmd::hook` writes exactly one JSON object to stdout, built with
  `serde_json` and never with `format!` (the D-25 rule `cmd::json` states),
  carrying `hookSpecificOutput.hookEventName` equal to the event that fired and
  `hookSpecificOutput.additionalContext` carrying the text; when an arm returns
  nothing, or when the event is `SessionEnd` or `PostCompact`, it writes nothing
  at all - neither has an `additionalContext` variant, and an object the harness
  cannot validate is a red banner on every session start rather than silence.
  Text that is empty or whitespace is nothing, not an empty `additionalContext`.
  The brief in this task renders only INJ-01's index pointer - the sessions and
  turns indexed for the scoped project, and the fact that search tools exist -
  counted through `config::visible::projects` (which `scope::resolve` already
  runs) plus a targeted `count(*)` over `session_meta` and over `turns` joined
  to it on `session_key` and filtered by `session_meta.project`, which
  `idx_session_meta_project` covers; never `config::visible::sessions`, whose
  per-session `count(*)` over `turns` costs the whole budget before the first
  useful row (D-09). The prompt arm returns nothing in this task; PLAN-3 fills
  it. Failure containment beyond what `Store::open_read_only` already gives -
  `Error::StoreNotFound` and `Error::StoreUnreadable` are values, not panics -
  is task 4's.
- **Verify:** a new `crates/verbatim/tests/inject.rs` that seeds a temp data
  directory by ingesting `session-recall.jsonl` through
  `testkit::copy_rooted_fixture_into` under a test-owned root, then spawns
  `verbatim hook SessionStart` with a payload whose `cwd` is that root's
  `project-alpha`, shows exactly one line on stdout that parses as JSON, whose
  `hookSpecificOutput.hookEventName` is `SessionStart` and whose
  `additionalContext` names the session and turn counts the store holds; the
  same store fed `SessionEnd` and `PostCompact` writes nothing; a payload whose
  `cwd` names a directory no archived project covers writes nothing; and
  `cargo test -p verbatim --test hook` still passes.

### Task 4: Every way injection can fail is silence, inside the deadline

- **Files:** crates/verbatim-core/src/inject/mod.rs,
  crates/verbatim/src/cmd/hook.rs, crates/verbatim/tests/inject.rs
- **Action:** INJ-06 and D-03. Two mechanisms, because they catch different
  failures. In the binary, the injection call runs on a thread the hook starts
  and never joins - the same shape and the same reason as `drain`, since a
  blocking SQLite call cannot be cancelled with `std` alone - and the hook waits
  on a channel for at most a named deadline, after which it writes nothing,
  says one line on stderr and exits 0 with the work abandoned. Choose 50 ms and
  say why in the constant's doc: it is five times the p99 budget
  `crates/verbatim/tests/hook.rs` asserts and a hundredth of the five-second
  wait a default busy handler would impose. In core, immediately after
  `Store::open_read_only` returns, lower that connection's busy timeout from the
  5 s `store::open::BUSY_TIMEOUT_MS` sets to a few milliseconds, so a prompt
  submitted while a backfill holds the store gets an immediate `SQLITE_BUSY` -
  which the phase 3 D-10 path already renders as an empty result - rather than a
  five-second block on exactly the machine state that provoked it. Every outcome
  that is not text is silence: a missing store, an unreadable store, a busy
  store, a store older than this build, a query error, and a panic inside the
  injection work, which must be caught so a panicking arm is one silent prompt
  and not a wedged hook (phase 4's `977d0b4` is the precedent). Nothing on this
  path may write, create a directory or take the ingest lock.
- **Verify:** `cargo test -p verbatim --test inject` passes with three cases
  proving AC6 - the store file deleted, the store held past the deadline by a
  second connection inside a `BEGIN EXCLUSIVE` transaction, and a
  `verbatim.db` whose bytes are not a database - each showing
  `verbatim hook UserPromptSubmit` exiting 0, writing nothing to stdout, and
  returning inside the deadline plus a margin for process startup; plus a core
  test showing the injection entry point returning nothing rather than an error
  for each of those three stores.

### Task 5: What a real compaction actually sends

- **Files:** tests/fixtures/hooks/session-start-compact.json,
  tests/fixtures/README.md, crates/verbatim/src/cmd/hook.rs
- **Action:** D-08 builds INJ-05's trigger on a `SessionStart` whose `source` is
  `"compact"`. The enum `"startup","resume","clear","compact","fork"` is
  verified present in bundle 2.1.237 and `SessionStart`'s hook matcher key is
  `n.source`, but the emit site was never located, so this is the one fact
  PLAN-4 is built on that nothing in the repository has yet observed. Capture
  the payload Claude Code writes when a compaction happens in a live session,
  save the line as a fifth hook fixture beside the four `tests/fixtures/hooks/`
  payloads, and record what it carries - the `hook_event_name` and the `source`
  - in the doc comment beside the field `cmd::hook` reads `source` off. Replace
  the captured `session_id`, `transcript_path` and `cwd` with the same synthetic
  values the other four fixtures carry and leave every other field exactly as
  written: this repository is public OSS and real transcripts never enter it.
  If a compaction fires no `SessionStart` at all, record that instead, in the
  same place and in the same words - PLAN-4 then takes D-08's named fallback,
  reading the store's boundary row at `UserPromptSubmit` and accepting the
  ingest race. Give the new fixture a row in `tests/fixtures/README.md`'s
  `hooks/` section saying which of the two it is.
- **Verify:** human-verify. With a temporary `SessionStart` hook entry in
  `~/.claude/settings.json` whose command appends its stdin to a file, start a
  Claude Code session, run `/compact`, and show the captured line; the fixture
  is those bytes with the three identity fields replaced, and the doc comment
  states the observed `hook_event_name` and `source`. Remove the temporary entry
  afterwards, showing the file's contents before and after the edit.

## Notes

- **A hazard D-11's reasoning does not cover.** D-11 says the three
  `assert!(output.stdout.is_empty())` sites in `crates/verbatim/tests/hook.rs`
  stay true because each runs against a temp data dir whose only project key can
  never be the fixture payload's `cwd`. That is true of two of them and true for
  a different reason of the third:
  `reading_the_hooks_stdout_to_eof_returns_at_once_while_the_ingest_still_runs`
  ingests `session-basic.jsonl`, whose every record carries
  `"cwd":"/data/code/verbatim"` - exactly the `cwd` the four hook payloads
  carry - and `project::Resolver` degrades a `cwd` that is not on disk to the
  `cwd` string itself, so that store's project key IS the payload's `cwd` on any
  machine where `/data/code/verbatim` does not exist. What keeps that test
  passing is timing: the pass it starts takes roughly 700 ms and commits at the
  end of the file, while the hook returns in about a millisecond, so no session
  is committed when the brief runs. Confirm that still holds after task 3 rather
  than assuming it; the same fact is why every test in this phase seeds its store
  through `testkit::copy_rooted_fixture_into` under a test-owned root, the way
  `FIXTURE_ROOT_TOKEN`'s doc comment asks.
- The plan-shape directive asked for multiple plans and this phase has four,
  but only PLAN-2 and PLAN-3 are independent of each other. The remaining
  ordering is real: every plan needs this seam, and PLAN-4 edits both of the
  arms PLAN-2 and PLAN-3 deepen.
- Task 5 needs a live Claude Code session and cannot be run by the executor.
  If it is still unanswered when PLAN-4 starts, PLAN-4's task 4 says what to do.
