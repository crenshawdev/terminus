{
  "findings": [
    {
      "file": "crates/verbatim-core/src/inject/decision.rs",
      "line": 199,
      "severity": "medium",
      "claim": "`read_all` filters to `*.json`, but `Decision::save` creates `*.tmp` first; if the hook dies after the temp file is written and before the rename, that orphan is never returned to the drain and therefore never removed.",
      "failure_scenario": "A `UserPromptSubmit` process is SIGKILLed after `.decision-<pid>-<attempt>.tmp` is created but before it is renamed. Every later ingest pass skips the temp file, so the decision directory accumulates dead `.tmp` files forever instead of being emptied."
    },
    {
      "file": "crates/verbatim-core/src/feedback/mod.rs",
      "line": 95,
      "severity": "medium",
      "claim": "The drain does not persist the `Decision.compacted` / `Decision.dropped` state at all; the SQL insert only carries the JSON lists and scalar fields, so compaction-specific decisions are silently flattened into ordinary rows.",
      "failure_scenario": "A prompt that triggers compaction (`dropped.is_some()`) writes a decision file with `compacted=true` and a non-empty `dropped` list. After ingest, the `decisions` row has no columns or payload for either field, so later replay/stats cannot tell that the prompt ran under a compacted pool and will misattribute that decision."
    }
  ]
}
