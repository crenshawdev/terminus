---
phase: 4
plan: 2
requirements:
  - INST-02
  - INST-03
  - INST-04
  - INST-05
  - INST-08
files:
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/install/mod.rs
  - crates/verbatim/src/cmd/install/binary.rs
  - crates/verbatim/src/cmd/install/json_file.rs
  - crates/verbatim/src/cmd/install/targets.rs
  - crates/verbatim/tests/install.rs
---

# Phase 4: Hooks And Install - Plan 2 of 5 (install)

**SEQUENTIAL: PLAN-1 -> PLAN-2 -> PLAN-3 -> PLAN-4.** This plan writes the hook
entries that name PLAN-1's event constant, and PLAN-3's `doctor` and `uninstall`
read the target resolution and the JSON editor this plan creates. Do not run it
before PLAN-1 or in parallel with PLAN-3 or PLAN-4.

## Goal

One command wires verbatim into Claude Code: the binary lands at a path that
never moves, both of Claude Code's settings files gain exactly the entries
verbatim owns and nothing else changes in them, and running it again - or after
an upgrade - changes nothing.

## Must be true when done

- Running `verbatim install` twice leaves exactly one hook entry per event in
  `settings.json` and exactly one `verbatim` entry under `.mcpServers`, with one
  backup of each file written on the first run only, and every other key in both
  files carrying its original value in its original position (AC4).
- Replacing the binary and rerunning install changes no byte of the `hooks`
  object (AC5, INST-05).
- Install against a stable path holding a binary that is not a verbatim build
  exits non-zero, writes nothing to either settings file, and prints a command
  that makes the same install succeed (AC5).
- `verbatim install --yes` completes with no prompt and no TTY; without `--yes`
  and without a TTY it refuses rather than assuming (INST-08).
- Install shows the exact diff for each file it will change and asks once before
  writing either (INST-03).
- Install reports a low `cleanupPeriodDays` and the auto-compact recommendation
  and changes neither setting itself (INST-04).

## Context

- D-05: two files. Hook entries go in `settings.json` under `.hooks`, the MCP
  registration goes in `.claude.json` under `.mcpServers`. `settings.json` has no
  `mcpServers` key and the 2.1.231 settings schema carries every MCP key except
  that one.
- Where those two files are is Claude Code's own rule, read out of the 2.1.231
  bundle's `Hzf`: `settings.json` is at `$CLAUDE_CONFIG_DIR/settings.json` and
  falls back to `$HOME/.claude/settings.json`; `.claude.json` is at
  `$CLAUDE_CONFIG_DIR/.claude.json` and falls back to `$HOME/.claude.json`.
  `verbatim.toml`'s `roots` does not decide this - it says which transcript trees
  to walk, not where Claude Code keeps its settings.
- D-06: install merges `.claude.json` itself and never shells out to
  `claude mcp add`. D-17: it must tolerate Claude Code writing the same file
  concurrently.
- D-20: confirmation, `--yes` and non-TTY handling are entirely new code -
  `grep -rn "stdin()"` across `crates/*/src/` returns one hit and
  `IsTerminal|is_terminal|atty` returns nothing.
- Out of scope: `doctor`, `uninstall` (PLAN-3), the backfill install kicks off
  (PLAN-4, which edits `cmd/install/mod.rs` after this plan lands), and anything
  under `npm/` (PLAN-5).

## Tasks

### Task 1: `verbatim install` places the binary at the stable path

- **Files:** crates/verbatim/src/cmd/install/mod.rs,
  crates/verbatim/src/cmd/install/binary.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs
