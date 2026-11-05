//! Every table a store carries, in one place, applied when one is initialized.
//!
//! Two groups, and the split is the whole architecture. `sessions`, `blocks`
//! and `session_meta` are the archive: the block rows are truth and the archive
//! tables never migrate. `turns`, `turns_fts`, `entities`, `paths` are derived
//! and rebuildable from the block rows alone, which is what lets a
//! derived-schema bump rebuild rather than migrate. `watermarks`, `runs` and
//! `meta` are operational state.
//!
//! `decisions` and `labels` belong to the archive group: a logged injection
//! decision is prompt-time state the archive does not contain, so it is kept
//! rather than rebuilt, and a `reindex` must never drop it. `observations`
//! belongs to it too: its facts come off the session's archived stream, and
//! retention eviction deletes that stream's block rows, so an evicted
//! session's observation cannot be recomputed.

/// Bits of a turn id reserved for the per-session turn ordinal.
///
/// 2^24 turns per session against a largest observed session of 10.1 MB (p90
/// 1.0 MB): even at an implausible 100 bytes per record that is a hundred
/// thousand turns, so the ordinal space has three orders of magnitude of room.
/// The remaining 39 bits carry the session surrogate, half a trillion sessions.
pub const TURN_SEQ_BITS: u32 = 24;

/// One past the largest turn ordinal a session may hold.
pub const MAX_TURNS_PER_SESSION: i64 = 1 << TURN_SEQ_BITS;

/// The deterministic `turns.id` encoding.
///
/// `sessions.session_no` is a stable surrogate that survives a rebuild because
/// `sessions` is never dropped, and `turn_seq` comes from byte order within the
/// transcript. So dropping `turns` and rebuilding it reproduces exactly the
/// same ids, which is what stops a fixed query set run across a rebuild from
/// returning the same row count while pointing at different turns.
///
/// Panics on a `turn_seq` outside the reserved space rather than silently
/// folding it into the next session's range.
pub fn turn_id(session_no: i64, turn_seq: i64) -> i64 {
    assert!(
        (0..MAX_TURNS_PER_SESSION).contains(&turn_seq),
        "turn_seq {turn_seq} outside the {MAX_TURNS_PER_SESSION}-turn space"
    );
    assert!(
        session_no >= 0,
        "session_no {session_no} must not be negative"
    );
    (session_no << TURN_SEQ_BITS) | turn_seq
}

/// The inverse of [`turn_id`]: `(session_no, turn_seq)`.
pub fn split_turn_id(id: i64) -> (i64, i64) {
    (id >> TURN_SEQ_BITS, id & (MAX_TURNS_PER_SESSION - 1))
}

/// The tables and indexes a store carries.
///
/// Foreign keys are enforced: the bundled SQLite is built with
/// `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`, so a child row goes in after its parent
/// and comes out before it, and `reindex` drops the derived children first.
/// Ingest also writes parent and child inside one transaction.
pub const CREATE_SQL: &str = "\
-- ARCHIVE. Truth. Never migrated, only ever appended to.

-- Keyed on the transcript FILE identity, never on the record's session id:
-- 812 sidecar files report their parent's sessionId, so keying on that would
-- write a 3 KB agent transcript over a parent session's archive.
CREATE TABLE IF NOT EXISTS sessions (
    session_key TEXT    PRIMARY KEY,
    session_no  INTEGER NOT NULL UNIQUE
);

