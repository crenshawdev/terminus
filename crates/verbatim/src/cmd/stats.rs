//! `verbatim stats`: whether injection helps (FEED-04).
//!
//! A CLI data command and never an MCP tool (D-14). Every tool description sits
//! in every session's context forever, and the model does not need to know how
//! its own recall is scoring - the three recall tools are the whole surface
//! `DESIGN-BRIEF.md:187` allows.
//!
//! It reads through the shared read path, so a machine that has never ingested
//! gets an empty answer with a reason and exit 0 rather than a store created as
//! the side effect of a question (D-15). Unlike `status` - which opens for
//! write because it reports on the store itself - this is a report about a log,
//! and nothing about it needs a writable connection.
//!
//! The numbers are characters, never tokens (D-12), and the output says so.

use verbatim_core::feedback::stats::{self, Stats};

use super::json::Document;
use super::read::{self, Opened};
use super::Failure;

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "stats";

pub fn run(json: bool) -> Result<(), Failure> {
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        Opened::Nothing(reason) => return read::empty(document(&Stats::default()), &reason, json),
    };

    let stats =
        stats::stats(reader.store().conn()).map_err(|e| Failure::Operational(e.to_string()))?;

    if json {
        // The same numbers the human output prints, and no others.
        document(&stats).emit();
        return Ok(());
    }

    println!("decisions         {}", stats.decisions);
    println!("injected turns    {}", stats.injected_turns);
    println!("hits              {}", stats.hits);
    println!("false positives   {}", stats.false_positives);
    // Two decimals, or the word: a precision printed as 0.00 on an archive
    // whose sessions are all still open would read as "injection never helps".
    match stats.precision {
        Some(precision) => println!("precision         {precision:.2}"),
        None => println!("precision         (nothing labelled yet)"),
    }
    println!("misses            {}", stats.misses);
    println!("wasted budget     {}", stats.wasted_budget);
    println!("chars injected    {}", stats.chars_injected);
    println!("chars referenced  {}", stats.chars_referenced);
    Ok(())
}

/// The envelope for this command, with the numbers already in it.
fn document(stats: &Stats) -> Document {
    Document::new(COMMAND)
        .field("decisions", stats.decisions)
        .field("injected_turns", stats.injected_turns)
        .field("hits", stats.hits)
        .field("false_positives", stats.false_positives)
        .field(
            "precision",
            match stats.precision {
                Some(precision) => serde_json::Value::from(precision),
                // Present and null rather than absent, the way the envelope
                // carries `reason`: a consumer reads one shape either way.
                None => serde_json::Value::Null,
            },
        )
        .field("misses", stats.misses)
        .field("wasted_budget", stats.wasted_budget)
        .field("chars_injected", stats.chars_injected)
        .field("chars_referenced", stats.chars_referenced)
}