- **Action:** Add an `install` subcommand whose first act is to put this build at
  the canonical stable path: `~/.local/bin/verbatim` on Linux and macOS,
  `%LOCALAPPDATA%\Programs\Verbatim\verbatim.exe` on Windows (D-08), creating the
  directory if it is missing and never touching PATH (INST-02). The source is
  `std::env::current_exe()` (D-09) - the npm shim execs the per-platform binary,
  so the running process is the copy source and install needs no knowledge of npm
  layout. Support a `VERBATIM_BIN_DIR` override for the directory, spelled after
  the existing `VERBATIM_DATA_DIR` and `VERBATIM_CONFIG_DIR`, so a test never
  writes into the developer's real `~/.local/bin`.
  The copy carries a marker (D-07). Embed a fixed ASCII sentinel in the binary
  image - unique enough not to collide, and referenced from code so it survives
  the release profile's `strip = "debuginfo"` - and, before writing, scan the
  file already at the stable path for it, streaming the read in bounded chunks
  with an overlap rather than loading the whole executable. If a file is there and
  does not carry the sentinel, exit non-zero having written nothing anywhere, and
  print the exact command that overwrites it. This is not hypothetical: as of
  2026-08-13 `/home/john/.local/bin/verbatim` is a 12,076,464-byte different
  program whose `--version` prints `verbatim 0.1.0`, byte-identical to this
  build's output (`crates/verbatim/src/main.rs:64` prints `CARGO_PKG_VERSION`), so
  version alone cannot tell the two apart.
  Write through a temporary file in the destination directory and rename into
  place, on every platform: that is atomic, and on Windows it is also the only way
  to replace an executable that may be running. Add `install` to `USAGE` and to
  `dispatch` in `main.rs`.
- **Verify:** In `crates/verbatim/tests/install.rs`, with `VERBATIM_BIN_DIR` at a
  temp directory: `verbatim install --yes` exits 0 and leaves an executable file
  at the stable path whose `--version` matches the running build's; a second run
  over that file succeeds; writing `/bin/true`'s bytes there and rerunning exits
  non-zero, leaves those bytes untouched, and prints a command string containing
  the stable path.

### Task 2: A settings file editor that changes only what it was asked to

- **Files:** crates/verbatim/src/cmd/install/json_file.rs
- **Action:** One module that reads, edits and writes a Claude Code settings file,
  used for both targets and later by `uninstall` and `doctor`. It must preserve
  every key it did not touch, with its value and its position: `serde_json`'s
  default `Map` is a `BTreeMap`, so a naive round trip re-sorts 29 top-level keys
  in `settings.json` and 240 KB of `projects` entries and caches in
  `.claude.json`. Do not enable `serde_json`'s `preserve_order` feature to fix
  this - it is a workspace-wide feature that would silently change the key order
  of every shipped `--json` document from the six commands phase 3 documented in
  `docs/json-shapes.md`. Instead deserialize into an insertion-ordered
  representation of your own through serde's `MapAccess`, holding pairs in a
  `Vec`, and re-emit with two-space indentation. Both real files are two-space
  indented; `settings.json` ends with a newline and `.claude.json` ends with `}`,
  so preserve whether the file it read ended with one.
  Writing is: read the file immediately before writing (D-06 - Claude Code
  rewrote `.claude.json` five times inside six minutes on 2026-08-13), write a
  temporary file in the same directory as the target, then rename into place.
  Resolve symlinks before choosing that directory and before renaming: on this
  machine `~/.claude.json` is a symlink to `/claude/.claude.json` to
  `/data/claude/.claude.json`, and renaming over the link would replace the user's
  symlink with a regular file and orphan the real one.
  A file that does not exist is created; a file that exists and is not valid JSON
  is an operational failure that writes nothing, never a file to overwrite.
  Provide the backup as part of this module: a whole-file copy to a
  verbatim-specific sibling name (not `.claude.json.backup`, which Claude Code
  writes itself), made once, never overwritten if it already exists, so a second
  install does not back up the file it just wrote. Provide a rendered diff of the
  pending change - the lines added and removed, not the whole file - which Task 5
  shows before confirming.
