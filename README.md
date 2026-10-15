# Terminus

When a Claude Code session ends, the next one starts without it, and when a long session compacts to make room, the earlier part of it drops out of what Claude can see. The work is still written down in Claude Code's transcripts, until Claude Code deletes them after `cleanupPeriodDays`, 30 days by default.

Terminus keeps a copy of every session you run in Claude Code and lets you and Claude find the work in it again. A new session starts knowing where the last one in the project left off. A prompt that names a file or an identifier from earlier work can bring back the turns where it came up, and after a compaction, the next prompt can bring back the turns that dropped out. The whole history stays searchable, from the command line and by Claude itself.

> **Status: built, with known gaps.** Archiving, the recall tools Claude uses, the context Claude is given at the start of a session and on a prompt, and every command below work today. No release is published yet and the macOS builds have never run, so today Terminus runs from a local build on Linux. The gaps that touch what this page says are under [Current state](#current-state).
