//! The `SessionStart` resume brief.
//!
//! What a session opens knowing: where the last session in this project left
//! off - its date, the branch it ended on, the last thing said in either
//! direction - and how much of the project is archived and reachable. Every
//! value comes from a stored row, so two runs against an unchanged store render
//! the same bytes.
//!
//! **The quoted prompt is a turn the person typed.** "The last thing
//! said" in the user's direction is not the last `user` record: the harness
//! writes tool results, `isMeta` command caveats and `<task-notification>`
//! envelopes as `user` records as well, and 12.3% of measured sessions end on
//! one. The discriminator is `turns.is_typed`, written at ingest by
//! `parse::record` and rebuilt from the archive alone, and the brief spends exactly
//! one `AND` on it - see [`last_turn`].
//!
//! The counts are read one way only: the project-only projection
//! `scope::resolve` has already run, plus one targeted `count(*)` per number,
//! and never `config::visible::sessions`. Measured on a store shaped like the
//! real one - 2,000 sessions, 250,000 turns, warm - `sessions()` costs 6.2-6.8
//! ms against 0.56-0.58 ms for the project-only join, because of its
//! per-session `count(*)` over `turns` and its watermark lookup. The whole
//! budget here is single digit milliseconds, so a brief built on it would spend
//! the budget before reading a useful row.
//!
//! **The working-state delta is the branch and nothing else.**
//! `session_meta.branch`, the branch the session's last record that named one
//! was on, is the only git fact the archive holds. A fuller delta
//! would add HEAD-then-versus-now and a dirty-file list, and both need `git`,
//! whose 10-30 ms process spawn is the whole budget several times over -
//! `project::Resolver` is already kept off the cold-start path for the same
//! reason. So no `std::process::Command` appears anywhere in this module; the
//! model can check live git itself in one cheap tool call, and this brief tells
//! it what the archive knew.
//!
//! **Nothing here reads a clock.** No `SystemTime`, no `Instant`, no
//! elapsed-time or "as of" phrasing, and no count that depends on when the read
//! happened: every value is a stored one, dates are rendered at day resolution,
//! and anything ordered is ordered by a stored key with a TOTAL order - the
//! reason `recall::search`'s `TAIL` is total as well. A tie broken differently
//! between two runs is a brief that changed while the archive did not, which is
//! the same defect as a timestamp in it. The stated rationale is the Anthropic
//! prefix cache, which nothing local can observe; the byte identity is the
//! property actually built here and it stands on its own.
//!
//! **The whole brief is framed.** A header line saying the block
//! is quoted history and not instructions, then everything below between the
//! `inject` module's fixed tags, the closing tag last. No turn is excluded
//! here: the brief quotes the last typed prompt and the last assistant turn,
//! and neither is ever a tool result.
//!
//! **The one thing it writes is which turns it quoted.** A brief that rendered
//! records its turn ids in the session's [`super::state`] file, so that the
//! prompt arm does not spend a prompt re-injecting a turn already sitting on
//! screen above it. Nothing else here writes anything, and a brief
//! that rendered nothing writes nothing at all.
//!
//! **Nothing here fails.** Every block is an `Option` and the brief is the ones
//! that rendered: a session with no meta row, no branch, no timestamp or block rows
//! that will not open drops that block and keeps the rest, because a brief that
//! returned an error would be a `SessionStart` that emitted nothing at all.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::{chars, clip, Payload};
use crate::config::Config;
use crate::observe::egress::Redaction;
use crate::recall::scope::Scoped;
use crate::recall::{excerpt, Query};
use crate::store::archive;

/// Render the resume brief for one `SessionStart`, or nothing.
///
/// Nothing is the ordinary answer on a machine with no archive, in a directory
/// no archived project covers, and in an excluded project. It is also every
/// failure: see the module doc on `inject`.
pub fn session_start(data_dir: &Path, config: &Config, payload: &Payload) -> Option<String> {
    // First, and before anything that can decline to render: the flag is what
    // the prompt after a compaction is owed, and a machine whose store is not
    // there yet still just compacted a session.
    if payload.source == Some(super::COMPACT_SOURCE) {
        owe_compaction(data_dir, payload);
    }
    let store = super::open(data_dir)?;
    let scoped = super::scoped(store.conn(), config, payload)?;
    // Resolved at the entry point that holds the `Config` and carried down. A
    // brief is hook stdout, which is the model's first context of the session,
    // so its quotations are an egress boundary in exactly the way a search
    // excerpt is.
    let brief = render(
        store.conn(),
        &scoped,
        config.brief_chars(),
        &Redaction::of(config),
    )?;
    remember(data_dir, payload, &brief);
    Some(brief.text)
}

