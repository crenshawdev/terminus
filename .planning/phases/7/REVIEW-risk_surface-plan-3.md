{
  "findings": [
    {
      "file": "crates/terminus-core/src/store/archive.rs",
      "line": 105,
      "severity": "medium",
      "claim": "Range arithmetic is unchecked, so a malformed negative turn offset or length can panic or address the wrong blocks instead of failing that range.",
      "failure_scenario": "The readers cast `stream_offset` and `byte_len` from `i64` to `u64` before calling `archive::ranges`. If a damaged turn row contains `stream_offset = -1` and a positive length, `offset + len` overflows here: debug builds panic, while optimized builds wrap `end` and subsequent `end - start` arithmetic can underflow or select invalid slice bounds. Thus search, get, context, or brief can crash on malformed archive coordinates rather than leaving only that turn unread."
    }
  ]
}
