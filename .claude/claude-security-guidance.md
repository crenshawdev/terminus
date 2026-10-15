# Terminus security guidance

The full threat model is `THREAT-MODEL.md` at the repo root. This is the short version for reviewing a diff.

## The threat model in brief

Terminus is a Rust CLI that copies Claude Code transcripts into a SQLite archive and hands archived text back to Claude Code. Claude Code starts it as a hook on `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`, and as a stdio MCP server with `recall_search`, `recall_context` and `recall_get`. The user also runs it by hand. It runs as the interactive user. It opens no network connection, calls no model and holds no API key.

Untrusted input:

- Transcript content, under `~/.claude/projects`, `CLAUDE_CONFIG_DIR` or the configured `roots`. It carries web pages, file contents and tool output Claude saw. Treat every field as attacker-controlled, including top-level ones like `cwd` and `gitBranch`. Files in that tree may be symlinks.
- MCP tool arguments. The model chooses them, so a prompt injection can choose them.
- Hook stdin. It comes from Claude Code but nothing authenticates it.
- The store, `injection/` and `decisions/` files and the `data-location` pointer. Any process running as the same user can edit them.

The worst outcome: archived text from one session acts as instructions in a later session, or pulls one project's secrets into another project's context. Next worst: losing or silently corrupting the archive, which can be the only copy once Claude Code deletes old transcripts.

## Invariants

A diff that breaks one of these is high severity.

1. No network. No socket types, no HTTP client, no dependency that brings one in. `crates/terminus-core/tests/no_network.rs` enforces this.
2. No model calls, and no reading an API key.
3. Nothing under the Claude projects tree is ever written, moved or deleted. Transcripts are opened read-only.
4. A project listed in `exclude` is never opened and never returned. That holds for search, context, get, the brief, the per-prompt injection, export and retention.
5. Hooks never block a prompt. Every known event exits 0, and the only output is `hookSpecificOutput.additionalContext`, built with serde_json.
6. Archived blocks never change after they are written, and `terminus verify` detects a mismatch.
7. With `[privacy] redact_recall` on, every path that hands text to Claude Code goes through `core/observe/egress.rs`.
8. `install.sh` verifies the manifest signature against the pinned key before it parses the manifest or downloads the binary, then checks the binary's SHA-256 before installing.

## Review checklist

Flag any of these in a diff:

- **SQL.** SQL text built with `format!` or `push_str` from anything other than a constant identifier or `?` placeholders. Every value must be bound.
- **FTS5.** A `MATCH` expression not produced by `Query::parse` and `match_expression` in `core/recall/query.rs`, which split on non-alphanumerics and double-quote each token.
- **Subprocesses.**
  - Any new `std::process::Command`.
  - Any shell (`sh -c`).
  - A program found by bare name instead of the absolute path from the `PATH` scan.
  - Any argument, environment value or working directory taken from transcript, MCP or hook data.
  - Today the only one is `git rev-parse --show-toplevel` in the transcript's `cwd`. Flag anything that widens it.
- **Untrusted paths.**
  - A file opened, created or deleted at a path that came from transcript content, hook stdin (`transcript_path`, `cwd`) or MCP arguments.
  - A `session_id` used in a file name without the hex-and-hyphen allow-list (8 to 64 characters).
- **File modes.** A data-directory file created without `core/owner_only.rs` (0700 directories, 0600 files, set in the creating call). Any use of `std::fs::write` or default `OpenOptions` for a file that holds archive data, prompts or settings.
- **Unbounded reads.** A new `read_to_end`, `read_until` or `fs::read` on transcripts, sidecar `.meta.json` files, stdin or MCP input with no size cap. The existing uncapped reads are listed in `THREAT-MODEL.md`; do not add more.
- **Deletion.** `remove_dir_all` or `remove_file` on a path from an environment variable, config or the pointer file, without proving the target belongs to Terminus.
- **Text reaching the model.**
  - Archived text that reaches `additionalContext` or an MCP result without the redaction filter when `redact_recall` is on.
  - A new field emitted into Claude's context unfiltered.
- **More archived text reaching the model.** A new hook output, a new MCP tool, a larger character budget, a wider scope (`project: "*"`, cross-project reads), or new record types eligible for automatic injection.
- **Exclusion drift.** A change to exclusion matching that lets the ingest check (encoded directory name) and the read check (stored project key) disagree.
- **Claude Code's settings.** A change to `settings.json` or `.claude.json` handling that:
  - drops the user's other keys;
  - writes non-atomically;
  - leaves a file wider than the one it replaces;
  - removes entries Terminus did not create.
- **Panics.** `panic!`, `unwrap`, `expect`, `assert!` or indexing reachable from transcript content on the hook or ingest path. One bad file must not stop ingest of the rest.
- **Terminal output.** Archived bytes written to a terminal (`show`, `search`) with no handling of control characters.
- **Release.** A change to `install.sh` or `scripts/sign-release.sh` that:
  - relaxes the pinned key;
  - reorders verify and download;
  - allows non-https sources;
  - adds sudo;
  - signs anywhere but the maintainer's machine.
- **Dependencies.** A new dependency that brings a network stack, an async runtime, or a build script that downloads.

## Known gaps, not yet fixed

These are recorded in `THREAT-MODEL.md`. Report any diff that widens one.

- Re-injected archived text carries no marker saying it is data, not instructions.
- `recall_search` with `project: "*"` reads every project that is not excluded.
- Symlinked transcript files are followed to any target. `terminus ingest <path>` accepts any path.
- MCP stdin, transcript lines and `recall_get` bodies have no size limit.
- Reads of the archive check no block hashes; only `verify` does.
- `data move` removes the old data directory without checking that it holds `terminus.db`.
