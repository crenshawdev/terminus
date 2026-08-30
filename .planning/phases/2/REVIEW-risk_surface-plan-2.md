{
  "findings": [
    {
      "file": "crates/verbatim-core/tests/egress.rs",
      "line": 334,
      "severity": "low",
      "claim": "`the_filtered_body_is_the_same_document_with_different_values` does not actually check that filtering changed any value; it only compares shape (message count, roles, top-level key order). If the redaction call were removed entirely, this test would still pass because the body layout stays the same.",
      "failure_scenario": "A regression makes `egress::for_destination` a no-op on message content; the request is still valid JSON with the same keys and roles, so this test remains green and misses the leak."
    }
  ]
}
