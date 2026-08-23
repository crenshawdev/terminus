{
 "findings": [
  {
   "file": "crates/verbatim-core/src/verify.rs",
   "line": 113,
   "severity": "low",
   "claim": "verify skips an evicted session entirely rather than asserting the one thing eviction guarantees about it - that its blob is empty - so it has a class of row it never checks.",
   "failure_scenario": "A row carrying is_evicted=1 beside a non-empty blob passes verify silently. evict_one is the only writer of that flag and it empties the blob in the same transaction, so the state takes external tampering or SQLite-level corruption to reach - which is what verify exists to detect. Downgraded from medium: the reviewer's scenario claimed real corruption hidden indefinitely, and the producing path cannot create the state."
  }
 ]
}
