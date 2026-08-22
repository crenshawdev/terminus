//! The judgment half: one call per finalized session, and every claim it
//! makes anchored to a turn that is really there (OBS-02, OBS-03).
//!
//! # The anchor is the product
//!
//! `decisions`, `learned` and `unresolved` are the only things a model is asked
//! for here, and each entry carries a `turn_id`. That id is checked against the
//! `turns` table BEFORE the row is written, and it must name a turn OF THIS
//! SESSION. A response whose claims do not anchor is not stored with the bad
//! anchors dropped - it takes the same path as a response that would not parse
//! at all, because an unverifiable claim is worse than no claim. The whole
//! reason this table exists rather than a second Claude instance's prose is
//! that a reader can go and look at the turn.
//!
//! A model cannot anchor a claim to an id it was never shown, so the input is
//! the session's own turns rendered one per line BESIDE their real `turns.id`.
//! The ids are the archive's own integers, not positions in the prompt: an
//! offset the caller would have to translate is a second place the anchoring
//! could be wrong.
//!
//! # Fixed, and versioned
//!
//! [`SCHEMA`] and the instruction turn are one prompt, and [`PROMPT_VERSION`]
//! names it. The version is stored in the row because `verbatim observations
//! regenerate --prompt-version` selects on it: the reason to pay for a session
//! twice is that the prompt changed, and a row that cannot say which prompt
//! produced it cannot be re-asked selectively.
//!
//! # An answer that will not do is asked for once more, then kept (OBS-04)
//!
//! A response whose content is not the document the schema asked for, and a
//! response whose claims do not anchor, take the same path: asked again exactly
//! [`RETRIES`] times, then stored with [`STATUS_PARSE_FAILED`] and the raw text.
//! Never dropped - the session is still in the archive and regeneration is
//! always available, and a silently discarded failure is how the incumbent's
//! schema drift became permanent invisible loss. Never a failed pass either:
//! every way this can go is a [`Verdict`] the caller turns into a note.
//!
//! # The row is claimed before the request, not after it (D-07)
//!
//! Two ingest passes may overlap by design: D-07 puts this call outside the
//! ingest lock precisely so a request in flight does not stop the next
//! hook-spawned pass from archiving. Both passes therefore see the same
//! unjudged session, and a gate that only READS the status - which is all
//! [`cost::admits`] can do - lets both of them pay for the same answer, with
//! the second write silently overwriting the first.
//!
//! [`reserve`] closes that in one statement. It compares and swaps
//! `observations.status` from the value it was found at to
//! [`STATUS_JUDGING`] plus the instant it was taken, and SQLite serializes the
//! two writers, so exactly one run comes away with a row changed. The loser
//! makes no request at all and returns [`cost::Skip::InFlight`] - a skip, not a
//! failure: nothing went wrong, the session is being judged, and the money is
//! the thing that must not be spent twice.
//!
//! Every later write in this module is compare-and-swapped against that same
//! token, so a run that somehow lost its reservation cannot overwrite the row
//! that took it.
//!
//! # A reservation always ends, one of two ways
//!
//! In the ordinary case the run itself ends it: a stored judgment, a stored
//! `parse_failed`, or - for a transport failure, a non-2xx, a blob that would
//! not read - [`release`], which puts the status back to exactly what it was
//! before. A released row is unjudged again on the next pass, which is the
//! whole point: a provider that was down must not make a session permanently
//! unjudgeable.
//!
//! The other way is time. A process killed mid-request leaves a token nothing
//! will ever release, so a reservation older than
//! [`RESERVATION_LEASE_SECONDS`] is treated as abandoned and may be taken by
//! the next run - see that constant for why the number is what it is.
//!
//! # `mechanical` is not touched
//!
//! Every write here names the judgment columns and no other. PLAN-1 owns
//! `mechanical`, a judgment run must not disturb it, and the two halves share a
//! row only because they describe one session.

