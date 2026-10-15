# Threat model: Terminus

The sentinel audit reads this file before every review of Terminus. Everything here comes from reading the code at the commit below. A claim marked [VERIFY] has not been confirmed and needs checking before a finding depends on it.

Paths below are relative to the repo root. `core/` is `crates/terminus-core/src/` and `bin/` is `crates/terminus/src/`.

Version or commit this describes: v0.1.1-28-g6a72ff4 (`6a72ff4`)
Last reviewed: 2026-10-15

## What this tool reads

**Transcripts.** Terminus walks `<root>/projects/` under each root. The roots are the `roots` list in `terminus.toml` when it is set, otherwise `$CLAUDE_CONFIG_DIR`, otherwise `$HOME/.claude` (`core/config.rs`). `roots` entries are used as written, with no `~` expansion and no absolute-path check. Each root is canonicalized once (`core/discover.rs`).

- A project is a directory directly under `projects/`. A symlinked project directory is skipped.
- Inside a project the walk has no depth limit, enters dot-directories, and does not enter symlinked directories.
- A transcript is chosen by file name only: `<uuid>.jsonl` directly in a project directory, or `agent-*.jsonl` at any depth.
- A symlinked transcript file is followed to its target wherever that is. The only check on the target is the `exclude` list (`keep_transcript` in `core/discover.rs`). The session is keyed by the link's path.
- For a sidecar transcript, the sibling `agent-<x>.meta.json` is read whole, through symlinks, with no size limit (`core/ingest/mod.rs`).
- Each transcript is opened read-only with `File::open` (`open_transcript` in `core/discover.rs`) and read from the stored byte offset to end of file in one buffer (`read_tail` in `core/ingest/mod.rs`).

**`terminus ingest <path>`.** Reads any file the user names. The path is canonicalized and checked against `exclude`, with no check on its name or location (`core/ingest/mod.rs`).

**Config directory.** `$TERMINUS_CONFIG_DIR`, otherwise `$XDG_CONFIG_HOME/terminus`, otherwise `$HOME/.config/terminus`, on Linux and macOS alike (`core/config.rs`).

- `terminus.toml`, read whole. A missing file means defaults. A file with a `[provider]` table fails to load.
- `data-location`, the pointer `terminus data move` writes. It must hold an absolute path (`core/store/open.rs`).

**Data directory.** `$TERMINUS_DATA_DIR`, then the `data-location` pointer, then `$XDG_DATA_HOME/terminus` or `$HOME/.local/share/terminus` on Linux, or `$HOME/Library/Application Support/terminus` on macOS (`core/store/open.rs`). `TERMINUS_DATA_DIR` is checked only for being non-empty, not for being absolute. Terminus reads `terminus.db` and its `-wal` and `-shm` files, `LOCK`, `injection/<session_id>.json`, `decisions/*.json` and `snapshots/`.

**Claude Code's own files**, read by `install`, `uninstall` and `doctor` (`bin/cmd/install/targets.rs`):

- `$CLAUDE_CONFIG_DIR/settings.json`, otherwise `$HOME/.claude/settings.json`, and its `<resolved path>.terminus-backup`.
- `$CLAUDE_CONFIG_DIR/.claude.json`, otherwise `$HOME/.claude.json`, read to find `.mcpServers.terminus`.
- `doctor` also reads `<cwd>/.claude/settings.json` and `<cwd>/.claude/settings.local.json`.
- These are parsed by Terminus's own JSON parser in `bin/cmd/install/json_file.rs`, which stops at nesting depth 128 and refuses duplicate keys.

**Binaries.** The stable path `$TERMINUS_BIN_DIR/terminus`, otherwise `$HOME/.local/bin/terminus`, is scanned whole for a marker string (`bin/cmd/install/binary.rs`). The versions folder is `$HOME/.local/lib/crenshawdev/terminus/versions`. The running executable is found with `current_exe()`, which reads `/proc/self/exe` on Linux.

**Outside the transcript tree during ingest.** For each session, the first `cwd` value in the transcript is checked with `is_dir()`. If it exists, `git rev-parse --show-toplevel` runs in that directory, and git reads that repository's config (`core/project.rs`). When a project name is ambiguous against an `exclude` entry, the exclusion check lists up to 4,096 entries under the excluded project's real path (`core/config.rs`).

**Environment variables.**

