# Changelog

All notable changes to Terminus are recorded here. Terminus was named Verbatim
until 2026-10-05. The two Verbatim versions at the bottom were local test
builds, tagged `verbatim-v0.1.0` and `verbatim-v0.1.1` and never released.
Terminus numbers its unreleased work 0.0.x, and its first release will be
0.1.0. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `install.sh` installs Terminus with one command:

  ```sh
  curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh
  ```

  It downloads `terminus-<os>-<arch>.manifest`,
  `terminus-<os>-<arch>.manifest.sig` and `terminus-<os>-<arch>` from
  `https://github.com/crenshawdev/terminus/releases/latest/download`, where
  `<os>` is `linux` or `macos` and `<arch>` is `x86_64` or `aarch64`. One
  `https://` source, passed by ending the line with
  `sh -s -- https://<source>` instead of `sh`, replaces that default, and the
  same three names are fetched under it. The manifest is exactly two lines,
  `version X.Y.Z` and `sha256 <64 lowercase hex>`. Before the script reads the
  manifest, it verifies the signature with `ssh-keygen -Y verify` against the
  crenshawdev release key pinned in `install.sh`, under the namespace
  `terminus-release`, and it downloads the binary only after that.
  `verify_download` then checks the binary's SHA-256 against the manifest
  before anything is placed. The script stores the binary by version, links
  `~/.local/bin/terminus` to it and runs `~/.local/bin/terminus install`,
  exiting non-zero when that does.
- `install.sh` refuses a missing signature, a signature made by another key or
  under another namespace, and a manifest changed after signing, and it never
  downloads the binary of a release it refuses. A missing signature is
  reported as a failed download, the others as a manifest that does not verify
  against the pinned key. It refuses a machine without `ssh-keygen` before it
  downloads anything, naming `openssh-client` (Debian, Ubuntu),
  `openssh-clients` (Fedora) and `openssh` (Arch). The key is never fetched,
  and no argument, setting or environment variable changes it, so a source
  passed as the argument can only serve releases signed with that key. No
  release is published yet.
- `install.sh` refuses to run when `TERMINUS_BIN_DIR` is set, and refuses a
  stable path holding a file that is not a Terminus build, leaving it as it is.
- `install.sh`'s `curl` download is the one network use. The Terminus binary
  opens no network connection.
- `terminus verify` names the block of a failure as well as the session. A
  failure's line reads `<session_key>: block N: <detail>`, and under `--json`
  each failure carries `block`, the block's number. A failure about the whole
  session, such as a session checksum that no longer matches the block hashes,
  has no `block N:` in its line and `block` is `null`.

### Changed

- `~/.local/bin/terminus` is now a symbolic link to
  `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`. The hooks and
  the MCP registration name the link, so they are not rewritten when the
  version behind it changes.
- `terminus install` stores its own binary as a version and points the link at
  it, so a local build is installed with
  `cargo build --release && target/release/terminus install`. Run through the
  link, it changes neither the link nor the version file.
- A rebuild at the same version replaces that version's file, through a rename,
  so a process running the old file keeps it. The same bytes are left alone.
- Old versions are kept, and nothing prunes them, so a running session keeps
  the file it started from.
- An existing install that is a plain file at `~/.local/bin/terminus` becomes
  the link on the next install.
- `terminus uninstall` removes the link, leaves the versions folder, and prints
  its path.
- Verbatim is now Terminus. The binary is `terminus`, the crates are `terminus`
  and `terminus-core`, and the MCP server registers as `terminus`, so its tools
  arrive as `mcp__terminus__recall_search`, `recall_context` and `recall_get`.
  The data directory is `~/.local/share/terminus` (macOS
  `~/Library/Application Support/terminus`) holding `terminus.db`, the config file is `terminus.toml`, every `VERBATIM_*`
  environment variable is `TERMINUS_*`, snapshots are `terminus-<time>.db`, and
  `lean` and `minimal` capture mark an elided body `{"terminusElided": <bytes>}`.
- Nothing reads the old names, so an existing install moves once by hand:
  `verbatim uninstall` with the old binary, checkpoint the WAL, rename the data
  directory, `verbatim.db` and its snapshots, then `terminus install`. A store
  written in `lean` or `minimal` mode keeps its old elision marks, which this
  build does not count. Archived calls to the old `mcp__verbatim__recall_search`
  are still recognized.
- `terminus install` asks nothing. It applies its defaults, never reads stdin,
  and ends with a receipt: every file it changed, the backfill estimate,
  `terminus status` as the place to watch the backfill, and the current
  `cleanupPeriodDays`, with a line saying transcripts older than that are
  already gone and the line that raises it.
