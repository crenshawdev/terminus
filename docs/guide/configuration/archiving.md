# What gets archived

These settings decide which transcripts Terminus reads and how much of each record it stores. [Keep less](../archive/keep-less.md) compares them with retention and snapshots.

## `roots`

**Default:** `[]` · **Type:** list of paths.

The Claude Code config directories whose transcripts Terminus archives. Each root's `projects` folder is walked. The empty default means `$CLAUDE_CONFIG_DIR`, otherwise `~/.claude`. A list here replaces the default and outranks `CLAUDE_CONFIG_DIR`. Paths are used exactly as written, and `~` isn't expanded in this key.

Set it when you run Claude Code under more than one config directory.

```toml
roots = ["/home/you/.claude", "/home/you/.claude-work"]
```

## `exclude`

**Default:** `[]` · **Type:** list of paths.

Project paths Terminus never reads and never returns. A project listed here is skipped before any of its transcript files is opened, and anything already archived from it is left out of every search, recall, brief and injection. Its prompts aren't written to the injection log, including those of a session that started in it and moved elsewhere with `cd`. Retention passes over its sessions too.

A path covers its subfolders, so `/data/code/demo` covers `/data/code/demo/sub` and doesn't cover `/data/code/demo-other`. A leading `~` is expanded, `..` is resolved, and a trailing `/` makes no difference. An entry that isn't an absolute path once `~` is expanded is dropped without a message.

```toml
exclude = ["/data/code/client-work", "~/scratch"]
```

Adding an entry deletes nothing that is already stored:

- A prompt still queued for the injection log is removed at the next ingest pass without being logged, and `terminus status` shows how many under that pass's notes.
- A prompt already in the injection log stays there. No command prints it, `terminus stats` still counts it and `terminus replay` scores it as injecting nothing. `terminus uninstall --purge` is the only command that removes it.
- [Snapshots](retention.md#snapshot) keep what they already hold.

## `[capture]`

How much of each transcript record the archive stores.

### `mode`

**Default:** `"full"` · **Type:** text · **Allowed values:** `"full"`, `"lean"`, `"minimal"`. Any other value means `full`, so a typo can't start throwing away tool output.

- `full` stores every byte. Only this mode reproduces the transcript exactly.
- `lean` replaces a record's tool output (`toolUseResult`) or attachment with a note of its size when it's over 8,192 bytes.
- `minimal` replaces every record's top-level `toolUseResult` and `attachment` that way, whatever their size, and keeps the prompts, Claude's replies, the tool names and the tool arguments. A `tool_result` block inside `message.content` is stored as it is.

Under `lean` and `minimal`, text that was only in the replaced output is no longer found by search. The mode applies as records are archived. Records already stored keep what they have.

```toml
[capture]
mode = "lean"
```