- **Verify:** `cargo test -p verbatim` includes unit tests over a fixture file
  carrying nested objects, unicode, and a duplicate-free key set: adding one key
  and writing leaves `jq -c 'del(.<addedkey>)'` byte-equal to the original and
  `jq -r 'keys_unsorted|join(",")'` unchanged apart from the added key; writing
  through a symlinked path leaves the symlink a symlink and updates its target;
  a second backup call over an existing backup leaves the first backup's bytes
  unchanged; an invalid-JSON target returns an operational failure and leaves no
  temporary file behind.

### Task 3: The four hook entries, in exec form

- **Files:** crates/verbatim/src/cmd/install/targets.rs,
  crates/verbatim/src/cmd/install/mod.rs
- **Action:** Write one hook entry per event into `settings.json` under `.hooks`,
  for exactly the four events PLAN-1's `cmd::hook` constant names, in exec form
  (D-01): `{"type":"command","command":"<stable path>","args":["hook","<event>"]}`.
  Exec form is the only shape satisfying ING-10's "no shell" on every platform -
  the 2.1.231 schema documents `args` as "Argument list for exec form. When
  present, `command` is resolved as an executable and spawned directly with these
  arguments - no shell", and the same bundle carries the "requires bash but Git
  Bash was not found" error the shell form produces. A shell-form entry also
  breaks outright on a home directory containing a space.
  Merge, never replace. As measured on 2026-08-13, `.hooks` already holds seven
  event keys on this machine, `.hooks.SessionStart` is `[]` and
  `.hooks.UserPromptSubmit` is a one-element array of
  `{"matcher":"","hooks":[...]}` carrying the user's own script. So: ensure the
  event key exists as an array, append one group of the same shape with
  `matcher: ""`, and leave every group already there untouched.
  Idempotency and upgrade are the same rule (D-13, INST-05): an entry is
  verbatim's when its `command` equals the stable path. If one is already present
  for an event, write nothing for that event - not a replacement, not a
  reordering. Because the stable path never changes (D-08), an upgrade that
  replaced the binary finds all four present and rewrites no byte of `hooks`,
  which is the version-skew class this design exists to retire.
- **Verify:** In `crates/verbatim/tests/install.rs`, with `CLAUDE_CONFIG_DIR` at a
  temp directory seeded with a `settings.json` carrying an unrelated
  `UserPromptSubmit` group and several top-level keys: `verbatim install --yes`
  twice leaves `jq '[.hooks[][] | .hooks[] | select(.command==$path)] | length'`
  equal to 4, leaves the pre-existing group present and unmodified,
  leaves `jq -c 'del(.hooks)'` byte-equal to the seeded file, and writes exactly
  one backup; capturing `jq -c .hooks` after the first run and after a third run
  made with a rebuilt binary shows the two strings equal.

### Task 4: The MCP registration in `.claude.json`

- **Files:** crates/verbatim/src/cmd/install/targets.rs,
  crates/verbatim/src/cmd/install/mod.rs
- **Action:** Write one `verbatim` entry under `.mcpServers` in `.claude.json`
  through Task 2's editor: a stdio server invoking the stable path with the `mcp`
  argument, which is the command `crates/verbatim/src/cmd/mcp/mod.rs` already
  serves. This file, not `settings.json` (D-05) - `settings.json` has no
  `mcpServers` key, and an `mcpServers` block written there would be ignored by
  Claude Code while `doctor` reported MCP registered and recall stayed unreachable
  from inside a session.
  Merge and back up exactly as Task 3 does, keyed on the `verbatim` key: an entry
  already present is left alone, and `context7` or any other server already
  registered is untouched. Treat this file as the durable home for `.mcpServers`,
  to be re-checked at 1.0 (D-19) - the 2.1.231 bundle carries migration strings
  for other state in that file but not for `mcpServers`.
  Never shell out to `claude mcp add -s user` (D-06): install must not depend on
  `claude` being on PATH, from a binary whose stated contract is no shell and no
  subprocess, and the file is 240 KB of the user's project history and caches to
  lose if that call misbehaves.
