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


## Seeds

## Notes

## Debt markers

- None.