use std::collections::BTreeSet;

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::config::{Config, Secret};
use crate::error::Result;
use crate::index::text;
use crate::observe::cost::{self, Skip};
use crate::observe::provider::{self, Message};

/// Which prompt produced a row, stored in `observations.prompt_version`.
///
/// Bump it whenever [`SCHEMA`] or [`instructions`] changes in a way that would
/// make an old row's claims mean something different. It is the selector
/// `verbatim observations regenerate --prompt-version` narrows on, so a bump
/// that is not made is a re-run that cannot be targeted.
pub const PROMPT_VERSION: &str = "obs-judgment-1";

/// `observations.status` for a row whose claims were validated and stored.
pub const STATUS_OK: &str = "ok";

/// `observations.status` for a row whose two answers could not be used.
pub const STATUS_PARSE_FAILED: &str = "parse_failed";

/// `observations.status` while a run is asking about the row.
///
/// Stored as this word, a space, and the ISO-8601 instant the reservation was
/// taken - `judging 2026-08-22T14:03:11.284Z` - because a reservation nothing
/// released has to be distinguishable from one taken a second ago, and there is
/// no column to put the instant in that would not be a schema change.
///
/// It is not a judgment and never a terminal state: [`STATUS_OK`] and
/// [`STATUS_PARSE_FAILED`] are the only two answers, and everything that reads
/// this column asks [`is_reservation`] rather than testing for null.
pub const STATUS_JUDGING: &str = "judging";

/// The `LIKE` pattern matching exactly the reservation statuses.
///
/// [`STATUS_JUDGING`] and a space, which SQL cannot build out of the constant.
/// It exists so `observe::unjudged`'s candidate query can select abandoned
/// reservations alongside the never-asked rows; keep the two in step.
pub const RESERVED_LIKE: &str = "judging %";

/// How long a reservation stands before another run may take the session.
///
/// Fifteen minutes, against a bound of four: `net`'s exchange timeout is 120
/// seconds and [`RETRIES`] allows two of them, so a live run cannot still be
/// inside its request when the lease lapses without the machine having been
/// suspended. That headroom is the point - a lease that expired under a slow
/// provider would hand the session to a second run and buy the second answer
/// this reservation exists to prevent, which is the failure the lease is
/// supposed to be the cheap end of.
///
/// The cost of the other direction is bounded and small: a process killed
/// mid-request leaves its session unjudgeable for this long, and then the next
/// pass picks it up. Nothing sweeps and nothing schedules - the hook spawn is
/// the scheduler, exactly as it is for the daily budget's reset.
pub const RESERVATION_LEASE_SECONDS: i64 = 900;

/// How many times an unusable answer is asked for again.
///
/// Exactly one. Not zero, which loses the transient case the retry exists for -
/// a thinking model that wrapped its JSON in a fence once will usually not do
/// it twice. Not more, which doubles the bill on a model that will never comply
/// and turns OBS-02's bounded "one call per session" into an unbounded loop
/// against a paid endpoint.
pub const RETRIES: usize = 1;

/// The most entries one claim list may carry.
///
/// Asked for in the schema and enforced again on the way in, because a schema
/// is a request and not a guarantee. Five is a summary; fifty is the transcript
/// again, which the archive already holds losslessly.
pub const MAX_CLAIMS: usize = 5;

/// The only values `outcome` may take.
pub const OUTCOMES: [&str; 4] = ["completed", "partial", "abandoned", "exploratory"];

/// One claim, and the turn it is read off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// A real `turns.id` of the session being judged. Nothing else is stored.
    pub turn_id: i64,
    pub text: String,
}

impl Claim {
    fn to_json(&self) -> Value {
        json!({ "turn_id": self.turn_id, "text": self.text })
    }
}

/// One validated answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgment {
    pub topic: String,
    pub outcome: String,
    pub decisions: Vec<Claim>,
    pub learned: Vec<Claim>,
    pub unresolved: Vec<Claim>,
}

