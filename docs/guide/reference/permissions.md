# Permissions and data

Terminus sends no telemetry, collects nothing about you and makes no model call. It holds no API key, and the binary opens no network connection. Everything else it touches is listed here, so you don't have to find it by reading the source.

## What leaves your machine

- **Install and update.** `install.sh`, run by you through `curl`, comes from `raw.githubusercontent.com`. It then fetches the manifest, its signature and the binary from this repository's GitHub releases, or from the `https://` source you pass.
- **What Terminus hands Claude Code.** The brief, each prompt's injection and the MCP tools' results go into your session, and Claude Code sends them to its model with the rest of the session. That's Claude Code's data handling and not something Terminus controls.

Nothing else. The `claude` commands listed below are Claude Code's own program, started with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` in their environment.

## What it reads

| What | When |
|---|---|
| Transcripts under each root's `projects` folder, other than excluded projects | Each archive pass. |
| `terminus.toml` and the `data-location` file in the config directory | Each command, hook and MCP tool call. |
| `settings.json` in Claude Code's config directory | `install`, `uninstall` and `doctor`. |
| `.claude/settings.json` and `.claude/settings.local.json` in the current directory | `doctor` only, for the [two settings it reports](../configuration/paths.md#claude-code-settings-terminus-reports). It never prints the file. |
| `.claude.json` in Claude Code's config folder, by default `~/.claude.json`, for its `mcpServers.terminus` entry | `install`, `uninstall` and `doctor`. Terminus never writes this file. |
| The event's details on stdin, at most 1 MiB, including your prompt | Each hook. |
| The environment variables named in [paths and environment](../configuration/paths.md#environment-variables) | Whenever they apply. |

## What it writes

| What | Where | When |
|---|---|---|
| The store, its lock, snapshots and the state the hooks keep | The [data directory](../configuration/paths.md#the-data-directory) | Each pass and hook. |
| The binary and its link | `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus` and `~/.local/bin/terminus` | `install.sh` and `terminus install`. |
| Four hook entries | Claude Code's `settings.json`, with a one-time copy beside it as `settings.json.terminus-backup` | `terminus install`, and removed by `uninstall`. |
| The MCP entry | Written by Claude Code, through `claude mcp` | `terminus install`, and removed by `uninstall`. |
| The `data-location` file | The config directory | `terminus data move`. |
| The exported transcripts | The folder you name | `terminus export`. |

Every file Terminus creates in the data directory is readable only by your user. Directories are created `0700` and files `0600`, with the mode set in the call that creates them.

## What it runs

- `git rev-parse --show-toplevel`, in a session's working directory, to find which project it belongs to. A missing `git` or one that hangs just means the working directory is used.
- `claude mcp add-json` and `claude mcp remove`, from `install` and `uninstall`, and `claude --version`, from `doctor`. Each is found on `PATH` and run with no shell.
- Itself, detached, for the archive pass each hook starts.

`install.sh` also runs `curl`, `ssh-keygen`, and `sha256sum` or `shasum`. Terminus edits no shell profile.

## What's protected

- **Your files against other users on the machine**, through the file modes above.
- **Excluded projects.** They aren't opened, returned or logged from the moment they're excluded. [Keep less](../archive/keep-less.md) says what already stored stays.
- **Your terminal against archived text.** Every command's human output, on stdout and stderr, prints a control character other than tab and newline as a `\u00XX` escape. `--json`, the hook's document and the record body `terminus show` prints stay byte for byte.
- **Claude against planted text, on the prompt path.** A tool result is never handed over on a prompt. Handed-over text is quoted between `terminus-recall` tags under a line saying it's history and not instructions, and anything that reads as the closing tag is replaced.
- **A release against tampering after it's signed.** [How a release is built and signed](release.md#what-the-signature-protects) lists exactly what the signature covers.

## What isn't protected

- **There's no encryption at rest.** Terminus's files are protected only by being readable by your user alone, so anything running as you can read them. Claude Code's own transcripts sit unencrypted in its folder too.
- **The archive holds whatever the transcript holds.** A secret you pasted, or that a command printed, is stored and searchable. Nothing filters what's stored, and there's no command that removes one turn.
- **The injection log holds your prompts whole.** That's how `terminus stats` and `terminus replay` work. See [keep less](../archive/keep-less.md) for what removes them.
- **Redaction goes by shape.** [`redact_recall`](../configuration/recall.md#privacy) is off by default, covers a fixed set of shapes, and applies on the way to Claude Code. It doesn't touch the store, the snapshots or `terminus export`.
- **Tool results through the MCP tools.** `recall_search`, `recall_context` and `recall_get` return what was archived, tool results included, because that's what they're for. Text Claude once read from a file or a web page can come back that way, without the framing the hooks add.
- **Snapshots and your own backups.** Snapshots keep what was excluded or deleted after they were taken, until they age out. Whatever backs up your home directory copies all of it.
- **`install.sh` itself.** It's fetched unsigned over HTTPS from `main`.
- **Network filesystems.** The store isn't safe on one, and `terminus data move` warns you.

Reports about any of this go through [SECURITY.md](https://github.com/crenshawdev/terminus/blob/main/SECURITY.md). [TRUST.md](https://github.com/crenshawdev/terminus/blob/main/TRUST.md) is the standard these claims are held to.
