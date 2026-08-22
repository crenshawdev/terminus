//! What a judgment run is allowed to cost (OBS-06).
//!
//! # Four gates, and only one of them is a setting (D-11)
//!
//! OBS-06 splits across two homes on purpose. The daily token budget is the
//! knob facing MONEY, so it is a config key
//! ([`crate::config::Config::provider_daily_token_budget`]) and a user can cap
//! what a remote provider charges them without rebuilding. [`MIN_TURNS`] and
//! [`TRUNCATION_BUDGET`] face QUALITY, so they are compile-time constants here
//! with their reasoning in their doc comments, following
//! `feedback::finalize::IDLE_HOURS`: a tunable minimum would turn OBS-02's "one
//! call per finalized session" into a setting, and a tunable truncation budget
//! would let a user quietly buy a prompt their model cannot read.
//!
//! The fourth gate is not a number at all: a session whose row already carries
//! a judgment status - `ok` or `parse_failed` alike - is never asked again by a
//! pass. `verbatim observations regenerate` is the one caller that may, and it
//! says so by passing `again`.
//!
//! # The spend is in the store, because there is no process to hold it (D-12)
//!
//! Every generation is a separate short-lived process - there is no daemon, the
//! hook spawn is the scheduler - so an in-process counter would start at zero
//! on every invocation and bound nothing at all. The day's spend is therefore
//! one `meta` row, stamped with the date it belongs to, and a row carrying any
//! other date reads as zero. That is the whole of the reset: nothing sweeps,
//! nothing schedules, and a machine that was off for a week comes back with a
//! fresh budget rather than a stale one.
//!
//! One row and not two, holding `"<date> <tokens>"`, so the date and the count
//! are written by one statement and cannot end up disagreeing with each other.
//!
//! # Truncation says what it took out
//!
//! [`truncate`] never simply stops. A session cut down to the budget with no
//! marker is a session the model reads as complete, and a summary of a
//! transcript's first half presented as a summary of the session is exactly the
//! unverifiable claim [`super::judgment`] exists to prevent. Every cut leaves
//! an [`ELIDED`] marker naming what went and saying the archive still holds it.

use std::fmt;

use rusqlite::{Connection, OptionalExtension};

use crate::config::Config;
use crate::error::Result;

/// How many turns a session needs before judgment is worth paying for.
///
/// A judgment call, set from the same posture as `feedback::finalize::
/// IDLE_HOURS`: the number that costs least when it is wrong. Too low buys a
/// paid summary of a session whose mechanical half - files, tools, commands,
/// errors, branch, duration - already says everything there was to say. Too
/// high leaves a real session unjudged permanently, because nothing re-checks a
/// session a pass has already walked past.
///
/// Six is a prompt, an answer, a correction and its answer: the shortest
/// exchange that can hold a decision AND the consequence that makes the
/// decision worth recording.
pub const MIN_TURNS: usize = 6;

/// How much of a session the model is shown, in characters.
///
/// Characters and not tokens, for the reason `config::DEFAULT_BRIEF_CHARS`
/// gives: the workspace has no tokenizer among its dependencies and every
/// existing budget in it is byte-shaped. At the four-characters-a-token rule
/// those budgets already use, this is roughly 15k tokens - which leaves the
/// instructions and the answer room inside a 32k context, and is far more than
/// the fifteen capped claims coming back can possibly need. A larger budget
/// buys a longer prompt for the same answer and starts overflowing the smaller
/// local models D-05's one code path is meant to serve.
pub const TRUNCATION_BUDGET: usize = 60_000;

/// How many sessions one ingest pass may buy a judgment for.
///
/// One. A pass is a DETACHED background process, a provider may take the whole
/// of `net`'s network timeout to answer, and a pass that judged a hundred
/// sessions would sit on the machine for hours - which is the long-lived
/// process `.planning/PROJECT.md` rules out as a design invariant, arrived at
/// sideways.
///
/// The backlog is not lost: the hook spawn is the scheduler, so "the next pass"
/// is the next prompt, and `verbatim observations regenerate` is the explicit
/// bulk path for a history somebody wants judged now. One call per pass is also
/// what keeps a first run against a remote provider from being a surprise bill
/// before [`crate::config::Config::provider_daily_token_budget`] is ever set.
pub const JUDGED_PER_PASS: usize = 1;

/// The most of the budget any one turn may take.
///
/// A single tool result can be megabytes, and without this the whole budget is
/// spent on one turn and [`truncate`] elides the entire rest of the session -
/// which is the shape that looks like truncation working and is not.
const MAX_TURN_CHARS: usize = TRUNCATION_BUDGET / 10;

/// The fragment every elision marker carries.
///
/// One spelling, so a reader scanning a stored prompt and a test asserting the
/// model was told what it was not seeing look for the same thing.
pub const ELIDED: &str = "elided; the archive holds";

/// The `meta` key holding `"<date> <tokens>"` for the day's provider spend.
const BUDGET_KEY: &str = "observe.daily_tokens";

/// Why a session was not sent to the provider.
///
/// Not an error: nothing here went wrong. A skip is the cost controls doing
/// their job, and the caller reports it as a note rather than a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// Fewer than [`MIN_TURNS`] indexed turns.
    TooShort { turns: usize },
    /// The row already carries a judgment status, and this caller is a pass.
    AlreadyJudged { status: String },
    /// Today's spend has reached the configured budget.
    BudgetSpent { spent: u64, budget: u64 },
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Skip::TooShort { turns } => write!(
                f,
                "not judged: {turns} turn(s) is fewer than the {MIN_TURNS} a judgment is bought for"
            ),
            Skip::AlreadyJudged { status } => write!(
                f,
                "not judged: this session already has a judgment ({status}); \
                 `verbatim observations regenerate` is what asks again"
            ),
            Skip::BudgetSpent { spent, budget } => write!(
                f,
                "not judged: {spent} of today's {budget}-token provider budget is spent"
            ),
        }
    }
}

