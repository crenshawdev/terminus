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
- A text key set to `""` means the same as the key left out. The text keys are `[retention] action`, `[retention.project."<path>"] action` and `[capture] mode`. Any other key set to `""`, whether it takes a number, true or false, or a list, fails the load with the `is not valid TOML` error below.
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

Project paths Terminus never reads and never returns. A project listed here is skipped before any of its transcript files is opened, and anything already archived from it is left out of every search, recall, brief and injection. Its prompts are not written to the injection log, including those of a session that started in it and moved elsewhere with `cd`. Retention passes over its sessions too. A path covers its subfolders, so `/data/code/demo` covers `/data/code/demo/sub` and does not cover `/data/code/demo-other`. A leading `~` is expanded, `..` is resolved, and a trailing `/` makes no difference. An entry that is not an absolute path once `~` is expanded is dropped without a message.

```toml
exclude = ["/data/code/client-work", "~/scratch"]
```

Adding an entry deletes nothing that is already stored:

- A prompt still queued for the injection log is removed at the next ingest pass without being logged, and `terminus status` shows how many under that pass's notes.
- A prompt already in the injection log stays there. No command prints it, `terminus stats` still counts it and `terminus replay` scores it as injecting nothing. `terminus uninstall --purge` is the only command that removes it.
- [Snapshots](#snapshot) keep what they already hold.

### `[injection]`

How much text Terminus hands Claude Code when a hook fires. Both counts are characters, not tokens, and each is capped at 10,000 whatever the file says, because Claude Code writes a hook's output longer than 10,000 characters to a file and gives the model the file's path instead of the text.

- `brief_chars`, default 6000. The most the brief at the start of a session may carry. The brief says where the last session in the project left off: its date, the branch it ended on, the last thing said in each direction, and how much of the project is archived. A smaller number shortens the quotes first.
- `prompt_chars`, default 4000. The most one prompt's injection may carry. An injection quotes at most three earlier turns that match the paths and identifiers the prompt names, and most prompts get none. After a compaction, the next such prompt looks only at the turns the compaction dropped, under the same limits, so it does not restore everything that dropped.

### `[retention]`

How much of each project's history the archive keeps. The default keeps everything.

- `action`, default `keep`. One of `keep`, `evict` or `delete`. Any other value means `keep`.
  - `keep` does nothing.
  - `evict` removes a session's archived text and keeps the session's record and everything derived from it, its search rows and observations included. The injection log keeps its copies of the session's prompts.
  - `delete` removes the session, everything derived from it, its observations, and the prompts and labels the injection log holds for it. `terminus reindex` cannot rebuild the observations or the log, and `terminus stats` and `terminus replay` stop counting those prompts. A subagent's session reports its parent's session id, so deleting one removes no prompts, and deleting the parent removes them. It acts only once the session's transcript is gone, which is usually Claude Code's cleanup, because a transcript still on disk would be archived again on the next pass.
- `age_days`, default 0. How many days old a session's last turn must be before `action` applies. 0 is off, and a negative number counts as 0.

Retention does nothing unless both keys are set: an `action` other than `keep` and an `age_days` above 0. It acts only on sessions Terminus has already closed, during the ingest pass each hook starts, at most 100 sessions per action per pass. The rest are counted and taken on the next pass. `terminus retention --dry-run` lists what the current rule would act on and changes nothing.

A deleted row's bytes can stay in the store file's free pages until [`terminus compact`](commands.md#terminus-compact) rebuilds the file. Prompts left behind by deletions under a version before this rule stay in the injection log. [Snapshots](#snapshot) are separate copies, and neither action reaches them.

### `[retention.project."<path>"]`

The same two keys, `action` and `age_days`, for one project. The table replaces the global rule for every session under that path, so a key it leaves out is at its default, not at the global table's value. Where two tables cover the same session, the one with the longer path wins. The path is written the way `exclude` is, and one that is not absolute is dropped the same way. It is matched only as written: an `exclude` entry also matches the path it resolves to through a symlink, and a retention path does not.

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

A snapshot is a copy of the store as it stood when it was taken. `exclude` and `[retention]` change the store and never a snapshot, so a snapshot keeps what was excluded or deleted after it was taken, prompts included, until the next snapshots push it past `keep`. With `enabled = false` no new snapshot pushes them out, so they stay until you delete them. To remove that data sooner, delete the files in the `snapshots` folder.

### `[capture]`

How much of each transcript record the archive stores.

- `mode`, default `full`. One of `full`, `lean` or `minimal`. Any other value means `full`.
  - `full` stores every byte. Only this mode reproduces the transcript exactly.
  - `lean` replaces a record's tool output (`toolUseResult`) or attachment with a note of its size when it is over 8,192 bytes.
  - `minimal` replaces every record's top-level `toolUseResult` and `attachment` that way, whatever their size, and keeps the prompts, Claude's replies, the tool names and the tool arguments. A `tool_result` block inside `message.content` is stored as it is.

Under `lean` and `minimal`, text that was only in the replaced output is no longer found by search. The mode applies as records are archived. Records already stored keep what they have.

### `[privacy]`

- `redact_recall`, default `false`. With `true`, archived text is filtered on its way into Claude Code: the brief, each prompt's injection, the MCP tools' results, and the excerpts `terminus search` and `terminus show` print. The filter replaces private key blocks, passwords in connection URLs, values named like secrets in headers, JSON, assignments and command-line flags, JSON Web Tokens, GitHub tokens and bare `sk-` keys. A value named like a secret in a header, JSON or an assignment becomes a plain `[redacted]`, and the rest become a marker that names what was taken out, such as `[redacted:a-github-token]`. Values named like secrets inside a tool's input, a tool's result and an attachment become `[redacted]` too. The archive itself is never filtered, and setting it back to `false` returns everything unfiltered. `terminus search --raw` and `terminus show --raw` print unfiltered text for that one run.

Only the file sets this. No environment variable or flag turns it on, and `--raw` affects only the command it is passed to, never the hooks or the MCP server.

## Paths outside terminus.toml

### The data directory

The archive, its snapshots and the state the hooks keep live in the data directory. Terminus uses the first of these that is set:

1. `TERMINUS_DATA_DIR`.
2. The location `terminus data move` recorded, in a `data-location` file in the config directory. A recorded path must be absolute; a relative one, or a file that exists and cannot be read, is an error rather than a fall back to the default.
3. The platform location. On Linux it is `$XDG_DATA_HOME/terminus`, otherwise `~/.local/share/terminus`. On macOS it is always `~/Library/Application Support/terminus`, and `XDG_DATA_HOME` is not read.

`TERMINUS_DATA_DIR` outranks a location `terminus data move` recorded.

### Environment variables

- `TERMINUS_DATA_DIR`: the data directory, as above.
- `TERMINUS_CONFIG_DIR`: the config directory, as above. It holds `terminus.toml` and the `data-location` file.
- `TERMINUS_BIN_DIR`: the folder for the `terminus` link, in place of `~/.local/bin`. `terminus install`, `terminus doctor` and `terminus uninstall` read it, and the hook and MCP entries install writes then name `$TERMINUS_BIN_DIR/terminus`. The versions folder stays under `~/.local/lib/crenshawdev/terminus/versions/`.
- `CLAUDE_CONFIG_DIR`: Claude Code's config directory. When set, it is the default for `roots`, and it is where `terminus install` writes the hook entries (`settings.json`) and where Claude Code keeps the MCP entry (`.claude.json`). When unset, those are `~/.claude/settings.json` and `~/.claude.json`.

The hooks and the MCP server are started by Claude Code, so they see the environment Claude Code runs in.

`install.sh` refuses to run while `TERMINUS_BIN_DIR` is set ([how a release is checked](release.md#what-installsh-checks)).

### Claude Code settings Terminus reports

`terminus doctor` reports two Claude Code settings and never changes either. It reads them from `settings.json` in Claude Code's config directory, then `.claude/settings.json` in the current directory, then `.claude/settings.local.json` in the current directory, the last one holding the key winning. It does not look for the project root, so run from a subfolder of a project, it does not read that project's settings.

- `cleanupPeriodDays`. How many days Claude Code keeps its transcripts, 30 when unset, and at least 1. A sweep after a session starts deletes the ones older than that. Claude Code pauses the sweep when it cannot determine the period, such as when a settings file will not parse, `claude -p --bare` does not run it, and Claude Desktop and Cowork transcripts follow their own rule. What Terminus has archived stays, and a session it never read is lost with its transcript. Below 90, `doctor` prints the edit that sets it to 3650.
- `autoCompactEnabled`. On when unset. `doctor` reports the trade-off and recommends neither value. On, Claude Code summarizes older history as context fills, which spends tokens and loses detail, and the session keeps going. Off, a session that reaches the context limit stops with `Context limit reached · /compact or /clear to continue` until you run one of them. The archive keeps the full history either way but does not manage active context: recall offers at most three matching turns on a prompt, and no measurement shows that replaces compaction. A `DISABLE_AUTO_COMPACT` environment variable set to anything other than `0` or `false` counts as off.
