# Keep less

By default Terminus keeps everything. Each setting below reaches some of what's stored and not the rest, so check what you actually want gone against this table.

| What you do | What it changes | What it leaves |
|---|---|---|
| Add a project to [`exclude`](../configuration/archiving.md#exclude) | The project is never read again and never returned. Its prompts aren't logged from then on. | What's already stored stays stored, just hidden. A prompt already in the injection log stays there, and snapshots keep what they hold. |
| Set [`[capture] mode`](../configuration/archiving.md#capture) to `lean` or `minimal` | New records store less tool output, and search stops finding what was only in it. | Records already stored keep what they have. |
| Set [`[retention] action`](../configuration/retention.md#retention) to `evict` | Removes an old session's archived text. | Its record and everything derived from it, its search rows and observations included, and the injection log's copies of its prompts. Snapshots. |
| Set [`[retention] action`](../configuration/retention.md#retention) to `delete` | Removes the session, everything derived from it, its observations, and its prompts and labels in the injection log, once Claude Code has deleted its transcript. | Snapshots, and the deleted bytes in the file's free pages until `terminus compact`. |
| Run [`terminus compact`](../reference/commands.md#terminus-compact) | Rebuilds the file without its free pages, clearing the bytes of deleted rows and giving the space back. | It deletes nothing itself. |
| Set [`[snapshot] enabled`](../configuration/retention.md#snapshot) to `false` | Stops new snapshots. | Snapshots already taken stay until you delete them. |
| Delete the files in `snapshots` | Removes those copies. | Nothing else is affected. |
| Run [`terminus uninstall --purge`](../getting-started/update-remove.md#delete-the-archive-too) | Deletes the whole data directory, and the injection log with it. | What's listed under [what stays behind](../getting-started/update-remove.md#what-stays-behind). |

There's no command that removes one turn or one session by hand. A secret that reached a transcript is in the archive until retention or a purge removes it, and it's in the snapshots until they age out or you delete them. Claude Code's own transcript is separate and stays in its folder until Claude Code cleans it up.

## Hide a project

```toml
exclude = ["/data/code/client-work", "~/scratch"]
```

An excluded project is skipped before any of its transcript files is opened. The filter applies to every search, recall, brief and injection, and to `export`. A path covers its subfolders.

## Age out old sessions

```toml
[retention]
action = "evict"
age_days = 365

[retention.project."/data/code/scratch"]
action = "delete"
age_days = 30
```

Retention does nothing unless both keys are set, and it acts only during an archive pass, at most 100 sessions per action per pass. Before you rely on a rule, see what it would do:

```sh
terminus retention --dry-run
```

`delete` waits for the transcript to be gone, because a transcript still on disk would be archived again on the next pass. That's usually Claude Code's own cleanup, and [`cleanupPeriodDays`](../configuration/paths.md#claude-code-settings-terminus-reports) sets when it runs.

## Filter what goes back to Claude

If the concern is what Claude is handed and not what's stored, [`[privacy] redact_recall`](../configuration/recall.md#privacy) filters text shaped like a secret on its way out. It removes nothing from the archive.
