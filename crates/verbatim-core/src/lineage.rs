//! Session lineage: what a transcript continues from.
//!
//! # Which namespace this is in
//!
//! `session_meta.continues_from` holds a **session id** - the value Claude Code
//! writes in a record's `sessionId` / `session_id` fields - while `sessions` is
//! keyed on **file identity**, the canonical transcript path (D-01). The two
//! namespaces are deliberately not the same thing, because 818 real sidecar
//! files report their parent's `sessionId` and keying the archive on that would
//! overwrite a parent session's blob with a 3 KB agent transcript.
//!
//! So a resolver crossing from `continues_from` to a `session_key` must expect
//! **zero, one or many** rows:
//!
//! - zero, because a predecessor may have been aged out by Claude Code's
//!   `cleanupPeriodDays` and never ingested at all - 2 of the 173 real files
//!   naming a foreign session name one that is not on disk (D-19). The column
//!   carries no foreign key and imposes no ingest order for exactly that
//!   reason;
//! - many, because one session id can span several files (a resumed session
//!   continues in a new file, and every sidecar reports its parent's id).
//!
//! # Two rules
//!
//! **A session never continues from itself (D-01).** Measured over all 1,253
//! top-level transcripts: 433 carry a `session_id` field, of which 253 carry
//! only their own id, 169 only another session's and 11 both. Taking the first
//! `session_id` without comparison makes one in five archived sessions claim to
//! continue from itself, and every phase 3 and phase 5 thread walk then either
//! cycles or special-cases the self-edge at each read site.
//!
//! **Continuation is a fan-out, not a chain (D-02).** One predecessor may be
//! named by several successors - session `ebd78c2b` is named by four separate
//! later transcripts in one project - so nothing here is allowed to assume a
//! single successor, and thread reconstruction is a tree walk.

use crate::parse::Scan;

/// The session id this transcript reports as its **own**: the first `sessionId`
/// in byte order.
///
/// A tail pass sees only the tail, which still carries `sessionId` on every
/// record, so this answers the same for a first pass and a resumed one.
pub fn own_session_id(scan: &Scan) -> Option<&str> {
    scan.records.iter().find_map(|r| r.session_id.as_deref())
}

/// Would linking to `candidate` link this session to itself (D-01)?
pub fn is_self_link(candidate: &str, own: Option<&str>) -> bool {
    own == Some(candidate)
}

/// The session this transcript continues from, out of the records' foreign
/// `session_id` fields.
///
/// Candidates equal to the transcript's own id are rejected - both the id the
/// record itself reports and the transcript's first one, since a file that
/// names two distinct ids must not link to either of its own. The first
/// surviving foreign id wins, so **byte order decides** when a file carries two
/// of them; 11 real files do.
pub fn foreign_predecessor(scan: &Scan) -> Option<String> {
    let own = own_session_id(scan);
    scan.records.iter().find_map(|record| {
        let candidate = record.foreign_session_id.as_deref()?;
        if is_self_link(candidate, own) || is_self_link(candidate, record.session_id.as_deref()) {
            return None;
        }
        Some(candidate.to_owned())
    })
}
