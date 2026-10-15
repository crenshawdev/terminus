{
  "findings": [
    {
      "file": ".planning/CAPTURE.md",
      "line": 270,
      "severity": "medium",
      "claim": "The blanket closure marks still-applicable release work as complete, including SECURITY.md/private disclosure and archive-format compatibility decisions, even though removing npm and Windows does not resolve them.",
      "failure_scenario": "Terminus remains public OSS with release channels and persistent user archives. Because these unresolved items are now checked off and the publish milestone is declared no longer applicable, a later release can ship without a private vulnerability-reporting channel or an explicit archive compatibility policy, with no open tracker item left to prevent it."
    }
  ]
}
