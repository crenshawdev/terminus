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
- [ ] (phase 3) `index/text.rs` MAX_BODY_BYTES does not bound the projected body: the newline separator is uncharged and `take` is 0 when the budget is below the next leaf's first char, so the short-circuit never fires
- [ ] (phase 3) `index/entity.rs` path/command/symbol entity values carry no length bound; only `error` does
- [ ] (phase 3) `index/entity.rs` splitting a command on `;&|<>()` cuts a real path containing one of them, and splitting on `|` multiplies BRE-escape debris (24.7% of unique Bash-derived path entities)
- [ ] (phase 3) `index/entity.rs` normalize_error leaves a rejected timestamp's date or hour in the value, so one failure logged twice can normalize two ways
- [ ] (phase 3) `index/entity.rs` program_of returns the first argv word, so a compound command records the wrapper (`cd` for 17.8% of Bash calls)
- [ ] (phase 3) `index/entity.rs` identifier_tokens glues a regex escape's letter onto the symbol (`\\bsearch_manager` -> `bsearch_manager`)
- [ ] (phase 3) `recall/excerpt.rs` first_token indexes the lowercased string while window slices the source, so the excerpt is displaced for any char whose lowercase changes length
- [ ] (phase 3) `recall/excerpt.rs` excerpt cutting is unbounded in one record's size: two Vec<char> materializations per hit to produce 240 chars
- [ ] (phase 3) `recall/search.rs` the per-value document-frequency query builds a temp b-tree (15.5-16.7 ms on a 250k-turn store) on the default search path
- [ ] (phase 3) `recall/query.rs` a token of characters Rust calls alphanumeric but unicode61 does not index becomes a zero-term phrase and silently returns zero hits
- [ ] (phase 3) `recall/search.rs` a query that reduces to no tokens returns no reason, so an unsearchable query cannot be told from an empty archive
- [ ] (phase 3) `cmd/json.rs` Document::emit uses println!, which panics with exit 101 on a closed stdout pipe; the human path has broken_pipe and the JSON path does not
- [ ] (phase 3) `cmd/mod.rs` time_bound validates shape but not the calendar, so `--since 2026-08-32` is accepted and silently hides the month with exit 0
- [ ] (phase 3) `cmd/show.rs` a closed stdout in human mode maps to Failure::Silent (exit 1), indistinguishable from an operational failure under pipefail
- [ ] (phase 3) `recall/get.rs` the exclusion arm names an excluded project's absolute path in a reason, where the scope arm answers NoSuchTurn so as not to confirm the id exists
- [ ] (phase 3) `docs/json-shapes.md` the sessions shape documents nullability nowhere, though five of its eleven fields are null in ordinary states
- [ ] (phase 3) the MCP tool result shape is pinned in no document; docs/json-shapes.md is the CLI contract only
- [ ] (phase 3) `recall::search::run` resolves scope before validating filters, so D-23's two-shape time rule now exists twice (cmd/mod.rs and recall::search::bound)
- [ ] (phase 3) a JSON-RPC line is materialized whole by read_until with no size cap on one message
- [ ] (phase 3) `cmd/mcp/tools.rs` optional_time inherits shape-only date validation, so `since: 2026-02-30` is accepted and returns reason:null with isError:false
- [ ] (phase 3) `cmd/mcp/mod.rs` serve has no size cap on one JSON-RPC line and amplifies wire bytes to resident memory ~20x (12.9 MB line -> 278.7 MiB RSS)
- [ ] (phase 3) `cmd/mcp/rpc.rs` an integer id outside i64/u64 is parsed as f64 so the echoed id differs from the id sent; a fractional id is accepted, which MCP forbids
- [ ] (phase 3) `cmd/mcp/tools.rs` time_bound's message hardcodes the CLI `--since` spelling in a tool result whose inputSchema forbids that argument
- [ ] (phase 3) `cmd/mcp/tools.rs` recall_get truncates to MAX_IDS before get::records deduplicates, so duplicate ids consume the budget
- [ ] (phase 3) `cmd/mcp/rpc.rs` PROTOCOL_VERSIONS offers 2025-03-26, which mandates JSON-RPC batching, but a batch is answered with one -32600 and every request in it goes unanswered
- [ ] (phase 3) `crates/verbatim/tests/mcp.rs` and `tests/recall_cli.rs` are
      `#![cfg(feature = "testkit")]` on the `verbatim` package, whose `testkit`
      feature is not default and is enabled by nothing in the workspace. A bare
      `cargo test --workspace` runs 0 of their 41 tests and still reports 304
      passed / 0 failed, so the two binaries carrying AC5/AC6/AC7's
      process-level assertions are silently absent from the default command.
