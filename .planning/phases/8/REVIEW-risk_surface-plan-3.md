{
  "findings": [
    {
      "file": "scripts/sign-release.sh",
      "line": 61,
      "severity": "high",
      "claim": "The script never verifies that the referenced release is a draft before uploading with --clobber and marking it latest.",
      "failure_scenario": "If the owner supplies a syntactically valid tag that names an already-published release, `gh release download` succeeds, all local checks can pass, and the later upload replaces that release's signature assets before `gh release edit --draft=false --latest` promotes it as latest. Thus the script can mutate and promote a non-draft release despite being specified to operate on a draft only."
    }
  ]
}
