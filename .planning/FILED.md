# Filed: issues this repository's gates opened

Written by `issue-filing.mjs file` when a gate's finding is ACCEPTED and filed
on the tracker, read by `planning.mjs recall` beside CAPTURE.md. One `- ` row
per filed issue: the date, the provider, the repository slug, the finding's
(file, claim) fingerprint and the issue's title. No finding body - a row is a
pointer to an issue, not a copy of it, and NOTHING here is a queue. A declined
finding is never written here; its record is a row in DECLINED.md beside this
file, which is deliberately outside the recall corpus. A line that is not a row
is skipped, so a note added here mints no recall entry.

A row carrying the word `unconfirmed` before its colon is a create this
repository could not confirm landed - the forge may hold that issue and may not.
It is kept so a later fire does not file the same finding twice; delete the row
once you have looked, and nothing else here is unverified.

- 2026-09-04 github crenshawdev/verbatim 2375a7edcad0e2d1: [cadence 2375a7edcad0e2d1] Enabling `redact_recall` does a lossy UTF-8 round-trip on every returned body, so any archived line that is not valid UTF-8 gets rewritten even when it contains no secret.
- 2026-10-13 github crenshawdev/terminus 11e3a640189bd38c: [cadence 11e3a640189bd38c] The replacement test does not prove that each planted shape's own rule emitted its marker; most PLANTED entries use the same generic REDACTED marker, and the assertion searc
- 2026-10-13 github crenshawdev/terminus 1fa1c0e8a2281d0a: [cadence 1fa1c0e8a2281d0a] Removing exact-value credential replacement leaves common bare provider credentials unredacted because none of the remaining shape rules recognizes OpenAI-style `sk-...` tok
- 2026-10-13 github crenshawdev/terminus 4a2bc7323f3cffd3: [cadence 4a2bc7323f3cffd3] The plan adds a new refusal test and verifies a total of 750 tests, while locked AC5 requires the completed suite to list 749 tests.
- 2026-10-13 github crenshawdev/terminus fdb317a4cb6bc600: [cadence fdb317a4cb6bc600] The doctor task removes Windows branches but does not make doctor say that Linux and macOS are the supported systems, leaving ROADMAP Phase 2 success criterion 5 without a d
- 2026-10-13 github crenshawdev/terminus 421469e0a971a140: [cadence 421469e0a971a140] Task 3's diff verification cannot pass after performing all of Task 3: it permits changes only to seven todo lines and the Publish milestone, but the same task also requires
- 2026-10-13 github crenshawdev/terminus 3bcc856d2ee2194e: [cadence 3bcc856d2ee2194e] The blanket closure marks still-applicable release work as complete, including SECURITY.md/private disclosure and archive-format compatibility decisions, even though removin
