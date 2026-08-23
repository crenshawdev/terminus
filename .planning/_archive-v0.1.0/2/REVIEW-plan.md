{ "findings": [
  { "file": ".planning/phases/2/PLAN-1.md",
    "line": 280,
    "severity": "high",
    "claim": "Task 4 turns no-arg `verbatim ingest` into a tree pass, which contradicts a committed phase 1 test the plan's own Verify runs, and the file holding that test is outside the plan's declared lease.",
    "failure_scenario": "`crates/verbatim/tests/cli.rs:282 misuse_exits_two_with_an_empty_stdout` asserts `verbatim ingest` with no argument exits 2 (cmd/ingest.rs:parse returns Failure::Misuse). After task 4 it exits 0, so `cargo test --workspace --test cli` - task 4's own Verify - fails. Worse, `verbatim()` at cli.rs:58 sets only VERBATIM_DATA_DIR, so the spawned no-arg ingest resolves roots from the real config and walks the developer's live ~/.claude: the test suite ingests 2,079 private transcripts / 988 MB into a temp dir before failing the exit-code assertion. Neither `crates/verbatim/tests/cli.rs` nor any transcript-root isolation for the existing CLI bench appears in PLAN-1's `files:` list." },

  { "file": ".planning/phases/2/PLAN-1.md",
    "line": 365,
    "severity": "high",
    "claim": "The phase 2 columns are added only by a bring-forward gated on `Store::rebuild_required`, but `status` and `verify` open through `Store::open`, which runs no DDL on an existing store - so both break on an upgraded store until an ingest happens to run.",
    "failure_scenario": "`Store::open` (store/open.rs:99) executes CREATE_SQL only when the store is Fresh; an existing phase 1 store gets no new column. Task 1 puts the ALTER TABLE bring-forward inside the rebuild that only `reindex::open_up_to_date` triggers. User upgrades the binary and runs `verbatim status` before any ingest: status selects the new `runs` files-committed/files-failed columns and rusqlite returns `no such column`, so ING-09's command exits 1 on a healthy store. Same path for `verbatim verify` after PLAN-3 task 3 adds the divergence column to `verify::walk`'s SELECT (verify.rs:77). Status cannot fix this by calling `open_up_to_date` instead: that runs a destructive derived-table rebuild with no ingest lock, the exact defect gate fix 3e5d9ff closed." },

  { "file": ".planning/phases/2/PLAN-1.md",
    "line": 184,
    "severity": "high",
    "claim": "Exact-equality on the encoded directory name cannot exclude a worktree project directory, while the read-path predicate hides it because D-06 folds worktrees into the parent repo - so an excluded project's own sessions are read, archived into every table, then filtered on read.",
    "failure_scenario": "Measured on the real tree: 15 of 69 project directories are `<repo>--claude-worktrees-<name>` for `-data-code-cadence`, `-data-code-hindsight` and `-data-projects-assistant`, and 50 of 69 directory names extend another directory's encoded name. Exclude `/data/projects/cadence`: the encoded test matches only `-data-projects-cadence`, so the 6 worktree directories are walked and every file under them is opened, blobbed and given rows in sessions/session_meta/turns/turns_fts/watermarks. PLAN-2 task 2 then resolves their project to `/data/projects/cadence` (D-06) and PLAN-2 task 6's path-prefix predicate hides them at read time. ING-08's 'never read rather than read-then-filtered' is violated for the majority of the excluded project's own sessions, which is precisely the claude-mem failure mode PROJECT.md cites. AC5 still passes because it only exercises one directory." },

  { "file": ".planning/phases/2/PLAN-1.md",
    "line": 9,
    "severity": "medium",
    "claim": "PLAN-1's `files:` list omits `crates/verbatim-core/src/reindex.rs`, but task 1 requires the additive bring-forward to run inside `reindex::reindex`'s transaction.",
    "failure_scenario": "Task 1 (line 136) says the ALTER TABLE ADD COLUMN sweep runs 'where `reindex::open_up_to_date` already acts on `Store::rebuild_required` and inside the same transaction as the rebuild'. That transaction is opened at reindex.rs:51 and committed at reindex.rs:118; `open_up_to_date` (reindex.rs:132) holds no transaction of its own. Implementing task 1 as written therefore edits a file outside the plan's lease, or the implementer puts the ALTER in a separate transaction and the stated invariant - 'a store stamped up to date whose bring-forward rolled back cannot exist' - is silently dropped. Phase 1 already took a deviation for exactly this class (`crates/verbatim-core/src/store/open.rs` outside PLAN-2's lease)." },

  { "file": ".planning/phases/2/PLAN-3.md",
    "line": 250,
    "severity": "medium",
    "claim": "AC1's corpus assertion - archived session count equals the file count the test walks itself - fails on correct behaviour against a live tree.",
    "failure_scenario": "Two ways it false-fails. (a) A transcript whose bytes past offset 0 hold no complete line makes `scan.consumed()==0`, `ingest_locked` returns `Outcome::UpToDate` (ingest/mod.rs:92-95) and writes no `sessions` row, so file count exceeds session count; a transcript created moments before the pass is exactly that shape. (b) The tree is live - the Claude Code session running the test is appending to its own transcript and spawns subagents that create new `agent-*.jsonl` files - so the independent walk and the pass see different file sets and the equality fails whichever way the race lands. The plan's Notes anticipate corpus growth between plans but not growth *inside* one run, and the assertion is AC1's only proof." },

  { "file": ".planning/phases/2/PLAN-3.md",
    "line": 241,
    "severity": "medium",
    "claim": "The corpus test isolates the data directory but not verbatim's own config directory, so the developer's real `verbatim.toml` roots and exclusions decide what the AC1 run actually walks.",
    "failure_scenario": "PLAN-1 task 2 makes `CLAUDE_CONFIG_DIR` replace only the *default* root, so an explicit `roots` list in `$XDG_CONFIG_HOME/verbatim/verbatim.toml` wins over whatever the corpus env var names. Once John configures exclusions (the feature this phase ships), the corpus run silently walks his configured roots and skips his excluded projects, while the test's independent walk counts the env-var tree - the AC1 equality then fails, or worse passes against a tree nobody asked it to measure. Task 5 names no config-directory override for the test process." },

  { "file": ".planning/phases/2/PLAN-3.md",
    "line": 179,
    "severity": "medium",
    "claim": "The pass cannot tell the short-transcript failure apart from other per-file failures, because `read_tail` reports it as a generic `Error::Io` and `crates/verbatim-core/src/error.rs` is outside PLAN-3's lease.",
    "failure_scenario": "read_tail (ingest/mod.rs:391) returns `Error::io(path, io::Error::new(InvalidData, \"is N bytes but its watermark is at M\"))`. Task 3 requires the pass to set the divergence column for exactly this case and not for others. The only discriminators available are the message text or `ErrorKind::InvalidData` - and `path_key` (ingest/mod.rs:373) also returns `InvalidData` for a non-UTF-8 transcript path, so an `InvalidData` match flags a path-encoding failure as a watermark divergence on a session that has no `session_meta` row to flag. A message-substring match breaks the first time the wording changes. Adding a dedicated error variant needs `error.rs`, which appears in no PLAN-3 task." },

  { "file": ".planning/phases/2/PLAN-3.md",
    "line": 147,
    "severity": "medium",
    "claim": "Task 2 writes the boundary row inside `derive::derive_turn`, but the compaction metadata parsed by task 1 onto `Record` is not reachable from `TurnRow`, and the second `TurnRow` construction site is in `reindex.rs`, outside PLAN-3's lease.",
    "failure_scenario": "`derive_turn` receives `TurnRow { session_key, session_no, turn: &Turn, stream_offset, byte_len, record }` (derive.rs:29-41). Task 1 puts `subtype` and the `compactMetadata` bytes on `Record::parse`'s output; `Record` is not passed to the seam. Carrying them through requires either a new `TurnRow` field - which forces edits to the literal at reindex.rs:90-98, a file in no PLAN-3 task - or re-parsing `row.record` inside `derive_turn`, which is a second parse site for a field the plan just said the parser owns. If the implementer instead skips reindex.rs, the rebuild path drops the boundary row and task 2's own Verify (`--test reindex`, 'reproduces the boundary row with the same turn id') fails." },

  { "file": ".planning/phases/2/PLAN-1.md",
    "line": 316,
    "severity": "low",
    "claim": "ING-03 says recovery runs at the top of every run, but task 5 wires the recovery callable into the pass module only, leaving `ingest::run`'s single-file path with the rebuild half and no watermark sweep.",
    "failure_scenario": "`ingest::run` (ingest/mod.rs:76) calls `reindex::open_up_to_date` and then `ingest_locked` directly. A store whose watermark for transcript X sits ahead of the bytes X's committed blob holds is repaired only when a tree pass runs; `verbatim ingest <path.jsonl>` on that same store proceeds with the bad watermark and `read_tail` reads from an offset the archive never reached, so the gap is skipped and the blob loses those bytes silently. Task 5's own Action sentence says 'every ingest run invokes' it, then wires exactly one caller." },

  { "file": ".planning/phases/2/PLAN-1.md",
    "line": 232,
    "severity": "low",
    "claim": "The instrumented-open counter is specified as a bare count, but AC5 and task 4's Verify need per-path attribution.",
    "failure_scenario": "Task 3 defines 'a testkit-gated counter that tests can read and reset'. Task 4's Verify asserts 'the counted-open total for files beneath that directory is zero', and AC5 asserts 'zero opens of any file under that directory on the instrumented open counter'. A scalar counter cannot answer either question - it can only say how many opens happened in total, which is satisfied by a pass that opened the excluded files and skipped an equal number elsewhere. The instrument must record paths, and the plan never says so." }
] }
