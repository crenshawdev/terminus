//! The SQLite store: one file, one connection, opened behind a version gate.

pub mod open;
pub mod schema;
pub mod snapshot;

pub use schema::{split_turn_id, turn_id, DERIVED_TABLES, TABLES};

pub use open::{
    data_dir, RebuildRequired, Store, ARCHIVE_FORMAT, DB_FILE_NAME, DERIVED_SCHEMA,
    META_ARCHIVE_FORMAT, META_DERIVED_SCHEMA,
};