-- A session's stream, cut into `session_meta.block_size` uncompressed bytes per
-- block, one row per block, each compressed on its own. `hash` is BLAKE3 over
-- the block's UNCOMPRESSED bytes, so a later zstd that encodes a block
-- differently fails no hash, and `session_meta.checksum` is BLAKE3 over these
-- hashes in `block_no` order.
--
-- An append rewrites at most the last row and adds rows after it; every other
-- row is never written again. A rowid table at SQLite's default page size,
-- measured against `WITHOUT ROWID` and larger pages on a copy of a real store:
-- it made the smallest file, the smallest append and the fastest point read.
--
-- Rows go in after their `sessions` row and come out before it. The reference
-- is enforced (see `reindex`'s drop order), so the other order fails.
CREATE TABLE IF NOT EXISTS blocks (
    session_no INTEGER NOT NULL REFERENCES sessions(session_no),
    block_no   INTEGER NOT NULL,
    bytes      BLOB    NOT NULL,
    hash       BLOB    NOT NULL,
    PRIMARY KEY (session_no, block_no)
);

CREATE TABLE IF NOT EXISTS session_meta (
    session_key       TEXT    PRIMARY KEY REFERENCES sessions(session_key),
    session_id        TEXT,
    transcript_path   TEXT    NOT NULL,
    checksum          BLOB    NOT NULL,
    uncompressed_len  INTEGER NOT NULL,
    -- Uncompressed bytes per block for this session, written on the session's
    -- first archive and never changed. Per session, so a later block size
    -- applies to new sessions without rewriting old ones. Nullable only so a
    -- hand-planted row needs no value; ingest always writes one.
    block_size        INTEGER,
    continues_from    TEXT,
    first_turn_at     TEXT,
    last_turn_at      TEXT,
    project           TEXT,
    cwd               TEXT,
    branch            TEXT,
    is_final          INTEGER,
    is_evicted        INTEGER,
    parent_session_key TEXT,
    -- The bytes of a sidecar's `agent-*.meta.json`, stored opaquely and never
    -- parsed into typed columns. The format is undocumented and may drift, and
    -- 816 real meta files carry `agentType`, `description`, `toolUseId`,
    -- `spawnDepth` and `model`. Ingest stores the bytes and only checks whether
    -- they are already stored; no code parses them. Keeping them means
    -- `description` can be extracted later without a reingest, and bets no
    -- column on the shape.
    agent_meta        BLOB,
    -- Set when the transcript on disk is shorter than this session's stored
    -- watermark. The pass skips such a file rather than re-ingesting it from
    -- offset 0 - the append-only property is an observation over one 25-day
    -- corpus, not a guarantee - and `terminus verify` reports the divergence
    -- off this column.
    transcript_diverged INTEGER,
    -- The project key this session had BEFORE worktree mapping folded it into
    -- its parent repo. Both keys are stored so a later deletion of the
    -- worktree directory cannot un-key an already-archived session, and so the
    -- read-side exclusion test can match either path.
    project_pre_worktree TEXT,
    -- Which `[capture]` mode this session's bytes were stored under. Per
    -- session rather than a store-wide `meta` key, because the byte-for-byte
    -- claim for `full` has to be assertable about a session in an existing
    -- store and not only about a store built from scratch.
    --
    -- Null is `full`: it is what every row written before this column existed
    -- means, and those rows were all written by a binary that had no other mode.
    -- The column reads `full` only while EVERY append to the session has been
    -- full - an append never rewrites a completed block, so bytes written under
    -- an earlier mode are never revisited and a store whose mode changes
    -- mid-life holds a mix inside one session.
    capture_mode      TEXT
);

CREATE INDEX IF NOT EXISTS idx_session_meta_session_id
    ON session_meta(session_id);
CREATE INDEX IF NOT EXISTS idx_session_meta_continues_from
    ON session_meta(continues_from);
CREATE INDEX IF NOT EXISTS idx_session_meta_path
    ON session_meta(transcript_path);
-- Project-scoped search filters on this column, the prompt-time recall's
-- search included, and the resume brief counts the scoped project's sessions
-- through this index. It reaches a store that already exists only because
-- `reindex` runs this whole batch and a `DERIVED_SCHEMA` bump forces that
-- reindex - `Store::open` runs `CREATE_SQL` only when a whole table is
-- missing, and `BRING_FORWARD_COLUMNS` covers columns, not indexes. An index
-- that existed on fresh stores and silently not on upgraded ones is two
-- machines running one binary at different speeds with nothing reporting why.
CREATE INDEX IF NOT EXISTS idx_session_meta_project
    ON session_meta(project);

-- One `UserPromptSubmit` decision, non-fires included. Archival, and
-- deliberately NOT in `DERIVED_TABLES`: a decision records prompt-time state -
-- what the prompt named, what the index answered, which thresholds were in
-- force, what was suppressed - that no archive replay can reconstruct, so a
-- `reindex` that dropped it would silently delete the entire history that
-- `terminus replay` diffs and `terminus stats` computes precision over.
--
-- The row is drained out of a per-prompt file by a later ingest pass and never
-- written by the hook itself: a SQLite write on the prompt path sits behind
-- the ingest writer, measured holding the store for 49 s.
--
-- Anchored by `session_id` and wall clock, never by `turn_seq`: the injector
-- never reads the live transcript, so it cannot know a turn index, and
-- labelling resolves the anchor to a turn afterwards.
CREATE TABLE IF NOT EXISTS decisions (
    id           INTEGER PRIMARY KEY,
    session_id   TEXT,
    -- Wall-clock UTC, ISO-8601, read on the prompt path.
    ts           TEXT,
    cwd          TEXT,
    prompt       TEXT,
    -- Max `sessions.session_no` when the decision was taken, which is what
    -- bounds a replay to the index as it stood. Null only until the drain
    -- stamps it: a prompt naming no candidate spelling never opens the store
    -- and so has no watermark of its own.
    watermark_session_no INTEGER,
    -- The character proxy, never tokens: 0 for every non-fire.
    chars_injected INTEGER,
    -- The list-shaped payload, each column one JSON document. JSON rather than
    -- five child tables because nothing joins on them - they are read back
    -- whole by `replay` and `stats` - and a decision must land in one insert.
    -- The spellings the prompt was asked about, strongest first.
    spellings    TEXT,
    -- The scored candidates: turn id, relevance, entity score, entity count and
    -- the matched `(kind, value)` pairs.
    candidates   TEXT,
    -- The turns injected, with the characters each one spent.
    injected     TEXT,
    -- The turns refused, with their reason.
    suppressed   TEXT,
    -- The threshold values in force, logged per decision because they are
    -- compile-time constants a later build may change.
    thresholds   TEXT,
    -- 1 when the prompt ran under the compacted pool, 0 when it did not.
    -- Null means the decision was drained before this column existed, so its
    -- value is unknown. Nullable rather than `NOT NULL DEFAULT 0`, which would
    -- record every earlier decision as ordinary for good.
    compacted    INTEGER,
    -- The turn ids the compaction dropped, one JSON document like the other
    -- list-shaped columns. Null for the same earlier decisions.
    dropped      TEXT
);

CREATE INDEX IF NOT EXISTS idx_decisions_session ON decisions(session_id);

-- What a decision turned out to be worth. Archival for the same reason
-- `decisions` is: recomputing labels under changed rules is what `terminus
-- replay` is for, and dropping them on reindex would leave `terminus stats`
-- reporting zero precision until the next pass relabels.
--
-- `turn_id` carries NO foreign key on purpose, unlike every other reference in
-- this schema. `turns` is a derived table `reindex` drops, foreign keys really
-- are enforced (the bundled SQLite is built with
-- `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`), and dropping a parent table with live
-- child rows is a constraint violation - so a declared reference here would
-- make the first `reindex` after the first label fail outright.
CREATE TABLE IF NOT EXISTS labels (
    id          INTEGER PRIMARY KEY,
    decision_id INTEGER NOT NULL REFERENCES decisions(id),
    turn_id     INTEGER,
    label       TEXT    NOT NULL,
    detail      TEXT,
    labeled_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_labels_decision ON labels(decision_id);

-- One session's account of itself. Archival, and deliberately NOT in
-- `DERIVED_TABLES`. The facts come off the session's archived stream, and
-- retention eviction deletes its block rows, so an evicted session's
-- observation cannot be recomputed and a `reindex` that dropped this table
-- would lose it for good. `terminus observations regenerate` is the only
-- rebuild path.
--
-- `mechanical` is one JSON document rather than a column per fact, the way
-- `decisions` carries its list-shaped fields: nothing joins on them and they
-- are read back whole.
CREATE TABLE IF NOT EXISTS observations (
    session_key    TEXT PRIMARY KEY REFERENCES sessions(session_key),
    session_id     TEXT,
    generated_at   TEXT,
    -- The mechanical facts: files, tools, commands with their arguments, errors,
    -- branch, commits, turn count, duration, compactions.
    mechanical     TEXT
);

-- DERIVED. Rebuildable from the block rows alone; dropped and recreated by
-- reindex.

-- stream_offset and byte_len address the UNCOMPRESSED session stream, never
-- the compressed block rows. The session's block size turns an offset into
-- block numbers, so a later block-size change rewrites no turn row - which is
-- the point of an archive that never migrates.
CREATE TABLE IF NOT EXISTS turns (
    id            INTEGER PRIMARY KEY,
    session_key   TEXT    NOT NULL REFERENCES sessions(session_key),
    turn_seq      INTEGER NOT NULL,
    uuid          TEXT,
    parent_uuid   TEXT,
    record_type   TEXT    NOT NULL,
    tool_name     TEXT,
    ts            TEXT,
    stream_offset INTEGER NOT NULL,
    byte_len      INTEGER NOT NULL,
    -- Whether the PERSON typed this turn or something else wrote it: 1 typed, 0
    -- written by the harness or sent by a script. The rule reads the record's
    -- own `message.content` blocks, `isMeta` and `entrypoint`, so it survives
    -- `[capture]` elision and needs no cross-record join.
    --
    -- Null means nothing derived this, which is what a non-`user` row is - the
    -- classification is written for `record_type = 'user'` and for no other
    -- type. Nullable rather than `NOT NULL DEFAULT` for exactly that reason: a
    -- default would silently read every other type as one class, and the
    -- no-third-state claim is asserted as a count query returning zero instead.
    -- `reindex` skips an evicted session, so its rows keep what ingest wrote.
    -- The `capture_mode` column above is the same shape - a nullable added
    -- column whose null has a documented meaning.
    is_typed      INTEGER,
    UNIQUE (session_key, turn_seq)
);

CREATE INDEX IF NOT EXISTS idx_turns_uuid ON turns(uuid);
CREATE INDEX IF NOT EXISTS idx_turns_ts ON turns(ts);

-- One row per compaction boundary, keyed by where its record sits in the
-- session stream rather than by a turn, so a boundary with no turn behind it -
-- one missing its `uuid` or `timestamp` - still has a place.
-- `stream_offset` is the same uncompressed-stream coordinate as
-- `turns.stream_offset`, so the two compare correctly under every capture mode,
-- and the dropped set is the turns at a lower offset than the boundary.
-- `metadata` holds the record's `compactMetadata` object bytes verbatim.
-- No dropped-turn set is computed: exactly one real boundary exists in 300,556
-- measured records and its own token counts contradict the design brief's
-- reading of `preservedMessages.uuids`, so injection derives the complement at
-- query time (`inject::compaction`), where a wrong reading costs no reingest.
CREATE TABLE IF NOT EXISTS compactions (
    session_key   TEXT    NOT NULL REFERENCES sessions(session_key),
    stream_offset INTEGER NOT NULL,
    metadata      BLOB,
    PRIMARY KEY (session_key, stream_offset)
);

-- One row per session that has a title, off its `ai-title` and `custom-title`
-- records. `is_custom` is what stops a later generated title replacing one a
-- person set.
CREATE TABLE IF NOT EXISTS session_titles (
    session_key TEXT    PRIMARY KEY REFERENCES sessions(session_key),
    title       TEXT    NOT NULL,
    is_custom   INTEGER NOT NULL
);

-- content='' because the text lives in the archive and storing it twice would
-- double the store; contentless_delete=1 because re-deriving a turn deletes
-- and reinserts at a known rowid, which is what makes a rebuild idempotent by
-- construction rather than by a uniqueness check. rowid IS turns.id.
CREATE VIRTUAL TABLE IF NOT EXISTS turns_fts
    USING fts5(body, content='', contentless_delete=1);

CREATE TABLE IF NOT EXISTS entities (
    turn_id    INTEGER NOT NULL REFERENCES turns(id),
    kind       TEXT    NOT NULL,
    value_norm TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_entities_lookup ON entities(kind, value_norm);
CREATE INDEX IF NOT EXISTS idx_entities_turn ON entities(turn_id);

CREATE TABLE IF NOT EXISTS paths (
    turn_id INTEGER NOT NULL REFERENCES turns(id),
    path    TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_paths_path ON paths(path);
CREATE INDEX IF NOT EXISTS idx_paths_turn ON paths(turn_id);

-- OPERATIONAL.

-- The byte offset just past the last newline of a transcript, never its end:
-- a trailing partial line is a record still being written.
CREATE TABLE IF NOT EXISTS watermarks (
    transcript_path TEXT    PRIMARY KEY,
    byte_offset     INTEGER NOT NULL,
    updated_at      TEXT
);

-- One ingest pass, whatever the file count. There is no log file; `status`
-- surfaces this table.
--
-- `files_seen` is what the pass WALKED. A pass over a tree walks thousands of
-- files, commits most of them and skips the damaged ones, and one integer
-- cannot carry those three numbers - so the other two get columns of their own
-- rather than being inferred from `error` being non-null.
--
-- `error` holds what went wrong and `notes` what a healthy pass did worth
-- saying (labels written, sessions retention evicted), so a reader that flags
-- a non-null `error` flags only failures.
CREATE TABLE IF NOT EXISTS runs (
    id           INTEGER PRIMARY KEY,
    started_at   TEXT    NOT NULL,
    finished_at  TEXT,
    duration_ms  INTEGER,
    files_seen   INTEGER NOT NULL DEFAULT 0,
    bytes_read   INTEGER NOT NULL DEFAULT 0,
    turns_added  INTEGER NOT NULL DEFAULT 0,
    error        TEXT,
    files_committed INTEGER NOT NULL DEFAULT 0,
    files_failed    INTEGER NOT NULL DEFAULT 0,
    notes           TEXT
);

-- The two version integers the gate reads before any write.
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
";

/// The tables a store carries, excluding FTS5's shadow tables.
pub const TABLES: &[&str] = &[
    "sessions",
    // Archive, never derived: `reindex` must never drop it.
    "blocks",
    "session_meta",
    // Both reach an already-initialized store through `bring_forward`'s
    // missing-table arm on the next `Store::open`, which is what lets them
    // arrive with no `DERIVED_SCHEMA` bump and no forced reindex.
    "decisions",
    "labels",
    // On the same terms: one more table declared here and absent from
    // `DERIVED_TABLES`, reaching an already-initialized store through the
    // same arm, with no `DERIVED_SCHEMA` bump and so no forced reindex.
    "observations",
    "turns",
    "compactions",
    "session_titles",
    "turns_fts",
    "entities",
    "paths",
    "watermarks",
    "runs",
    "meta",
];

/// The derived tables `reindex` drops and rebuilds from the block rows.
/// `sessions`, `blocks` and `session_meta` are absent by design, and so are
/// `decisions` and `labels`: nothing in the archive can reconstruct what an
/// injector decided at prompt time. `observations` is absent too: its facts
/// come off the session's archived stream, and retention eviction deletes its
/// block rows, so a `reindex` that dropped the table could not recompute an
/// evicted session's row.
///
/// In creation order, because `reindex` drops in reverse: `entities` and
/// `paths` reference `turns(id)`, so they sit **after** `turns` here and are
/// therefore dropped **before** it. `compactions` references only
/// `sessions(session_key)`, which no rebuild drops, so its place among them is
/// free, and so does `session_titles`.
pub const DERIVED_TABLES: &[&str] = &[
    "turns",
    "compactions",
    "session_titles",
    "turns_fts",
    "entities",
    "paths",
];

/// Columns added to an existing table after that table first shipped, with the
/// type clause that adds each one.
///
/// Every statement in [`CREATE_SQL`] is `IF NOT EXISTS`, so re-running it over
/// a store that already has a table adds no *column* to it. This list is the
/// other half: `Store::open` compares it against `PRAGMA table_info` and issues
/// `ALTER TABLE ... ADD COLUMN` for whatever is missing. A read-only open, which
/// `status` and every other read command use, adds nothing: it measures the
/// same list and reports what is absent through `Store::missing_columns`.
///
/// Empty until a column is added to a table a store already has.
///
/// Order matters: `ALTER TABLE ADD COLUMN` appends, so a column listed here
/// must be declared at the END of its table in [`CREATE_SQL`], in the same
/// order it is listed. A fresh store and an upgraded store then carry identical
/// column order. Adding a column means appending it in both places, never
/// inserting it.
///
/// Only additive changes belong here. Adding a column is safe on any opener;
/// dropping, renaming or rewriting one is not, and the archive tables
/// (`sessions`, `session_meta`) never migrate beyond an added column.
pub const BRING_FORWARD_COLUMNS: &[(&str, &[(&str, &str)])] = &[];