- **Verify:** In `crates/verbatim/tests/install.rs`, against a temp
  `.claude.json` seeded with a `context7` entry under `.mcpServers` and a
  multi-key `projects` object: `verbatim install --yes` twice leaves
  `jq '.mcpServers | keys_unsorted'` equal to `["context7","verbatim"]`, leaves
  `jq -c 'del(.mcpServers.verbatim)'` byte-equal to the seeded file, and leaves
  one backup; `jq -r '.mcpServers.verbatim.command'` equals the stable path.

### Task 5: The diff, the one confirmation, and `--yes`

- **Files:** crates/verbatim/src/cmd/install/mod.rs
- **Action:** Show the rendered diff for each file install will change and ask
  once, before either file is written (INST-03) - not once per file, and never
  after a write. `--yes` accepts every default and prompts for nothing
  (INST-08). With no `--yes` and no TTY on stdin, refuse with a message naming
  `--yes` rather than assuming an answer: a scripted install that silently took
  defaults it never showed anyone is the failure this rule prevents. All of this
  is new code (D-20); `std::io::IsTerminal` is the whole of the TTY test and needs
  no dependency.
  Nothing is written before the answer, and the binary copy of Task 1 counts as a
  write: order the command so the refusable checks and the diff rendering all
  happen first, then one confirmation, then every mutation. A declined
  confirmation exits 0 having changed nothing and says so.
  `install` stays human-only with no `--json` (D-24): an interactive confirmation
  and a single JSON document on stdout contradict each other, and no INST
  requirement asks for one.
- **Verify:** `verbatim install` with stdin closed and no `--yes` exits non-zero,
  prints a message naming `--yes`, and leaves both settings files and the stable
  path untouched; `verbatim install --yes` with stdin closed exits 0 and writes
  all three; a run whose confirmation is answered `n` exits 0 and leaves all
  three untouched; `verbatim install --json` exits 2.

### Task 6: The advisories install gives and the settings it never changes

- **Files:** crates/verbatim/src/cmd/install/mod.rs
- **Action:** After writing, report and recommend, never change (INST-04). Read
  the effective `cleanupPeriodDays` from `settings.json`; when it is low, explain
  that Claude Code's transcripts are verbatim's second recovery path and offer to
  raise it - an offer that is a separate answer, defaulting to no change, and
  accepted by `--yes` only as the default it prints. Print the auto-compact
  recommendation as one line naming `autoCompactEnabled`, and do not set it:
  compaction burns tokens summarizing context this store already holds losslessly.
  Then print the closing summary: which config roots resolved and how many
  (`Config::roots` already resolves them, and more than one is worth naming
  because only the first got the hooks), what changed in each file, where the
  backups are, where the data directory is, and how to undo it. Say that the hook
  entries are live in an already-running Claude Code without a restart (D-17) -
  that is measured behaviour on the versions on this machine and not a documented
  contract, so word it as observed rather than guaranteed.
- **Verify:** `verbatim install --yes` against a temp `settings.json` carrying
  `"cleanupPeriodDays": 7` prints a line naming `cleanupPeriodDays` and a line
  naming `autoCompactEnabled`, and `jq -c '{cleanupPeriodDays,autoCompactEnabled}'`
  on that file is byte-identical before and after; the summary's printed data
  directory equals `verbatim status`'s `store` path's parent, and its printed
  backup paths exist.

## Notes

- Task 2 is the load-bearing one: AC4's "every other key byte-identical" and
  AC7's "restores it to its pre-install bytes" both rest on it, and the 240 KB
  `.claude.json` is the file with the most to lose. It is worth writing its unit
  tests before the two callers exist.
- The `VERBATIM_BIN_DIR` override in Task 1 is new surface introduced for
  testability. It follows the shape of `VERBATIM_DATA_DIR` and
  `VERBATIM_CONFIG_DIR` and PLAN-3's `doctor` and `uninstall` must honour the same
  variable, or their tests will reach the developer's real `~/.local/bin`.
- Flagged for the human, not planned: D-17's mid-session hot reload is not a
  documented Claude Code contract. If it turns out to be false, the summary line
  Task 6 prints is wrong and users will read a working install as a failed one.
