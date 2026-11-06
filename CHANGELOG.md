# Changelog

All notable changes to Terminus are recorded here, starting with 1.0.0. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

Each release bumps the middle number of the version, a release with only fixes
bumps the last, and the first moves only for a large change. The number does not
promise compatibility; the entry for each release says what it changes.

## [2.0.0] - 2026-11-05

The first pass after this update rebuilds your archive's search tables, once.
It took about 4 minutes on a 3 GB archive. Until it finishes, new sessions get
no brief, prompts get no earlier turns, and search results may be incomplete.
Your archived sessions aren't touched, and `terminus status` stops saying the
tables predate this build when it's done.

### Added

- `search --typed` and `typed: true` on `recall_search` find what a person typed
  in a main session. `--kind user` also matches the tool results, command caveats
  and task notifications Claude Code writes as `user` records. (#141)
- `sessions` ends each line with the session's title, and `--json` carries
  `title`. (#144)
- The brief lists the five newest other sessions with a typed prompt and a title.
  Claude Code writes a title once, so it describes how a session opened. (#144)

### Changed

- The brief prefers the last session on your branch, passes over sessions with
  nothing typed, quotes up to three prompts and lists the files the session
  edited. (#139)
- A prompt sent through the Agent SDK or `claude -p` no longer counts as typed,
  including a `claude -p` you typed yourself, because its record looks the same
  as a script's. Plain search still finds it. (#143)
- `delete` in `[retention]` removes a session's logged prompts too, so `stats` and
  `replay` lose that history.
- `doctor` and `install` say what each auto-compact setting costs, and `doctor`
  no longer recommends turning it off.
- The repository moved to `crenshawdev/terminus`, and the user docs moved into a
  guide.

### Fixed

- The brief reports the branch a session ended on instead of the first one it
  named. A finished session keeps the branch it already has.

### Security

- With `redact_recall` on, a secret written as `name = value` is filtered, as in
  `~/.aws/credentials`, INI, TOML, HCL and Python settings. (#131)
- A project in `exclude` no longer has its prompts logged. Rows already stored
  stay. (#133 covers removing them.)
- Releases are signed with a new key, pinned in `install.sh`, because the old one
  can no longer sign.

## [1.1.0] - 2026-10-28

### Changed

- Every command's human output escapes control characters in archived text, on
  stdout and stderr. `--json`, the hook's document and `show`'s record are still
  written byte for byte. (#72)
- Every command that fails under `--json` prints a JSON document with
  `ok: false` and the reason. (#107)
- Every file and folder Terminus creates is readable only by your user from the
  moment it exists, `data move`'s pointer and the config folder included. (#73)

### Fixed

- A search that includes a word SQLite's tokenizer can't index returns the hits
  for the other words instead of nothing. A query with no searchable word left
  says so. (#23)
- A `--since` or `--until` that is not a real calendar date, such as
  `2026-02-30`, is refused with the flag named, from `search`, `sessions`,
  `observations regenerate` and `recall_search`. (#24)
- `search --path` repeated past SQLite's variable limit still returns results,
  and `recall_search` takes any number of paths. (#108)
- `show` and the MCP tools link a session to the one it continues from even when
  it was archived before its predecessor. (#11)
- Retention checks a transcript again just before it deletes the session, and
  keeps the session when the transcript is back. (#43)
- `status` and `doctor` show no error for a pass whose only notes came from
  recovery. (#96)
- `observations regenerate` and each pass read only a session's own entities.
  (#70)
- `data move` refuses a destination that went missing during the copy instead
  of making it again. (#73)
- `uninstall --purge` counts every file it deletes, snapshots included, in the
  size it shows before it asks. (#95)
- A quoted turn the prompt line cuts shows the cut mark even when its share is
  under four characters. (#94)

### Security

- zstd moves to 0.14, which fixes RUSTSEC-2026-0391 to RUSTSEC-2026-0410.
- Signing a release runs no code read out of `install.sh`. (#99)

## [1.0.0] - 2026-10-20

The first release of Terminus as a product. It keeps every Claude Code session
you run and gives that history back to Claude and to you.

### What it does

- Archives every session and every subagent's transcript, byte for byte under
  the default capture mode, in one local SQLite file readable only by your user.
- Starts each session with a brief of where the last session in the project left
  off.
- Hands Claude earlier turns when a prompt names the same paths or identifiers,
  and after a compaction, the turns the compaction dropped.
- Gives Claude three MCP tools, `recall_search`, `recall_context` and
  `recall_get`, to search the whole history and read it back.
- Searches from the command line with `terminus search`, `show`, `sessions` and
  `observations`.
- Checks and rebuilds itself with `doctor`, `verify`, `reindex` and rolling
  snapshots.
- Installs with one line that checks the release's signature before it installs
  anything, and updates by running the same line again.

It calls no model, runs no daemon and opens no network connection.

[2.0.0]: https://github.com/crenshawdev/terminus/releases/tag/v2.0.0
[1.1.0]: https://github.com/crenshawdev/terminus/releases/tag/v1.1.0
[1.0.0]: https://github.com/crenshawdev/terminus/releases/tag/v1.0.0
