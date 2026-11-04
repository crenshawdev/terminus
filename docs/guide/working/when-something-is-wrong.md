# When something is wrong

| What you're seeing | Start with |
|---|---|
| New sessions start with no brief, or Claude has no recall tools | [`terminus doctor`](#is-it-wired-in) |
| A session is missing from the archive | [`terminus status`](#did-the-last-pass-work) |
| You suspect the archive is damaged | [`terminus verify`](#is-the-archive-intact) |
| Search results look wrong | [`terminus reindex`](#rebuild-the-search-index) |
| A read command warns that the derived tables predate this build | `terminus ingest`, which rebuilds them. |
| You need to go back to an earlier copy of the store | [Restore a snapshot](#restore-a-snapshot) |
| Most prompts get nothing | That's expected. See [what Claude gets](what-claude-gets.md#on-a-prompt). |

## Is it wired in?

`terminus doctor` reports what is and isn't wired into Claude Code: the hook entries, the MCP entry, the store and the last ingest pass. Where there's a fix to run, it prints the command. It writes nothing and repairs nothing. Running the install line again, or `terminus install`, puts back a missing entry and leaves current ones alone.

## Did the last pass work?

`terminus status` prints the last pass: when it started, how long it took, the files walked, archived and failed, the bytes read and the turns added, then its notes and errors. A pass's failures are recorded there and nowhere else, since there's no log file. It takes no lock, so it works while a pass runs.

Only `<uuid>.jsonl` files directly inside a project folder, and `agent-*.jsonl` files below one, count as transcripts. A project listed in `exclude` is skipped, and a folder under a root that can't be listed is reported and skipped.

To retry one file and see what happens, run `terminus ingest <path.jsonl>`. A file that can't be archived is named under `error` in `terminus status`, and the command exits 1.

## Is the archive intact?

`terminus verify` checks every session's archived blocks against the hash recorded for each, and each session's checksum over those hashes. The sessions that fail are printed, and it exits 1 when any session fails.

It also reports a session whose transcript on disk has become shorter than what the archive holds. That's a statement about the file, and the archive is intact. An evicted session has no blocks to check, so it's only checked for having none left.

## Rebuild the search index

`terminus reindex` rebuilds the search index and the other tables derived from the archived blocks, with the same turn ids. It doesn't touch the archive, the observations or the injection log. An evicted session keeps its rows as they are, because its blocks are gone. It takes the ingest lock and exits 1 when another process holds it.

Observations have their own rebuild, `terminus observations regenerate`. The injection log has none.

## Restore a snapshot

Terminus keeps rolling copies of the store in the `snapshots` folder of the [data directory](../configuration/paths.md#the-data-directory), by default the last three, taken at most once every 24 hours. Each one is a complete SQLite database. There's no restore command, so you put one back by hand.

Close every Claude Code session first, because each hook starts a pass. Then, from the data directory:

```sh
rm -f terminus.db terminus.db-wal terminus.db-shm
cp -p snapshots/terminus-<time>.db terminus.db
terminus verify
terminus ingest
```

The last line archives whatever is still on disk that the snapshot doesn't hold, because each transcript's position is stored in the store. A session whose transcript Claude Code has since deleted can't come back that way, and neither can anything else written after the snapshot, such as the injection log's newer rows.

## Move the store

`terminus data move <path>` moves the whole data directory and records the new location. The store isn't safe on a network filesystem, and the command warns you about that. The [command reference](../reference/commands.md#terminus-data-move-path) lists what it refuses.
