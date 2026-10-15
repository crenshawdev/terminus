{
  "findings": [
    {
      "file": "crates/terminus-core/tests/egress.rs",
      "line": 391,
      "severity": "low",
      "claim": "The replacement test does not prove that each planted shape's own rule emitted its marker; most PLANTED entries use the same generic REDACTED marker, and the assertion searches the entire joined context window rather than the turn containing that sentinel.",
      "failure_scenario": "If one rule removes its sentinel without inserting a marker, any other credential redacted elsewhere in the fixture supplies the shared `[redacted]` text, so `redacted.contains(marker)` still passes while the test's stated per-shape marker guarantee is false."
    },
    {
      "file": "crates/terminus-core/src/observe/mod.rs",
      "line": 9,
      "severity": "low",
      "claim": "The module documentation still says observations must remain archival because their judgment half is a paid model call that replay cannot reproduce, contradicting this change's deletion of judgment and model calls.",
      "failure_scenario": "A future schema/reindex change follows this stated invariant and continues treating the now-mechanical observations table as irreproducible archival data, preserving stale rows or preventing a correct derived-table rebuild for a reason that no longer exists."
    }
  ]
}