/// What one judgment attempt did.
///
/// Nothing here is an `Err`: OBS-04 says judgment never blocks ingest, and a
/// caller handed a `Result` would eventually be tempted to `?` it out of a
/// pass. Every way this can go wrong is a value the caller turns into a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A validated judgment replaced the row's judgment columns.
    Stored { tokens: u64 },
    /// No request was made, because a cost control said not to (OBS-06).
    ///
    /// Distinct from [`Verdict::Failed`] because nothing went wrong: a session
    /// under the minimum turn count, one already judged, or a day whose budget
    /// is spent are all the controls working.
    Skipped(Skip),
    /// Both answers were unusable; the row carries `parse_failed` and the raw
    /// text of the second one (OBS-04).
    ///
    /// Not a [`Verdict::Failed`]: something WAS stored, the provider was paid
    /// for it, and the session must not be asked a third time by a pass.
    ParseFailed { tokens: u64, reason: String },
    /// Nothing was stored. `reason` has already been through
    /// [`crate::observe::egress::scrub`] on its way out of the provider, so it
    /// is safe for `runs.error` and for stderr (D-16).
    Failed { reason: String },
}

/// Ask the configured provider to judge one archived session, and store the
/// answer.
///
/// `credential` is whatever [`crate::credentials::resolve`] produced - `None`
/// for a local endpoint that wants no key. The caller resolves it because
/// PRIV-02's refusal is a different failure from a provider failure and the two
/// must not be reported as one.
///
/// The row must already exist: [`crate::observe::observe_new`] writes the
/// mechanical half, and this fills the judgment columns of that row.
pub fn judge(
    conn: &Connection,
    config: &Config,
    credential: Option<&Secret>,
    session_key: &str,
) -> Verdict {
    run(conn, config, credential, session_key, false)
}

/// Ask again for a session that already carries a judgment status.
///
/// The one waiver of [`cost`]'s fourth gate, and it exists for exactly one
/// caller: `verbatim observations regenerate`, whose whole point is that the
/// prompt changed. Every other control still applies - a session under
/// [`cost::MIN_TURNS`] is still not bought, and a spent daily budget still
/// stops the run.
pub fn judge_again(
    conn: &Connection,
    config: &Config,
    credential: Option<&Secret>,
    session_key: &str,
) -> Verdict {
    run(conn, config, credential, session_key, true)
}

/// Both entry points, with the store's own failures folded into a verdict.
fn run(
    conn: &Connection,
    config: &Config,
    credential: Option<&Secret>,
    session_key: &str,
    again: bool,
) -> Verdict {
    match attempt(conn, config, credential, session_key, again) {
        Ok(verdict) => verdict,
        Err(reason) => Verdict::Failed { reason },
    }
}

/// The body of [`judge`], with the store's own failures as strings.
///
/// The reservation is taken here and ended here, so every path out of [`ask`]
/// leaves the row in a state some later run can act on: a terminal status the
/// row writes itself, or the status it was found at.
fn attempt(
    conn: &Connection,
    config: &Config,
    credential: Option<&Secret>,
    session_key: &str,
    again: bool,
) -> std::result::Result<Verdict, String> {
    let anchors = anchors(conn, session_key).map_err(|e| note(credential, e))?;
    // Every gate, before a URL is built or a credential is read: OBS-06's
    // controls are only worth anything if nothing was already sent by the time
    // they are consulted, and `net`'s attempt log is what a test reads that off.
    if let Some(skip) = cost::admits(conn, config, session_key, anchors.len(), again)
        .map_err(|e| note(credential, e))?
    {
        return Ok(Verdict::Skipped(skip));
    }

    // Last, and still before anything is sent: the gates above are reads that
    // two overlapping runs both pass, and this is the write only one of them
    // wins (D-07).
    let reservation = match reserve(conn, session_key, again).map_err(|e| note(credential, e))? {
        Reserved::Taken(reservation) => reservation,
        Reserved::Held(skip) => return Ok(Verdict::Skipped(skip)),
    };

    let verdict = ask(
        conn,
        config,
        credential,
        session_key,
        &anchors,
        &reservation,
    );

    // A verdict that wrote a terminal status IS the release; anything else puts
    // the row back the way it was found, so a provider that was down for one
    // pass does not make the session unjudgeable until the lease lapses.
    if !matches!(
        verdict,
        Ok(Verdict::Stored { .. }) | Ok(Verdict::ParseFailed { .. })
    ) {
        // A release that fails is not worth losing the verdict over: the lease
        // is the backstop for exactly the case where nothing was able to say
        // the reservation is over.
        let _ = release(conn, &reservation);
    }
    verdict
}

