//! Every table phase 1 needs, in one place, applied when a store is initialized.
//!
//! Two groups, and the split is the whole architecture. `sessions` and
//! `session_meta` are the archive: the blob is truth and the archive table
//! never migrates (`DESIGN-BRIEF.md:94`). `turns`, `turns_fts`, `entities`,
//! `paths` are derived and rebuildable from the blobs alone, which is what lets
//! a format bump rebuild rather than migrate. `watermarks`, `runs` and `meta`
//! are operational state.

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
    parent_session_key TEXT
);

CREATE INDEX IF NOT EXISTS idx_session_meta_session_id
    ON session_meta(session_id);
CREATE INDEX IF NOT EXISTS idx_session_meta_continues_from
    ON session_meta(continues_from);
CREATE INDEX IF NOT EXISTS idx_session_meta_path
    ON session_meta(transcript_path);

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
    UNIQUE (session_key, turn_seq)
);

CREATE INDEX IF NOT EXISTS idx_turns_uuid ON turns(uuid);
CREATE INDEX IF NOT EXISTS idx_turns_ts ON turns(ts);

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

-- One committed ingest pass. There is no log file; `status` surfaces this.
CREATE TABLE IF NOT EXISTS runs (
    id           INTEGER PRIMARY KEY,
    started_at   TEXT    NOT NULL,
    finished_at  TEXT,
    duration_ms  INTEGER,
    files_seen   INTEGER NOT NULL DEFAULT 0,
    bytes_read   INTEGER NOT NULL DEFAULT 0,
    turns_added  INTEGER NOT NULL DEFAULT 0,
    error        TEXT
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
    "turns",
    "turns_fts",
    "entities",
    "paths",
    "watermarks",
    "runs",
    "meta",
];

/// The derived tables `reindex` drops and rebuilds from the blobs (STOR-04).
/// `sessions` and `session_meta` are absent by design.
pub const DERIVED_TABLES: &[&str] = &["turns", "turns_fts", "entities", "paths"];