- [ ] (phase 4) The npm shim does not forward signals to the child: spawnSync blocks the event loop, so a SIGTERM aimed at the shim's own PID kills node and orphans the binary. Ctrl-C and job control signal the whole process group and are unaffected. risk_surface review finding, adjudicated downgraded (medium).
- [ ] (phase 4) Hoist pass::record_pass and pass::walk's per-file failure arm into ingest/mod.rs (or make them pub(crate)) so pass and backfill share one runs-row writer and one skip rule. Needs a plan whose lease covers crates/verbatim-core/src/ingest/pass.rs.
- [ ] (phase 4) The four-worker backfill pipeline is only 8% faster than the sequential pass on the real corpus (49,089 ms vs 53,007 ms). The remaining cost is the single SQLite writer's: batch derived-row inserts, or move derive_turn's text expansion and entity extraction onto the workers. Measurement-led task, not a guess.
- [ ] (phase 4) verbatim status can fail with 'sqlite: database is locked' in the ~1 ms window while a backfill creates the store and sets journal_mode=wal (1 of 20 polls, at t=1 ms). Pre-existing in Store::open.
- [ ] (phase 4) AC3's Windows half - no console window, no handle inherited from the hook - is unrunnable on Linux and stays a human-verify on a Windows machine.
- [ ] (phase 4) npm's os/cpu selection of an optionalDependency cannot be exercised locally, only the shim's resolution of an already-placed platform package. AC9's remaining risk lives in the publish step (D-21).
- [ ] (phase 4) Four of the five npm platform packages carry no binary; they arrive with cross-compilation in a later shipping step. pack-local.sh fails with a named message on any host it cannot stage.
- [ ] (phase 4) The npm name 'verbatim' and the '@verbatim' scope have not been checked for availability, and crates.io's 'verbatim' is already taken. Publish-step question for the human.
- [ ] (phase 4) ingest::backfill deliberately does not honour fault::pass_fails_after (the 'a pass that died still leaves its runs row' fault). The sequential walk still has it; add it if a later task wants to kill a backfill's walk rather than its process.
- [ ] (phase 4) cargo fmt --check reports two pre-existing diffs in crates/verbatim/tests/hook.rs, both on lines committed in plan 1 and neither in code any later pass wrote. cargo fmt closes them whenever the file is next edited for its own reasons.
- [ ] (phase 4) Declined an engines.node field on the thin npm package - nothing in the Verify exercises a Node floor and the shim uses only long-present APIs. Add one when a task states a minimum.
- [ ] (phase 4) shellcheck is not installed on this machine, so npm/pack-local.sh's static analysis was bash -n only.
- [ ] (phase 5) AC6's watchdog timeout arm has no test that forces it: the exclusive-writer case is bounded by SQLite before the 50 ms watchdog fires on Linux. Forcing it needs a fault point in the injection path (phase 4's testkit pattern).
- [ ] (phase 5) crates/verbatim/tests/hook.rs flakes with ETXTBSY at its spawn-the-copied-binary sites under parallel tests; reproduced on the pre-phase-5 baseline 3fd5b19, so it pre-dates phase 5.
- [ ] (phase 5) cargo fmt --check reports pre-existing diffs in crates/verbatim-core/src/ingest/backfill.rs:250 and crates/verbatim/tests/hook.rs:252,681 under rustfmt 1.9.0; every phase-5 file is clean.
- [ ] (phase 5) inject/brief.rs keeps a private chars/clip pair identical to the shared definitions PLAN-3 put in inject/mod.rs; brief.rs was out of that plan's lease, so the fold-together is one pending deletion.
- [ ] (phase 5) entity_score dwarfs bm25 whenever query terms are common (measured: -bm25 ~1e-6 per row vs entity weight 1.2-2.4 over a 48-turn store), so rank 1..3 is closer to "the three strongest entity matches" than a text ranking; phase 6's auto-tuner gates should be told before tuning anything.
- [ ] (phase 5) The error entity kind has no candidate spelling of its own in the UserPromptSubmit query: a failure whose text is pure prose and numbers names no candidate and opens no store. Left because a normalized stderr line is not a spelling a user retypes.
- [ ] (phase 5) inject::state::MAX_SUPPRESSED caps the suppression list at 100 oldest-dropped while injected/brief are uncapped; a very long session's state file forgets its earliest refusals. Nothing in phase 5 reads them back; FEED-01 owns the durable record.
- [ ] (phase 5) crates/verbatim/src/cmd/hook.rs's inject doc says the abandoned injection thread writes nothing; since plan 4 task 2 it writes the D-06 per-session scratch file (the store still never sees a write). One clause of one comment, outside plan 4's lease.


## Seeds

## Notes

## Debt markers

- None.