/// What the brief [`session_start`] would render read from the archive: the
/// block rows fetched and blocks decompressed for the two turns it quotes.
/// Renders the same brief, writes no state and emits nothing, so the
/// hook's output stays exactly what it was. Test support only.
#[cfg(feature = "testkit")]
pub fn session_start_reads(
    data_dir: &Path,
    config: &Config,
    payload: &Payload,
) -> Option<excerpt::Reads> {
    let store = super::open(data_dir)?;
    let scoped = super::scoped(store.conn(), config, payload)?;
    let brief = render(
        store.conn(),
        &scoped,
        config.brief_chars(),
        &Redaction::of(config),
    )?;
    Some(brief.reads)
}

/// Leave the next prompt an injection to make.
///
/// A compaction fires a `SessionStart` of its own, and the brief it renders is
/// the ordinary one - what changed is that the model just lost most of its
/// context, and the turns it lost are worth offering back to whatever it asks
/// next. That cannot be answered here: the boundary row is written by the
/// ingest this hook spawned, the prompt has not been typed, and the brief is on
/// a single-digit-millisecond budget. So it becomes a flag, and the prompt arm
/// spends it.
///
/// A flag rather than a timestamp comparison, and it persists until it fires:
/// if the boundary row is not committed by the time the next prompt arrives,
/// the debt carries rather than being dropped on the losing side of that race.
fn owe_compaction(data_dir: &Path, payload: &Payload) {
    let mut state = super::state::State::load(data_dir, payload.session_id);
    if state.compaction_owed {
        return;
    }
    state.compaction_owed = true;
    state.save(data_dir, payload.session_id);
}

/// Tell this session's state file which turns the brief just quoted.
///
/// The one write this arm makes, and the reason it exists is on the other side:
/// the prompt arm refuses to inject a turn the session already has, and a turn
/// the brief quoted at `SessionStart` is one the session already has. Without
/// this, the first prompt naming what the last session was doing would be
/// answered with the exact text sitting above it on screen.
///
/// **Only after the brief has actually rendered**, and only for the turns that
/// survived the budget into it: a machine with no archive acquires no files
/// from asking, and a quotation the budget cut away was never carried.
fn remember(data_dir: &Path, payload: &Payload, brief: &Brief) {
    if brief.turns.is_empty() {
        return;
    }
    let mut state = super::state::State::load(data_dir, payload.session_id);
    for turn_id in &brief.turns {
        state.record_brief(*turn_id);
    }
    state.save(data_dir, payload.session_id);
}

/// The ceiling on a brief, whatever `terminus.toml` configures.
///
/// Bundle 2.1.237 persists a hook stdout longer than 10,000 characters to disk
/// and hands the model a reference to the file instead of the text. A brief
/// past that stops being context and becomes a path, so the configured budget
/// is clamped here rather than trusted: a user who writes a larger number gets
/// the largest brief that is still a brief.
pub const MAX_BRIEF_CHARS: usize = 10_000;

/// A rendered brief: the text the hook emits and the turns it quoted.
///
/// The ids travel with the text rather than being recovered from it, because
/// the brief does not name them - it is prose, and stays prose - while the
/// prompt arm still has to know which turns this session was handed.
struct Brief {
    text: String,
    /// The turns quoted in [`Brief::text`], after the budget had its say.
    turns: Vec<i64>,
    /// What reading the quoted turns cost. Never rendered.
    #[cfg_attr(not(feature = "testkit"), allow(dead_code))]
    reads: excerpt::Reads,
}

