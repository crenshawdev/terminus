# Retention and snapshots

Retention decides how much of each project's history the archive keeps, and snapshots keep rolling copies of the store. [Keep less](../archive/keep-less.md) compares what each one reaches.

## `[retention]`

The default keeps everything. Retention does nothing unless both keys are set: an `action` other than `keep` and an `age_days` above 0. It acts only on sessions Terminus has already closed, during the ingest pass each hook starts, at most 100 sessions per action per pass. The rest are counted and taken on the next pass. `terminus retention --dry-run` lists what the current rule would act on and changes nothing.

### `action`

**Default:** `"keep"` · **Type:** text · **Allowed values:** `"keep"`, `"evict"`, `"delete"`. Any other value means `keep`, so a typo can't make a store start deleting.

- `keep` does nothing.
- `evict` removes a session's archived text and keeps the session's record and everything derived from it, its search rows and observations included. The injection log keeps its copies of the session's prompts.
- `delete` removes the session, everything derived from it, its observations, and the prompts and labels the injection log holds for it. `terminus reindex` can't rebuild the observations or the log, and `terminus stats` and `terminus replay` stop counting those prompts. A subagent's session reports its parent's session id, so deleting one removes no prompts, and deleting the parent removes them. It acts only once the session's transcript is gone, which is usually Claude Code's cleanup, because a transcript still on disk would be archived again on the next pass.

### `age_days`

**Default:** `0` · **Type:** whole number · **Range:** 0 is off, and a negative number counts as 0.

How many days old a session's last turn must be before `action` applies.

A deleted row's bytes can stay in the store file's free pages until [`terminus compact`](../reference/commands.md#terminus-compact) rebuilds the file. Prompts left behind by deletions under a version before this rule stay in the injection log. [Snapshots](#snapshot) are separate copies, and neither action reaches them.

## `[retention.project."<path>"]`

The same two keys, `action` and `age_days`, for one project. The table replaces the global rule for every session under that path, so a key it leaves out is at its default, not at the global table's value. Where two tables cover the same session, the one with the longer path wins. The path is written the way `exclude` is, and one that isn't absolute is dropped the same way. It's matched only as written: an `exclude` entry also matches the path it resolves to through a symlink, and a retention path doesn't.

```toml
[retention]
action = "evict"
age_days = 365

[retention.project."/data/code/scratch"]
action = "delete"
age_days = 30
```

## `[snapshot]`

Rolling copies of the archive, kept in the `snapshots` folder of the data directory. On by default. Each one is a complete copy of the store, so the defaults can take about four times the store's size on disk.

### `enabled`

**Default:** `true` · **Type:** true or false.

`false` stops new snapshots. Snapshots already taken stay.

### `interval_hours`

**Default:** `24` · **Type:** whole number · **Range:** 0 counts as 24.

How long after the last snapshot the next one is taken, checked on each ingest pass. Zero doesn't mean every pass, because a snapshot on every pass would write a full copy of the archive on every prompt.

### `keep`

**Default:** `3` · **Type:** whole number · **Range:** the lowest value is 1.

How many snapshots are kept. After each new one, the oldest past this number are deleted. To keep none, set `enabled = false`.

A snapshot is a copy of the store as it stood when it was taken. `exclude` and `[retention]` change the store and never a snapshot, so a snapshot keeps what was excluded or deleted after it was taken, prompts included, until the next snapshots push it past `keep`. With `enabled = false` no new snapshot pushes them out, so they stay until you delete them. To remove that data sooner, delete the files in the `snapshots` folder.

[Restore a snapshot](../working/when-something-is-wrong.md#restore-a-snapshot) shows how to put one back.