- Install places the MCP entry with `claude mcp add-json --scope user terminus`
  and never writes `~/.claude.json`, so Claude Code is that file's only writer.
  When the entry under `.mcpServers.terminus` is already correct, no `claude`
  runs. An entry in another shape is removed with
  `claude mcp remove --scope user terminus` and then added again.
- A `claude` that is not on `PATH`, times out or exits non-zero, or a
  `~/.claude.json` that does not parse, leaves the install incomplete rather
  than aborting it. The binary and the hooks are still placed and the backfill
  still starts, the receipt prints the exact command that finishes the
  registration, and install exits 1.
- Every `claude` Terminus starts, doctor's `claude --version` included, gets
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, so wiring Terminus up opens no
  network connection.
- `terminus uninstall` removes the MCP entry with
  `claude mcp remove --scope user terminus`, only when `~/.claude.json` holds
  one, and no longer writes that file. An existing
  `.claude.json.terminus-backup` from an earlier install is left in place and
  its path printed. It is never restored, because it holds an older copy of
  Claude Code's own state. When `claude` is missing or fails, uninstall prints
  the command, finishes its other steps and exits 1.
- When `claude` is not on `PATH`, `terminus doctor` gives the
  `claude mcp add-json` command as the fix for a missing or wrong MCP entry,
  preceded by `claude mcp remove` when an entry in another shape is there,
  instead of `terminus install`.
- A `settings.json` that is not in the two-space format is rewritten in full
  without a question, and its backup holds the original bytes.
- Each session's archive is stored as numbered blocks, one row per block in a
  `blocks` table, instead of one blob in `sessions.blob`. A row holds the
  block's zstd-compressed bytes and a BLAKE3 hash of its uncompressed bytes,
  and the session checksum in `session_meta` is a BLAKE3 hash over the block
  hashes in order.
- The block size is recorded per session, in `session_meta.block_size`, and is
  64 KB.
- An append rewrites at most the session's last block and adds rows after it.
  It checks the stored block hashes against the session checksum and the last
  block against its own hash, and reads no other block. Reading a turn fetches
  and decompresses only the blocks its range covers.
- `terminus usage` counts a session's archive bytes as the sum of its block
  rows' lengths.
- The archive format is now 2. A store written by an earlier build is refused
  by every command that opens it, `terminus install` included, which exits
  non-zero, and until it is replaced the hooks archive nothing. The message
  gives the clean install: move the data folder and the current binary aside,
  then rerun `install.sh`. The refused store is not changed and there is no
  conversion. The new store holds the sessions whose transcripts are still on
  disk, and its injection decision log and labels start empty; the folder moved
  aside keeps the rest.
- What a healthy ingest pass did, such as labels written and sessions retention
  evicted, is recorded apart from what went wrong. `terminus status` prints it
  under `notes` rather than `error`, `status --json` carries `notes` beside
  `error` in `last_run`, and `terminus doctor` no longer reports a problem for a
  pass that only did routine work.

### Removed

- Terminus no longer calls a model, reads an API key or opens a network
  connection. Observations keep their mechanical facts (files, tools, commands,
  errors, branch, commits, turn count, duration and compactions), which never
  needed a model, and lose the session summary a model used to write.
- A `terminus.toml` that still has a `[provider]` table now fails to load, with
  a message naming the file and saying model calls were removed. Until the table
  is deleted, every command that reads the config exits non-zero and the hooks
  archive nothing.
- `~/.config/jcrenshaw/credentials.toml` is no longer read. Terminus leaves the
  file where it is.
- Stored session summaries are dropped the first time an existing store opens
  with this build. The ten columns `status`, `model`, `prompt_version`, `topic`,
  `outcome`, `decisions`, `learned`, `unresolved`, `raw` and `tokens` leave the
  `observations` table, and every row's mechanical facts stay as they were. A
  fresh store and an upgraded one have the same columns. Snapshots taken before
  the upgrade still hold the old text until they rotate out.
- `kind: observation` is refused. `terminus search --kind observation` exits 2,
  and `recall_search` answers `isError`, both naming the reason.
- `terminus observations regenerate --prompt-version` is gone. `--since` stays.
- `terminus doctor` no longer reports `credentials`, `config_mode`,
  `mode_checks` or `provider_local`.
- The npm package is gone. The `npm/` directory, which held the thin `terminus`
  package, its five platform packages and `pack-local.sh`, was deleted, and
  Terminus has no npm channel.
