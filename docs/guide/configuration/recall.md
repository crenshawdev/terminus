# What goes back to Claude

These settings cap and filter the text Terminus hands Claude Code. [What Claude gets](../working/what-claude-gets.md) explains when that text is handed over. The rules that decide which turns qualify aren't settings.

## `[injection]`

How much text Terminus hands Claude Code when a hook fires. Both counts are characters, not tokens, and each is capped at 10,000 whatever the file says, because Claude Code writes a hook's output longer than 10,000 characters to a file and gives the model the file's path instead of the text.

### `brief_chars`

**Default:** `6000` · **Type:** whole number · **Range:** up to 10,000 takes effect.

The most the brief at the start of a session may carry. The brief says where the last session in the project left off: its date, its branch, what it was last asked, what it last answered, the files it edited, and how much of the project is archived. A smaller number drops the file list first, then shortens the quotes. A number too small for the brief's own wording means no brief.

### `prompt_chars`

**Default:** `4000` · **Type:** whole number · **Range:** up to 10,000 takes effect.

The most one prompt's injection may carry. An injection quotes at most three earlier turns that match the paths and identifiers the prompt names, and most prompts get none. After a compaction, the next such prompt looks only at the turns the compaction dropped, under the same limits, so it doesn't restore everything that dropped. A number too small for the injection's own wording means nothing is handed over.

```toml
[injection]
brief_chars = 3000
prompt_chars = 2000
```

## `[privacy]`

### `redact_recall`

**Default:** `false` · **Type:** true or false.

With `true`, archived text is filtered on its way into Claude Code: the brief, each prompt's injection, the MCP tools' results, and the excerpts `terminus search` and `terminus show` print.

The filter replaces private key blocks, passwords in connection URLs, values named like secrets in headers, JSON, assignments and command-line flags, JSON Web Tokens, GitHub tokens and bare `sk-` keys. A value named like a secret in a header, JSON or an assignment becomes a plain `[redacted]`, and the rest become a marker that names what was taken out, such as `[redacted:a-github-token]`. Values named like secrets inside a tool's input, a tool's result and an attachment become `[redacted]` too.

The filter goes by shape, so a secret in a shape it doesn't know passes through. The archive itself is never filtered, and setting it back to `false` returns everything unfiltered. `terminus export` is never filtered either, and says so. `terminus search --raw` and `terminus show --raw` print unfiltered text for that one run.

It's off by default because the model saw this text once already, when it was typed, and filtering would cost exact recall to hide it again. Turn it on when your transcripts hold credentials you'd rather not have quoted back into a fresh context.

Only the file sets this. No environment variable or flag turns it on, and `--raw` affects only the command it's passed to, never the hooks or the MCP server.

```toml
[privacy]
redact_recall = true
```
