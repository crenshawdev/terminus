# Task: say-block-rows-where-the-text-says-blob

## What shipped

- Help text, the two archive error messages, the watermark repair line, DESIGN-BRIEF, json-shapes, the changelog elision line and the listed comments say block rows or the archive instead of blob. DESIGN-BRIEF no longer says a format bump rebuilds derived tables only, and the terminusElided size is documented as re-encoded JSON.

## Commits

| Task | Commit | Description |
| --- | --- | --- |
| 1 | fcc19f7ba1dc8ad4ca23235f18e4660d7413086c | docs: say block rows where help, messages and the brief still said blob |
| 1 | 2ce2aa89dec3c21295da07553cf2e5df0c94fd56 | docs: describe block rows in the comments that still described the session blob |
| 1 | 128a9057a0e2d087fda44e45216bede0bc1a50db | docs: drop the two wording todos now that they are fixed |
| 1 | df8999a9d1970fb439347d32fc589667234fb3c2 | docs: task plan say-block-rows-where-the-text-says-blob |

## Files

### Task 1: say-block-rows-where-the-text-says-blob

- **Files:** .planning/CAPTURE.md, .planning/tasks/say-block-rows-where-the-text-says-blob/PLAN.md, CHANGELOG.md, DESIGN-BRIEF.md, crates/terminus-core/src/capture.rs, crates/terminus-core/src/error.rs, crates/terminus-core/src/lib.rs, crates/terminus-core/src/observe/mod.rs, crates/terminus-core/src/parse/mod.rs, crates/terminus-core/src/recall/search.rs, crates/terminus-core/src/recover.rs, crates/terminus-core/src/reindex.rs, crates/terminus-core/tests/backfill.rs, crates/terminus-core/tests/recover.rs, crates/terminus-core/tests/reindex.rs, crates/terminus-core/tests/retention.rs, crates/terminus/src/cmd/reindex.rs, crates/terminus/src/main.rs, docs/json-shapes.md