| Variable | What it controls |
|---|---|
| `HOME` | Fallback for every directory below, and `~` in `exclude` |
| `XDG_CONFIG_HOME` | Config directory |
| `XDG_DATA_HOME` | Data directory, Linux only |
| `TERMINUS_CONFIG_DIR` | Config directory, and where the `data-location` pointer lives |
| `TERMINUS_DATA_DIR` | Data directory, ahead of the pointer |
| `TERMINUS_BIN_DIR` | Directory of the stable binary path |
| `CLAUDE_CONFIG_DIR` | Transcript root when `roots` is unset; location of `settings.json` and `.claude.json` |
| `PATH` | Where `git` and `claude` are found |
| `DISABLE_AUTO_COMPACT` | Reported by `doctor`, nothing else |

`TERMINUS_FAULT_AT`, `TERMINUS_FAULT_READY`, `TERMINUS_FAULT_SPLIT_WATERMARK` and `TERMINUS_FAULT_AFTER_FILES` are read only when the `testkit` feature is on. Every function that reads them has a no-op stub for builds without it (`core/ingest/mod.rs`), and the release workflow builds `-p terminus` with no features (`.github/workflows/release.yml`). Every variable except the pointer and the versions folder accepts a relative value, which resolves against the process's working directory. The environment variables read by the bundled SQLite and by mimalloc on musl builds are not listed here [VERIFY].

**stdin.** The hook payload is one line, at most 1 MiB, abandoned after 500 ms (`bin/cmd/hook.rs`). The MCP server reads newline-delimited JSON-RPC with no size limit (`bin/cmd/mcp/mod.rs`). `install`, `uninstall --purge` and `data move` read a y/n answer.

**CLI arguments.** Three take a filesystem path: `ingest <path>`, `export <dir>` and `data move <path>`. Search words become an FTS query. `--project` is a scope path or `*`. `--tool`, `--kind`, `--path`, `--since` and `--until` become bound SQL parameters. The full list is in the README's Commands section.

**Sockets and network.** None. `crates/terminus-core/tests/no_network.rs` fails if any source file in either crate names a socket type.

**Other users, projects and tools.** Ingest reads every project under every root that is not excluded, and it reads Claude Code's own files. By default it reads nothing owned by another user. `roots`, `CLAUDE_CONFIG_DIR`, a symlinked transcript or `ingest <path>` can point it at any file the user can read.

## What it writes

**File modes.** Files in the data directory are created through `core/owner_only.rs`: directories 0700 and files 0600, passed in the creating syscall. Nothing calls `umask()`, so the result can be narrower but never wider. Parent directories are made with `create_dir_all` at the default mode, and only the leaf gets 0700. Every writable open of the store (`Store::open` in `core/store/open.rs`) tightens the data directory, `terminus.db`, `-wal`, `-shm`, `LOCK`, `snapshots/`, `injection/` and `decisions/` and the regular files one level inside them. It skips anything that is not the expected type and ignores failures. Read-only opens tighten nothing.

**Inside the data directory.**

| Path | Mode | What it holds and who writes it |
|---|---|---|
| `<data>/` | 0700 | Created by the lock, by `Store::open` and by the injection state save |
| `LOCK` | 0600 | `flock` target for ingest, `compact`, `reindex`, `observations regenerate`, `data move` and `uninstall --purge` |
| `terminus.db` | 0600 | Pre-created with `create_new` before SQLite opens it. `status` and `verify` open the store writable, so they create the data directory and an empty database if none exists |
| `terminus.db-wal`, `-shm` | Created by SQLite; the code expects SQLite to copy the main file's mode [VERIFY] | WAL mode is set on every writable open |
| `snapshots/terminus-<UTC stamp>.db` | 0600 | A full copy of the database made with `VACUUM INTO`, every 24 hours by default, 3 kept by default (`core/store/snapshot.rs`) |
| `injection/<session_id>.json` | 0600 | Turn ids already injected and a compaction flag. Never pruned |
| `decisions/<session_id>-<pid>-<ms>-<n>.json` | 0600 | The full prompt text, the cwd and the candidate turns for one prompt. Written only if the data directory exists. Ingest moves them into the `decisions` table and deletes the files |
| `.inject-*.tmp`, `.decision-*.tmp` | 0600 | Temporary files. A hook killed at its deadline can leave one, and nothing sweeps them |

