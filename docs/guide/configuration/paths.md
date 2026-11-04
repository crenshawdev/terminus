# Paths and environment

These sit outside `terminus.toml`.

## The data directory

The archive, its snapshots and the state the hooks keep live in the data directory. Terminus uses the first of these that is set:

1. `TERMINUS_DATA_DIR`.
2. The location `terminus data move` recorded, in a `data-location` file in the config directory. A recorded path must be absolute; a relative one, or a file that exists and can't be read, is an error rather than a fall back to the default.
3. The platform location. On Linux it's `$XDG_DATA_HOME/terminus`, otherwise `~/.local/share/terminus`. On macOS it's always `~/Library/Application Support/terminus`, and `XDG_DATA_HOME` isn't read.

`TERMINUS_DATA_DIR` outranks a location `terminus data move` recorded.

## Environment variables

| Variable | What it sets |
|---|---|
| `TERMINUS_DATA_DIR` | The data directory, as above. |
| `TERMINUS_CONFIG_DIR` | The [config directory](index.md#where-the-file-lives). It holds `terminus.toml` and the `data-location` file. |
| `TERMINUS_BIN_DIR` | The folder for the `terminus` link, in place of `~/.local/bin`. `terminus install`, `terminus doctor` and `terminus uninstall` read it, and the hook and MCP entries install writes then name `$TERMINUS_BIN_DIR/terminus`. The versions folder stays under `~/.local/lib/crenshawdev/terminus/versions/`. |
| `CLAUDE_CONFIG_DIR` | Claude Code's config directory. When set, it's the default for [`roots`](archiving.md#roots), and it's where `terminus install` writes the hook entries (`settings.json`) and where Claude Code keeps the MCP entry (`.claude.json`). When unset, those are `~/.claude/settings.json` and `~/.claude.json`. |

The hooks and the MCP server are started by Claude Code, so they see the environment Claude Code runs in.

`install.sh` refuses to run while `TERMINUS_BIN_DIR` is set ([what install.sh checks](../reference/release.md#what-installsh-checks)).

## Claude Code settings Terminus reports

`terminus doctor` reports two Claude Code settings and never changes either. It reads them from `settings.json` in Claude Code's config directory, then `.claude/settings.json` in the current directory, then `.claude/settings.local.json` in the current directory, the last one holding the key winning. It doesn't look for the project root, so run from a subfolder of a project, it doesn't read that project's settings.

### `cleanupPeriodDays`

How many days Claude Code keeps its transcripts, 30 when unset, and at least 1. A sweep after a session starts deletes the ones older than that. Claude Code pauses the sweep when it can't determine the period, such as when a settings file won't parse, `claude -p --bare` doesn't run it, and Claude Desktop and Cowork transcripts follow their own rule. What Terminus has archived stays, and a session it never read is lost with its transcript. Below 90, `doctor` prints the edit that sets it to 3650.

### `autoCompactEnabled`

On when unset. `doctor` reports the trade-off and recommends neither value. On, Claude Code summarizes older history as context fills, which spends tokens and loses detail, and the session keeps going. Off, a session that reaches the context limit stops with `Context limit reached · /compact or /clear to continue` until you run one of them. The archive keeps the full history either way but doesn't manage active context: recall offers at most three matching turns on a prompt, and no measurement shows that replaces compaction. A `DISABLE_AUTO_COMPACT` environment variable set to anything other than `0` or `false` counts as off.
