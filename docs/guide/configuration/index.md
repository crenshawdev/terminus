# The settings file

Terminus runs on its defaults with no configuration file at all, and you don't need one to start. This reference covers the one file you can write, `terminus.toml`, every key in it with its default and what it changes, and the paths, environment variables and Claude Code settings Terminus reads from outside it.

## Where the file lives

Terminus reads `terminus.toml` from its config directory, which is the first of these that is set:

1. `$TERMINUS_CONFIG_DIR`
2. `$XDG_CONFIG_HOME/terminus`
3. `~/.config/terminus`

The order is the same on Linux and macOS. An environment variable set to the empty string counts as unset.

## How it's read

Every setting is changed by editing the file by hand, because Terminus never writes it. It reads the file each time a command, a hook or an MCP tool call runs, so there's nothing to restart.

- A missing file means every default below.
- Unknown keys are ignored, so a file written for a newer version still loads.
- A text key set to `""` means the same as the key left out. The text keys are `[retention] action`, `[retention.project."<path>"] action` and `[capture] mode`. Any other key set to `""`, whether it takes a number, true or false, or a list, fails the load with the `is not valid TOML` error below.
- A file that isn't valid TOML fails the load with `<path> is not valid TOML: <detail>`. When the parser's message names `api_key`, the excerpt of the line is left out and only the line and column are printed.
- A `[provider]` table fails the load with `<path> still has a [provider] table. Model calls were removed from Terminus, so nothing reads that table any more. Delete it from the file to continue.` Nothing in the table is read.

## Every key at its default

```toml
roots = []
exclude = []

[injection]
brief_chars = 6000
prompt_chars = 4000

[retention]
action = "keep"
age_days = 0

[snapshot]
enabled = true
interval_hours = 24
keep = 3

[capture]
mode = "full"

[privacy]
redact_recall = false
```

## Find a setting

| Key | What it changes | Page |
|---|---|---|
| `roots` | Which Claude Code folders are archived. | [What gets archived](archiving.md#roots) |
| `exclude` | Projects never read or returned. | [What gets archived](archiving.md#exclude) |
| `[capture] mode` | How much of each record is stored. | [What gets archived](archiving.md#capture) |
| `[injection] brief_chars`, `prompt_chars` | How much text a hook may hand over. | [What goes back to Claude](recall.md#injection) |
| `[privacy] redact_recall` | Whether text shaped like a secret is filtered on its way out. | [What goes back to Claude](recall.md#privacy) |
| `[retention] action`, `age_days` | How long history is kept. | [Retention and snapshots](retention.md#retention) |
| `[retention.project."<path>"]` | The same, for one project. | [Retention and snapshots](retention.md#retentionprojectpath) |
| `[snapshot] enabled`, `interval_hours`, `keep` | Rolling copies of the store. | [Retention and snapshots](retention.md#snapshot) |

Where the archive lives, the environment variables, and the two Claude Code settings Terminus reports are in [paths and environment](paths.md).
