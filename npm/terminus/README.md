# terminus

Persistent, cross-session memory for Claude Code. Claude Code already writes
every prompt, tool call, tool result and assistant turn to
`~/.claude/projects/**/*.jsonl`. Terminus tails those files, stores each session
unmodified and permanently, indexes it at turn granularity, and gives the model
precise recall over its own history.

A single static Rust binary. Local-only: no daemon, no port, no telemetry, and no
network except a model provider you configure yourself.

## Install

```
npx terminus install
```

That is two steps in one line, and the split is deliberate:

1. `npx` downloads this package and the one prebuilt binary that matches your
   platform, and runs nothing else. **There is no postinstall script** - no
   `postinstall`, no `preinstall`, no `install`, not even an empty one. Nothing
   executes on your machine as a side effect of downloading.
2. `terminus install` is a command you run. It copies the binary to a stable
   path, registers Terminus's hooks in `~/.claude/settings.json` and its MCP
   server in `~/.claude.json`, backs up both files first, and tells you what it
   changed.

Install is something you do, never something that happens to you.

To see what is installed and healthy without changing anything:

```
npx terminus doctor
```

`doctor` is read-only. It never repairs, and it never creates the data
directory in order to check it.

To remove it:

```
terminus uninstall
```

## How the package is shaped

`terminus` is a thin package. Its only payload is `bin/terminus.js`, a shim.
The actual executables live in five per-platform packages
(`@terminus/linux-x64` and its siblings), declared here as
`optionalDependencies` and marked with `os` and `cpu` so npm downloads exactly
the one that matches your machine and skips the other four.

The shim exists because npm's `bin` field can only point at a path inside the
declaring package, so it cannot point into an optional dependency. The shim
resolves the platform package, execs the binary with your arguments, and exits
with the binary's own exit code. It runs at install time only: once
`terminus install` has placed the binary at its stable path, Claude Code's hooks
call that path directly and Node is never on the runtime path.

## License

Apache-2.0.
