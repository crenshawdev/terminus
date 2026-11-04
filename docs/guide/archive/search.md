# What search finds

`terminus search`, `recall_search` and the lookup on each prompt all run the same search. It's SQLite FTS5 with its `unicode61` tokenizer, ranked by BM25 and then by the structured things a turn shares with the query. There's no model and there are no embeddings.

That makes it exact and fast, and it means search finds words. A search for `backoff` won't find a turn that only said `retry`.

## What's in the index

For each turn, up to 256 KiB of text:

- prompts and replies;
- tool names and the text in tool inputs;
- tool results;
- attachments.

It leaves out text past that limit, thinking blocks, JSON keys, JSON number and boolean values, and records that aren't turns. `terminus show` and `recall_get` read those from the archive, and search doesn't find them. Under [`[capture] mode`](../configuration/archiving.md#capture) `lean` or `minimal`, text that was only in the replaced output isn't found either.

## How words match

- A turn must match every word you type. The words need no quotes, and case doesn't matter.
- Punctuation splits words, so `snake_case`, `kebab-case` and path parts are searchable piece by piece: `worker` finds `src/worker/S.ts`.
- An identifier is also indexed by its camelCase and PascalCase parts, so `manager` finds `SearchManager`.
- Only the first 32 distinct terms are used, and stderr says so when a query is cut.
- A query with nothing left to search, such as `///`, finds no turns, says so, and exits 0.

## The structured matches

Beside the text, each turn records the things its tool fields name, at most 48 per turn. A search counts these when it ranks, and the prompt lookup hands a turn over only when one matches.

| Kind | Where it comes from |
|---|---|
| `tool` | The name of each tool call. |
| `path` | A tool's file path, the path-shaped words of a `Bash` command, a `recall_search` call's query words and paths, and the file paths in a tool's result. |
| `command` | The program a `Bash` command runs. |
| `symbol` | The identifier-shaped words of a `Grep` pattern, an `Edit`'s old and new text, and a `recall_search` query. |
| `error` | The text of a failed tool result and a tool's `stderr`, with ids, timestamps, addresses and line numbers normalized, up to 512 bytes. |

`--path` matches this structured field exactly. A turn that only mentions the path in its text doesn't match, and neither does a parent folder.

## What a hit shows

An excerpt of up to 240 characters around the match, cut from the archive. A session whose text retention evicted is still found, because its search rows stay, and its hits come back with an empty excerpt. If that's more than you want to keep, see [keep less](keep-less.md).

Excluded projects are never returned, under any scope.