- Windows support is gone. Terminus builds on Linux and macOS only, and a build
  for any other system fails at compile time with "Terminus supports Linux and
  macOS only." That includes other Unix systems, which used to build against a
  generic data-directory fallback. `terminus doctor` and `terminus install`
  print only `chmod` and `rm -f` fixes.
- The install diff and its confirmation are gone.
- `terminus install --yes` is gone. Install takes no arguments, so `--yes` now
  exits 2 as an unexpected argument. `terminus uninstall --purge --yes` is
  unchanged.
- Install no longer offers to raise `cleanupPeriodDays`. Terminus never changes
  the setting, and the receipt prints the line that raises it.

## [Verbatim 0.1.1] - 2026-09-04 (local test build, never released)

A privacy and correctness release. Three of the four things in it are defects
in shipped 0.1.0 behaviour, and one of them meant the redaction you thought you
had was mostly not running.

### Fixed

- The `SessionStart` resume brief quoted the wrong thing. Claude Code writes
  tool results as `type: "user"` records, so any session that ended inside a
  tool loop had its harness payload quoted back under "It last asked", which
  attributes words to you that you never typed. Every `user` record is now
  classified typed or not at parse time into a new `turns.is_typed` column, and
  the brief reads it. A session with no typed prompt renders no prompt line
  rather than falling back to the tool result. An existing store gains the
  column by backfill from its own blobs, no re-ingest.
- Egress redaction was mostly inert on the wire. The filter ran on the
  already-serialized JSON body, and a serialized body has no real newlines, so
  the header rule saw one enormous line whose first colon sat inside `{"model":`
  and rejected it, and the JSON-pair rule consumed the whole transcript as a
  single candidate name. Only two of the five rules ever fired. The filter now
  runs per message content before the body is built, and the rule set went from
  5 rules to 9: bare JWTs, GitHub tokens, connection URLs carrying userinfo and
  space-separated `--token` flags are caught now, `cookie` joined the secret
  names, and the header rule tests every colon on a line instead of the first.
  The tests assert over the exact bytes handed to the socket, not hand-written
  samples.

### Added

- Everything Verbatim writes lands owner-only. The data directory and
  `verbatim.db` are created 0700/0600 under umask 022, snapshots, the lock file,
  injection scratch, decision records, export output and `data move`'s
  destination tree all inherit it, and a store that was already group- or
  world-readable is tightened once on a writable open. A group- or
  world-readable `verbatim.toml` is refused where its `api_key` is consumed, the
  same refusal the shared credentials file already had. Unix only; `doctor`
  reports the mode and states the Windows deferral.
- `[privacy] redact_recall`, off by default. Turn it on and the egress filter
  runs over every projection that renders archived text into model context:
  `recall_search`'s excerpt and observation claims, `recall_context`'s window,
  `recall_get`'s body, the `SessionStart` brief's quotes and the per-prompt
  injection. A judgment's `topic`, `outcome` and claim text are filtered before
  they reach SQLite, and observation rows written before you flipped the knob
  are filtered on read, so nothing needs regenerating. `verbatim search --raw`
  and `verbatim show --raw` are the owner's escape hatch, per invocation, and no
  environment variable can reach past the knob into the hook or the MCP server.
  The archive on disk is never touched in either setting, and `verbatim export`
  stays unfiltered with the notice it already carries.

### Known issues

- With `redact_recall` on, `recall_get` runs its body through a lossy UTF-8
  round trip, so an archived line that is not valid UTF-8 comes back with
  U+FFFD replacement characters even when it carries no secret. All 47,157
  archived records measured on the development corpus are valid UTF-8, so this
  costs nothing observable there. Tracked as
  [#1](https://github.com/crenshawdev/verbatim/issues/1).

## [Verbatim 0.1.0] - 2026-08-23 (local test build, never released)

First release. A single static Rust binary that tails Claude Code's session
transcripts, stores every session unmodified and permanently, indexes it at turn
granularity, and gives the model recall over its own history through three MCP
tools and hook-driven context injection. Local only, no daemon, no port, no
telemetry.

[Unreleased]: https://github.com/crenshawdev/terminus/compare/verbatim-v0.1.1...HEAD
[Verbatim 0.1.1]: https://github.com/crenshawdev/terminus/compare/verbatim-v0.1.0...verbatim-v0.1.1
[Verbatim 0.1.0]: https://github.com/crenshawdev/terminus/tree/verbatim-v0.1.0
