# Configuration

Terminus runs on its defaults with no configuration file at all. This page covers the one file you can write, `terminus.toml`, every key in it with its default and what it changes, and the paths, environment variables and Claude Code settings Terminus reads from outside it.

## The file

Terminus reads `terminus.toml` from its config directory, which is the first of these that is set:

1. `$TERMINUS_CONFIG_DIR`
2. `$XDG_CONFIG_HOME/terminus`
3. `~/.config/terminus`

The order is the same on Linux and macOS. An environment variable set to the empty string counts as unset.

- A missing file means every default below.
- Terminus never writes the file. Every setting is changed by editing it by hand.
- Unknown keys are ignored, so a file written for a newer version still loads.
- A key set to `""` means the same as a key left out.
- A file that is not valid TOML fails the load with `<path> is not valid TOML: <detail>`. When the parser's message names `api_key`, the excerpt of the line is left out and only the line and column are printed.
- A `[provider]` table fails the load with `<path> still has a [provider] table. Model calls were removed from Terminus, so nothing reads that table any more. Delete it from the file to continue.` Nothing in the table is read.

A file with every key at its default:

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

## Settings

### `roots`

Default: an empty list, which means `$CLAUDE_CONFIG_DIR`, otherwise `~/.claude`.

The Claude Code config directories whose transcripts Terminus archives. Each root's `projects` folder is walked. A list here replaces the default and outranks `CLAUDE_CONFIG_DIR`. Paths are used exactly as written, and `~` is not expanded in this key.

### `exclude`

Default: none.

Project paths Terminus never reads and never returns. A project listed here is skipped before any of its transcript files is opened, and anything already archived from it is left out of every search, recall, brief and injection. Retention passes over its sessions too. A path covers its subfolders, so `/data/code/demo` covers `/data/code/demo/sub` and does not cover `/data/code/demo-other`. A leading `~` is expanded, `..` is resolved, and a trailing `/` makes no difference.

```toml
exclude = ["/data/code/client-work", "~/scratch"]
```

### `[injection]`

How much text Terminus hands Claude Code when a hook fires. Both counts are characters, not tokens, and each is capped at 10,000 whatever the file says, because Claude Code writes a hook's output longer than 10,000 characters to a file and gives the model the file's path instead of the text.

- `brief_chars`, default 6000. The most the brief at the start of a session may carry. The brief says where the last session in the project left off: its date, the branch it ended on, the last thing said in each direction, and how much of the project is archived. A smaller number shortens the quotes first.
- `prompt_chars`, default 4000. The most one prompt's injection may carry. An injection quotes at most three earlier turns that match the paths and identifiers the prompt names, and most prompts get none.

### `[retention]`

How much of each project's history the archive keeps. The default keeps everything.

- `action`, default `keep`. One of `keep`, `evict` or `delete`. Any other value means `keep`.
  - `keep` does nothing.
  - `evict` removes a session's archived text and keeps the session's record and everything derived from it.
  - `delete` removes the session and everything derived from it, its observations included, which `terminus reindex` cannot rebuild. It acts only once Claude Code has already deleted the session's transcript, because a transcript still on disk would be archived again on the next pass.
- `age_days`, default 0. How many days old a session's last turn must be before `action` applies. 0 is off, and a negative number counts as 0.

Retention does nothing unless both keys are set: an `action` other than `keep` and an `age_days` above 0. It acts only on sessions Terminus has already closed, during the ingest pass each hook starts, at most 100 sessions per action per pass. The rest are counted and taken on the next pass. `terminus retention --dry-run` lists what the current rule would act on and changes nothing.

### `[retention.project."<path>"]`

The same two keys, `action` and `age_days`, for one project. The table replaces the global rule for every session under that path, so a key it leaves out is at its default, not at the global table's value. Where two tables cover the same session, the one with the longer path wins. The path is written and matched the way `exclude` is.

```toml
[retention]
action = "evict"
age_days = 365

[retention.project."/data/code/scratch"]
action = "delete"
age_days = 30
```