/// The request, the retry and the write - everything the reservation covers.
fn ask(
    conn: &Connection,
    config: &Config,
    credential: Option<&Secret>,
    session_key: &str,
    anchors: &BTreeSet<i64>,
    reservation: &Reservation,
) -> std::result::Result<Verdict, String> {
    let shown = transcript(conn, session_key).map_err(|e| note(credential, e))?;
    let messages = [Message::system(instructions()), Message::user(shown)];

    // What the last unusable answer was, and why. Kept rather than discarded:
    // the row that gets written carries it, because a silently dropped failure
    // is how the incumbent's schema drift became permanent invisible loss.
    let mut unusable: Option<(String, String)> = None;
    let mut tokens = 0u64;

    for _ in 0..=RETRIES {
        let completion = match provider::complete(config, credential, &messages) {
            Ok(completion) => completion,
            // Already scrubbed: `provider::Error`'s text is written by one
            // constructor and that constructor runs the scrubber.
            //
            // No retry and no `parse_failed` row: a dead socket or a 429 is not
            // an answer this build could not read, it is an answer that never
            // arrived, and writing a status for it would stop the next pass from
            // ever trying again.
            Err(e) => {
                return Ok(Verdict::Failed {
                    reason: e.to_string(),
                })
            }
        };
        // Charged for whatever the answer turns out to be, and charged per
        // request rather than once at the end: the provider billed the moment it
        // answered, and both requests of a retried session count (D-12).
        let charge = completion.usage.map(|u| u.total_tokens).unwrap_or(0);
        tokens = tokens.saturating_add(charge);
        cost::spend(conn, charge).map_err(|e| note(credential, e))?;

        match read(&completion.content, anchors) {
            Ok(judgment) => {
                let written = store(conn, session_key, config, &judgment, tokens, reservation)
                    .map_err(|e| note(credential, e))?;
                if !written {
                    return Ok(Verdict::Failed { reason: lapsed() });
                }
                return Ok(Verdict::Stored { tokens });
            }
            Err(reason) => unusable = Some((completion.content, reason)),
        }
    }

    // Stored, never dropped (OBS-04). The session is still in the archive and
    // `verbatim observations regenerate` is always available, so what this row
    // buys is that a reader can see the model was asked and see what came back.
    let (raw, reason) = unusable.expect("the loop runs at least once");
    let raw = crate::observe::egress::scrub(credential, &raw);
    let reason = crate::observe::egress::scrub(credential, &reason);
    let written = store_failure(conn, session_key, config, &raw, tokens, reservation)
        .map_err(|e| note(credential, e))?;
    if !written {
        return Ok(Verdict::Failed { reason: lapsed() });
    }
    Ok(Verdict::ParseFailed { tokens, reason })
}

/// What a run says when the row it reserved is no longer the row it holds.
///
/// Only reachable when this run's reservation lapsed mid-request and another
/// took the session, which needs [`RESERVATION_LEASE_SECONDS`] to pass inside
/// one call. The answer is dropped rather than written over the run that owns
/// the row now: an overwrite here is the exact loss the reservation exists to
/// stop, and it would be the paid one.
fn lapsed() -> String {
    format!(
        "the answer arrived after this run's reservation had lapsed ({RESERVATION_LEASE_SECONDS}s) \
         and another run holds the row, so nothing was stored"
    )
}

