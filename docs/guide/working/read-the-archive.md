# Read the archive yourself

Everything Claude can reach, you can reach from the command line. The commands below read the archive and change nothing. They answer for the project you're in, found from your current directory. `--project <path>` picks another, and `--project '*'` covers every project. An excluded project is never shown, with `*` too.

## List the sessions

```console
$ terminus sessions
2026-08-13  4f1c8d2a-7b3e-4c91-a0d5-2e6b9f13c7e2.jsonl  3 turn(s)  /home/you/code/project-alpha  phase-5
2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  6 turn(s)  /home/you/code/project-alpha  phase-3
2 session(s)
```

Each line is the day of the session's first turn, the transcript's file name, the number of turns, the project and the branch its last record named. A subagent's session is marked `(subagent)`, and one whose text retention removed is marked `(evicted)`.

## Find a turn

```console
$ terminus search retry budget
50331652  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    Grep src retryBudget
50331648  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    where did SearchManager get its retry budget
50331649  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    the retry budget lives in src/worker/S.ts near the top
3 hit(s)
```

Every word has to match, and the words need no quotes. You get 10 hits by default and at most 50 with `--limit`. [What search finds](../archive/search.md) covers what's in the index and what isn't.

To narrow it down:

| Option | What it keeps |
|---|---|
| `--tool Bash` | Turns that called this tool. |
| `--kind user` | Turns of this record type: `user`, `assistant`, `system` or `attachment`. |
| `--path src/worker/S.ts` | Turns whose tool call or result names this exact path in a structured field. Repeat it to match any of several. |
| `--since 2026-10-01`, `--until 2026-10-31` | Turns inside these UTC bounds, both inclusive. |

## Read a turn

The number at the start of each hit is its turn id:

```console
$ terminus show 50331648
$ terminus show 50331648 --before 3 --after 3
```

`show` prints the archived record byte for byte. `--before` and `--after` also print up to 25 turns on either side from the same session, stopping at the session's ends. When the session continues from another, a line after the window names that session's id.

## See what happened in a session

`terminus observations` lists the facts recorded for each closed session, taken from the transcript's own records with no model involved: the files modified and read, the tools, the commands, the errors and the commits.

```console
$ terminus observations --project ~/code/project-beta
c3e21f07-9d4b-4a6e-8f15-b02a7d9e4c61.jsonl  phase-3  6 turn(s)  2m 9s  0 compaction(s)
  tools           Bash
  commands        cargo test -p terminus-core --features testkit, cargo build --release, sleep 600
  errors          Bash command failed, error: could not compile terminus-core due to 7 previous errors
1 observation(s)
```

A session gets its entry once a pass has closed it, which happens when its last turn is more than 6 hours old. One pass observes at most 100 closed sessions, and the rest wait for the next pass.

## What your subagents did

A subagent's transcript is archived as a session of its own, so everything above works on it. It's marked `(subagent)` in `sessions` and in `search` hits, and `terminus show` prints what it was asked, what it ran and what it reported, turn by turn.

## Use it from a script

Every read command takes `--json` and writes one JSON document on stdout, with diagnostics on stderr. [JSON output and exit codes](../reference/json-shapes.md) is the contract.

To take the transcripts out of Terminus altogether, `terminus export <dir>` writes every session you can see, across every project, as one `.jsonl` file each with a `manifest.json`. The files are the unfiltered transcripts, whatever `redact_recall` says. Excluded projects are left out, and `<dir>` must not exist or must be empty.

Every command and option is in the [command reference](../reference/commands.md).
