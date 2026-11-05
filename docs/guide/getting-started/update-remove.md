# Update or remove

## Update

Run the install line again:

```sh
curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh
```

`install.sh` stores the newer version beside the older ones and moves the link to it, then runs `terminus install`. The hook entries and the MCP entry name `~/.local/bin/terminus`, which never moves, so install finds them current and rewrites neither.

- Old version folders are kept. Nothing deletes them.
- An MCP server a running session started keeps running the file it started from. The next hook call runs the new version.
- An older release that the pinned key signed installs too. There's no version floor.

Some updates change the search tables, and then the archive rebuilds them once. `terminus install` says so when it happens, and the rebuild starts in the background as the update finishes. It took about 3 minutes on a 3 GB archive. Until it's done, new sessions get no brief, prompts get no earlier turns, and search results may be incomplete. Your archived sessions aren't touched, and `terminus status` stops saying the tables predate this build when it's finished.

## Version numbers

Each release bumps the middle number of the version, a release with only fixes bumps the last, and the first moves only for a large change. The number doesn't promise compatibility. The [changelog](https://github.com/crenshawdev/terminus/blob/main/CHANGELOG.md) says what each release changes, and `terminus --version` prints the version you have.

## Uninstall

```sh
terminus uninstall
```

It removes the four hook entries from `settings.json`, the MCP entry by running `claude mcp remove --scope user terminus` when `~/.claude.json` holds one, and the link at `~/.local/bin/terminus`. When nothing but Terminus's entries changed in `settings.json` since install copied it, the copy is put back and deleted. When you edited the file since, your edited file stays, and so does the copy. A step that can't be done is reported, the rest still happen, and the command exits 1.

It keeps two folders and prints both paths:

- **The versions folder**, `~/.local/lib/crenshawdev/terminus/versions/`, because a session already running may be using one of its files.
- **The data directory**, which holds the archive ([where it is](../configuration/paths.md#the-data-directory)).

## Delete the archive too

```sh
terminus uninstall --purge
```

`--purge` also deletes the whole data directory, its snapshots and the state the hooks keep included. It shows the directory's path and the size of everything it deletes, and asks once, with no as the default answer. When the directory holds no `terminus.db`, it prints that there's nothing to purge and deletes nothing. `-y` or `--yes` answers yes without asking, and does nothing without `--purge`. With no answer to read, a store it can't read, or while an ingest is running, nothing is deleted and the command exits 1. `--purge` never deletes the versions folder.

### What stays behind

After `terminus uninstall --purge`, these are still on disk:

- `~/.local/lib/crenshawdev/terminus`, which holds the versions folder.
- The config directory, `~/.config/terminus` by default, with `terminus.toml` if you wrote one and the `data-location` file if you ran `terminus data move` ([where it is](../configuration/index.md#where-the-file-lives)).
- `settings.json.terminus-backup`, when you edited `settings.json` after install.
- `.claude.json.terminus-backup`, when one is there. Uninstall prints its path and doesn't restore it, because it holds an earlier copy of Claude Code's own state.

To remove the rest of Terminus, delete the config directory and `~/.local/lib/crenshawdev/terminus` once no session is running. Either backup file above stays where it is until you delete it. Leave `~/.local/lib/crenshawdev` itself, because Baley uses it too.

The full options are in [commands](../reference/commands.md#terminus-uninstall).
