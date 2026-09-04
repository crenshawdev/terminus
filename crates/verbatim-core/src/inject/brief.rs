//! The `SessionStart` resume brief (INJ-01, INJ-02).
//!
//! What a session opens knowing: where the last session in this project left
//! off - its date, the branch it ended on, the last thing said in either
//! direction - and how much of the project is archived and reachable. Every
//! value comes from a stored row, so two runs against an unchanged store render
//! the same bytes (INJ-02).
//!
//! **The quoted prompt is a turn the person typed (INJ-07).** "The last thing
//! said" in the user's direction is not the last `user` record: the harness
//! writes tool results, `isMeta` command caveats and `<task-notification>`
//! envelopes as `user` records as well, and 12.3% of measured sessions end on
//! one. The discriminator is `turns.is_typed`, written at ingest by
//! `parse::record` and rebuilt from blobs alone, and the brief spends exactly
//! one `AND` on it - see [`last_turn`].
//!
//! The counts are read the way D-09 requires: the project-only projection
//! `scope::resolve` has already run, plus one targeted `count(*)` per number,
//! and never `config::visible::sessions`. Measured on a store shaped like the
//! real one - 2,000 sessions, 250,000 turns, warm - `sessions()` costs 6.2-6.8
//! ms against 0.56-0.58 ms for the project-only join, because of its
//! per-session `count(*)` over `turns` and its watermark lookup. The whole
//! budget here is single digit milliseconds, so a brief built on it would spend
//! the budget before reading a useful row.
//!
//! **The working-state delta is the branch and nothing else (D-10).**
//! `session_meta.branch` is the only git fact the archive holds, and
//! `DESIGN-BRIEF.md:230` asks for HEAD-then-versus-now and a dirty-file list as
//! well. Both need `git`, whose 10-30 ms process spawn is the whole budget
//! several times over - phase 3 D-12 already bars `project::Resolver` from the
//! cold-start path for the same reason. So no `std::process::Command` appears
//! anywhere in this module; the model can check live git itself in one cheap
//! tool call, and this brief tells it what the archive knew.
//!
//! **Nothing here reads a clock (INJ-02).** No `SystemTime`, no `Instant`, no
//! elapsed-time or "as of" phrasing, and no count that depends on when the read
//! happened: every value is a stored one, dates are rendered at day resolution,
//! and anything ordered is ordered by a stored key with a TOTAL order - the
//! reason `recall::search`'s `TAIL` is total as well. A tie broken differently
//! between two runs is a brief that changed while the archive did not, which is
//! the same defect as a timestamp in it. The stated rationale is the Anthropic
//! prefix cache, which nothing local can observe; the byte identity is the
//! property actually built here and it stands on its own.
//!
//! **The one thing it writes is which turns it quoted.** A brief that rendered
//! records its turn ids in the session's [`super::state`] file, so that the
//! prompt arm does not spend a prompt re-injecting a turn already sitting on
//! screen above it (INJ-04). Nothing else here writes anything, and a brief
//! that rendered nothing writes nothing at all.
//!
//! **Nothing here fails.** Every block is an `Option` and the brief is the ones
//! that rendered: a session with no meta row, no branch, no timestamp or a blob
//! that will not open drops that block and keeps the rest, because a brief that
//! returned an error would be a `SessionStart` that emitted nothing at all.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::Payload;
use crate::blob::BlobReader;
use crate::config::Config;
use crate::observe::egress::Redaction;
use crate::recall::scope::Scoped;
use crate::recall::{excerpt, Query};

