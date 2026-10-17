# Installing, updating and uninstalling

Terminus installs with one line, checks the release's signature before it installs anything, and updates by running the same line again. This page covers what it needs, what the installer and `terminus install` write, a local build, updating, and what `terminus uninstall` takes out and leaves behind.

## What it needs

- Linux or macOS, on x86_64 or aarch64.
- `curl`, to download the release.
- `ssh-keygen`, to check the release's signature. On Linux it comes in `openssh-client` on Debian and Ubuntu, `openssh-clients` on Fedora and `openssh` on Arch.
- `sha256sum`, or `shasum` where `sha256sum` is missing, to check the binary against its manifest.
- `claude` on `PATH`. Without it the MCP server is not registered, and the install exits 1.

`git` is optional. With it, a session belongs to the repository its working directory is in, found with `git rev-parse --show-toplevel`. Without it, or when that directory is not a repository or no longer exists, the session belongs to the working directory itself, so a session started in a subfolder of a repository is filed under that subfolder.

## Installing

```sh
curl -fsSL https://raw.githubusercontent.com/jcrenshaw-dev/terminus/main/install.sh | sh
```

`install.sh` downloads three files from the latest GitHub release for your system: `terminus-<os>-<arch>.manifest`, `terminus-<os>-<arch>.manifest.sig` and the binary `terminus-<os>-<arch>`. It checks the manifest's signature against the key pinned in the script before it reads the manifest or downloads the binary, then checks the binary's SHA-256 against the manifest. Every check it makes, and the condition behind each refusal, is in [How a release is built, signed and checked](release.md#what-installsh-checks).

When every check passes, it stores the binary as `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, points the link `~/.local/bin/terminus` at it, prints the version it placed and the versions it holds, and runs `terminus install`. The script exits with `terminus install`'s status.

Claude Code runs Terminus by its full path, so the hooks and the MCP server work whether or not `~/.local/bin` is on `PATH`. Typing `terminus` by name needs it there, and nothing edits your shell profile to add it.

### From another source

```sh
curl -fsSL https://raw.githubusercontent.com/jcrenshaw-dev/terminus/main/install.sh | sh -s -- https://<source>
```

The argument replaces the GitHub release as the place the three files come from, and must start with `https://`. The same three file names are fetched under it, and the same pinned key has to verify the manifest, so a source can only serve a release that key signed.

### What `terminus install` writes

`terminus install` asks nothing and takes no options. It checks before it writes anything, and refuses when a file it did not put there sits at `~/.local/bin/terminus`. Then it writes:

- **The binary.** The running binary, stored under its version in `~/.local/lib/crenshawdev/terminus/versions/`, with `~/.local/bin/terminus` linked to it. When it is already running from the file the link names, as it is after `install.sh`, nothing is written.
- **The hook entries.** Four entries in Claude Code's `~/.claude/settings.json`, one each for `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`, each running `~/.local/bin/terminus hook <event>` directly, with no shell. Your own entries stay where they are. The first time it changes the file, it keeps a copy beside it as `settings.json.terminus-backup`.
- **The MCP server.** An entry named `terminus` in `~/.claude.json`, placed by running `claude mcp add-json --scope user`, so Claude Code stays the only program that writes that file. Terminus reads it and never writes it.

With `CLAUDE_CONFIG_DIR` set, both files are in that folder instead: `$CLAUDE_CONFIG_DIR/settings.json` and `$CLAUDE_CONFIG_DIR/.claude.json` ([configuration](configuration.md#environment-variables)).

It then reports `cleanupPeriodDays` and `autoCompactEnabled`, which it never changes ([configuration](configuration.md#claude-code-settings-terminus-reports)), and prints a receipt: the link and the version file it names, `settings.json` with whether the four entries were added or already current, `.claude.json` with how the MCP server was registered, any backup it wrote, the data directory, the transcript roots, and `terminus uninstall` as the undo. Last, it starts archiving the transcripts already on disk in a detached process and returns without waiting. `terminus status` shows that pass.

### When `claude` runs, and when install exits 1

The MCP step reads `~/.claude.json` first and decides from what is under `mcpServers.terminus`:

- An entry with `type` `stdio`, `command` the full path of `~/.local/bin/terminus` and `args` `["mcp"]` is current, and no `claude` runs. Other keys beside those three, such as `env`, do not matter.
- No entry: `claude mcp add-json --scope user terminus '<entry>'` runs.
- An entry in any other shape: `claude mcp remove --scope user terminus` runs, then the add. Keys you put beside the three are kept in the new entry. A remove that fails stops the add.
- `~/.claude.json` cannot be read: no `claude` runs.

`claude` is found on `PATH`, run by its full path with no shell, and given 30 seconds. When the file cannot be read, `claude` is not on `PATH` or cannot be started, takes longer than 30 seconds, or exits non-zero, the registration is incomplete. Install still does everything else, the receipt opens with `install incomplete: the mcp server is not registered`, prints the reason and the exact command that finishes it, and install exits 1.

Install also exits 1 when the store in the data directory is in an older archive format than this build reads. It prints the refusal with the steps for a clean install, and nothing in the store is changed. A store in a newer archive format, which installing an older release leaves behind, prints `problem: the backfill could not be sized (...)` with the reason inside the brackets, and does not by itself make install exit 1.

Run it again at any time. It writes nothing to a file that is already current.

## A local build

```sh
cargo build --release && target/release/terminus install
```

`terminus install` stores the binary it is running from under that build's version in `~/.local/lib/crenshawdev/terminus/versions/` and moves the link to it, the same as after `install.sh`. A rebuild at the same version replaces that version's file, and a process already running the old file keeps it. A local build is not checked against any signature.

## Updating

Run the install line again. `install.sh` stores the newer version beside the older ones and moves the link to it, then runs `terminus install`. The hook entries and the MCP entry name `~/.local/bin/terminus`, which never moves, so install finds them current and rewrites neither.

- Old version folders are kept. Nothing deletes them.
- An MCP server a running session started keeps running the file it started from. The next hook call runs the new version.
- An older release that the pinned key signed installs too. There is no version floor.

## Uninstalling

```sh
terminus uninstall
```

It removes the four hook entries from `settings.json`, the MCP entry by running `claude mcp remove --scope user terminus` when `~/.claude.json` holds one, and the link at `~/.local/bin/terminus`. When nothing but Terminus's entries changed in `settings.json` since install copied it, the copy is put back and deleted. When you edited the file since, your edited file stays, and so does the copy. A step that cannot be done is reported, the rest still happen, and the command exits 1.

It keeps two folders and prints both paths:

- **The versions folder**, `~/.local/lib/crenshawdev/terminus/versions/`, because a session already running may be using one of its files.
- **The data directory**, which holds the archive ([where it is](configuration.md#the-data-directory)).

`terminus uninstall --purge` also deletes the data directory. It shows the directory's path and size and asks once, with no as the default answer. `-y` or `--yes` answers yes without asking, and does nothing without `--purge`. With no answer to read, or while an ingest is running, nothing is deleted and the command exits 1. `--purge` never deletes the versions folder.

The full options are in [commands](commands.md#terminus-uninstall).

### What stays behind

After `terminus uninstall --purge`, these are still on disk:

- `~/.local/lib/crenshawdev/terminus`, which holds the versions folder.
- The config directory, `~/.config/terminus` by default, with `terminus.toml` if you wrote one and the `data-location` file if you ran `terminus data move` ([where it is](configuration.md#the-file)).
- `settings.json.terminus-backup`, when you edited `settings.json` after install.
- `.claude.json.terminus-backup`, when an older install left one. Uninstall prints its path and does not restore it, because it holds an older copy of Claude Code's own state.

To remove the rest of Terminus, delete the config directory and `~/.local/lib/crenshawdev/terminus` once no session is running. Either backup file above stays where it is until you delete it. Leave `~/.local/lib/crenshawdev` itself, because Baley uses it too.
