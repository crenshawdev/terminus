//! Every table phase 1 needs, in one place, applied when a store is initialized.
//!
//! Two groups, and the split is the whole architecture. `sessions` and
//! `session_meta` are the archive: the blob is truth and the archive table
//! never migrates (`DESIGN-BRIEF.md:94`). `turns`, `turns_fts`, `entities`,
//! `paths` are derived and rebuildable from the blobs alone, which is what lets
//! a format bump rebuild rather than migrate. `watermarks`, `runs` and `meta`
//! are operational state.
//!
//! `decisions` and `labels` join the archive group in phase 6 (D-03): a logged
//! injection decision is prompt-time state the blobs do not contain, so it is
//! kept rather than rebuilt, and a `reindex` must never drop it. `observations`
//! joins it in phase 7 (D-02): its judgment half is one paid model call, which
//! no blob replay reproduces either.

/// Bits of a turn id reserved for the per-session turn ordinal.
///
/// 2^24 turns per session against a largest observed session of 10.1 MB (p90
/// 1.0 MB): even at an implausible 100 bytes per record that is a hundred
/// thousand turns, so the ordinal space has three orders of magnitude of room.
/// The remaining 39 bits carry the session surrogate, half a trillion sessions.
pub const TURN_SEQ_BITS: u32 = 24;

/// One past the largest turn ordinal a session may hold.
pub const MAX_TURNS_PER_SESSION: i64 = 1 << TURN_SEQ_BITS;

/// The deterministic `turns.id` encoding (D-10).
///
/// `sessions.session_no` is a stable surrogate that survives a rebuild because
/// `sessions` is never dropped, and `turn_seq` comes from byte order within the
/// transcript (D-02). So dropping `turns` and rebuilding it reproduces exactly
/// the same ids, which is what stops AC4's fixed query set from returning the
/// same row count while pointing at different turns.
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
/// Foreign keys are declared for documentation and are deliberately not
/// enforced: `PRAGMA foreign_keys` stays off, because `reindex` drops the
/// derived children wholesale and enforcement would only dictate a drop order.
/// Nothing relies on the database rejecting an orphan; ingest writes parent and
/// child inside one transaction (STOR-02), which is the stronger guarantee.
pub const CREATE_SQL: &str = "\
-- ARCHIVE. Truth. Never migrated, only ever appended to.

-- Keyed on the transcript FILE identity, never on the record's session id:
-- 812 sidecar files report their parent's sessionId, so keying on that would
-- overwrite a parent session's blob with a 3 KB agent transcript (D-01).
CREATE TABLE IF NOT EXISTS sessions (
    session_key TEXT    PRIMARY KEY,
    session_no  INTEGER NOT NULL UNIQUE,
    blob        BLOB    NOT NULL
);

CREATE TABLE IF NOT EXISTS session_meta (
    session_key       TEXT    PRIMARY KEY REFERENCES sessions(session_key),
    session_id        TEXT,
    transcript_path   TEXT    NOT NULL,
    checksum          BLOB    NOT NULL,
    uncompressed_len  INTEGER NOT NULL,
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
    -- parsed into typed columns (D-04). The format is undocumented and may
    -- drift, and 816 real meta files carry `agentType`, `description`,
    -- `toolUseId`, `spawnDepth` and `model`; keeping the bytes lets phase 3
    -- extract `description` without a reingest and bets no column on the shape.
    agent_meta        BLOB,
    -- Set when the transcript on disk is shorter than this session's stored
    -- watermark (D-13). The pass skips such a file rather than re-ingesting it
    -- from offset 0 - the append-only property is an observation over one
    -- 25-day corpus, not a guarantee - and `verbatim verify` reports the
    -- divergence off this column.
    transcript_diverged INTEGER,
    -- The project key this session had BEFORE worktree mapping folded it into
    -- its parent repo (D-06, ING-05). Both keys are stored so a later deletion
    -- of the worktree directory cannot un-key an already-archived session, and
    -- so the read-side exclusion test can match either path.
    project_pre_worktree TEXT,
    -- Which `[capture]` mode this session's bytes were stored under (ING-07,
    -- phase 8 D-13). Per session rather than a store-wide `meta` key, because
    -- the byte-for-byte claim for `full` has to be assertable about a session in
    -- an existing store and not only about a store built from scratch.
    --
    -- Null is `full`: it is what every row written before this column existed
    -- means, and those rows were all written by a binary that had no other mode.
    -- The column reads `full` only while EVERY append to the session has been
    -- full - `blob::append` copies completed blocks across untouched, so bytes
    -- written under an earlier mode are never revisited and a store whose mode
    -- changes mid-life holds a mix inside one blob.
    --
    -- Declared last, and appended last in `BRING_FORWARD_COLUMNS`, which is that
    -- constant's own rule: `ALTER TABLE ADD COLUMN` appends, so a column placed
    -- mid-table would give a fresh store and an upgraded store different column
    -- orders.
    capture_mode      TEXT
);