/// The brief's blocks, in order, joined by a blank line, inside `budget`.
///
/// The last session first and the index pointer last: continuity is what the
/// session is resuming, and the pointer is the standing fact that outlives it.
/// A block that cannot be rendered is dropped rather than failing the brief.
///
/// `redaction` reaches the quotes before this function measures anything, on
/// purpose: the filter is spent out of the budget rather than applied to what
/// the budget left. A marker consuming room a quotation would otherwise
/// have had is the decided behaviour. Filtering after [`clip`] is what it buys
/// out - a marker cut in half leaves a nameless partial value no shape rule can
/// catch, and the brief would be the one output where a long turn leaks.
///
/// The head line, the branch and the index pointer are built here out of
/// `session_meta` and counts and are not redacted. The day, the branch and the
/// project label are archived strings all the same, so each has any closing tag
/// neutralized where it is built.
///
/// **All of it goes inside the frame.** The header and both tags
/// are a fixed cost that comes off first; a budget they do not fit renders no
/// brief at all, so [`remember`] writes nothing. Nor does a budget whose room
/// after the frame cannot hold more than [`excerpt::ELISION`]: any cut there
/// would be a few raw characters with nothing saying so (#46). The pointer goes
/// inside with the rest because it repeats the project label, which is
/// archived text.
fn render(
    conn: &Connection,
    scoped: &Scoped,
    budget: usize,
    redaction: &Redaction,
) -> Option<Brief> {
    let framed = budget.min(MAX_BRIEF_CHARS);
    let budget = framed
        .checked_sub(super::frame_cost(DESCRIPTION))
        .filter(|&room| room > chars(excerpt::ELISION))?;
    let (continuity, reads) = last_session(conn, scoped, redaction);
    let pointer = index_pointer(conn, scoped);
    if continuity.is_none() && pointer.is_none() {
        return None;
    }

    let full = assemble(continuity.as_ref(), pointer.as_deref());
    if chars(&full) <= budget {
        return Some(Brief {
            turns: quoted(continuity.as_ref()),
            text: super::frame(DESCRIPTION, &full, framed)?,
            reads,
        });
    }

    // The variable-length parts are cut first and the blocks are not dropped:
    // the head line and the pointer are the cheapest text in the brief and the
    // most useful - the pointer is the ~30 tokens that tell the model
    // searchable memory exists at all - while the quoted prompt and reply are
    // the only parts whose size the archive controls.
    let spent = chars(&assemble(
        continuity.as_ref().map(Continuity::bare).as_ref(),
        pointer.as_deref(),
    ));
    let cut = continuity.map(|continuity| continuity.within(budget.saturating_sub(spent)));
    // The backstop inside `frame`, and only that: it fires when the head line,
    // the pointer and the marks of turns cut to nothing are over what the frame
    // left, which no cut to the quoted turns can fix. It cuts the pointer's tail
    // with a mark of its own, inside the frame, never the closing tag.
    Some(Brief {
        turns: quoted(cut.as_ref()),
        text: super::frame(
            DESCRIPTION,
            &assemble(cut.as_ref(), pointer.as_deref()),
            framed,
        )?,
        reads,
    })
}

/// What this arm says the block holds, ahead of [`super::QUOTED_HISTORY`] on
/// the header line.
const DESCRIPTION: &str =
    "Terminus resume brief - where the last session in this project left off.";

/// The turns a rendered continuity block quotes.
///
/// Read off the block that was actually assembled, never off the one that was
/// selected: a quotation the budget left no room for shows as its label and
/// [`excerpt::ELISION`] alone (see [`Continuity::within`]), and a turn the model
/// was not shown is a turn the prompt arm must still be free to inject.
fn quoted(continuity: Option<&Continuity>) -> Vec<i64> {
    let Some(continuity) = continuity else {
        return Vec::new();
    };
    [continuity.prompt.as_ref(), continuity.reply.as_ref()]
        .into_iter()
        .flatten()
        .filter(|quote| quote.shown)
        .map(|quote| quote.turn_id)
        .collect()
}

/// The brief as text: the continuity block, then the pointer.
fn assemble(continuity: Option<&Continuity>, pointer: Option<&str>) -> String {
    let mut blocks: Vec<String> = Vec::new();
    if let Some(continuity) = continuity {
        blocks.push(continuity.text());
    }
    if let Some(pointer) = pointer {
        blocks.push(pointer.to_owned());
    }
    blocks.join("\n\n")
}

/// The continuity block, kept in pieces until the budget is known.
///
/// The head is fixed-length in everything but the project key and the branch
/// name; the two quoted turns are whatever the archive holds. Rendering them
/// separately is what lets the budget cut the second without touching the first.
struct Continuity {
    head: String,
    prompt: Option<Quote>,
    reply: Option<Quote>,
}

/// One quoted turn: the text the brief shows and the id it came from.
///
/// The id is never rendered - the brief is prose - and is carried only so that
/// [`remember`] can tell the prompt arm what this session has already been
/// shown.
struct Quote {
    turn_id: i64,
    text: String,
    /// Whether any of the turn's own text is in [`Quote::text`]. False for a
    /// turn the budget cut to the mark alone, which the brief did not carry.
    shown: bool,
}