/// Render the resume brief for one `SessionStart`, or nothing.
///
/// Nothing is the ordinary answer on a machine with no archive, in a directory
/// no archived project covers, and in an excluded project. It is also every
/// failure: see the module doc on `inject`.
pub fn session_start(data_dir: &Path, config: &Config, payload: &Payload) -> Option<String> {
    // First, and before anything that can decline to render: the flag is what
    // INJ-05 is owed, and a machine whose store is not there yet still just
    // compacted a session (D-08).
    if payload.source == Some(super::COMPACT_SOURCE) {
        owe_compaction(data_dir, payload);
    }
    let store = super::open(data_dir)?;
    let scoped = super::scoped(store.conn(), config, payload)?;
    // Resolved at the entry point that holds the `Config` and carried down
    // (phase 4 D-01). A brief is hook stdout, which is the model's first
    // context of the session, so its quotations are an egress boundary in
    // exactly the way a search excerpt is.
    let brief = render(
        store.conn(),
        &scoped,
        config.brief_chars(),
        &Redaction::of(config),
    )?;
    remember(data_dir, payload, &brief);
    Some(brief.text)
}

/// Leave the next prompt an injection to make (INJ-05, D-08).
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

/// Tell this session's state file which turns the brief just quoted (INJ-04).
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

/// The ceiling on a brief, whatever `verbatim.toml` configures.
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
/// the brief does not name them - it is prose, and INJ-01 wants it to stay
/// prose - while INJ-04 still has to know which turns this session was handed.
struct Brief {
    text: String,
    /// The turns quoted in [`Brief::text`], after the budget had its say.
    turns: Vec<i64>,
}