CREATE INDEX IF NOT EXISTS idx_session_meta_session_id
    ON session_meta(session_id);
CREATE INDEX IF NOT EXISTS idx_session_meta_continues_from
    ON session_meta(continues_from);
CREATE INDEX IF NOT EXISTS idx_session_meta_path
    ON session_meta(transcript_path);
-- Project-scoped search filters on this column, and phase 5's injection will
-- read it on every prompt. D-19: it reaches a store that already exists only
-- because `reindex` runs this whole batch and phase 3's `DERIVED_SCHEMA` bump
-- forces that reindex - `Store::open` runs `CREATE_SQL` only when a whole table
-- is missing, and `BRING_FORWARD_COLUMNS` covers columns, not indexes. An index
-- that existed on fresh stores and silently not on upgraded ones is two
-- machines running one binary at different speeds with nothing reporting why.
CREATE INDEX IF NOT EXISTS idx_session_meta_project
    ON session_meta(project);

-- One `UserPromptSubmit` decision, non-fires included (FEED-01). Archival, and
-- deliberately NOT in `DERIVED_TABLES`: a decision records prompt-time state -
-- what the prompt named, what the index answered, which thresholds were in
-- force, what was suppressed - that no blob replay can reconstruct, so a
-- `reindex` that dropped it would silently delete the entire history FEED-03's
-- replay diff and FEED-04's precision are computed over (D-03, a recorded
-- divergence from `DESIGN-BRIEF.md:92`).
--
-- The row is drained out of a per-prompt file by a later ingest pass and never
-- written by the hook itself (D-01): a SQLite write on the prompt path sits
-- behind the ingest writer, measured holding the store for 49 s.
--
-- Anchored by `session_id` and wall clock, never by `turn_seq` (D-04): the
-- injector never reads the live transcript, so it cannot know a turn index, and
-- labelling resolves the anchor to a turn afterwards.
CREATE TABLE IF NOT EXISTS decisions (
    id           INTEGER PRIMARY KEY,
    session_id   TEXT,
    -- Wall-clock UTC, ISO-8601, read on the prompt path.
    ts           TEXT,
    cwd          TEXT,
    prompt       TEXT,
    -- Max `sessions.session_no` when the decision was taken, which is what
    -- bounds a replay to the index as it stood (D-10). Null only until the
    -- drain stamps it: a prompt naming no candidate spelling never opens the
    -- store and so has no watermark of its own.
    watermark_session_no INTEGER,
    -- The character proxy, never tokens (D-12): 0 for every non-fire.
    chars_injected INTEGER,
    -- The list-shaped payload, each column one JSON document. JSON rather than
    -- five child tables because nothing joins on them - they are read back
    -- whole by `replay` and `stats` - and a decision must land in one insert.
    -- The spellings the prompt was asked about, strongest first.
    spellings    TEXT,
    -- The scored candidates: turn id, relevance, entity score, entity count and
    -- the matched `(kind, value)` pairs (D-05).
    candidates   TEXT,
    -- The turns injected, with the characters each one spent.
    injected     TEXT,
    -- The turns refused, with their reason.
    suppressed   TEXT,
    -- The threshold values in force, logged per decision because they are
    -- compile-time constants a later build may change (D-08).
    thresholds   TEXT
);

CREATE INDEX IF NOT EXISTS idx_decisions_session ON decisions(session_id);

-- What a decision turned out to be worth (FEED-02). Archival for the same
-- reason `decisions` is: recomputing labels under changed rules is what
-- `verbatim replay` is for, and dropping them on reindex would leave
-- `verbatim stats` reporting zero precision until the next pass relabels.
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