`compact` runs a plain `VACUUM`. Terminus sets no `temp_store` pragma, so SQLite's own temporary files go wherever SQLite puts them, with whatever mode it gives them [VERIFY]. Terminus sets no `secure_delete` pragma, so deleted rows stay in free pages until `compact` and stay in snapshots until those are pruned.

**Config directory.** `terminus data move` writes `data-location` through `data-location.tmp` and a rename. It uses `std::fs::write`, not `owner_only`, so the file gets the umask mode and the write follows a symlink at the temporary name (`bin/cmd/data.rs`). `terminus.toml` is never written.

**Outside Terminus's own directories.**

- **`settings.json`** (`bin/cmd/install/json_file.rs`).
  - Install merges into the existing file, keeping key order and every scalar's source text. It refuses invalid JSON, a non-object root and duplicate keys.
  - The file is read again just before writing. A symlink is resolved and the write goes beside the real target, through `.settings.json.terminus-<pid>`, fsync and rename.
  - The temporary file is created with default `OpenOptions`, so its mode is 0666 minus the umask. The existing file's permissions are copied onto it only after the contents are written. A `settings.json` that did not exist before keeps the umask mode.
  - A one-time backup, `<resolved path>.terminus-backup`, is made with a hard link so an existing backup is never replaced. It has the same permission window.
- **`.claude.json`** is never written by Terminus. Install changes it by running `claude mcp add-json --scope user terminus <json>`, and install and uninstall run `claude mcp remove --scope user terminus`.
- **The binary.** Install writes `$HOME/.local/lib/crenshawdev/terminus/versions/<version>/terminus` at mode 0755, through a temporary file, fsync and rename. The versions folder always comes from `HOME`, even when `TERMINUS_BIN_DIR` is set. The stable path is a symlink to it, made at a temporary name and renamed into place (`bin/cmd/install/binary.rs`). `install.sh` does the same with umask 022.
- **`terminus export <dir>`** refuses a non-empty directory. It creates the leaf at 0700 and writes `NNNNNN-<sanitized name>.jsonl` files and `manifest.json` at 0600 with `create_new`. The output is the verbatim, unredacted transcripts (`bin/cmd/export.rs`).
- **`terminus data move <path>`** copies the data directory with 0700 directories and 0600 files, following symlinked files. It writes the pointer, then removes the source with `remove_dir_all` (`bin/cmd/data.rs`).
- **Never written:** shell rc files, `PATH`, systemd, launchd, cron, and anything under the Claude projects tree.

**What it deletes.**

| What deletes | What it removes | What guards it |
|---|---|---|
| `uninstall` | Every hook entry in `settings.json`, under any event, whose `command` equals the stable path string, including entries added by hand. If the result equals the backup, the backup's bytes are written back and the backup deleted | The command string only (`bin/cmd/uninstall.rs`) |
| `uninstall` | The `.mcpServers.terminus` entry, through `claude mcp remove`, whoever created it | The key name only |
| `uninstall` | The stable path, if the file it resolves to contains the marker or it is a dangling link into the versions folder | A byte-substring match on the marker |
| `uninstall --purge` | `remove_dir_all` on the data directory | Only that `<data>/terminus.db` exists, then a confirmation or `--yes`, then the lock. No check that the directory is where Terminus would put it |
| `data move` | `remove_dir_all` on the source data directory after the copy | The lock and a confirmation. No `terminus.db` check. The destination checks (not the source, not inside it, empty) are lexical, so a symlinked alias is not caught |
| Retention | Database rows only, never files | `delete` acts only once Claude Code has deleted the transcript. It never deletes `decisions` or `labels` rows (`core/retention/`) |
| Snapshot prune | `snapshots/terminus-*.db` beyond `keep` | Name prefix and suffix, regular files only |
| Decisions drain | Every regular `*.json` in `decisions/`, unparseable ones included | Regular files only, deleted after the database commit |

`uninstall` never removes the versions folder, the `data-location` pointer, `terminus.toml` or the backup of `.claude.json`.

**stdout and stderr.**