### `[snapshot]`

Rolling copies of the archive, kept in the `snapshots` folder of the data directory. On by default.

- `enabled`, default `true`. `false` stops new snapshots. Snapshots already taken stay.
- `interval_hours`, default 24. How long after the last snapshot the next one is taken, checked on each ingest pass. 0 counts as 24, because a snapshot on every pass would write a full copy of the archive on every prompt.
- `keep`, default 3. How many snapshots are kept. After each new one, the oldest past this number are deleted. The lowest value is 1; to keep none, set `enabled = false`.

### `[capture]`

How much of each transcript record the archive stores.

- `mode`, default `full`. One of `full`, `lean` or `minimal`. Any other value means `full`.
  - `full` stores every byte. Only this mode reproduces the transcript exactly.
  - `lean` replaces a record's tool output (`toolUseResult`) or attachment with a note of its size when it is over 8,192 bytes.
  - `minimal` replaces every record's tool output and attachments that way, and keeps the prompts, Claude's replies, the tool names and the tool arguments.

Under `lean` and `minimal`, text that was only in the replaced output is no longer found by search. The mode applies as records are archived. Records already stored keep what they have.

### `[privacy]`

- `redact_recall`, default `false`. With `true`, archived text is filtered on its way into Claude Code: the brief, each prompt's injection, the MCP tools' results, and the excerpts `terminus search` and `terminus show` print. The filter replaces private key blocks, passwords in connection URLs, values named like secrets in headers, JSON, assignments and command-line flags, JSON Web Tokens and GitHub tokens with a marker that names what was taken out. The archive itself is never filtered, and setting it back to `false` returns everything unfiltered. `terminus search --raw` and `terminus show --raw` print unfiltered text for that one run.

Only the file sets this. No environment variable or flag turns it on, and `--raw` affects only the command it is passed to, never the hooks or the MCP server.

## Paths outside terminus.toml

### The data directory

The archive, its snapshots and the state the hooks keep live in the data directory. Terminus uses the first of these that is set:

1. `TERMINUS_DATA_DIR`.
2. The location `terminus data move` recorded, in a `data-location` file in the config directory. A recorded path must be absolute; a relative one, or a file that exists and cannot be read, is an error rather than a fall back to the default.
3. The platform location: `$XDG_DATA_HOME/terminus`, otherwise `~/.local/share/terminus` on Linux, and `~/Library/Application Support/terminus` on macOS.

`TERMINUS_DATA_DIR` outranks a location `terminus data move` recorded.

### Environment variables

- `TERMINUS_DATA_DIR`: the data directory, as above.
- `TERMINUS_CONFIG_DIR`: the config directory, as above. It holds `terminus.toml` and the `data-location` file.
- `CLAUDE_CONFIG_DIR`: Claude Code's config directory. When set, it is the default for `roots`, and it is where `terminus install` writes the hook entries (`settings.json`) and where Claude Code keeps the MCP entry (`.claude.json`). When unset, those are `~/.claude/settings.json` and `~/.claude.json`.

The hooks and the MCP server are started by Claude Code, so they see the environment Claude Code runs in.

`install.sh` refuses to run while `TERMINUS_BIN_DIR` is set ([how a release is checked](release.md#what-installsh-checks)).

### Claude Code settings Terminus reports

`terminus doctor` reports two Claude Code settings and never changes either. It reads them from `settings.json` in Claude Code's config directory, then the project's `.claude/settings.json`, then the project's `.claude/settings.local.json`, the last one holding the key winning.

- `cleanupPeriodDays`. Claude Code deletes its transcripts this many days after they are written, 30 when unset. What Terminus has archived stays, and a session it never read is lost with its transcript. Below 90, `doctor` prints the edit that sets it to 3650.
- `autoCompactEnabled`. On when unset. `doctor` recommends off, because compaction spends tokens summarizing context the archive already holds, and prints the edit that sets it to `false`. A `DISABLE_AUTO_COMPACT` environment variable set to anything other than `0` or `false` counts as off.