/// One held reservation: what was written, and what it replaced.
struct Reservation {
    /// The row it is over. Carried because a token is an instant and two runs
    /// reserving two different sessions in the same millisecond write the same
    /// one - so no statement here may match on the token alone.
    session_key: String,
    /// The exact `observations.status` this run wrote. Every later write is
    /// conditioned on the column still holding it.
    token: String,
    /// The status the row carried before, restored by [`release`]. `None` for
    /// the ordinary never-judged row; a terminal status when
    /// `observations regenerate` is asking again, whose stored claims stay
    /// standing until this run has an answer to replace them with.
    previous: Option<String>,
}

/// The two ways [`reserve`] can end.
enum Reserved {
    Taken(Reservation),
    /// The row is not this run's to ask about, and it asks nothing. The [`Skip`]
    /// says which of the two reasons it was.
    Held(Skip),
}

/// Is this stored status a reservation rather than an answer?
pub fn is_reservation(status: &str) -> bool {
    status
        .strip_prefix(STATUS_JUDGING)
        .is_some_and(|rest| rest.starts_with(' '))
}

/// Claim one row for this run, or say when the run that holds it took it.
///
/// **One statement decides it.** The `UPDATE` names the exact status the row
/// was read at, so of two runs that both read `NULL` the second changes no rows
/// and asks nothing - SQLite serializes the writers and the loser can tell,
/// because `execute` answers with the number of rows it changed. A read
/// followed by an unconditional write would put both runs through the provider
/// and let the slower one overwrite the answer the faster one paid for.
///
/// **`again` waives the terminal status and nothing else.**
/// `verbatim observations regenerate` may ask about a row carrying `ok` or
/// `parse_failed`; it may not ask about one another run is asking about right
/// now, because two answers bought for one session is what this is about
/// whichever caller asks.
///
/// **An abandoned reservation is takeable by anyone.** A token older than
/// [`RESERVATION_LEASE_SECONDS`] belongs to a process that is gone, and leaving
/// it standing would make the session permanently unjudgeable. Taking it costs
/// one call - and where the dead run was a regenerate over an already-judged
/// row, that call is a second answer for a session that had one. That is the
/// price of the row being repaired at all, it is bounded by
/// `cost::JUDGED_PER_PASS`, and it is only ever paid after something killed a
/// run mid-request.
fn reserve(conn: &Connection, session_key: &str, again: bool) -> Result<Reserved> {
    let previous = cost::status(conn, session_key)?;

    if let Some(status) = previous.as_deref() {
        if is_reservation(status) {
            if status >= lapsed_before(conn)?.as_str() {
                return Ok(Reserved::Held(Skip::InFlight {
                    since: taken_at(status),
                }));
            }
        } else if !again {
            // `cost::admits` refuses this before the caller ever reaches here.
            // Repeated because this function is what the column's invariant
            // actually rests on, and it must not depend on being called second.
            return Ok(Reserved::Held(Skip::AlreadyJudged {
                status: status.to_owned(),
            }));
        }
    }

    let token = format!("{STATUS_JUDGING} {}", now(conn)?);
    let changed = conn.execute(
        "UPDATE observations SET status = ?2 WHERE session_key = ?1 AND status IS ?3",
        rusqlite::params![session_key, token, previous],
    )?;
    if changed == 0 {
        // The column moved between the read above and this write, which is the
        // race itself - or there is no row to reserve, which the caller is what
        // guarantees. Either way this run is not holding the row, so it asks
        // nothing and says what the column says rather than guessing who moved
        // it.
        return Ok(Reserved::Held(match cost::status(conn, session_key)? {
            Some(status) if is_reservation(&status) => Skip::InFlight {
                since: taken_at(&status),
            },
            Some(status) => Skip::AlreadyJudged { status },
            None => Skip::InFlight {
                since: "moments ago".to_owned(),
            },
        }));
    }
    Ok(Reserved::Taken(Reservation {
        session_key: session_key.to_owned(),
        token,
        previous,
    }))
}

