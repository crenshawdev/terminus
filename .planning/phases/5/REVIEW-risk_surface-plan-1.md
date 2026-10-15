{
  "findings": [
    {
      "file": "crates/terminus/tests/install_script.rs",
      "line": 676,
      "severity": "medium",
      "claim": "The piped-install test never truncates the script, so it does not test the stated guarantee that a pipe cut off mid-transfer runs nothing.",
      "failure_scenario": "An implementation that leaves the entire installer body at top level would pass this test because the test writes the complete script before closing stdin. Such an implementation would begin installing when only a prefix had arrived and could leave a partial install if the transfer then stopped. The harness needs to pipe representative truncated prefixes and assert that they cause no downloads or filesystem changes."
    }
  ]
}
