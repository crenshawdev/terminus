//! What one turn contributes to the index.
//!
//! The blob is truth and this module is the derivation from it: given one
//! record's parsed JSON, it produces the text that belongs in `turns_fts.body`
//! and the entities that belong in `entities` and `paths`. Nothing here touches
//! the store - [`crate::derive::derive_turn`] is the only writer, so ingest and
//! rebuild cannot drift.
//!
//! Every rule is a pure function over a `serde_json::Value` and is unit-testable
//! on its own. That is deliberate: the design bars a custom FTS5 tokenizer
//! precisely so normalization stays in Rust where each rule can be called with
//! an input and compared against an answer (`.planning/PROJECT.md`).

pub mod entity;
pub mod expand;
pub mod text;

pub use entity::{Entity, KINDS};
pub use expand::MAX_EXPANSION_BYTES;
pub use text::{MAX_BODY_BYTES, MAX_DEPTH};

use serde_json::Value;

/// The whole `turns_fts.body` value for one turn record: D-01's text projection
/// followed by RCL-01's expansion tokens.
///
/// One column and not two. A second FTS column would change the table
/// declaration at `crates/verbatim-core/src/store/schema.rs` and the
/// `contentless_delete=1` rebuild property with it, for a separation nothing
/// queries against.
pub fn project(record: &Value) -> String {
    let mut body = text::project(record);
    expand::append_to(&mut body);
    body
}

/// Every entity one turn record leaves behind (RCL-02, RCL-03), capped.
///
/// The same parsed record the body came from, so a turn is read once and every
/// derived row for it comes out of that one read.
pub fn entities(record: &Value) -> Vec<Entity> {
    entity::extract(record)
}
