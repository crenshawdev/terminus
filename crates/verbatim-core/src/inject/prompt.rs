//! The `UserPromptSubmit` relevance injection (INJ-03, INJ-04, INJ-05).
//!
//! Silence is the answer this arm gives most of the time and the one it is
//! built to protect: three near-misses are worse than nothing at all
//! (`DESIGN-BRIEF.md:245`), so a turn is injected only on a structural match -
//! a rank-1-to-3 exact entity, or two independent entities co-occurring - and
//! never on a BM25 score, which is not comparable across queries.
//!
//! PLAN-3 fills the retrieval in and PLAN-4 the suppressions. What is here is
//! the seam: the signature the binary calls, which does not change as those
//! land, and the answer this task gives to every prompt.

use std::path::Path;

use super::Payload;
use crate::config::Config;

/// Render the injection for one `UserPromptSubmit`, or nothing.
///
/// Nothing, always, until PLAN-3 lands the retrieval. That is the correct
/// behaviour for this commit rather than a placeholder: an event with no
/// injection is an event that writes nothing to stdout and exits 0, which is
/// exactly what the four events did before this phase started.
pub fn user_prompt_submit(
    _data_dir: &Path,
    _config: &Config,
    _payload: &Payload,
) -> Option<String> {
    None
}