impl Continuity {
    fn text(&self) -> String {
        let mut out = self.head.clone();
        if let Some(prompt) = &self.prompt {
            out.push_str(&format!("\nIt last asked: {}", prompt.text));
        }
        if let Some(reply) = &self.reply {
            out.push_str(&format!("\nIt last answered: {}", reply.text));
        }
        out
    }

    /// The same block with both quoted turns emptied, for measuring what the
    /// fixed parts cost. It keeps the labels, which every cut renders, and not
    /// the marks: a turn cut to the mark alone costs at most the mark's length
    /// over its share, and the backstop in [`super::frame`] takes that from the
    /// pointer's tail.
    fn bare(&self) -> Continuity {
        let empty = |quote: &Option<Quote>| {
            quote.as_ref().map(|quote| Quote {
                turn_id: quote.turn_id,
                text: String::new(),
                shown: quote.shown,
            })
        };
        Continuity {
            head: self.head.clone(),
            prompt: empty(&self.prompt),
            reply: empty(&self.reply),
        }
    }

    /// The block with its two quoted turns sharing `allowance` characters.
    ///
    /// Even shares, except that a turn shorter than its half hands the
    /// remainder to the other rather than wasting it - a one-line prompt
    /// against a long reply is the ordinary shape of a session. A turn whose
    /// share cannot hold a marked cut renders as its label and
    /// [`excerpt::ELISION`] alone, so the brief says it cut the turn, and is
    /// not recorded as briefed (#46).
    fn within(self, allowance: usize) -> Continuity {
        let (prompt, reply) = shares(self.prompt.as_ref(), self.reply.as_ref(), allowance);
        Continuity {
            head: self.head,
            prompt,
            reply,
        }
    }
}

/// Split `allowance` characters between the two quoted turns.
fn shares(
    prompt: Option<&Quote>,
    reply: Option<&Quote>,
    allowance: usize,
) -> (Option<Quote>, Option<Quote>) {
    let want = |quote: Option<&Quote>| quote.map(|quote| chars(&quote.text)).unwrap_or(0);
    let half = allowance / 2;

    // First pass: nobody takes more than half, and nobody takes more than it
    // has. What is left over is then offered to whichever still wants it.
    let mut given_prompt = want(prompt).min(half);
    let mut given_reply = want(reply).min(allowance - half);
    let mut spare = allowance - given_prompt - given_reply;
    for (given, wanted) in [
        (&mut given_prompt, want(prompt)),
        (&mut given_reply, want(reply)),
    ] {
        let extra = wanted.saturating_sub(*given).min(spare);
        *given += extra;
        spare -= extra;
    }

    // A share smaller than the text and no larger than the mark cannot hold a
    // marked cut: `clip` would hand back a few raw characters, and a share of
    // nothing would drop the turn. Either way the turn is shown as the mark.
    let cut = |quote: Option<&Quote>, given: usize| -> Option<Quote> {
        let quote = quote?;
        let shown = given >= chars(&quote.text) || given > chars(excerpt::ELISION);
        Some(Quote {
            turn_id: quote.turn_id,
            text: if shown {
                clip(&quote.text, given)
            } else {
                excerpt::ELISION.to_owned()
            },
            shown,
        })
    };
    (cut(prompt, given_prompt), cut(reply, given_reply))
}

/// What the brief calls the project it is about.
///
/// `None` is the every-project scope, which reaches here only from a config
/// that excludes nothing and a `cwd` that resolved to no key.
fn project_label(scoped: &Scoped) -> &str {
    scoped.project().unwrap_or("this machine")
}

/// The `session_meta` row the brief is about, and the two coordinates it needs.
struct LastSession {
    session_key: String,
    last_turn_at: Option<String>,
    branch: Option<String>,
}

/// The continuity blocks: which session was last here, when it ended, on
/// what branch, and the last thing said in each direction, with what reading
/// those two turns cost.
fn last_session(
    conn: &Connection,
    scoped: &Scoped,
    redaction: &Redaction,
) -> (Option<Continuity>, excerpt::Reads) {
    let Some(row) = last_session_row(conn, scoped).ok().flatten() else {
        return (None, excerpt::Reads::default());
    };
    // Archived strings, every one: the label is a `cwd` out of a transcript,
    // the day is cut from a stored `ts` and the branch is `gitBranch`.
    let project = super::neutralize(project_label(scoped));
    let day = row
        .last_turn_at
        .as_deref()
        .map(|ts| super::neutralize(day(ts)));
    let branch = row.branch.as_deref().map(super::neutralize);
    let head = match (day, branch) {
        (Some(day), Some(branch)) => {
            format!("The last session in {project} ended {day}, on branch {branch}.")
        }
        (Some(day), None) => format!("The last session in {project} ended {day}."),
        (None, Some(branch)) => format!("The last session in {project} was on branch {branch}."),
        (None, None) => format!("The last session in {project} is archived."),
    };

    let (prompt, reply, reads) = last_exchange(conn, &row.session_key, redaction);
    let continuity = Continuity {
        head,
        prompt,
        reply,
    };
    (Some(continuity), reads)
}