/// May this session be sent to the provider?
///
/// `Some(skip)` is a refusal and `None` is permission. `again` waives the
/// already-judged gate and nothing else: `verbatim observations regenerate` is
/// the one path allowed to buy a second answer for a session, and it still pays
/// the minimum-turn and daily-budget gates like every other caller.
///
/// The budget is read here, BEFORE the call, and moved by [`spend`] after it.
/// Two invocations racing could each pass this gate and overshoot by one call;
/// that is accepted, because the alternative is holding a transaction across an
/// HTTP request and D-07 puts this whole step outside the ingest lock precisely
/// so a provider cannot hold anything.
pub fn admits(
    conn: &Connection,
    config: &Config,
    session_key: &str,
    turns: usize,
    again: bool,
) -> Result<Option<Skip>> {
    if !again {
        if let Some(status) = status(conn, session_key)? {
            return Ok(Some(Skip::AlreadyJudged { status }));
        }
    }
    if turns < MIN_TURNS {
        return Ok(Some(Skip::TooShort { turns }));
    }
    if let Some(budget) = config.provider_daily_token_budget() {
        let spent = spent_today(conn)?;
        if spent >= budget {
            return Ok(Some(Skip::BudgetSpent { spent, budget }));
        }
    }
    Ok(None)
}

/// The judgment status one row carries, if it carries one.
fn status(conn: &Connection, session_key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT status FROM observations WHERE session_key = ?1",
            [session_key],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// How many tokens today's provider calls have cost.
///
/// Zero for a row stamped with any date but today's: that IS the daily reset,
/// and it needs no sweep because nothing ever reads yesterday's number.
pub fn spent_today(conn: &Connection) -> Result<u64> {
    let today = today(conn)?;
    let stored: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [BUDGET_KEY], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(stored) = stored else {
        return Ok(0);
    };
    let Some((date, tokens)) = stored.split_once(' ') else {
        // A value this build did not write. Zero rather than a failure: a
        // budget that refuses to answer would stop judgment entirely over a
        // counter.
        return Ok(0);
    };
    if date != today {
        return Ok(0);
    }
    Ok(tokens.trim().parse().unwrap_or(0))
}

/// Add one call's tokens to today's spend, answering with the new total.
///
/// Called for every request that was made, including the one whose answer would
/// not parse: the provider charged for both, so both count (OBS-04).
pub fn spend(conn: &Connection, tokens: u64) -> Result<u64> {
    let total = spent_today(conn)?.saturating_add(tokens);
    let today = today(conn)?;
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![BUDGET_KEY, format!("{today} {total}")],
    )?;
    Ok(total)
}

/// Today's date, from the store rather than from a clock crate.
///
/// The same `strftime` shape that stamped every timestamp in this store, so the
/// budget's day and the archive's day cannot drift apart.
fn today(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT strftime('%Y-%m-%d', 'now')", [], |r| r.get(0))?)
}

/// The session's lines, cut to [`TRUNCATION_BUDGET`], saying what it cut.
///
/// Two cuts, in order. A turn longer than [`MAX_TURN_CHARS`] is clipped first,
/// so one giant tool result cannot eat the budget the rest of the session needs.
/// Then, if the whole is still over, the MIDDLE goes: a session's opening says
/// what it was for and its ending says how it came out, and those are the two
/// ends a summary is read off.
pub fn truncate(lines: &[String]) -> String {
    let clipped: Vec<String> = lines
        .iter()
        .map(|line| clip(line, MAX_TURN_CHARS))
        .collect();
    if width(&clipped) <= TRUNCATION_BUDGET {
        return clipped.join("\n");
    }

    // Half the budget from each end, so a long session comes back with both its
    // opening and its close rather than as much of the front as fits.
    // The `+ 1` added to `used` is the newline the join will put back; the
    // tests are spelled without it because `a + 1 <= b` and `a < b` are the
    // same thing over integers and clippy says so.
    let half = TRUNCATION_BUDGET / 2;
    let mut head = 0;
    let mut used = 0;
    while head < clipped.len() && used + clipped[head].len() < half {
        used += clipped[head].len() + 1;
        head += 1;
    }
    let mut tail = clipped.len();
    let mut used = 0;
    while tail > head && used + clipped[tail - 1].len() < half {
        used += clipped[tail - 1].len() + 1;
        tail -= 1;
    }

    let elided = tail - head;
    if elided == 0 {
        return clipped.join("\n");
    }
    let mut out: Vec<String> = clipped[..head].to_vec();
    out.push(format!(
        "[... {elided} turn(s) {ELIDED} every one of them ...]"
    ));
    out.extend_from_slice(&clipped[tail..]);
    out.join("\n")
}

/// One line, cut to `max` characters with a marker naming what went.
fn clip(line: &str, max: usize) -> String {
    if line.len() <= max {
        return line.to_owned();
    }
    // Back off to a character boundary: the cut is a byte index and a turn's
    // text is arbitrary UTF-8, so slicing at `max` would panic on the day a
    // multi-byte character straddles it.
    let mut end = max;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    let gone = line.len() - end;
    format!(
        "{} [... {gone} byte(s) of this turn {ELIDED} it in full ...]",
        &line[..end]
    )
}

/// The joined length of a line list, newlines included.
fn width(lines: &[String]) -> usize {
    lines.iter().map(|line| line.len() + 1).sum::<usize>()
}
