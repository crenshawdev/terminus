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
- [ ] (phase 2) `bring_forward` (`store/open.rs`) does check-then-act `ALTER
      TABLE ADD COLUMN` outside the ingest lock, on every `Store::open`
      including lock-free read commands; two processes upgrading one phase-1
      store race and the loser exits 1.
- [ ] (phase 2) `record_run`'s single-file path omits `files_committed` /
      `files_failed`, so a single-file ingest reports `0 committed` in status.
- [ ] (phase 2) `ingest::run`, the single-file entry point, applies no
      exclusion test; `verbatim ingest <path>` archives an excluded transcript.
- [ ] (phase 2) Exclusion strings are unnormalized: a trailing separator, or
      `/`, disables the pre-open test while the read-side test still hides
      everything.
- [ ] (phase 2) Sessions archived before 02592b5 keep `project_pre_worktree =
      NULL`; nothing backfills it, so D-23's retroactive exclusion does not hold
      for existing stores.
- [ ] (phase 2) `verify`'s divergence message is length-only and now also fires
      for a content-rewritten transcript, reporting a longer file as shorter.
      The state never clears on its own.
- [ ] (phase 2) `prefix_matches` runs after `read_tail`, so a file swapped
      between the two reads can have one file's prefix validated and another's
      tail appended. Closable by reading head and tail from one handle.
- [ ] (phase 2) `parse/record.rs`: an escaped `compactMetadata` key stores NULL
      silently, a duplicated key stores the first value while every other JSON
      reader sees the last, and a record past serde_json's 128-level nesting
      limit is demoted to a fieldless Record with no diagnostic.
- [ ] (phase 2) The boundary signal reads the untrusted `subtype` with no
      cross-check against `type`: a `user` record carrying it gets a boundary
      row, a real boundary missing `uuid` gets none.
- [ ] (phase 2) The crash harness `Snapshot` excludes `compaction_boundaries`,
      `session_meta`, `entities` and `paths`, and no killed fixture carries a
      boundary, so AC6 proves nothing about the row phase 2 added.
- [ ] (phase 2) The forced derived rebuild measures 50.4 s over the real
      corpus, inside the ingest lock, on the first hook-spawned pass after a
      DERIVED_SCHEMA bump.
- [ ] (phase 2) `verbatim status` now loads Config, so an unparseable
      `verbatim.toml` fails a read command where it previously succeeded.
- [ ] (phase 2) `crates/verbatim/Cargo.toml` has no `rusqlite`; richer read
      commands in the binary will need it.
- [ ] (phase 2) Three copies of the same reverse-drop loop exist
      (`tests/reindex.rs`, `crates/verbatim/tests/cli.rs`,
      `tests/compaction.rs`); a `testkit::drop_derived` would leave one.
- [ ] (phase 2) ROADMAP still lists ING-07 under phase 2 after its deferral to
      phase 8; needs a `/cad-phase` edit.
- [ ] (phase 2) AC2's linkage rate: 188 `continues_from` links is 8.8% of
      sessions against CONTEXT's measured 1.2% of files; the denominators have
      not been reconciled.

## Seeds

## Notes

## Debt markers

- None.