/// The instant half of a reservation token, for a message a person reads.
fn taken_at(status: &str) -> String {
    status
        .split_once(' ')
        .map(|(_, stamp)| stamp.to_owned())
        .unwrap_or_else(|| status.to_owned())
}

/// Put the status back to what it was, if this run still holds the row.
///
/// Conditioned on the token like every other write here: a run whose
/// reservation lapsed must not clear the status of the run that took the
/// session from it.
fn release(conn: &Connection, reservation: &Reservation) -> Result<()> {
    conn.execute(
        "UPDATE observations SET status = ?3 WHERE session_key = ?1 AND status = ?2",
        rusqlite::params![
            reservation.session_key,
            reservation.token,
            reservation.previous
        ],
    )?;
    Ok(())
}

/// The instant the store calls now, in the shape every timestamp in it carries.
fn now(conn: &Connection) -> Result<String> {
    Ok(
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |r| {
            r.get(0)
        })?,
    )
}

/// The reservation token a stored one must be BELOW to count as abandoned.
///
/// A whole token rather than a bare timestamp, so a caller compares it against
/// the column directly: every reservation carries the same fixed-width prefix
/// and stamp, so ordering the strings orders the instants.
pub fn lapsed_before(conn: &Connection) -> Result<String> {
    let stamp: String = conn.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
        [format!("-{RESERVATION_LEASE_SECONDS} seconds")],
        |r| r.get(0),
    )?;
    Ok(format!("{STATUS_JUDGING} {stamp}"))
}

/// A store failure as a line safe to print, scrubbed like every other (D-16).
///
/// A SQLite message quotes the statement back, and the statement this module
/// runs carries the model's text; scrubbing it costs nothing and the one place
/// it is skipped is the one that leaks.
fn note(credential: Option<&Secret>, e: crate::error::Error) -> String {
    crate::observe::egress::scrub(credential, &e.to_string())
}

/// Every `turns.id` of this session - the whole of what a claim may anchor to.
fn anchors(conn: &Connection, session_key: &str) -> Result<BTreeSet<i64>> {
    let mut statement = conn.prepare("SELECT id FROM turns WHERE session_key = ?1")?;
    let rows = statement.query_map([session_key], |r| r.get::<_, i64>(0))?;
    let mut out = BTreeSet::new();
    for row in rows {
        out.insert(row?);
    }
    Ok(out)
}

