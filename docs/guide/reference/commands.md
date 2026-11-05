# Commands

Every command is `terminus <command>`. This page lists each one from the usage text the binary prints, with every argument and option it accepts. The `--json` document each data command writes, and the exit codes every command uses, are in [JSON output and exit codes](json-shapes.md).

Two commands, `hook` and `mcp`, are run by Claude Code and never typed. They are at the [end of this page](#run-by-claude-code).

## Without a command

- `terminus`, `terminus --help` or `terminus -h` prints the usage text.
- `terminus --version` or `terminus -V` prints the version number.

Both options go before the command name. After it, they are an option that command does not take. An unknown command or option prints the error and the usage text and exits 2.

## Archiving

### `terminus ingest`

Archives every transcript under the configured [`roots`](../configuration/archiving.md#roots), skipping every project in [`exclude`](../configuration/archiving.md#exclude) before any of its files is opened. Each file is read from where the last pass stopped, so a pass reads only what is new. When the files are done, the same pass:

1. Closes every session whose last turn is more than 6 hours old, and labels that session's injections for `terminus stats`.
2. Records the observations for the sessions it just closed.
3. Applies the [`[retention]`](../configuration/retention.md#retention) rule.
4. Takes a [snapshot](../configuration/retention.md#snapshot) when one is due.

One pass runs at a time. When another Terminus process holds the ingest lock, `ingest` exits 0 having done nothing, because that pass reads the same files. A transcript that cannot be read is named on stderr, the rest are still archived, and the command exits 0. A pass that cannot run at all exits 1.

Every hook Claude Code fires starts this pass in the background, so running it by hand is rarely needed.

### `terminus ingest <path.jsonl>`

Archives one transcript file. A file inside an excluded project is not read, and the command says so on stderr and exits 0. It does not close sessions, apply retention or take a snapshot. Those are steps of the whole-tree pass above.

Each run that holds the ingest lock leaves one row, which `terminus status` shows as the last run, whether the file was archived, already up to date, or failed. A file that cannot be archived is named under `error` there, and the command exits 1. When another pass holds the lock, it exits 0 having done nothing.

### `terminus backfill`

Archives every transcript already on disk without holding the terminal. It prints how many transcripts there are, how many bytes are not yet archived, and an estimate of the time at the rate measured for its 4 workers. Then it starts the work in a detached process and returns. When it finds no transcripts, it prints the zero counts and starts nothing. A folder under a root that cannot be listed is printed as a problem, and nothing under it is counted or archived.

There is no log file. `terminus status` shows the pass under "last run" once it finishes. A backfill that is stopped part way picks up where it left off the next time a pass runs, because each file keeps its own position. `terminus install` runs a backfill as its last step.

Takes no options. `--work` is the flag the detached worker runs under, and it is not meant to be typed.

## Reading the archive

### Which project a read command sees

`search`, `show`, `sessions` and `observations` answer about one project at a time.

- Without `--project`, the project is the one the current directory sits inside, matched against the archived projects by the longest matching path.
- `--project <path>` names a path inside another project.
- `--project '*'` means every project.

An excluded project is never shown, with `*` too, including sessions archived before it was excluded. A directory no archived project covers gets an empty result, the reason, and exit 0.

### Time bounds

`--since` and `--until` take a UTC time as `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS.mmmZ`. A bare date in `--since` means the start of that day and in `--until` the end of it, so `--since 2026-10-01 --until 2026-10-01` covers the whole day. Both bounds are inclusive. A value in any other form, or one that is not a real date or time, such as `2026-02-30`, month 13 or hour 25, exits 2 with a message naming the flag. A turn or session with no timestamp is outside every bound.

### `terminus search <words>`

Finds past turns by text, a path, an error line or an identifier. The index holds what each turn said, up to 256 KiB of text per turn: prompts and replies, tool names and the text in tool inputs, tool results, and attachments. It leaves out text past that limit, thinking blocks, JSON keys, JSON number and boolean values, and records that are not turns. `terminus show` and `recall_get` read those from the archive, and search does not find them. Every word after `search` that is not an option is part of the query, so the words need no quotes, and a turn must match all of them. A word made only of letters the search index makes no token from, such as circled letters, is dropped and takes no place in the count. Only the first 32 distinct terms are used, and stderr says so when a query is cut. A query with nothing left to search, such as `///`, finds no turns, says so, and exits 0.

Each hit is two lines: the turn id, the day, the transcript's file name, the project, and `(subagent)` for a subagent's turn, then an excerpt around the match. The count of hits goes to stderr. The turn id is what `terminus show` takes. As in every command's human output, a control character in archived text other than tab and newline prints as a `\u00XX` escape, such as `\u001b` for ESC, with or without `--raw`.

- `--project <path>`: which project to search, as [above](#which-project-a-read-command-sees).
- `--tool <name>`: only turns that called this tool, such as `Bash` or `Read`.
- `--kind <type>`: only turns of this record type: `user`, `assistant`, `system` or `attachment`. Any other value is accepted and matches nothing, so a misspelled type returns no hits. Claude Code writes tool results, command caveats and task notifications as `user` records too, so `--kind user` finds much more than what you typed.
- `--typed`: only what a person typed, in a main session. It leaves out the `user` records Claude Code wrote and the prompts subagents were given. An evicted session's prompts are still found, with an empty excerpt.
- `--path <path>`: only turns whose tool call or tool result names this path in a structured field, such as a `Read`'s `file_path`. The match is exact, not a prefix, and a turn that only mentions the path in text does not match. Repeat it to match any of several paths.
- `--since <time>`, `--until <time>`: only turns inside these [bounds](#time-bounds).
- `--limit <n>`: how many hits, default 10. A number above 50 is treated as 50, and 0 exits 2.
- `--raw`: with [`[privacy] redact_recall`](../configuration/recall.md#privacy) on, prints the excerpts unfiltered for this run only.
- `--json`

### `terminus show <id>...`

Prints the archived record behind each turn id, byte for byte as it was archived. It takes one or more ids, and a word that is not a number exits 2. Each record has a header line with the id, the time, the transcript's file name, the project, and `(body evicted)` when retention has evicted the session's text. An id that is not in the project is named on stderr. As in every command's human output, a control character in archived text other than tab and newline, in the header and the `--before`/`--after` turns, prints as a `\u00XX` escape, such as `\u001b` for ESC, with or without `--raw`. The record itself is still printed byte for byte.

- `--project <path>`: which project the ids are looked up in, as [above](#which-project-a-read-command-sees). An id from another project needs that project or `*`.
- `--before <n>`, `--after <n>`: also prints up to this many turns before and after each record, from the same session, at most 25 each. A larger number is treated as 25. The window stops at the session's first and last turn and says which end it reached. When the session continues from another, a line after the window names that session's id.
- `--raw`: with [`[privacy] redact_recall`](../configuration/recall.md#privacy) on, prints the records unfiltered for this run only.
- `--json`

### `terminus sessions`

Lists every archived session in the project, one line each: the day of its first turn, the transcript's file name, the number of turns, the project, the branch its last record named, `(subagent)` for a subagent's session, and `(evicted)` when retention has evicted its text. The count goes to stderr.

- `--project <path>`: which project to list, as [above](#which-project-a-read-command-sees).
- `--since <time>`, `--until <time>`: only sessions that overlap these [bounds](#time-bounds), meaning still going at `--since` and started by `--until`.
- `--limit <n>`: at most this many sessions. The default is all of them, and 0 exits 2.
- `--json`

### `terminus observations`

Lists the facts recorded for each closed session in the project: the transcript's file name, branch, turns, duration and compactions, then the files modified and read, the tools, the commands, the errors and the commits. The facts come from the transcript's own records, and no model is involved. A session has an entry once a pass has closed and observed it. One pass observes at most 100 closed sessions, and the rest wait for the next pass. A list that hit its cap is named under `truncated`. As in every command's human output, a control character in archived text other than tab and newline prints as a `\u00XX` escape, such as `\u001b` for ESC.

- `--project <path>`: which project to list, as [above](#which-project-a-read-command-sees).
- `--json`

### `terminus observations regenerate`

Recomputes the observations already recorded, from the archive. It is the only way to rebuild them, because `terminus reindex` does not touch them. A session that cannot be recomputed, such as one whose text retention evicted, is named on stderr. It takes the ingest lock, and exits 1 when another process holds it.

`regenerate` comes straight after `observations`. An option before it exits 2.

- `--since <time>`: only sessions whose last turn is at or after this [time](#time-bounds). Without it, every session.
- `--json`

## The store

### `terminus verify`

Checks every session's archived blocks against the hash recorded for each. The sessions that fail are printed, and the count goes to stderr. It exits 1 when any session fails. With no store, it says so and exits 0, and creates no store. On a store older than the binary, it says so on stderr, runs the check, and changes nothing.

- `--json`

### `terminus reindex`

Rebuilds the tables derived from the archived blocks: `turns`, `compactions`, the search index `turns_fts`, `entities` and `paths`. Turn ids stay the same. It does not change the blocks, the session metadata, the observations, or the injection log and its labels. `terminus observations regenerate` is the only rebuild for observations.

An evicted session has no blocks to rebuild from, so its rows in those tables stay as they are and it stays searchable. `reindex` prints how many evicted sessions it carried across, on stderr when there are any. It takes the ingest lock and exits 1 when another process holds it. A session that cannot be rebuilt is skipped and named on stderr, every other session is still rebuilt, and the command exits 1.

- `--json`

### `terminus status`

Prints the store's path and size, the number of sessions and turns, how far into the transcripts the archive has read, the excluded projects, and the last ingest pass: when it started, how long it took, the files walked, archived and failed, the bytes read, the turns added, and its notes and errors. The lines recovery leaves, such as a watermark it lowered, print under `notes`, so a pass that only repaired a watermark shows no error. A pass's failures are recorded here and nowhere else, since there is no log file. It takes no lock, so it works while a pass runs. With no store, it says so and exits 0, and creates no store. On a store older than the binary, it says so on stderr, reports what is there, and changes nothing.

- `--json`

### `terminus compact`

Gives back the disk space retention freed. It rebuilds the database file without its free pages, which is also what clears the bytes of deleted rows out of the file, and then empties the write-ahead log, and prints the store's size before and after and the bytes reclaimed. It deletes nothing itself. It takes the ingest lock and exits 1 when a pass is running. With no store, it says there is nothing to reclaim and exits 0. A store it cannot read is an operational failure and exits 1.

- `--json`

### `terminus usage`

Prints where the bytes went, in three tables: the archive by project, the archive by month, and the database file by table. The first two add up to the archive's size, and the third adds up to the whole file. They are kept apart because the file is larger than the archive by the size of the derived tables.

- `--json`

### `terminus export <dir>`

Writes every session you can see, across every project, into `<dir>`: one `.jsonl` file per session holding its transcript exactly as archived, and a `manifest.json` describing them. Excluded projects are left out. A session whose text retention evicted is an empty file.

The export is not filtered. `[privacy] redact_recall` applies to text on its way into Claude Code, and a folder you name is not that, so the command prints a notice saying the files are the unredacted transcripts and writes the same notice into the manifest. The search index, the injection log and the observations are not in the export.

`<dir>` must not exist or must be empty. A folder that already holds anything is refused, and nothing is written. A folder `export` creates is readable only by you.

- `--json`

### `terminus data move <path>`

Moves the whole data directory to `<path>` and records the new location in the config directory ([the data directory](../configuration/paths.md#the-data-directory)). It prints where the data is now, where it is going and its size, warns that the store is not safe on a network filesystem, and asks once. The default answer is no.

Trailing slashes and `.` parts are dropped from `<path>` before it is checked, and the location is recorded without them. It refuses a `<path>` that is the data directory itself or is inside it. A `<path>` that does not exist is created 0700. One that exists has to be an empty directory owned by the user running the command, with no group or world permission bits. It refuses a symlink, with or without a trailing slash, a file, a directory another user owns, a directory with any group or world bit, a directory it cannot list, and a directory that already holds files. A directory made with `mkdir` under the usual umask 022 is 0755, so it is refused until `chmod 700 <path>` narrows it, and the refusal prints that command. Terminus does not change the mode of a directory it did not create. After the answer, and under the ingest lock, it checks `<path>` again and refuses if it was created, removed, replaced or changed while the question waited. It copies under the ingest lock, and refuses while another process holds that lock, then records the new location, then deletes the old directory. A copy that fails deletes nothing, and the old location stays recorded. Nothing in Claude Code's settings changes, because the hook and MCP entries never name the data directory. `TERMINUS_DATA_DIR`, when set, still outranks the recorded location.

- `-y`, `--yes`: answers the question yes. Without it and with no answer to read, as when stdin is closed, nothing moves and the command exits 1.

## Injection

### `terminus stats`

Reports whether the text Terminus hands Claude Code on each prompt is helping: how many prompts were logged, how many turns were injected, the hits and false positives among them, the precision, the misses, the wasted budget, and the characters injected and the part of them the hits carried. The prompt count is split into compacted decisions, taken on the one prompt after a compaction, and ordinary ones, and a prompt logged by a build that did not record which it was is in neither. The counts are characters, not tokens. An injection is labelled only once its session is closed. A prompt in an [excluded](../configuration/archiving.md#exclude) project is not logged. One logged before its project was excluded stays in these counts. What each number means is in [JSON output](json-shapes.md#stats).

- `--json`

### `terminus replay`

Re-scores the logged prompts of closed sessions under other injection thresholds, and prints how each label count would move from what was recorded. It opens the store read-only and writes nothing. A logged prompt from a project excluded since then is scored as injecting nothing.

Each option takes a whole number and defaults to the value Terminus injects with. They change this report and nothing else. No setting, option or environment variable changes the thresholds the hooks use.

- `--ranked <n>`, default 10: how many ranked hits the threshold is applied over.
- `--compacted-ranked <n>`, default 50: the wider window used for the one prompt after a compaction.
- `--entity-rank <n>`, default 3: the rank within which one matched path, command, error, identifier or tool is enough.
- `--co-occurring <n>`, default 2: how many matched entities make a hit eligible at any rank.
- `--max-turns <n>`, default 3: the most turns one prompt may be given. 0 shows the archive with injection off.
- `--max-candidates <n>`, default 8: how many paths and identifiers from one prompt are looked up.
- `--json`

## Retention

### `terminus retention --dry-run`

Prints what the next ingest pass would evict and what it would delete under the current [`[retention]`](../configuration/retention.md#retention) rule, and the time it measured every age from. Sessions over the per-pass limit, and sessions in excluded projects it passed over, are counted on stderr.

It changes nothing. Retention acts only inside the ingest pass, and this command only reports, so `--dry-run` is accepted and changes nothing. `terminus retention` without it prints the same report.

- `--dry-run`
- `--json`

## Setup

`~/.local/bin/terminus` below is the path when `TERMINUS_BIN_DIR` is unset. With it set, `doctor`, `install` and `uninstall` use `$TERMINUS_BIN_DIR/terminus` instead ([environment variables](../configuration/paths.md#environment-variables)).

### `terminus doctor`

Reports what is wired into Claude Code and what is not. It checks the binary at `~/.local/bin/terminus` and its version, Claude Code's `settings.json`, each of the four hook entries, the MCP entry, the `claude` command, the transcript roots, the data directory, the store, the last ingest run, and the two Claude Code settings Terminus [reports and never changes](../configuration/paths.md#claude-code-settings-terminus-reports). Each check is `ok`, `note`, `unknown` or `problem`, and where there is something to run, it carries the command that fixes it. The last ingest run is a `problem` when it failed a file or recorded an error, including `terminus ingest <path.jsonl>` on a damaged file, and it stays one until a later run records no failure.

It writes nothing and repairs nothing. It reads `settings.json` only for the keys it checks and never prints the file. It exits 1 only when a check is a `problem`.

- `--json`

### `terminus install`

Wires Terminus into Claude Code. It stores the running binary as `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus` and points the link at `~/.local/bin/terminus` to it, adds the four hook entries to Claude Code's `settings.json`, registers the MCP server through `claude mcp`, prints a receipt of every file it touched, and starts a backfill. `install.sh` runs it, and `target/release/terminus install` installs a local build the same way.

An MCP entry already current runs no `claude`. A missing entry is added with `claude mcp add-json`, and an entry in any other shape is removed with `claude mcp remove` first ([when `claude` runs](../getting-started/install.md#when-claude-runs-and-when-install-exits-1)).

Before it writes anything, it refuses when a file it did not put there sits at `~/.local/bin/terminus`. The first time it changes `settings.json` it keeps a copy of the file, and a later install never replaces that copy. It writes nothing to a file that is already current, so running it again is safe. It exits 1 when the MCP server could not be registered, after doing the rest and printing the command that finishes it, and when the store is in an older archive format than this build reads.

It asks nothing and takes no options. `--yes` and `--json` exit 2.

### `terminus uninstall`

Removes what `install` added: the four hook entries from `settings.json`, the MCP entry by running `claude mcp remove` when Claude Code holds one, and the link at `~/.local/bin/terminus`. When nothing but Terminus's entries changed in `settings.json` since install copied it, the copy is put back. When the file was edited since, the edited file stays, and so does the copy.

It keeps the versions folder, because a session already running may be using one of its files, and it keeps the data directory, because the archive is the product. It prints both paths. A step that cannot be done is reported, the rest still happen, and the command exits 1.

- `--purge`: also deletes the whole data directory, its snapshots and the state the hooks keep included, after showing its path and a size and asking once. The size counts everything `--purge` deletes, the snapshots and the state the hooks keep included. With no `terminus.db` in the directory, it prints that there is nothing to purge and deletes nothing. The default answer is no. It takes the ingest lock and deletes nothing while a pass is running. With no answer to read, or a store it cannot read, nothing is deleted and the command exits 1. It never deletes the versions folder.
- `-y`, `--yes`: answers the `--purge` question yes. Without `--purge` it changes nothing.

## Run by Claude Code

### `terminus hook <event>`

`terminus install` writes one hook entry for each of four events: `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`. Each run starts an [ingest pass](#terminus-ingest) in the background before it does anything else, then reads the event's details from stdin.

- On `SessionStart` it may hand Claude Code the brief of where the last session in the project left off.
- On `UserPromptSubmit` it may hand Claude Code up to three earlier turns that match what the prompt names.
- On `SessionEnd` and `PostCompact` it hands over nothing.

How much text each may carry is set by [`[injection]`](../configuration/recall.md#injection), and [`[privacy] redact_recall`](../configuration/recall.md#privacy) filters it. The injection has 50 ms. Past that, the hook hands over nothing for that event. It reads at most 1 MiB of stdin and waits at most 500 ms for it.

Once the event name is known it exits 0, whatever happens after, so it can never block a prompt. Anything that goes wrong is one line on stderr. A missing or unknown event exits 2.

### `terminus mcp`

The MCP server, started by Claude Code for each session over stdio and registered by `terminus install`. It takes no arguments. It reads requests on stdin until Claude Code closes it, then exits. It opens no port and takes no lock, and it opens the store read-only for each call, so a session archived while it runs is found by the next call.

It serves three tools. All three are read-only, answer about the session's project unless given another, and never return an excluded project:

- `recall_search`: the search `terminus search` runs, with the same query, `project`, `paths`, `tool`, `kind`, `since`, `until` and `limit` (default 10, at most 50).
- `recall_context`: the turns before and after one turn id, default 5 each side and at most 25, stopping at the session's ends.
- `recall_get`: the full archived text of up to 25 turn ids.

With [`[privacy] redact_recall`](../configuration/recall.md#privacy) on, every result is filtered. There is no unfiltered form, because the results go to Claude Code's model.

## `--json`

These commands accept `--json`: `search`, `show`, `sessions`, `observations`, `observations regenerate`, `verify`, `reindex`, `status`, `compact`, `usage`, `export`, `stats`, `replay`, `retention` and `doctor`. Each then writes one JSON document on stdout, described in [JSON output and exit codes](json-shapes.md).

`ingest`, `backfill`, `install`, `uninstall` and `data move` print for a person, and `--json` exits 2. `hook` and `mcp` speak Claude Code's own formats.