/// The greatest `last_turn_at` among the scoped project's own sessions.
///
/// Three things this query is not. It is not `is_final`: that column is
/// declared in the schema and never written, so ordering by it would name a
/// session for a reason nothing establishes. It is not a date
/// comparison: 22,412 turns of a 180-file sample carry exactly the shape
/// `NNNN-NN-NNTNN:NN:NN.NNNZ`, which sorts lexicographically, so
/// no parsing happens on the stored side. And it does not consider sidecars: a
/// session with a `parent_session_key` is a subagent transcript, and the brief
/// carries no subagent turns.
///
/// The tie-break on `session_key` is the brief's own, not SQL's: two sessions
/// whose last turn carries the same timestamp must not be ordered by whatever
/// the query planner did that run, or the brief changes without the archive
/// changing.
///
/// A null `last_turn_at` sorts last under `DESC`, so a session that has one
/// always wins over one that does not, and a project where nothing has a
/// timestamp still names a session.
fn last_session_row(
    conn: &Connection,
    scoped: &Scoped,
) -> crate::error::Result<Option<LastSession>> {
    let mut sql = String::from(
        "SELECT m.session_key, m.last_turn_at, m.branch FROM session_meta m \
         WHERE m.parent_session_key IS NULL",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    scoped.push_filter(&mut sql, &mut params);
    sql.push_str(" ORDER BY m.last_turn_at DESC, m.session_key DESC LIMIT 1");

    let row = conn
        .query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| {
            Ok(LastSession {
                session_key: r.get(0)?,
                last_turn_at: r.get(1)?,
                branch: r.get(2)?,
            })
        })
        .optional()?;
    Ok(row)
}

/// The day of a stored timestamp, at day resolution.
///
/// `get` and not `&ts[..10]`, for the reason `cmd::search::day` states: `len`
/// counts bytes, the slice needs a char boundary, and a stored `ts` is any JSON
/// string the transcript carried - `parse::record` validates neither shape nor
/// encoding. A timestamp with a multi-byte character across byte 10 panicked a
/// whole command once already, and a panic here is a `SessionStart` that emits
/// nothing.
fn day(ts: &str) -> &str {
    ts.get(..10).unwrap_or(ts)
}

/// The last turn the person TYPED and the last assistant turn of one session,
/// as text.
///
/// The prompt side is not the last `user` record: Claude Code writes
/// tool results, `isMeta` command caveats and `<task-notification>` envelopes as
/// `user` records too, and one session in eight ends on one of them. Quoting
/// that back is a brief that opens by telling the model a file listing was the
/// last thing it was asked. [`last_turn`] carries the rule; a session with no
/// typed `user` record at all renders no "It last asked" line, because the
/// lookup returns nothing and [`Continuity::text`] omits what it has not got.
///
/// **One read for the pair, of only the rows it covers.** Both turns are
/// in the same session, so one read fetches and decompresses each block row the
/// two turns cover once - usually the session's last row - on the path with the
/// tightest budget in the product. What it cost comes back beside the quotes.
///
/// `turn_seq` order and never `ts` order: 31 of 62 sampled real transcripts
/// carry a record whose timestamp runs backwards, so the last
/// turn by clock is not the last turn of the conversation.
///
/// The text is cut through [`excerpt::of_record_with`] under the caller's
/// redaction, which is the same projection under the same filter that ingest
/// indexed these turns with and the search path reads them through - the brief
/// carries the last prompt and the last assistant turn truncated, and a second
/// definition of what a turn said would be a second thing to keep true. The
/// query is empty because there is no query here: the window then falls back to
/// the head of the turn, which is what the turn opened with.
fn last_exchange(
    conn: &Connection,
    session_key: &str,
    redaction: &Redaction,
) -> (Option<Quote>, Option<Quote>, excerpt::Reads) {
    let mut reads = excerpt::Reads::default();
    let prompt = last_turn(conn, session_key, "user");
    let reply = last_turn(conn, session_key, "assistant");
    if prompt.is_none() && reply.is_none() {
        return (None, None, reads);
    }

    let pair = [prompt, reply];
    let wanted: Vec<(u64, u64)> = pair
        .iter()
        .flatten()
        .map(|&(_, offset, len)| (offset as u64, len as u64))
        .collect();
    let Some(read) = archive::ranges(conn, session_key, &wanted).ok().flatten() else {
        return (None, None, reads);
    };
    reads.add(&read);

    let query = Query::parse("");
    let mut bytes = read.bytes.into_iter();
    let mut cut = |coordinates: Option<(i64, i64, i64)>| -> Option<Quote> {
        let (turn_id, _, _) = coordinates?;
        let bytes = bytes.next()?.ok()?;
        let text = excerpt::of_record_scrubbed(&query, &bytes, redaction, super::neutralize);
        (!text.trim().is_empty()).then_some(Quote {
            turn_id,
            text,
            shown: true,
        })
    };
    let [prompt, reply] = pair;
    (cut(prompt), cut(reply), reads)
}