/// The session as the model is shown it: one line per turn, each carrying the
/// turn's real id.
///
/// One blob read, the way `mechanical::from_blob` does it - the stream is
/// decompressed once and every turn is sliced out of the one copy at the
/// `stream_offset` and `byte_len` already on its row (D-04 addresses the
/// uncompressed stream, never the blob).
///
/// The text is [`crate::index::text::project`]'s, which is the same rule ingest
/// indexed the turn with and the same rule an excerpt is cut through. A second
/// projection written here would be a second definition of what a turn said.
fn transcript(conn: &Connection, session_key: &str) -> Result<String> {
    let coordinates: Vec<(i64, String, i64, i64)> = {
        let mut statement = conn.prepare(
            "SELECT id, record_type, stream_offset, byte_len
               FROM turns WHERE session_key = ?1 ORDER BY turn_seq",
        )?;
        let rows = statement.query_map([session_key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    let bytes: Vec<u8> = conn.query_row(
        "SELECT blob FROM sessions WHERE session_key = ?1",
        [session_key],
        |r| r.get(0),
    )?;
    let stream = crate::blob::read_all(&bytes)?;

    let mut lines: Vec<String> = Vec::new();
    for (turn_id, record_type, offset, len) in coordinates {
        let (from, to) = (offset as usize, offset as usize + len as usize);
        // A row whose range is outside the stream is archive damage `verbatim
        // verify` reports; here it is one line the model does not see.
        if to > stream.len() || from > to {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&stream[from..to]) else {
            continue;
        };
        let said = one_line(&text::project(&value));
        if said.is_empty() {
            continue;
        }
        lines.push(format!("turn_id={turn_id} {record_type}: {said}"));
    }
    // Cut to the budget with markers naming what went (OBS-06): a session shown
    // short and silently would be summarized as if it were whole.
    Ok(cost::truncate(&lines))
}

/// Runs of whitespace as single spaces, so one turn is one line.
///
/// The projection is newline-joined by construction - one line per text block,
/// per tool-input leaf, per stderr line - and a turn spread over forty lines
/// would make `turn_id=` stop marking where a turn begins.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending = false;
    for c in text.chars() {
        if c.is_whitespace() {
            pending = !out.is_empty();
            continue;
        }
        if pending {
            out.push(' ');
            pending = false;
        }
        out.push(c);
    }
    out
}

/// The response shape, fixed and versioned with [`PROMPT_VERSION`].
///
/// Sent in the instruction turn rather than in `response_format`: the request
/// builder is `observe::provider`'s and it sends D-09's
/// `{"type":"json_schema","strict":true}` for every caller. Stating the schema
/// in the prompt is what tells this particular model what those two words are
/// about, and it is the only channel this module has to say it.
pub fn schema() -> Value {
    json!({
        "name": "session_observation",
        "schema": {
            "type": "object",
            "additionalProperties": false,
            "required": ["topic", "outcome", "decisions", "learned", "unresolved"],
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "One line: what this session was about.",
                },
                "outcome": { "type": "string", "enum": OUTCOMES },
                "decisions": claim_list("Choices that were made and acted on."),
                "learned": claim_list("Facts about this codebase discovered here."),
                "unresolved": claim_list("What was left open when the session ended."),
            },
        },
    })
}

/// One of the three claim arrays, in the schema.
fn claim_list(description: &str) -> Value {
    json!({
        "type": "array",
        "maxItems": MAX_CLAIMS,
        "description": description,
        "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["turn_id", "text"],
            "properties": {
                "turn_id": {
                    "type": "integer",
                    "description": "The turn_id printed beside the turn this claim is read off.",
                },
                "text": { "type": "string" },
            },
        },
    })
}

/// The instruction turn, [`schema`] included.
fn instructions() -> String {
    format!(
        "You are summarising one archived coding session for an audit log.\n\
         Answer with exactly one JSON object matching this schema, and nothing \
         else - no prose before it, no markdown fence around it:\n\
         {}\n\
         Every line of the session below begins with `turn_id=<id>`. Every entry \
         of `decisions`, `learned` and `unresolved` must carry the turn_id of the \
         line the claim is read off, copied exactly. A claim you cannot point at a \
         line for is left out; an empty array is a correct answer. Never invent a \
         turn_id and never use one that is not printed below.",
        schema()
    )
}

/// Parse one response and validate every anchor in it (OBS-03).
///
/// `Err` carries why, and the caller stores nothing under a success status.
/// Every rejection here is one of two things: an answer that is not the
/// document that was asked for, or a claim pointing at a turn that is not in
/// this session. The second is the one that matters - a plausible summary
/// anchored to somebody else's turn is worse than silence, because it reads as
/// evidence.
fn read(content: &str, anchors: &BTreeSet<i64>) -> std::result::Result<Judgment, String> {
    let value: Value =
        serde_json::from_str(content).map_err(|e| format!("the answer is not JSON: {e}"))?;

    let topic = value
        .get("topic")
        .and_then(Value::as_str)
        .ok_or("the answer carries no `topic` string")?
        .trim()
        .to_owned();
    let outcome = value
        .get("outcome")
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or("the answer carries no `outcome` string")?;
    if !OUTCOMES.contains(&outcome) {
        return Err(format!(
            "`outcome` is {outcome:?}, which is none of {OUTCOMES:?}"
        ));
    }

    Ok(Judgment {
        topic,
        outcome: outcome.to_owned(),
        decisions: claims(&value, "decisions", anchors)?,
        learned: claims(&value, "learned", anchors)?,
        unresolved: claims(&value, "unresolved", anchors)?,
    })
}

