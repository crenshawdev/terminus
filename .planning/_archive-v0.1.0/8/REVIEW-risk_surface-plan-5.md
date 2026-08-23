{
 "findings": [
  {
   "file": "crates/verbatim-core/src/capture.rs",
   "line": 148,
   "severity": "low",
   "claim": "The verbatimElided figure is the canonical reserialized size of the dropped subtree, not the source bytes it occupied on the line, and the doc comment does not say so.",
   "failure_scenario": "A source line that is not compact JSON - escape sequences for characters serde_json would emit literally, a different number spelling - shrinks by more or fewer bytes than the mark records. Nothing in production reads the figure today (elided_bytes and ELISION_MARK appear only in capture.rs and its tests), so the cost is a documentary inaccuracy rather than a wrong accounting. Downgraded from medium: the reviewer's scenario required an audit or archive-delta comparison that does not exist."
  }
 ]
}
