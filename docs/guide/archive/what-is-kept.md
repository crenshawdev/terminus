# What's kept

Terminus keeps the session: every prompt, every reply, every tool call and its output, with nothing summarized. No model decides what's worth keeping, so a detail you didn't know you'd want is still there three weeks later.

## The transcript, in blocks

Claude Code writes each session to a `.jsonl` transcript. Terminus finds them by file name: a `<uuid>.jsonl` directly inside a project folder, or an `agent-*.jsonl` anywhere below one, which is a subagent's. Nothing else in Claude Code's folder is opened.

Each pass reads a file from where the last pass stopped. The new bytes are cut into 64 KiB blocks, each compressed with zstd and stored in one SQLite file. A pass commits each file in one transaction, so a pass that's killed partway leaves each transcript as its last commit left it, and recovery at the start of the next pass loses no archived byte.

Under the default [`[capture] mode`](../configuration/archiving.md#capture) of `full`, the archived bytes are the transcript exactly. `lean` and `minimal` trade that away for space.

## What sits beside the blocks

| In the store | What it holds | Can it be rebuilt? |
|---|---|---|
| The blocks | The transcript. This is the record everything else comes from. | No. |
| Turns, compactions and the search index | One row per turn, each compaction, and what search reads. | Yes, with `terminus reindex`, except for an evicted session, whose blocks are gone. |
| Observations | The files, tools, commands, errors and commits of each closed session, read from the transcript's own records. | Yes, with `terminus observations regenerate`, except for an evicted session. |
| The injection log | Each prompt outside an excluded project, whole, with what Terminus considered and handed over, and how that was labelled. | No. |

Snapshots sit beside the store in the data directory. They're complete copies of it, so with the defaults of three kept, the directory can hold about four times the store's size. `terminus usage` prints where the bytes went, by project, by month and by table.

## When a session closes

A session closes when its last turn is more than 6 hours old. Closing is what lets a pass record its observations and label what was handed over in it, so `terminus observations` and `terminus stats` lag your live sessions by that much. Retention acts only on closed sessions too.

## Which project a session belongs to

A session belongs to the git repository its working directory is in. Without `git`, or when that directory isn't a repository or no longer exists, it belongs to the working directory itself, so a session started in a subfolder of a repository is filed under that subfolder. A working directory at `<repo>/.claude/worktrees/<name>`, or below it, is treated as `<repo>`.

## Checking it

Each block carries a BLAKE3 hash of its uncompressed bytes, and each session a hash over its block hashes. `terminus verify` checks both.

That tells you the blocks still match the hashes recorded when they were written, so it catches corruption. It isn't tamper evidence: the hashes aren't keyed and they live in the same file, so something that can write the store can rewrite both. It says nothing about a session Terminus never read, and it doesn't check SQLite's own pages.

What Terminus does and doesn't guarantee overall is in [checks and their limits](../how-it-works/checks.md).