/// One claim list, every entry of it anchored.
fn claims(
    value: &Value,
    field: &str,
    anchors: &BTreeSet<i64>,
) -> std::result::Result<Vec<Claim>, String> {
    let entries = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("`{field}` is missing or is not an array"))?;

    let mut out = Vec::new();
    // The cap is applied and is not a rejection: a sixth entry is a model being
    // generous, not a claim that cannot be verified.
    for entry in entries.iter().take(MAX_CLAIMS) {
        let turn_id = entry
            .get("turn_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("an entry of `{field}` carries no integer `turn_id`"))?;
        if !anchors.contains(&turn_id) {
            return Err(format!(
                "an entry of `{field}` anchors to turn {turn_id}, which is not a turn of this \
                 session"
            ));
        }
        let text = entry
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("an entry of `{field}` carries no `text` string"))?
            .trim()
            .to_owned();
        out.push(Claim { turn_id, text });
    }
    Ok(out)
}

/// Write the judgment columns of one row, and only those.
///
/// `raw` is cleared: a row that failed to parse once and answered on a later
/// run must not keep the failed text beside a good answer. `mechanical`,
/// `generated_at`, `session_id` and `session_key` are untouched - PLAN-1 owns
/// them and this half must not disturb the other.
///
/// Conditioned on the reservation this run holds, which is what makes the write
/// safe with two passes running: an `UPDATE ... WHERE session_key` alone would
/// let a run that reserved nothing overwrite the answer of the run that did.
/// `false` means the row is no longer this run's and nothing was written.
fn store(
    conn: &Connection,
    session_key: &str,
    config: &Config,
    judgment: &Judgment,
    tokens: u64,
    reservation: &Reservation,
) -> Result<bool> {
    let list =
        |claims: &[Claim]| Value::Array(claims.iter().map(Claim::to_json).collect()).to_string();
    let changed = conn.execute(
        "UPDATE observations
            SET status = ?2, model = ?3, prompt_version = ?4, topic = ?5, outcome = ?6,
                decisions = ?7, learned = ?8, unresolved = ?9, raw = NULL, tokens = ?10
          WHERE session_key = ?1 AND status = ?11",
        rusqlite::params![
            session_key,
            STATUS_OK,
            config.provider_model(),
            PROMPT_VERSION,
            judgment.topic,
            judgment.outcome,
            list(&judgment.decisions),
            list(&judgment.learned),
            list(&judgment.unresolved),
            tokens as i64,
            reservation.token,
        ],
    )?;
    Ok(changed > 0)
}

/// Write the failure columns of one row, and only those (OBS-04).
///
/// `raw` has already been through [`crate::observe::egress::scrub`]: it is a
/// provider response this build did not author, `verbatim observations` prints
/// the column, and `runs.error` keeps whatever travels with it (D-16).
///
/// The claim columns are CLEARED rather than left standing. A row whose status
/// says `parse_failed` while it still carries `topic`, `outcome` and three
/// claim lists is a row that lies about where its contents came from - the
/// claims would be an older prompt version's, presented under this run's
/// status. `mechanical` is untouched, like everywhere else in this module.
fn store_failure(
    conn: &Connection,
    session_key: &str,
    config: &Config,
    raw: &str,
    tokens: u64,
    reservation: &Reservation,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE observations
            SET status = ?2, model = ?3, prompt_version = ?4, topic = NULL, outcome = NULL,
                decisions = NULL, learned = NULL, unresolved = NULL, raw = ?5, tokens = ?6
          WHERE session_key = ?1 AND status = ?7",
        rusqlite::params![
            session_key,
            STATUS_PARSE_FAILED,
            config.provider_model(),
            PROMPT_VERSION,
            raw,
            tokens as i64,
            reservation.token,
        ],
    )?;
    Ok(changed > 0)
}
