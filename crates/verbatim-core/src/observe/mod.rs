//! Observations: an auditable account of what happened in a finalized session
//! (OBS-01..OBS-04).
//!
//! **Two halves, one row.** The mechanical half is [`mechanical`]: parser-
//! derived facts that are always available, never involve a model and open no
//! network connection. The judgment half is a later plan's: one optional call
//! per finalized session, whose every claim carries a `turn_id` resolving to a
//! real turn. They share a row because they describe one session and are read
//! back together.
//!
//! **Archival, never derived (D-02).** `observations` is deliberately absent
//! from [`crate::store::schema::DERIVED_TABLES`]: the judgment half is a paid
//! model call that no blob replay reproduces, so a `reindex` that dropped the
//! table would delete summaries a user bought. `verbatim observations
//! regenerate` (OBS-07) is the only rebuild path.

pub mod mechanical;

pub use mechanical::{observe, Mechanical};
