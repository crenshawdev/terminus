{
  "findings": [
    {
      "file": "crates/verbatim-core/src/observe/egress.rs",
      "line": 594,
      "severity": "high",
      "claim": "Rule 7 does not redact space-separated secret flags when the value is quoted. `value_run_end()` returns immediately on the opening quote, so inputs like `--token \"tok-SECRET\"` or `--token 'tok-SECRET'` fall through unchanged.",
      "failure_scenario": "A CLI transcript or logged shell command containing a quoted secret flag value is emitted with the secret intact instead of `[redacted:a-command-line-flag-value]`."
    }
  ]
}