/// `(id, stream_offset, byte_len)` of one session's last turn of a record type,
/// skipping the `user` records nobody typed.
///
/// Both offsets address the UNCOMPRESSED session stream, never a block.
/// `UNIQUE (session_key, turn_seq)` covers the lookup. The id comes back too
/// because the prompt arm suppresses a turn the brief quoted and cannot
/// recognize one from its text.
///
/// **`is_typed IS NOT 0` and not `is_typed = 1`**, which is what lets one
/// statement serve all three callers. `IS NOT` is SQLite's null-safe
/// comparison, so a null passes: an `assistant` row carries null by
/// construction (`parse::record` classifies `user` records and nothing else),
/// and a preserved evicted session's rows keep null because `reindex` skips
/// them. For an evicted session the choice is
/// unobservable anyway - `retention::apply` deletes its block rows, so
/// [`last_exchange`] reads no bytes and renders no quote either way.
///
/// One `AND` and no new index. Measured against the live 1.16 GB store
/// (450,834 turns), the lookup costs 0.002 ms and a variant forced to scan a
/// 2,465-turn session backwards without matching costs 0.362 ms, both planned
/// as `SEARCH turns USING INDEX sqlite_autoindex_turns_1`, against a 10 ms
/// wall; real scan depth is p50 27, p90 181, p99 404, max 520 rows.
fn last_turn(conn: &Connection, session_key: &str, record_type: &str) -> Option<(i64, i64, i64)> {
    conn.query_row(
        "SELECT id, stream_offset, byte_len FROM turns \
         WHERE session_key = ?1 AND record_type = ?2 AND is_typed IS NOT 0 \
         ORDER BY turn_seq DESC LIMIT 1",
        rusqlite::params![session_key, record_type],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .ok()
    .flatten()
}

/// The index pointer: how much of this project is archived, and that there
/// are tools to reach it with.
///
/// The tool names are spelled here rather than taken from the MCP module
/// because that module is in the binary and this is the library; the two lists
/// are held together by `crates/terminus/tests/inject.rs`. A project with
/// nothing archived renders no pointer at all - "0 sessions indexed" is a line
/// that costs context and tells the model to stop asking.
fn index_pointer(conn: &Connection, scoped: &Scoped) -> Option<String> {
    let sessions = super::count(conn, SESSIONS, scoped).ok()?;
    let turns = super::count(conn, TURNS, scoped).ok()?;
    if sessions == 0 {
        return None;
    }
    let project = super::neutralize(project_label(scoped));
    Some(format!(
        "Terminus has {} and {} archived for {project}, searchable at turn granularity.\n\
         Reach them with the recall_search, recall_context and recall_get tools \
         rather than guessing.",
        plural(sessions, "session"),
        plural(turns, "turn"),
    ))
}

/// Sessions of the scoped project. `idx_session_meta_project` covers it.
const SESSIONS: &str = "SELECT count(*) FROM session_meta m";

/// Turns of those sessions. The join is on `session_key`, which is
/// `session_meta`'s primary key and the leading column of `turns`'
/// `UNIQUE (session_key, turn_seq)` index, so neither side is a scan.
const TURNS: &str = "SELECT count(*) FROM turns t JOIN session_meta m USING (session_key)";

/// `1 session` / `2 sessions`, for a brief that reads as English.
fn plural(count: i64, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}