/// The brief's blocks, in order, joined by a blank line, inside `budget`.
///
/// The last session first and the index pointer last: continuity is what the
/// session is resuming, and the pointer is the standing fact that outlives it.
/// A block that cannot be rendered is dropped rather than failing the brief.
///
/// `redaction` reaches the quotes before this function measures anything, which
/// is phase 4 D-06: the filter is spent out of the budget rather than applied to
/// what the budget left. A marker consuming room a quotation would otherwise
/// have had is the decided behaviour. Filtering after [`clip`] is what it buys
/// out - a marker cut in half leaves a nameless partial value no shape rule can
/// catch, and the brief would be the one output where a long turn leaks.
///
/// The head line, the branch and the index pointer are built here out of
/// `session_meta` and counts, not out of archived text, and are not filtered.
fn render(
    conn: &Connection,
    scoped: &Scoped,
    budget: usize,
    redaction: &Redaction<'_>,
) -> Option<Brief> {
    let budget = budget.min(MAX_BRIEF_CHARS);
    let continuity = last_session(conn, scoped, redaction);
    let pointer = index_pointer(conn, scoped);
    if continuity.is_none() && pointer.is_none() {
        return None;
    }

    let full = assemble(continuity.as_ref(), pointer.as_deref());
    if chars(&full) <= budget {
        return Some(Brief {
            turns: quoted(continuity.as_ref()),
            text: full,
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
    // The backstop, and only that: it fires when the head line and the pointer
    // alone are over budget, which no cut to the quoted turns can fix.
    Some(Brief {
        turns: quoted(cut.as_ref()),
        text: clip(&assemble(cut.as_ref(), pointer.as_deref()), budget),
    })
}

/// The turns a rendered continuity block quotes.
///
/// Read off the block that was actually assembled, never off the one that was
/// selected: a quotation the budget cut down to nothing is dropped from the
/// text (see [`Continuity::within`]), and a turn the model was not shown is a
/// turn the prompt arm must still be free to inject.
fn quoted(continuity: Option<&Continuity>) -> Vec<i64> {
    let Some(continuity) = continuity else {
        return Vec::new();
    };
    [continuity.prompt.as_ref(), continuity.reply.as_ref()]
        .into_iter()
        .flatten()
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

/// INJ-01's continuity block, kept in pieces until the budget is known.
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
/// shown (INJ-04).
struct Quote {
    turn_id: i64,
    text: String,
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
    /// fixed parts cost. It keeps the labels, so the measurement is an upper
    /// bound on the fixed cost and never an under-count.
    fn bare(&self) -> Continuity {
        let empty = |quote: &Option<Quote>| {
            quote.as_ref().map(|quote| Quote {
                turn_id: quote.turn_id,
                text: String::new(),
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
    /// share is nothing is dropped rather than rendered as a label with an
    /// empty quotation.
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

    let cut = |quote: Option<&Quote>, given: usize| -> Option<Quote> {
        let quote = quote?;
        (given > 0).then(|| Quote {
            turn_id: quote.turn_id,
            text: clip(&quote.text, given),
        })
    };
    (cut(prompt, given_prompt), cut(reply, given_reply))
}

/// How many characters a string is, which is what the budget counts (D-16).
///
/// Characters and not bytes and not tokens: the workspace has no tokenizer and
/// will not grow one on the cold-start path, and bytes would under-count a brief
/// by a factor of three against the same text in another script.
fn chars(text: &str) -> usize {
    text.chars().count()
}

/// `text`, cut to `budget` characters with [`excerpt::ELISION`] where it was cut.
///
/// One spelling of three dots in the product, and cut on a character boundary:
/// a byte-indexed slice through a multi-byte character is a panic, and a panic
/// on this path is a `SessionStart` that emits nothing.
fn clip(text: &str, budget: usize) -> String {
    if chars(text) <= budget {
        return text.to_owned();
    }
    let marker = chars(excerpt::ELISION);
    if budget <= marker {
        return text.chars().take(budget).collect();
    }
    let mut out: String = text.chars().take(budget - marker).collect();
    out.push_str(excerpt::ELISION);
    out
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

/// INJ-01's continuity blocks: which session was last here, when it ended, on
/// what branch, and the last thing said in each direction.
fn last_session(
    conn: &Connection,
    scoped: &Scoped,
    redaction: &Redaction<'_>,
) -> Option<Continuity> {
    let row = last_session_row(conn, scoped).ok()??;
    let project = project_label(scoped);

    let day = row.last_turn_at.as_deref().map(day);
    let head = match (day, row.branch.as_deref()) {
        (Some(day), Some(branch)) => {
            format!("The last session in {project} ended {day}, on branch {branch}.")
        }
        (Some(day), None) => format!("The last session in {project} ended {day}."),
        (None, Some(branch)) => format!("The last session in {project} was on branch {branch}."),
        (None, None) => format!("The last session in {project} is archived."),
    };

    let (prompt, reply) = last_exchange(conn, &row.session_key, redaction);
    Some(Continuity {
        head,
        prompt,
        reply,
    })
}

/// The greatest `last_turn_at` among the scoped project's own sessions.
///
/// Three things this query is not. It is not `is_final`: that column is
/// declared in the schema and never written, so ordering by it would name a
/// session for a reason nothing establishes (D-17). It is not a date
/// comparison: 22,412 turns of a 180-file sample carry exactly the shape
/// `NNNN-NN-NNTNN:NN:NN.NNNZ` (phase 3 D-23), which sorts lexicographically, so
/// no parsing happens on the stored side. And it does not consider sidecars: a
/// session with a `parent_session_key` is a subagent transcript, and the brief
/// carries no subagent turns (`DESIGN-BRIEF.md:239`).
///
/// The tie-break on `session_key` is INJ-02's, not SQL's: two sessions whose
/// last turn carries the same timestamp must not be ordered by whatever the
/// query planner did that run, or the brief changes without the archive
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

    if let Some(project) = scoped.project() {
        sql.push_str(" AND m.project = ?");
        params.push(Box::new(project.to_owned()));
    }
    super::push_exclusion(
        &mut sql,
        &mut params,
        "m.project",
        scoped.excluded_projects(),
    );
    super::push_exclusion(
        &mut sql,
        &mut params,
        "m.project_pre_worktree",
        scoped.excluded_pre_worktree(),
    );
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

/// The day of a stored timestamp, at day resolution (INJ-02).
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
/// The prompt side is not the last `user` record (INJ-07): Claude Code writes
/// tool results, `isMeta` command caveats and `<task-notification>` envelopes as
/// `user` records too, and one session in eight ends on one of them. Quoting
/// that back is a brief that opens by telling the model a file listing was the
/// last thing it was asked. [`last_turn`] carries the rule; a session with no
/// typed `user` record at all renders no "It last asked" line, because the
/// lookup returns nothing and [`Continuity::text`] omits what it has not got.
///
/// **One blob read for the pair.** Both turns are in the same session and
/// rusqlite's incremental blob I/O is behind a feature this workspace does not
/// enable, so selecting the blob twice would decompress the same session twice -
/// up to 10.1 MB uncompressed (phase 1 D-05) on the path with the tightest
/// budget in the product.
///
/// `turn_seq` order and never `ts` order: 31 of 62 sampled real transcripts
/// carry a record whose timestamp runs backwards (phase 3 D-22), so the last
/// turn by clock is not the last turn of the conversation.
///
/// The text is cut through [`excerpt::of_record_with`] under the caller's
/// redaction, which is the same projection under the same filter that ingest
/// indexed these turns with and the search path reads them through -
/// `DESIGN-BRIEF.md:230` asks for the last prompt and the last assistant turn
/// "truncated", and a second definition of what a turn said would be a second
/// thing to keep true. The query is empty because there is no query here: the
/// window then falls back to the head of the turn, which is what the turn
/// opened with.
fn last_exchange(
    conn: &Connection,
    session_key: &str,
    redaction: &Redaction<'_>,
) -> (Option<Quote>, Option<Quote>) {
    let prompt = last_turn(conn, session_key, "user");
    let reply = last_turn(conn, session_key, "assistant");
    if prompt.is_none() && reply.is_none() {
        return (None, None);
    }

    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [session_key],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    let Some(blob) = blob else {
        return (None, None);
    };
    let Ok(reader) = BlobReader::open(&blob) else {
        return (None, None);
    };

    let query = Query::parse("");
    let cut = |coordinates: Option<(i64, i64, i64)>| -> Option<Quote> {
        let (turn_id, offset, len) = coordinates?;
        let bytes = reader.read_range(offset as u64, len as u64).ok()?;
        let text = excerpt::of_record_with(&query, &bytes, redaction);
        (!text.trim().is_empty()).then_some(Quote { turn_id, text })
    };
    (cut(prompt), cut(reply))
}

/// `(id, stream_offset, byte_len)` of one session's last turn of a record type,
/// skipping the `user` records nobody typed (INJ-07).
///
/// D-04: both offsets address the UNCOMPRESSED session stream, never the blob.
/// `UNIQUE (session_key, turn_seq)` covers the lookup. The id comes back too
/// because INJ-04 suppresses a turn the brief quoted and cannot recognize one
/// from its text.
///
/// **`is_typed IS NOT 0` and not `is_typed = 1`**, which is what lets one
/// statement serve all three callers. `IS NOT` is SQLite's null-safe
/// comparison, so a null passes: an `assistant` row carries null by
/// construction (`parse::record` classifies `user` records and nothing else),
/// and a preserved evicted session's rows keep null because `reindex` skips
/// them (v0.1.1 phase 1 D-04). For an evicted session the choice is
/// unobservable anyway - `retention::apply` empties the blob, so
/// [`last_exchange`] finds no reader and renders no quote either way.
///
/// One `AND` and no new index. Measured against the live 1.16 GB store
/// (450,834 turns), the lookup costs 0.002 ms and a variant forced to scan a
/// 2,465-turn session backwards without matching costs 0.362 ms, both planned
/// as `SEARCH turns USING INDEX sqlite_autoindex_turns_1`, against a 10 ms
/// wall; real scan depth is p50 27, p90 181, p99 404, max 520 rows (D-11).
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

/// INJ-01's index pointer: how much of this project is archived, and that there
/// are tools to reach it with.
///
/// The tool names are spelled here rather than taken from the MCP module
/// because that module is in the binary and this is the library; the two lists
/// are held together by `crates/verbatim/tests/inject.rs`. A project with
/// nothing archived renders no pointer at all - "0 sessions indexed" is a line
/// that costs context and tells the model to stop asking.
fn index_pointer(conn: &Connection, scoped: &Scoped) -> Option<String> {
    let sessions = super::count(conn, SESSIONS, scoped).ok()?;
    let turns = super::count(conn, TURNS, scoped).ok()?;
    if sessions == 0 {
        return None;
    }
    let project = project_label(scoped);
    Some(format!(
        "Verbatim has {} and {} archived for {project}, searchable at turn granularity.\n\
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
