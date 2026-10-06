//! `terminus retention --dry-run`: what the next ingest pass would do (RET-03).
//!
//! **It applies nothing, ever.** Retention runs as a bounded step at the end of
//! every pass (D-10) and that is the only place it acts; this command reports
//! and returns. A second application path here would race the pass's own, which
//! is precisely what one shared evaluation exists to prevent - so `--dry-run` is
//! the documented spelling of the only behaviour this command has rather than a
//! mode it can be talked out of.
//!
//! **One evaluation, shared with the pass (D-04).**
//! [`terminus_core::retention::evaluate`] is the function both callers use.
//! Two independently written evaluations of a `'now'`-relative predicate can
//! legitimately disagree by one session and there would be no way to tell that
//! from a bug.
//!
//! **The instant is in the document because the rule is shared and the clock is
//! not.** The evaluation instant is read once, here, and every age in the
//! report is measured from it; a pass running a minute later reads its own, and
//! a session sitting on the age boundary can legitimately fall on the other
//! side of it. Reporting the instant is what makes that difference explainable
//! rather than invisible - so the report says what it judged against instead of
//! implying the next pass will judge against the same thing.
//!
//! It opens through [`super::read`] the way `stats` and `observations` do: a
//! read must not create a store as the side effect of a question, and a
//! read-only connection is all an evaluation that writes nothing needs.

use terminus_core::retention::{self, Selection};

use super::json::Document;
use super::read::{self, Opened};
use super::Failure;

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "retention";

/// What a store whose `terminus.toml` has no `[retention]` table is told.
///
/// Off is the default and the whole product's posture (RET-01), so this is the
/// ordinary answer rather than a complaint. It names the file, because the file
/// is the only thing that can change it: nothing in this binary writes it.
const OFF: &str = "retention is off: no `[retention]` table in terminus.toml selects anything, \
                   so no ingest pass will evict or delete a session";

/// What a configured policy that has nothing to act on yet is told.
const NOTHING_YET: &str = "the configured retention policy names no session yet: nothing closed \
                           is past its age, and a delete waits for Claude Code's own cleanup to \
                           remove the transcript first";

pub struct Args {
    pub json: bool,
}

pub fn run(args: Args) -> Result<(), Failure> {
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        Opened::Nothing(reason) => {
            return read::empty(document(&Selection::default()), &reason, args.json)
        }
    };
    let conn = reader.store().conn();
    let config = reader.config();

    // Read ONCE, and before the evaluation rather than inside it. `'now'`
    // re-evaluates on every SQL call, so a per-row clock would measure the
    // sessions at the top of the list against a different instant than the ones
    // at the bottom - and the instant reported would be neither.
    let now = retention::evaluated_now(conn).map_err(op)?;
    let selection = retention::evaluate(conn, config, &now).map_err(op)?;

    // Off is not the same answer as "configured and nothing is due", and a user
    // who wrote a policy an hour ago deserves to know which one they are
    // looking at.
    if config.retention_selects_nothing() {
        return read::empty(document(&selection), OFF, args.json);
    }
    if selection.is_empty() {
        return read::empty(document(&selection), NOTHING_YET, args.json);
    }

    if args.json {
        document(&selection).emit();
        return Ok(());
    }

    // One parseable line per session, the shape `sessions` prints in, with the
    // instant first because every line below it is relative to that one.
    println!("cutoff  {}", selection.evaluated_at);
    for key in &selection.evict {
        println!("evict   {key}");
    }
    for key in &selection.delete {
        println!("delete  {key}");
    }

    // Commentary on stderr, so stdout stays the list. `status` reports its own
    // counts the same way.
    eprintln!(
        "{} session(s) would be evicted, {} deleted, by the next ingest pass",
        selection.evict.len(),
        selection.delete.len()
    );
    if selection.over > 0 {
        eprintln!(
            "{} more are waiting: one pass acts on at most {} per action",
            selection.over,
            retention::MAX_PER_PASS
        );
    }
    if selection.excluded > 0 {
        eprintln!(
            "{} session(s) in excluded projects were left alone",
            selection.excluded
        );
    }
    Ok(())
}

/// The envelope for this command, with the selection already in it.
///
/// `cutoff` is the evaluation INSTANT and not `now` less some number of days,
/// because there is no single such number: the age is per project (D-01), so
/// what one report can honestly state is the one clock reading every one of
/// those ages was measured from.
fn document(selection: &Selection) -> Document {
    Document::new(COMMAND)
        .field(
            "cutoff",
            // Present and null rather than absent on the no-store path, the way
            // the envelope carries `reason`: a consumer reads one shape either
            // way. Every other path has read a clock.
            match selection.evaluated_at.is_empty() {
                true => serde_json::Value::Null,
                false => serde_json::Value::from(selection.evaluated_at.as_str()),
            },
        )
        .field("evict", selection.evict.clone())
        .field("delete", selection.delete.clone())
        .field("over", selection.over as i64)
        .field("excluded", selection.excluded as i64)
}

/// A store error is operational (exit 1), never misuse.
fn op(error: terminus_core::Error) -> Failure {
    Failure::Operational(error.to_string())
}

/// Two flags, so the loop rather than `super::json_flag` - the shape `search`,
/// `show` and `sessions` use.
///
/// `--dry-run` is accepted and changes nothing, because there is nothing for it
/// to change: this command has exactly one behaviour and the flag is the
/// documented spelling of it (RET-03, D-04). Accepting it silently is what lets
/// the documented command line keep working if a second verb is ever added
/// here; rejecting every OTHER flag is the rule that stays.
pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut json = false;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long("dry-run") => {}
            Long(super::JSON_FLAG) => json = true,
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }
    Ok(Args { json })
}
