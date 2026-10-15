{
  "findings": [
    {
      "file": "scripts/sign-release.sh",
      "line": 60,
      "severity": "low",
      "claim": "The draft check is vulnerable to a time-of-check/time-of-use race, so it does not guarantee that the release is still a draft when the later upload and edit occur.",
      "failure_scenario": "The lookup returns exactly `true`, then another actor or workflow publishes the release while this script downloads, verifies, and signs artifacts. The script subsequently continues to `release upload --clobber` and `release edit` against that now-published release, replacing its signature assets and marking it latest. Lookup failures and unexpected outputs fail closed, but this concurrent transition leaves the original finding open."
    }
  ]
}
