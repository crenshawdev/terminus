# After you install

There's nothing else to set up. Terminus runs on its defaults with no configuration file, and every hook Claude Code fires starts an archive pass in the background. Start a new Claude Code session and it's working.

These checks are worth a minute the first time.

## Check the wiring

```sh
terminus doctor
```

`doctor` reports the binary, Claude Code's `settings.json`, each of the four hook entries, the MCP entry, the `claude` command, the transcript roots, the data directory, the store, the last archive pass, and two Claude Code settings. It looks like this, cut down:

```text
terminus doctor

  ok       binary                 /home/you/.local/bin/terminus is this build (2.1.0)
  ok       settings_file          /home/you/.claude/settings.json reads as JSON
  ok       hook_SessionStart      /home/you/.claude/settings.json runs /home/you/.local/bin/terminus for SessionStart in exec form
  ok       mcp_server             /home/you/.claude.json runs /home/you/.local/bin/terminus as the 'terminus' server
  ok       store                  /home/you/.local/share/terminus/terminus.db: 2 session(s), 14 turn(s), 2 watermark(s)
  note     cleanup_period_days    ...
  fix                             set "cleanupPeriodDays": 3650 in /home/you/.claude/settings.json

no problems. doctor changed nothing and never does.
```

Each check is `ok`, `note`, `unknown` or `problem`, and where there's something to run, it carries the command that fixes it. `doctor` writes nothing and exits 1 only when a check is a `problem`.

## See the first pass

`terminus install` starts archiving the transcripts already on disk, in the background. There's no log file. `terminus status` shows the store's size, its session and turn counts, and the last pass with anything that failed.

```text
store          /home/you/.local/share/terminus/terminus.db
size           200704 byte(s) (196.0 KiB)
sessions       2
turns          14
watermarks     2 covering 8554 byte(s)
last run       2026-11-04T04:57:37.265Z
  duration     6 ms
  files        2 walked, 2 committed, 0 failed
  bytes read   8554
```

A backfill that's stopped part way picks up where it left off the next time a pass runs, because each file keeps its own position.

## Decide about Claude Code's two settings

Terminus reports these two settings and never changes them. [Paths and environment](../configuration/paths.md#claude-code-settings-terminus-reports) has the full detail.

`cleanupPeriodDays` is how long Claude Code keeps its own transcripts, 30 days when it's unset. What Terminus has archived stays when Claude Code deletes the file. A session Terminus never read is lost with its transcript. Below 90 days, `doctor` prints the edit that sets it to 3650. Apply it if you'd like the extra margin; a lower number saves disk in Claude Code's folder and leaves Terminus's archive alone.

`autoCompactEnabled` is a trade-off, and `doctor` recommends neither value. On, Claude Code summarizes older history as context fills, which spends tokens and loses detail, and the session keeps going. Off, a session that reaches the context limit stops until you run `/compact` or `/clear`. Terminus keeps the full history either way, but it doesn't manage your active context, and no measurement shows that its recall replaces compaction.

## Start a session

Open Claude Code in a project you've worked on before. When the archive holds an earlier session for that project, the new session starts with a brief of where it left off. A project with no archived session gets nothing, and it starts building its history from this session on.

From here, read [what Claude gets](../working/what-claude-gets.md) to see when Claude is handed earlier work, or [read the archive yourself](../working/read-the-archive.md).
