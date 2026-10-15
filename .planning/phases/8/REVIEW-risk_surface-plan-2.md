{
  "findings": [
    {
      "file": "crates/terminus/tests/release_workflows.rs",
      "line": 8,
      "severity": "low",
      "claim": "The regression test does not actually guarantee that workflows contain no secret reads or signing steps; it only rejects four exact, case-sensitive substrings.",
      "failure_scenario": "A workflow can sign artifacts with a command such as `gpg --detach-sign artifact`, `cosign sign artifact`, or `openssl dgst -sign key.pem artifact`, and this test passes because none contains `-Y sign`. Likewise, alternate expression formatting or an action that accepts a secret-backed signing key can evade these literals. The asserted policy that CI never signs or reads release secrets can therefore regress without failing the test."
    }
  ]
}
