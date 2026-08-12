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

pub mod text;

pub use text::{project, MAX_BODY_BYTES, MAX_DEPTH};