- **Hooks.** `SessionStart` and `UserPromptSubmit` print one line, `{"hookSpecificOutput":{"hookEventName":"<event>","additionalContext":"<text>"}}`, built with serde_json, or nothing. Claude Code puts `additionalContext` into the model's context. Whether the user sees that text is up to Claude Code [VERIFY]. `SessionEnd` and `PostCompact` print nothing. stderr gets one fixed line naming the event and an OS error. Every known event exits 0.
- **The detached ingest** each hook starts has stdin, stdout and stderr all on `/dev/null`. Its errors go to the `runs.error` column.
- **MCP server.** stdout carries JSON-RPC only, and tool results go into the model's context. A failed call's `reason` can contain absolute paths, raw SQLite error text, and a TOML parse excerpt (withheld only when it contains `api_key`).
- **CLI.** Data goes to stdout and diagnostics to stderr. That reaches a terminal, or the model's context when Claude runs the command through Bash. `terminus show` writes raw archived bytes, and nothing filters control characters out of `show` or `search` output, so an escape sequence stored in a transcript may reach the terminal live [VERIFY].

## What it executes

No shipped code runs a shell. The only `/bin/sh` is inside a `#[cfg(test)]` module in `bin/cmd/install/targets.rs`.

| Program | How it is found | Arguments | Working directory | When | Limits |
|---|---|---|---|---|---|
| `git` | Terminus scans `PATH` itself and takes the first `git` that is a file. The exec bit is not checked and relative `PATH` entries are not skipped (`core/project.rs`) | `rev-parse --show-toplevel` | The session's first `cwd` value from the transcript | Ingest, once per distinct `cwd` | 2 s then kill. stdin and stderr null. Environment inherited |
| `terminus` | `current_exe()`, or the stable path when install starts it | `--reparent ingest` or `--reparent backfill --work`. The intermediate process re-runs its own argv after `--reparent` | Inherited | Every hook event; the end of install | Detached in its own process group, all stdio null, never waited on (`bin/cmd/spawn.rs`) |
| `claude` | The same `PATH` scan (`bin/cmd/install/claude.rs`) | `mcp add-json --scope user terminus <json>` or `mcp remove --scope user terminus` | Inherited | `install`, `uninstall` | 30 s then kill. stderr capped at 64 KiB. Environment inherited plus `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` |
| `claude` | The same `PATH` scan | `--version` | Inherited | `doctor` | 2 s |
| The stable `terminus` | `$TERMINUS_BIN_DIR/terminus` or `$HOME/.local/bin/terminus` | `--version` | Inherited | `doctor`, only if the file contains the marker | 2 s |

The `<json>` passed to `claude mcp add-json` keeps every key of an existing `.mcpServers.terminus` entry in `.claude.json`, so data from that file goes into argv. No shell interprets it.

**Hooks installed in Claude Code.** Install appends one group per event to `settings.json` for `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`:

```json
{"matcher": "", "hooks": [{"type": "command", "command": "<stable path>", "args": ["hook", "<Event>"]}]}
```

This is exec form with no timeout set. Each event fires on every occurrence because the matcher is empty.

**MCP server.** User scope, key `terminus`: `{"type": "stdio", "command": "<stable path>", "args": ["mcp"]}`. `install.sh` ends by running `<stable path> install`.

**Built from data it read.**

- **SQL.** Every value is a bound parameter. Each `format!` that builds SQL inserts only constant table or column names or runs of `?` placeholders.
- **FTS5.** Search text is split on every non-alphanumeric character, deduplicated, capped at 32 tokens, and each token is double-quoted and joined with `AND` (`match_expression` in `core/recall/query.rs`). The per-prompt injection joins at most 8 such expressions with `OR`. FTS5 operators, column filters, `*` and `^` cannot get through.
- **Text for Claude's context.** The `SessionStart` brief names the project path and the branch, then gives "It last asked:" and "It last answered:" excerpts and a line pointing at the recall tools (`core/inject/brief.rs`). The `UserPromptSubmit` injection is a head line followed by `- turn <id> (<day>): <excerpt>` lines (`core/inject/prompt.rs`). Both are archived transcript text. Excerpts are flattened to one line. There are no delimiters and no label saying the text is data rather than instructions.
- **Shell commands printed for the user to run** are single-quote escaped by `shell_quote`. The `rm -f '<path>'` suggestion in `bin/cmd/doctor.rs` does not escape a `'` in the path.

No command line is built from transcript content. Transcript content sets only the directory `git` runs in.

## What it must never touch

**Actions it must never take.**

