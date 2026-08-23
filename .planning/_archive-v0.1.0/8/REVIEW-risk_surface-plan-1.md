{
 "findings": [
  {
   "file": "crates/verbatim-core/src/retention/apply.rs",
   "line": 165,
   "severity": "low",
   "claim": "delete_one does not re-check transcript_is_gone before removing a session; the precondition is evaluated once, in evaluate, at retention/mod.rs:206.",
   "failure_scenario": "A transcript restored in the microseconds between evaluate and apply inside one retain() call is still deleted. The archived bytes are recoverable - the watermark row goes in the same transaction (apply.rs:186) so discover re-ingests from offset 0 on the next pass - but the observations rows removed at apply.rs:177 are not reproducible by any reingest. Downgraded from high: the reviewer claimed lost session bytes and dry-run divergence, and neither holds."
  }
 ]
}
