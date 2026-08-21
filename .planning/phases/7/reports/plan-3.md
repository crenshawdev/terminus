PLAN PARTIAL
Plan: .planning/phases/7/PLAN-3.md
Tasks: 1 of 6
| Task | Commit | Note |
|---|---|---|
| 1 - The judgment request and the anchoring contract | 1ce8223 | `observe/judgment.rs`: fixed versioned schema, one turn per line beside its real `turns.id`, anchors validated against `turns` of that session before any row is written. 4 tests in `tests/judgment.rs` pass. |
Deviations: none
Open items:
- The response schema travels in the system message, not in `response_format`. PLAN-2's `observe::provider::complete` sends D-09's `{"type":"json_schema","strict":true}` for every caller from a private constant, and `provider.rs` is outside this plan's `files:` lease. The Verify only asks that a schema-shaped answer be parsed and anchored, which the prompt-borne schema meets. Worth noting for AC4's remote half: a strict OpenAI-compatible endpoint may reject a `json_schema` response_format that carries no `json_schema` object, which would need a one-line change in `provider.rs`.