- Open a network connection. Enforced by `crates/terminus-core/tests/no_network.rs`.
- Call a model or read an API key. A `[provider]` table in `terminus.toml` fails the load.
- Write, move or delete anything under the Claude projects tree. Transcripts are opened read-only.
- Read the transcripts of a project listed in `exclude`, or return its sessions through search, recall, the brief or injection.
- Change Claude Code's `cleanupPeriodDays` or `autoCompactEnabled`. `doctor` only reports them.
- Block a prompt or decide a permission. Hooks exit 0 for every known event, and their only output is `additionalContext`.
- Commit, push or change git state. The only git command it runs is `rev-parse --show-toplevel`.
- Edit shell rc files, `PATH`, systemd, launchd or cron, or use sudo.
- Sign a release in CI. No workflow holds a signing key.

**Data that must stay put.**

- Terminus sends nothing off the machine. It hands archived text to Claude Code on the same machine, and Claude Code sends it to its model with the rest of the session.
- Text from an excluded project must never reach Claude Code.
- With `[privacy] redact_recall` on, a secret of a shape the filter is built to replace must never reach Claude Code through a hook or an MCP tool.
- Archived text leaves the data directory only through hook output, MCP results, CLI output, `export` and `data move`.

**Invariants a finding can break.**

- `install.sh` installs nothing whose manifest fails the pinned key or whose SHA-256 does not match the manifest, and downloads the binary only after the manifest verifies.
- Archived blocks never change after they are written, and `terminus verify` fails when stored blocks no longer match their recorded hashes.

## Who runs it and with what privileges

- **User.** The interactive user. There is no service user, no root and no setuid.
- **Started by:**
  - Claude Code, as a hook process for each of the four events and as an MCP server over stdio. Every hook event starts a detached `terminus ingest`.
  - The user, by hand, for every CLI command.
  - `install.sh`, which runs `terminus install`, which starts a detached `terminus backfill --work`.
  - Nothing else. There is no timer, daemon or login item.
- **Working directory.** For hooks and the MCP server, the session's working directory as Claude Code sets it [VERIFY]. The default recall scope and relative environment values depend on it.

**What that user can already do without Terminus:** read every transcript under `~/.claude/projects`, edit `settings.json` and `.claude.json`, run `git` and `claude`, and read and delete the data directory. A finding is a real escalation when Terminus lets something the user did not choose do one of these:

- Transcript content, which carries tool output, file contents and web pages, causes a program to run, a file outside the projects tree to be read, or anything to be deleted.
- Text from one project, or from an excluded project, reaches a session's context where it was not asked for.
- Text from an old session acts as instructions in a new session without the user seeing it.
- A secret is copied somewhere the configuration says it should not go.
- The archive changes or disappears without `terminus verify` or the user noticing.

A same-user process that can already write `~/.claude/projects` or the data directory is not an escalation in itself. What Terminus does with what that process wrote can be.

## What the worst outcome looks like

**The single worst outcome** is archived text acting as instructions in a later session. A web page, file or tool result Claude saw once is archived verbatim. It can come back into a new session three ways: automatically through the `UserPromptSubmit` injection (any record type qualifies, including tool results, so a file Claude read can come back when the user names that file), through the `SessionStart` brief, or through the `recall_*` tools when the model calls them. There it is read with that session's tool permissions. Because `recall_search` accepts `project: "*"`, text planted in one project can steer the model into pulling another project's archived secrets into the current session, where that session's tools can send them out.

**What would hurt the most:**

- **Losing the archive.** Once Claude Code's `cleanupPeriodDays` deletes a transcript, the archive can be the only copy. `uninstall --purge` and `data move` both run `remove_dir_all` on a directory that comes from the environment or the pointer file. Nothing checks hashes when the archive is read, so a corrupted or edited block is served until `terminus verify` runs.
- **Leaking secrets.** The archive keeps every secret ever pasted into a prompt or printed by a tool, verbatim. That includes `terminus.db`, up to three full snapshots, `export` output, the `decisions` files and table (full prompt text, never removed by retention) and the `observations` table (full Bash command lines). It hurts the user and whoever owns the secrets.
- **Clobbering `settings.json`**, which holds the user's other hooks and permissions.

**What the user would not notice:**

- What the hooks put into `additionalContext` [VERIFY whether Claude Code shows it].
- The detached ingest after every hook, and `git` running in every directory a transcript names.
- A symlinked file in the projects tree being archived.
- An edited row in the database, until `terminus verify` runs.
- Secrets piling up in snapshots and decision records.
- The `settings.json` temporary file and backup sitting at the umask mode for a moment.

## Trust boundaries (where untrusted data enters)

**1. Transcript content.** It is untrusted: it carries web pages, files and tool output Claude saw.

