# Install

Terminus installs with one line and checks the release's signature before it installs anything. This page covers what it needs, what the installer and `terminus install` write, and how to install a local build.

## What it needs

- Linux or macOS, on x86_64 or aarch64. Windows isn't supported, and a build for it fails at compile time. [TRUST.md](https://github.com/crenshawdev/terminus/blob/main/TRUST.md) says why.
- Claude Code. Terminus reads only Claude Code's transcripts and wires only into Claude Code.
- `curl`, to download the release.
- `ssh-keygen`, to check the release's signature. On Linux it comes in `openssh-client` on Debian and Ubuntu, `openssh-clients` on Fedora and `openssh` on Arch.
- `sha256sum`, or `shasum` where `sha256sum` is missing, to check the binary against its manifest.
- `claude` on `PATH`, when the MCP entry in `~/.claude.json` is missing or in another shape ([when `claude` runs](#when-claude-runs-and-when-install-exits-1)). Without it the MCP server isn't registered, and the install exits 1.

`git` is optional. With it, a session belongs to the repository its working directory is in, found with `git rev-parse --show-toplevel`. Without it, or when that directory isn't a repository or no longer exists, the session belongs to the working directory itself, so a session started in a subfolder of a repository is filed under that subfolder. A working directory at `<repo>/.claude/worktrees/<name>`, or below it, is treated as `<repo>`, with or without `git`.

## Install it

```sh
curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh
```

`install.sh` downloads three files from the latest GitHub release for your system: `terminus-<os>-<arch>.manifest`, `terminus-<os>-<arch>.manifest.sig` and the binary `terminus-<os>-<arch>`. It checks the manifest's signature against the key pinned in the script before it reads the manifest or downloads the binary, then checks the binary's SHA-256 against the manifest. Every check it makes, and the condition behind each refusal, is in [How a release is built and signed](../reference/release.md#what-installsh-checks).

When every check passes, it stores the binary as `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, points the link `~/.local/bin/terminus` at it, prints the version it placed and the versions it holds, and runs `terminus install`. The script exits with `terminus install`'s status.

Claude Code runs Terminus by its full path, so the hooks and the MCP server work whether or not `~/.local/bin` is on `PATH`. Typing `terminus` by name needs it there, and nothing edits your shell profile to add it.

### From another source

```sh
curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh -s -- https://<source>
```

The argument replaces the GitHub release as the place the three files come from, and must start with `https://`. The same three file names are fetched under it, and the same pinned key has to verify the manifest, so a source can only serve a release that key signed.

## What `terminus install` writes

`terminus install` asks nothing and takes no options. It checks before it writes anything, and refuses when a file it didn't put there sits at `~/.local/bin/terminus`. Then it writes:

- **The binary.** The running binary, stored under its version in `~/.local/lib/crenshawdev/terminus/versions/`, with `~/.local/bin/terminus` linked to it. When it's already running from the file the link names, as it is after `install.sh`, nothing is written.
- **The hook entries.** Four entries in Claude Code's `~/.claude/settings.json`, one each for `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`, each running `~/.local/bin/terminus hook <event>` directly, with no shell. Your own entries stay where they are. When the file isn't already in two-space form, such as a minified, four-space or CRLF file, the write that adds the entries rewrites all of it in two-space form with its keys and values unchanged, and the receipt says so. The first time it changes the file, it keeps a copy beside it as `settings.json.terminus-backup`.
- **The MCP server.** An entry named `terminus` in `~/.claude.json`, placed by running `claude mcp add-json --scope user`, so Claude Code stays the only program that writes that file. Terminus reads it and never writes it.

With `CLAUDE_CONFIG_DIR` set, both files are in that folder instead: `$CLAUDE_CONFIG_DIR/settings.json` and `$CLAUDE_CONFIG_DIR/.claude.json` ([environment variables](../configuration/paths.md#environment-variables)).

It then reports the `cleanupPeriodDays` set in that `settings.json`, reading no project's settings, and prints the same note on the trade-off of `autoCompactEnabled` whatever it's set to. It changes neither ([the two settings](../configuration/paths.md#claude-code-settings-terminus-reports)). Then it prints a receipt: the link and the version file it names, `settings.json` with whether the four entries were added or already current, `.claude.json` with how the MCP server was registered, any backup it wrote, the data directory, the transcript roots, and `terminus uninstall` as the undo. Last, it starts archiving the transcripts already on disk in a detached process and returns without waiting. `terminus status` shows that pass.

### When `claude` runs, and when install exits 1

The MCP step reads `~/.claude.json` first and decides from what is under `mcpServers.terminus`:

- An entry with `type` `stdio`, `command` the full path of `~/.local/bin/terminus` and `args` `["mcp"]` is current, and no `claude` runs. Other keys beside those three, such as `env`, don't matter.
- No entry: `claude mcp add-json --scope user terminus '<entry>'` runs.
- An entry in any other shape: `claude mcp remove --scope user terminus` runs, then the add. Keys you put beside the three are kept in the new entry. A remove that fails stops the add.
- `~/.claude.json` can't be read: no `claude` runs.

`claude` is found on `PATH`, run by its full path with no shell, with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` set in its environment, and given 30 seconds. When the file can't be read, `claude` isn't on `PATH` or can't be started, takes longer than 30 seconds, or exits non-zero, the registration is incomplete. Install still does everything else, the receipt opens with `install incomplete: the mcp server is not registered`, prints the reason and the exact command that finishes it, and install exits 1.

Install also exits 1 when the store in the data directory is in an older archive format than this build reads. It prints the refusal with the steps for a clean install, and nothing in the store is changed. A store in a newer archive format, which installing an older release leaves behind, prints `problem: the backfill could not be sized (...)` with the reason inside the brackets, and doesn't by itself make install exit 1.

Run it again at any time. It writes nothing to a file that is already current.

## A local build

```sh
cargo build --release && target/release/terminus install
```

`terminus install` stores the binary it is running from under that build's version in `~/.local/lib/crenshawdev/terminus/versions/` and moves the link to it, the same as after `install.sh`. A rebuild at the same version replaces that version's file, and a process already running the old file keeps it. A local build isn't checked against any signature.

Once it's installed, see [what to do next](after-install.md).