-- One session's account of itself (OBS-01..OBS-04). Archival, and deliberately
-- NOT in `DERIVED_TABLES`, for the reason `decisions` is not (phase 7 D-02).
-- The mechanical half could in principle be recomputed from the blob, but the
-- judgment half cannot: it is one paid model call whose answer nothing in the
-- archive reproduces, so a `reindex` that dropped this table would silently
-- delete every purchased summary in it. `verbatim observations regenerate`
-- (OBS-07) is the only rebuild path, and the two halves share one row because
-- they describe one session and are written and read together.
--
-- `mechanical` is one JSON document rather than a column per fact, the way
-- `decisions` carries its list-shaped fields: nothing joins on them and they
-- are read back whole.
--
-- The claim lists carry NO foreign key to `turns` and cannot (phase 7 D-03,
-- and the same constraint the `labels` comment above records). Each entry of
-- `decisions`, `learned` and `unresolved` anchors itself to a `turn_id`
-- (OBS-03), but the anchor lives INSIDE a JSON document, so there is no column
-- a reference could be declared on - which is just as well: `turns` is a table
-- `reindex` drops, and a declared reference would make the first `reindex`
-- after the first observation fail outright.
--
-- `observations.decisions` and the `decisions` TABLE are different objects
-- with the same name. Every statement naming the column qualifies it.
CREATE TABLE IF NOT EXISTS observations (
    session_key    TEXT PRIMARY KEY REFERENCES sessions(session_key),
    session_id     TEXT,
    generated_at   TEXT,
    -- The OBS-01 facts: files, tools, commands with their arguments, errors,
    -- branch, commits, turn count, duration, compactions.
    mechanical     TEXT,
    -- The judgment half. Null until a provider is configured and answers.
    status         TEXT,
    model          TEXT,
    prompt_version TEXT,
    topic          TEXT,
    outcome        TEXT,
    decisions      TEXT,
    learned        TEXT,
    unresolved     TEXT,
    -- The response as it arrived, kept for the OBS-04 `parse_failed` arm: a
    -- response that would not parse is never dropped silently.
    raw            TEXT,
    tokens         INTEGER
);

-- DERIVED. Rebuildable from the blobs alone; dropped and recreated by reindex.

-- stream_offset and byte_len address the UNCOMPRESSED session stream, never
-- the compressed blob (D-04). The blob header owns the translation, so a
-- future block-size change re-blobs and rewrites no turn row - which is the
-- point of an archive table that never migrates.
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
    -- Whether the PERSON typed this turn or the harness wrote it (INJ-07,
    -- phase 1 D-02): 1 typed, 0 harness-authored. The rule reads the record's
    -- own `message.content` blocks and nothing else, so it survives `[capture]`
    -- elision and needs no cross-record join (D-01, D-07).
    --
    -- Null means nothing derived this, which is what a non-`user` row is - the
    -- classification is written for `record_type = 'user'` and for no other
    -- type - and what a preserved evicted session's rows are: `reindex` skips
    -- them, so they keep null whatever the declaration says. Nullable rather
    -- than `NOT NULL DEFAULT` for exactly that reason (D-04): a default would
    -- silently read every preserved row as one class, and the no-third-state
    -- claim is asserted as a count query returning zero instead. The `capture_mode`
    -- column above is the same shape - a nullable added column whose null has a
    -- documented meaning.
    --
    -- Declared last, and appended last in `BRING_FORWARD_COLUMNS`, which is that
    -- constant's own rule: `ALTER TABLE ADD COLUMN` appends, so a column placed
    -- mid-table would give a fresh store and an upgraded store different column
    -- orders.
    is_typed      INTEGER,
    UNIQUE (session_key, turn_seq)
);

CREATE INDEX IF NOT EXISTS idx_turns_uuid ON turns(uuid);
CREATE INDEX IF NOT EXISTS idx_turns_ts ON turns(ts);

-- One row per compaction boundary, keyed on the turn that IS the boundary
-- (D-21: a `type: system` record with `subtype: compact_boundary` already
-- classifies as a turn, so this is a derived row and not a new record class).
-- `metadata` holds that record's `compactMetadata` object bytes verbatim
-- (D-08). No dropped-turn set is computed: exactly one real boundary exists in
-- 300,556 measured records and its own token counts contradict the design
-- brief's reading of `preservedMessages.uuids`, so the complement is derived at
-- query time in phase 5 (INJ-05) where a wrong reading costs no reingest.
CREATE TABLE IF NOT EXISTS compaction_boundaries (
    turn_id  INTEGER PRIMARY KEY REFERENCES turns(id),
    metadata BLOB
);

-- content='' because the text lives in the blob and storing it twice would
-- double the store; contentless_delete=1 because re-deriving a turn deletes
-- and reinserts at a known rowid, which is what makes a rebuild idempotent by
-- construction rather than by a uniqueness check (D-10). rowid IS turns.id.
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