- **Parsing.** Each line goes through `serde_json::from_slice::<Value>`. serde_json's recursion limit of 128 applies, because nothing enables `unbounded_depth` (`cargo tree -e features` shows only `default` and `std`). A line that is not a JSON object becomes a record with no fields, and its bytes are still archived.
- **Size.** There is no limit on line length, on the unread tail of a file, or on the sidecar `.meta.json`. Indexing is capped at 256 KiB of text per turn, depth 8, 48 entities per turn and 512 bytes per error value (`core/index/`).
- **A panic.** A session with 2^24 turns or more trips an `assert!` in `turn_id` (`core/store/schema.rs`). The hook-started ingest does not catch panics per file; only backfill does.
- **Fields that steer behavior.**
  - `cwd` sets the directory `git` runs in and the project key, which drives scope and exclusion.
  - `gitBranch` is printed into the brief unfiltered. A cloned repository controls its own branch names.
  - `sessionId` is joined on by feedback labelling.
  - `parentUuid` sets the `continues_from` link, looked up with `LIMIT 1` and no `ORDER BY`.
  - `compactMetadata.preservedMessages` decides which turns count as dropped after a compaction, which makes them injectable.
  - Message content feeds the FTS index, the entity index and injection.
- **Session identity.** The session key is the canonical file path, never the record's `sessionId`, so a forged id cannot overwrite another session.
- **Who controls top-level fields.** Claude Code writes the top-level fields, and tool output appears only inside message content [VERIFY]. Anyone who can write a file under `~/.claude/projects` controls every field.

**2. The shape of the projects tree.** A symlinked `<uuid>.jsonl` or `agent-*.jsonl` is followed to any target that is not excluded. `agent-*.meta.json` is read through symlinks with no size limit.

**3. Hook stdin.** It comes from Claude Code and is not authenticated.

- **Read limits.** One line, at most 1 MiB, abandoned after 500 ms. A truncated payload parses as all fields absent.
- **Event name.** It comes from argv. `hook_event_name` in the payload is ignored.
- **Fields.**
  - `session_id` is allow-listed to hex digits and `-`, 8 to 64 characters, before it goes into a file name.
  - `transcript_path` is canonicalized and compared as a string, never opened.
  - `cwd` is used as a string. It selects which archived project's brief or recall is emitted, so a crafted `cwd` can select any project that is not excluded.
  - `prompt` yields at most 8 candidate terms for FTS, and is stored whole in the decision record.
  - `source: "compact"` sets the compaction flag.

**4. MCP tool inputs.** The model chooses them, so anything in its context can steer them.

- **Transport.** No limit on message size. Input is converted to UTF-8 lossily. Batches are refused.
- **Unknown argument keys** are ignored, even though the schemas declare `additionalProperties: false`.
- **`recall_search`.**
  - `query` is required and has no length cap; it is tokenized as above.
  - `paths` is cut to 25, and `limit` is clamped to 50.
  - `tool` and `kind` are bound parameters, and `kind: "observation"` is refused.
  - `since` and `until` must be `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS.mmmZ`.
- **`recall_context`.** `turn_id` must be an integer, and `before` and `after` are clamped to 25 each.
- **`recall_get`.** `turn_ids` is cut to 25. Each `body` is the whole archived JSON line, with no size cap.
- **Scope.** `project` defaults to the server's working directory. `"*"` or any path widens or moves the scope to any project that is not excluded, with no confirmation. Excluded projects stay hidden under every scope. Turn ids are `session_no << 24 | turn_seq`, so they are easy to enumerate, and id lookups outside the scope answer `NoSuchTurn`.
- **Leaks in error text.** Naming a path inside an excluded project returns a reason that includes the stored project key. Error reasons can also carry absolute paths and raw SQLite messages.

**5. CLI arguments.** These come from the user. `ingest <path>`, `export <dir>` and `data move <path>` take any path. `data move` makes it absolute against the working directory and refuses a destination inside the source or not empty.

**6. Environment variables.** These are inherited from the user's shell, or from Claude Code for hooks and MCP. `TERMINUS_DATA_DIR` has no absolute-path check. `PATH` decides which `git` and `claude` run.

**7. `terminus.toml`.** Owned by the user and read whole. `roots` is used as written. In `exclude`, `~` is expanded, `..` is resolved as text, relative entries are dropped, and `/` excludes everything.

