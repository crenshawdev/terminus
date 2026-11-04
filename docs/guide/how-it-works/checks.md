# Checks and their limits

Some of what Terminus promises is checked by code and some of it isn't. You should be able to tell which one you're relying on.

| Check | What it establishes | Its limit |
|---|---|---|
| `terminus verify` | Every block still matches the hash recorded for it, and each session's checksum matches its block hashes. | It catches corruption and isn't tamper evidence, since the hashes are unkeyed and live in the same file. It doesn't check SQLite's own pages or a session Terminus never read. |
| `terminus doctor` | The hooks, the MCP entry, the store and the last pass look right, and it reports two Claude Code settings. | It reads and reports. It can't show that a hook fired in a given session, or that recall helped. |
| `terminus stats` | How often a later turn in a session named the same thing as a turn handed over. | That's a proxy for usefulness and doesn't prove the turn handed over is why. |
| The no-network test | No source file in either crate names a socket type, comment lines included. | It reads Terminus's own source for four names. It doesn't scan dependencies or run the binary under a network monitor. |
| The release signature | The manifest was signed by the pinned key, and through its SHA-256, the binary it names. | `install.sh` itself is unsigned, and an older signed release still installs. [The release page](../reference/release.md#what-the-signature-does-not-protect) has the full list. |
| `redact_recall` | Secrets of the shapes the filter knows don't reach Claude Code through a hook or an MCP tool, when it's on. | A secret in another shape passes. It's off by default, and it never touches the store. |
| `exclude` | The project isn't read, returned or logged from then on. | What's already stored stays stored, and snapshots keep what they hold. |
| The test suite | The tests pass on the debug and the release build, on Linux and macOS, on x86_64 and aarch64. | CI skips a hook spawn's p99 tests and, on macOS, three spawn time budgets, because a hosted VM's timing measures the VM. The tests over a private transcript corpus skip themselves without that data. CI doesn't run the fuzz targets. |

## Recall is an offer

Nothing makes Claude use what it's handed, or reach for the MCP tools. No measurement shows recall replaces compaction, and Terminus doesn't manage your active context. The handed-over text says it's history and not instructions, and that framing is a statement to the model rather than a boundary it can't cross.

## Compatibility

The version number doesn't promise compatibility. A store in an older archive format than the build reads is refused, and the refusal prints the steps for a clean install. It's never converted. The [changelog](https://github.com/jcrenshaw-dev/terminus/blob/main/CHANGELOG.md) says what each release changes.

## How the claims are held to

[SECURITY.md](https://github.com/jcrenshaw-dev/terminus/blob/main/SECURITY.md) covers what's in scope for a report, the fuzz targets, and the one `unsafe` block under Miri. Every fixed finding gets a regression test that fails before the fix and passes after.
