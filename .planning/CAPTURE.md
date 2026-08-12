# Capture

## Todos

- [ ] (phase 1) `cargo test -p verbatim-core` runs zero of the five
      `#![cfg(feature = "testkit")]` test files and exits 0; they only run via
      `verbatim`'s dev-dependency and resolver-v2 feature unification, so any
      `-p`-scoped CI step is a green run over an empty test set.
- [ ] (phase 1) The crash harness's negative test asserts only the weaker
      "watermark with no archived session" branch, and every kill targets a
      first pass into a fresh data dir, so the append/resume path is never
      killed.
- [ ] (phase 1) `store/schema.rs` says foreign keys are "deliberately not
      enforced" and the pragma stays off; the bundled amalgamation is compiled
      with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1` and `reindex.rs` relies on the
      opposite fact for its reverse drop order.
- [ ] (phase 1) `lock_race.rs::the_lock_is_released_by_process_death` never
      starts or kills a process, so the stale-lock-after-SIGKILL property it is
      named for is asserted nowhere.
- [ ] (phase 1) `session_meta.continues_from` is written in the session-id
      namespace with a `parentUuid` fallback that depends on the predecessor
      already being ingested; D-11 does not say which namespace the fallback
      writes. Confirm when ING-04 consumes it.
- [ ] (phase 1) The 64 KB block size is still the flagged unmeasured
      assumption; D-08's 11.2% framing penalty was not re-measured.

## Seeds

## Notes

## Debt markers

- None.