**8. Claude Code's `settings.json` and `.claude.json`.** Parsed with nesting capped at 128. The keys of `.mcpServers.terminus` are copied into argv for `claude mcp add-json`.

**9. Network responses (`install.sh` only).**

- **Download.** `curl --proto '=https' --tlsv1.2` fetches the manifest, its signature and the binary from the GitHub release, or from an `https://` source passed as an argument.
- **Signature first.** `ssh-keygen -Y verify` checks the manifest against the ed25519 key pinned in the script, identity `crenshawdev-release`, namespace `terminus-release`, before the manifest is parsed or the binary downloaded.
- **Manifest and binary.** The manifest must be exactly `version X.Y.Z` and `sha256 <64 hex>`. The binary's SHA-256 must match it.
- **What is not covered.**
  - The signed text names no platform or file name.
  - There is no protection against rollback to an older signed release.
  - `curl` and `ssh-keygen` come from `PATH`.
  - Whether `--proto '=https'` also restricts redirects [VERIFY].
- **No sudo.**

**10. What Terminus trusts because it wrote it.**

- **The SQLite store.** Reads check no hashes; `core/store/archive.rs` says so, and only `verify` and the append path check them. Block sizes are bounded to 1 byte through 64 MiB, ranges are checked, and decompressed length must match. Terminus sets no `trusted_schema=OFF` and no `SQLITE_DBCONFIG_DEFENSIVE`, and runs no `integrity_check` on open. `project`, `branch`, `ts`, entities, compaction metadata and `continues_from` are used as stored.
- **The ingest watermark** is trusted as the place to resume. For a diverged session, `vec![0u8; watermark]` makes an allocation of a size taken from the database.
- **State files.** Injection state files are read whole with `fs::read`, which follows symlinks, and must have `format == 1`. Decision files are read only when they are regular `.json` files, with no size limit.
- **The `data-location` pointer** is trusted once it holds an absolute path.
- **The marker.** A file containing the marker bytes counts as Terminus's own. It is a substring match, not a signature, and `doctor` runs such a file with `--version`.

## Secrets that may pass through

**Where they come from:**

- API keys and tokens pasted into prompts.
- Credentials in tool output: environment dumps, `.env` files read by the Read tool, private key files, `curl -H 'Authorization: ...'` lines.
- Secrets typed inline in Bash commands.
- `user:password@` in URLs.

**Where they end up:**

- Verbatim in the archive blocks in `terminus.db`.
- As searchable tokens in the FTS index.
- In the `entities` table, and in the `observations` table, which keeps full Bash command lines and error strings.
- In the `decisions` files and table, which keep full prompt text and are never removed by retention.
- In up to three snapshots and in `export` output.
- In what reaches Claude Code: hook `additionalContext` and MCP results.
- In CLI output.
- In error messages. A TOML parse excerpt is withheld only when it contains `api_key`.

**Redaction.** `[privacy] redact_recall` is off by default (`core/config.rs`).

- **Where it applies.** When on, it filters hook output, MCP results and CLI `search` and `show` output, which `--raw` bypasses. It never runs at ingest, so the archive stays verbatim.
- **What it matches** (`core/observe/egress.rs`):
  - PEM blocks whose label contains `PRIVATE KEY`.
  - `user:password` in URLs.
  - `name: value` lines, JSON `"name": "value"` pairs, `NAME=value`, and `--name value` flags, where the name contains `token`, `password`, `passwd`, `secret`, `key`, `api`, `bearer`, `cookie` or `authorization`.
  - JWTs starting with `eyJ`.
  - GitHub tokens with the prefixes `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` and `github_pat_`.
- **What it misses.**
  - A bare `sk-` token in prose (issue #5), AWS `AKIA` keys, Slack `xox` tokens, and any vendor key with no secret-looking name beside it. There is no entropy check.
  - It never touches the project path and branch in the brief, or the `session_key`, `project` and `tool_name` fields of MCP results.

**Secrets Terminus itself holds:** none. It reads no API key and refuses a `[provider]` table. The release signing key, `~/.ssh/crenshawdev-release`, is mode 0600 on the maintainer's machine and is used only by `scripts/sign-release.sh`, never by the binary or by CI. Whether that key has a passphrase [VERIFY].

**CI:** the `ANTHROPIC_API_KEY` Actions secret, read only by the `security-review` job in `.github/workflows/fast.yml` on GitHub pull requests. GitHub does not pass it to workflows run from a fork.
