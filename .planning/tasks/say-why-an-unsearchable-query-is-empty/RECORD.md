# Task: say-why-an-unsearchable-query-is-empty

## What shipped

- A search whose query has no searchable words now returns the reason "the query has no words that can be searched" instead of an empty result with no reason. Also formats the two test hunks fix 4a938c2 added. The matching capture todo is removed.

## Commits

| Task | Commit | Description |
| --- | --- | --- |
| 1 | acef22f60b8e279675e675e33248b2d46da96987 | style(recall): format the two test hunks the excluded-project fix added |
| 1 | 0261f8baa79082126b9d3440097439622128deb1 | fix(search): say why a query with no searchable words found nothing |
| 1 | d66a5eca0af90477f6ec6a4906012fa71df5e9d8 | docs: drop the empty-search todo now that it is fixed |

## Files

### Task 1: say-why-an-unsearchable-query-is-empty

- **Files:** .planning/CAPTURE.md, crates/terminus-core/src/recall/scope.rs, crates/terminus-core/src/recall/search.rs, crates/terminus-core/tests/recall.rs, crates/terminus/tests/mcp.rs