-- The byte offset just past the last newline of a transcript, never its end
-- (D-14): a trailing partial line is a record still being written.
CREATE TABLE IF NOT EXISTS watermarks (
    transcript_path TEXT    PRIMARY KEY,
    byte_offset     INTEGER NOT NULL,
    updated_at      TEXT
);

-- One ingest pass, whatever the file count (D-10). There is no log file;
-- `status` surfaces this table.
--
-- `files_seen` is what the pass WALKED. A pass over a tree walks thousands of
-- files, commits most of them and skips the damaged ones (D-12), and one
-- integer cannot carry those three numbers - so the other two get columns of
-- their own rather than being inferred from `error` being non-null.
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
    files_failed    INTEGER NOT NULL DEFAULT 0
);

-- The two version integers the gate reads before any write (D-09).
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
";

/// The tables a store carries, excluding FTS5's shadow tables.
pub const TABLES: &[&str] = &[
    "sessions",
    "session_meta",
    // Both reach an already-initialized store through `bring_forward`'s
    // missing-table arm on the next `Store::open`, which is what lets phase 6
    // add two tables with no `DERIVED_SCHEMA` bump and no forced reindex (D-02).
    "decisions",
    "labels",
    // Phase 7 D-01, on the same terms: one more table declared here and absent
    // from `DERIVED_TABLES`, reaching an already-initialized store through the
    // same arm, with no `DERIVED_SCHEMA` bump and so no forced reindex.
    "observations",
    "turns",
    "compaction_boundaries",
    "turns_fts",
    "entities",
    "paths",
    "watermarks",
    "runs",
    "meta",
];

/// The derived tables `reindex` drops and rebuilds from the blobs (STOR-04).
/// `sessions` and `session_meta` are absent by design, and so are `decisions`
/// and `labels`: nothing in a blob can reconstruct what an injector decided at
/// prompt time (D-03). `observations` is absent for the same reason and one
/// more (phase 7 D-02): its judgment half is a paid model call, so a `reindex`
/// that dropped the table would delete summaries that cost money to produce.
///
/// In creation order, because `reindex` drops in reverse: a boundary row
/// references `turns(id)`, so `compaction_boundaries` sits **after** `turns`
/// here and is therefore dropped **before** it.
pub const DERIVED_TABLES: &[&str] = &[
    "turns",
    "compaction_boundaries",
    "turns_fts",
    "entities",
    "paths",
];

/// Columns added to an existing table after that table first shipped, with the
/// type clause that adds each one back.
///
/// Every statement in [`CREATE_SQL`] is `IF NOT EXISTS`, so re-running it over
/// a store that already has `session_meta` and `runs` creates a new *table* and
/// adds no *column* to either. This list is the other half: `Store::open`
/// compares it against `PRAGMA table_info` and issues
/// `ALTER TABLE ... ADD COLUMN` for whatever is missing, which is why a binary
/// carrying a newer schema can run `status` against a store written by an older
/// one without hitting `no such column`.
///
/// Order matters: `ALTER TABLE ADD COLUMN` appends, so these must be listed in
/// the same order [`CREATE_SQL`] declares them and must be declared at the END
/// of their table there. A fresh store and an upgraded store then carry
/// identical column order. Adding a column in a later phase means appending it
/// in both places, never inserting it.
///
/// Only additive changes belong here. Adding a column is safe on any opener;
/// dropping, renaming or rewriting one is not, and the archive tables
/// (`sessions`, `session_meta`) never migrate beyond an added column
/// (`DESIGN-BRIEF.md:94`).
pub const BRING_FORWARD_COLUMNS: &[(&str, &[(&str, &str)])] = &[
    (
        "session_meta",
        &[
            ("agent_meta", "BLOB"),
            ("transcript_diverged", "INTEGER"),
            ("project_pre_worktree", "TEXT"),
            ("capture_mode", "TEXT"),
        ],
    ),
    (
        "runs",
        &[
            ("files_committed", "INTEGER NOT NULL DEFAULT 0"),
            ("files_failed", "INTEGER NOT NULL DEFAULT 0"),
        ],
    ),
    // `turns` is a DERIVED table, and it is here anyway (phase 1 D-08). The
    // preserving rebuild path - a store holding an evicted session - skips the
    // DROP loop and runs only `CREATE TABLE IF NOT EXISTS`, a no-op on a table
    // that already exists, so nothing there would ever create the column;
    // `derive::derive_turn` names it in an explicit INSERT list and would then
    // fail with `no such column` on every future pass, permanently, because
    // `reindex::open_up_to_date` runs at the top of each one.
    ("turns", &[("is_typed", "INTEGER")]),
];
