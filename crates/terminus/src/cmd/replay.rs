//! `terminus replay`: what the logged history would have decided under other
//! rules (FEED-03).
//!
//! A shipped verb and not a testkit hook (D-09). "Retrieval is measurable
//! against history" is a claim about a user's own archive, and a diff that only
//! exists inside `cargo test` is a claim about the fixtures.
//!
//! **It cannot write.** The store is opened through
//! [`terminus_core::Store::open_read_only`], exactly as `search`, `show` and
//! `sessions` open it - so "without touching the live store" is enforced by
//! connection flags rather than by discipline (D-15), and the engine behind it
//! issues nothing but `SELECT`s.
//!
//! **The overrides stop here.** Every threshold flag below reaches
//! [`terminus_core::inject::prompt::Thresholds`] for this one read-only report
//! and nothing else: the live injection path constructs `Thresholds::default()`
//! inline, there is no config key and no environment variable, and
//! `DESIGN-BRIEF.md:245` keeps the precision-first defaults non-detunable
//! (D-08). Sweeping variants is what one build is for; shipping one is a code
//! change.

use terminus_core::feedback::replay::{self, Replayed};
use terminus_core::inject::prompt::Thresholds;

use super::json::Document;
use super::read::{self, Opened};
use super::Failure;

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "replay";

pub fn run(args: Args) -> Result<(), Failure> {
    let empty = document(&args.thresholds, &Replayed::default());
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        // A machine that has never ingested has no decisions to replay, which
        // is an answer rather than a failure (RCL-06).
        Opened::Nothing(reason) => return read::empty(empty, &reason, args.json),
    };

    // The same catch `stats` makes, for the same reason: nothing logged is an
    // empty diff with a reason, never a raw sqlite line escaping the envelope,
    // and half a log is a failure rather than a diff of nothing.
    match read::decision_log(&reader) {
        read::DecisionLog::Present => {}
        read::DecisionLog::Absent => return read::empty(empty, read::NO_DECISION_LOG, args.json),
        read::DecisionLog::Damaged(table) => {
            return Err(read::unusable(empty, read::damaged(table), args.json))
        }
    }

    let replayed = replay::replay(reader.store(), reader.config(), &args.thresholds)
        .map_err(|e| Failure::Operational(e.to_string()))?;

    if args.json {
        document(&args.thresholds, &replayed).emit();
        return Ok(());
    }

    println!("decisions        {}", replayed.decisions);
    for movement in &replayed.labels {
        // The arrow is the point of the line: a per-label count on its own says
        // what the store holds, and what a replay is asked is what would change.
        println!(
            "{:<16} {} -> {}",
            movement.label, movement.old, movement.new
        );
    }
    println!("changed          {} decision(s)", replayed.changed.len());
    Ok(())
}

/// The envelope for this command, with the diff already in it.
///
/// The thresholds travel with the answer because a diff means nothing without
/// the numbers that produced it - including on the empty-store path, where they
/// are the only thing there is to report.
fn document(thresholds: &Thresholds, replayed: &Replayed) -> Document {
    let labels: Vec<serde_json::Value> = replayed
        .labels
        .iter()
        .map(|movement| {
            serde_json::json!({
                "label": movement.label,
                "old": movement.old,
                "new": movement.new,
            })
        })
        .collect();
    Document::new(COMMAND)
        .field("thresholds", thresholds_of(thresholds))
        .field("decisions", replayed.decisions)
        .field("labels", labels)
        .field("changed", replayed.changed.clone())
}

/// The six numbers in force, whether or not a flag moved any of them.
fn thresholds_of(thresholds: &Thresholds) -> serde_json::Value {
    serde_json::json!({
        "ranked": thresholds.ranked,
        "compacted_ranked": thresholds.compacted_ranked,
        "entity_rank": thresholds.entity_rank,
        "co_occurring": thresholds.co_occurring,
        "max_turns": thresholds.max_turns,
        "max_candidates": thresholds.max_candidates,
    })
}

/// The parsed command line for `replay`.
#[derive(Debug)]
pub struct Args {
    pub json: bool,
    /// The compiled-in values with whatever the flags moved.
    pub thresholds: Thresholds,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut json = false;
    // The default is the shipped value of every one of the six, so a run with
    // no flags is the question "does this build reproduce its own labels".
    let mut thresholds = Thresholds::default();

    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long("ranked") => thresholds.ranked = count(parser, "ranked")?,
            Long("compacted-ranked") => {
                thresholds.compacted_ranked = count(parser, "compacted-ranked")?
            }
            Long("entity-rank") => thresholds.entity_rank = count(parser, "entity-rank")?,
            Long("co-occurring") => thresholds.co_occurring = count(parser, "co-occurring")?,
            Long("max-turns") => thresholds.max_turns = count(parser, "max-turns")?,
            Long("max-candidates") => thresholds.max_candidates = count(parser, "max-candidates")?,
            Long(super::JSON_FLAG) => json = true,
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    Ok(Args { json, thresholds })
}

/// One threshold's value, as a whole number.
///
/// Zero is accepted. `--max-turns 0` asks what the archive would look like with
/// injection switched off, and that is a legitimate end of a sweep rather than
/// a mistake in the command line - the point of the verb is to answer questions
/// about rules nobody has shipped.
fn count(parser: &mut lexopt::Parser, flag: &str) -> Result<usize, Failure> {
    let raw = super::value(parser, flag)?;
    raw.parse::<usize>()
        .map_err(|_| Failure::Misuse(format!("--{flag} {raw:?} is not a whole number")))
}
