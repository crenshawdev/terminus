//! When a session is over (FEED-02, D-06).
//!
//! A decision can only be labelled against the turns that followed it, so
//! something has to say "no more turns are coming". There is no event that says
//! it: `SessionEnd` does not fire on a crash (`DESIGN-BRIEF.md:116`), and the
//! hook argv phases 4 and 5 settled means a spawned pass cannot know which
//! session just ended anyway. So the rule is idleness, evaluated at ingest, and
//! it covers the crash and the clean exit with one mechanism.
//!
//! **One UPDATE and no un-finalize arm.** A session that goes quiet for longer
//! than the threshold and then resumes is finalized early, and its later turns
//! are labelled against a decision set that has already closed. D-06 accepts
//! that: the alternative is a flag that flips back and forth and labels that
//! change under a reader, and the fix if the risk ever bites is to raise one
//! constant.

use rusqlite::Connection;

use crate::error::Result;

/// How long a session must be idle before a pass calls it finished.
///
/// Measured 2026-08-20 over 150 sampled transcripts: 1 internal idle gap longer
/// than 1 hour, and 0 longer than 6. So at this value no sampled session would
/// have been closed while it was still being written, which is the failure that
/// costs a permanent wrong label - while a threshold much higher would leave
/// yesterday's work unlabelled for a day.
pub const IDLE_HOURS: i64 = 6;

/// Mark every session whose last turn is older than [`IDLE_HOURS`], reporting
/// how many this pass closed.
///
/// **Lexicographic, in SQL, against `'now'`.** Stored timestamps are ISO-8601
/// UTC text with a fixed field width, so string order is time order and no date
/// parsing happens at all - and the same `strftime` shape that wrote every other
/// timestamp in this store computes the cutoff, so the two spellings cannot
/// drift apart.
///
/// A row with no `last_turn_at` is left alone: a session with no turn to read a
/// clock off is not a session that has been idle, it is one nothing is known
/// about. `is_final IS NULL` is what makes the statement incremental - an
/// already-closed session is never rewritten, so the count means "closed on this
/// pass" rather than "closed".
///
/// Sidecars get no special case. A sidecar carries its own `last_turn_at`, and a
/// sidecar that finished six hours ago is finished whatever its parent is doing.
pub fn finalize(conn: &Connection) -> Result<usize> {
    let closed = conn.execute(
        "UPDATE session_meta
            SET is_final = 1
          WHERE is_final IS NULL
            AND last_turn_at IS NOT NULL
            AND last_turn_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
        [format!("-{IDLE_HOURS} hours")],
    )?;
    Ok(closed)
}
