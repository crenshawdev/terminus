//! The `SessionStart` resume brief.
//!
//! What a session opens knowing: where the last session in this project left
//! off - its date, its branch, the last few things it was asked, the last thing
//! it said, the files it edited - and how much of the project is archived and
//! reachable. Every value comes from a stored row, so two runs against an
//! unchanged store on the same branch render the same bytes.
//!
//! **Which session is the one on your branch.** The newest session is often a
//! one-shot or another branch's work, so the pick goes newest with a typed
//! prompt on this branch, then newest with a typed prompt, then just newest.
//! `HEAD` only chooses a row and never reaches the text.
//!
//! **The other sessions are named by their titles.** Nothing else shows Claude
//! what else is here, and it can't search for what it doesn't know exists. A
//! title describes how a session opened and never changes, so the list says that
//! rather than letting it read as what the session did.
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
//! **The live git fact is the branch, read as a file.**
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
//! here: the brief quotes typed prompts and the last assistant turn, and none
//! of them is ever a tool result.
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

use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{chars, clip, Payload};
use crate::config::Config;
use crate::observe::egress::Redaction;
use crate::observe::mechanical::WRITING_TOOLS;
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
        payload.cwd.and_then(checked_out_branch).as_deref(),
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
        payload.cwd.and_then(checked_out_branch).as_deref(),
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
    branch: Option<&str>,
) -> Option<Brief> {
    let framed = budget.min(MAX_BRIEF_CHARS);
    let budget = framed
        .checked_sub(super::frame_cost(DESCRIPTION))
        .filter(|&room| room > chars(excerpt::ELISION))?;
    let (continuity, reads) = last_session(conn, scoped, redaction, branch);
    let pointer = index_pointer(conn, scoped);
    if continuity.is_none() && pointer.is_none() {
        return None;
    }
    let picked = continuity.as_ref().map(|c| c.session_key.as_str());
    let others = other_sessions(conn, scoped, picked, redaction);

    let full = assemble(continuity.as_ref(), others.as_deref(), pointer.as_deref());
    if chars(&full) <= budget {
        return Some(Brief {
            turns: quoted(continuity.as_ref()),
            text: super::frame(DESCRIPTION, &full, framed)?,
            reads,
        });
    }

    // The other sessions go first, because this brief is about the last one.
    let full = assemble(continuity.as_ref(), None, pointer.as_deref());
    if chars(&full) <= budget {
        return Some(Brief {
            turns: quoted(continuity.as_ref()),
            text: super::frame(DESCRIPTION, &full, framed)?,
            reads,
        });
    }

    // The file list goes before any quote gets cut: what was asked beats what
    // was touched, and the model can get the rest from the archive.
    let continuity = continuity.map(Continuity::without_edited);
    let full = assemble(continuity.as_ref(), None, pointer.as_deref());
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
        None,
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
            &assemble(cut.as_ref(), None, pointer.as_deref()),
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
    continuity
        .prompts
        .iter()
        .chain(continuity.reply.as_ref())
        .filter(|quote| quote.shown)
        .map(|quote| quote.turn_id)
        .collect()
}

/// The brief as text: the continuity block, the other sessions, then the pointer.
fn assemble(
    continuity: Option<&Continuity>,
    others: Option<&str>,
    pointer: Option<&str>,
) -> String {
    let mut blocks: Vec<String> = Vec::new();
    if let Some(continuity) = continuity {
        blocks.push(continuity.text());
    }
    if let Some(others) = others {
        blocks.push(others.to_owned());
    }
    if let Some(pointer) = pointer {
        blocks.push(pointer.to_owned());
    }
    blocks.join("\n\n")
}

/// The continuity block, kept in pieces until the budget is known.
///
/// The head is fixed-length in everything but the project key and the branch
/// names; the quoted turns are whatever the archive holds. Rendering them
/// separately is what lets the budget cut the quotes without touching the head.
struct Continuity {
    session_key: String,
    head: String,
    /// Oldest first, so the block reads in the order it happened.
    prompts: Vec<Quote>,
    reply: Option<Quote>,
    edited: Option<String>,
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
        if let Some((last, earlier)) = self.prompts.split_last() {
            for prompt in earlier {
                out.push_str(&format!("\nEarlier it asked: {}", prompt.text));
            }
            out.push_str(&format!("\nIt last asked: {}", last.text));
        }
        if let Some(reply) = &self.reply {
            out.push_str(&format!("\nIt last answered: {}", reply.text));
        }
        if let Some(edited) = &self.edited {
            out.push_str(&format!("\nIt edited: {edited}"));
        }
        out
    }

    fn without_edited(self) -> Continuity {
        Continuity {
            edited: None,
            ..self
        }
    }

    /// The same block with every quoted turn emptied, for measuring what the
    /// fixed parts cost. It keeps the labels, which every cut renders, and not
    /// the marks: a turn cut to the mark alone costs at most the mark's length
    /// over its share, and the backstop in [`super::frame`] takes that from the
    /// pointer's tail.
    fn bare(&self) -> Continuity {
        let empty = |quote: &Quote| Quote {
            turn_id: quote.turn_id,
            text: String::new(),
            shown: quote.shown,
        };
        Continuity {
            session_key: self.session_key.clone(),
            head: self.head.clone(),
            prompts: self.prompts.iter().map(empty).collect(),
            reply: self.reply.as_ref().map(empty),
            edited: self.edited.clone(),
        }
    }

    /// The block with its quoted turns sharing `allowance` characters.
    ///
    /// Even shares, except that a turn shorter than its share hands the
    /// remainder to the others rather than wasting it - a one-line prompt
    /// against a long reply is the ordinary shape of a session. A turn whose
    /// share cannot hold a marked cut renders as its label and
    /// [`excerpt::ELISION`] alone, so the brief says it cut the turn, and is
    /// not recorded as briefed (#46).
    fn within(self, allowance: usize) -> Continuity {
        let wants: Vec<usize> = self
            .prompts
            .iter()
            .chain(&self.reply)
            .map(|quote| chars(&quote.text))
            .collect();
        let mut given = shares(&wants, allowance).into_iter();
        let mut cut = |quote: &Quote| cut_to(quote, given.next().unwrap_or(0));
        Continuity {
            session_key: self.session_key,
            head: self.head,
            prompts: self.prompts.iter().map(&mut cut).collect(),
            reply: self.reply.as_ref().map(&mut cut),
            edited: self.edited,
        }
    }
}

/// Split `allowance` characters between quotes that each want so many: an even
/// share each round, until everybody has enough or nothing is left.
fn shares(wants: &[usize], allowance: usize) -> Vec<usize> {
    let mut given = vec![0; wants.len()];
    let mut spare = allowance;
    loop {
        let short: Vec<usize> = (0..wants.len()).filter(|&i| given[i] < wants[i]).collect();
        if short.is_empty() || spare == 0 {
            return given;
        }
        let share = (spare / short.len()).max(1);
        for i in short {
            let take = (wants[i] - given[i]).min(share).min(spare);
            given[i] += take;
            spare -= take;
        }
    }
}

/// One quote cut to its share.
///
/// A share smaller than the text and no larger than the mark cannot hold a
/// marked cut: `clip` would hand back a few raw characters, and a share of
/// nothing would drop the turn. Either way the turn is shown as the mark.
fn cut_to(quote: &Quote, given: usize) -> Quote {
    let shown = given >= chars(&quote.text) || given > chars(excerpt::ELISION);
    Quote {
        turn_id: quote.turn_id,
        text: if shown {
            clip(&quote.text, given)
        } else {
            excerpt::ELISION.to_owned()
        },
        shown,
    }
}

/// What the brief calls the project it is about.
///
/// `None` is the every-project scope, which reaches here only from a config
/// that excludes nothing and a `cwd` that resolved to no key.
fn project_label(scoped: &Scoped) -> &str {
    scoped.project().unwrap_or("this machine")
}

/// The `session_meta` row the brief is about, and the coordinates it needs.
struct LastSession {
    session_key: String,
    last_turn_at: Option<String>,
    branch: Option<String>,
    project: Option<String>,
    cwd: Option<String>,
    pre_worktree: Option<String>,
}

/// The continuity blocks: which session was last here, when it ended, on
/// what branch, what it was asked, what it said and what it edited, with what
/// reading those turns cost.
fn last_session(
    conn: &Connection,
    scoped: &Scoped,
    redaction: &Redaction,
    branch: Option<&str>,
) -> (Option<Continuity>, excerpt::Reads) {
    let Some(pick) = pick_session(conn, scoped, branch).ok().flatten() else {
        return (None, excerpt::Reads::default());
    };
    let row = &pick.row;
    let head = head(scoped, row, pick.later.as_ref());
    let (prompts, reply, reads) = last_exchange(conn, &row.session_key, redaction);
    let continuity = Continuity {
        session_key: row.session_key.clone(),
        head,
        prompts,
        reply,
        edited: edited_files(conn, row, redaction),
    };
    (Some(continuity), reads)
}

/// The head line, and a second line when a later session exists on another
/// branch, so preferring this branch never hides the newer work.
///
/// Archived strings, every one: the label is a `cwd` out of a transcript, the
/// day is cut from a stored `ts` and the branch is `gitBranch`.
fn head(scoped: &Scoped, row: &LastSession, later: Option<&LastSession>) -> String {
    let project = super::neutralize(project_label(scoped));
    let day_of = |row: &LastSession| {
        row.last_turn_at
            .as_deref()
            .map(|ts| super::neutralize(day(ts)).into_owned())
    };
    let branch_of = |row: &LastSession| {
        row.branch
            .as_deref()
            .map(|branch| super::neutralize(branch).into_owned())
    };
    let Some(later) = later else {
        return match (day_of(row), branch_of(row)) {
            (Some(day), Some(branch)) => {
                format!("The last session in {project} ended {day}, on branch {branch}.")
            }
            (Some(day), None) => format!("The last session in {project} ended {day}."),
            (None, Some(branch)) => {
                format!("The last session in {project} was on branch {branch}.")
            }
            (None, None) => format!("The last session in {project} is archived."),
        };
    };
    let branch = branch_of(row).unwrap_or_default();
    let mut head = match day_of(row) {
        Some(day) => format!("The last session on branch {branch} in {project} ended {day}."),
        None => format!("The last session on branch {branch} in {project} is archived."),
    };
    head.push_str(&match (day_of(later), branch_of(later)) {
        (Some(day), Some(branch)) => format!("\nA later session ended {day}, on branch {branch}."),
        (Some(day), None) => format!("\nA later session ended {day}."),
        (None, Some(branch)) => format!("\nA later session was on branch {branch}."),
        (None, None) => "\nA later session is archived.".to_owned(),
    });
    head
}

/// How many other sessions the brief names.
const SESSIONS_NAMED: usize = 5;

/// The longest a named session's title gets. Most run far shorter, and the
/// longest on a live archive is 109.
const TITLE_CHARS: usize = 80;

/// The newest sessions other than the one the brief is about, with a typed
/// prompt and a title, one `<day> <title>` line each.
///
/// `CROSS JOIN` for the reason [`edited_files`] gives: left alone, SQLite may start
/// from every title in the store. A session a script ran has no typed prompt, so
/// it isn't listed. Any failure is just no list.
fn other_sessions(
    conn: &Connection,
    scoped: &Scoped,
    picked: Option<&str>,
    redaction: &Redaction,
) -> Option<String> {
    let mut sql = String::from(
        "SELECT m.session_key, m.last_turn_at, s.title FROM session_meta m \
         CROSS JOIN session_titles s ON s.session_key = m.session_key \
         WHERE m.parent_session_key IS NULL",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    scoped.push_filter(&mut sql, &mut params);
    sql.push_str(" ORDER BY m.last_turn_at DESC, m.session_key DESC LIMIT ?");
    params.push(Box::new(SESSIONS_LOOKED_AT));

    let mut statement = conn.prepare(&sql).ok()?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .ok()?;

    let mut lines: Vec<String> = Vec::new();
    for (session_key, last_turn_at, title) in rows.flatten() {
        if lines.len() == SESSIONS_NAMED {
            break;
        }
        if picked == Some(session_key.as_str())
            || !has_typed_prompt(conn, &session_key).unwrap_or(false)
        {
            continue;
        }
        let title = clip(
            &excerpt::flatten(&super::neutralize(&redaction.apply(&title))),
            TITLE_CHARS,
        );
        lines.push(match last_turn_at.as_deref() {
            Some(ts) => format!("{} {title}", super::neutralize(day(ts))),
            None => title,
        });
    }
    (!lines.is_empty()).then(|| {
        format!(
            "Other recent sessions, titled by how each began:\n{}",
            lines.join("\n")
        )
    })
}

/// The session the brief is about, and the later one it has to mention when it
/// passed over a newer session for this branch's.
struct Pick {
    row: LastSession,
    later: Option<LastSession>,
}

/// Newest with a typed prompt on this branch, then newest with a typed prompt,
/// then newest. See the module doc for why.
fn pick_session(
    conn: &Connection,
    scoped: &Scoped,
    branch: Option<&str>,
) -> crate::error::Result<Option<Pick>> {
    let newest_typed = newest(conn, scoped, None, true)?;
    let on_branch = match branch {
        Some(name)
            if newest_typed
                .as_ref()
                .is_none_or(|r| r.branch.as_deref() != Some(name)) =>
        {
            newest(conn, scoped, Some(name), true)?
        }
        _ => None,
    };
    Ok(match (on_branch, newest_typed) {
        (Some(row), later) => Some(Pick { row, later }),
        (None, Some(row)) => Some(Pick { row, later: None }),
        (None, None) => newest(conn, scoped, None, false)?.map(|row| Pick { row, later: None }),
    })
}

/// How many of the newest sessions get looked at for a typed prompt. This runs
/// inside the hook's 50 ms, and a project this deep in one-shots isn't worth more.
const SESSIONS_LOOKED_AT: i64 = 64;

/// The greatest `last_turn_at` among the scoped project's own sessions, on one
/// branch if asked and with a typed prompt if asked.
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
///
/// The typed check runs row by row after the sort. As an `EXISTS` in the `WHERE`
/// SQLite would run it for every session in the project before sorting.
fn newest(
    conn: &Connection,
    scoped: &Scoped,
    branch: Option<&str>,
    typed: bool,
) -> crate::error::Result<Option<LastSession>> {
    let mut sql = String::from(
        "SELECT m.session_key, m.last_turn_at, m.branch, m.project, m.cwd, \
         m.project_pre_worktree FROM session_meta m WHERE m.parent_session_key IS NULL",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    scoped.push_filter(&mut sql, &mut params);
    if let Some(branch) = branch {
        sql.push_str(" AND m.branch = ?");
        params.push(Box::new(branch.to_owned()));
    }
    sql.push_str(" ORDER BY m.last_turn_at DESC, m.session_key DESC LIMIT ?");
    params.push(Box::new(SESSIONS_LOOKED_AT));

    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok(LastSession {
            session_key: r.get(0)?,
            last_turn_at: r.get(1)?,
            branch: r.get(2)?,
            project: r.get(3)?,
            cwd: r.get(4)?,
            pre_worktree: r.get(5)?,
        })
    })?;
    for row in rows {
        let row = row?;
        if !typed || has_typed_prompt(conn, &row.session_key)? {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// `UNIQUE (session_key, turn_seq)` covers it, and it stops at the first one.
fn has_typed_prompt(conn: &Connection, session_key: &str) -> crate::error::Result<bool> {
    let mut statement =
        conn.prepare_cached("SELECT 1 FROM turns WHERE session_key = ?1 AND is_typed = 1 LIMIT 1")?;
    Ok(statement.exists([session_key])?)
}

/// The most a read of `HEAD` or a worktree's `.git` file takes.
const GIT_FILE_BYTES: u64 = 4096;

/// The branch checked out under `cwd`, read as files, because starting `git`
/// costs more than this hook has.
///
/// None for a detached `HEAD`, no repository, or a `cwd` that isn't there, and
/// every none just means the pick ignores the branch. What it reads only ever
/// gets compared to a stored branch, so a hostile `.git` can choose among this
/// project's own sessions and put nothing in the text.
fn checked_out_branch(cwd: &str) -> Option<String> {
    let cwd = Path::new(cwd);
    if !cwd.is_dir() {
        return None;
    }
    let dot_git = cwd
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|path| path.exists())?;
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        worktree_git_dir(&dot_git)?
    };
    let head = read_git_file(&git_dir.join("HEAD"))?;
    let branch = head.trim_end().strip_prefix("ref: refs/heads/")?;
    (!branch.is_empty()).then(|| branch.to_owned())
}

/// A worktree's `.git` is a file that says where its real git directory is.
fn worktree_git_dir(dot_git: &Path) -> Option<PathBuf> {
    let file = read_git_file(dot_git)?;
    let target = Path::new(file.trim_end().strip_prefix("gitdir: ")?);
    Some(dot_git.parent()?.join(target))
}

/// Regular files only, because opening a FIFO named `HEAD` would sit there until
/// the hook gave up and cost the session its brief.
fn read_git_file(path: &Path) -> Option<String> {
    if !path.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(GIT_FILE_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
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

/// The last few turns the person TYPED and the last assistant turn of one
/// session, as text.
///
/// The prompt side is not the last `user` records: Claude Code writes
/// tool results, `isMeta` command caveats and `<task-notification>` envelopes as
/// `user` records too, and one session in eight ends on one of them. Quoting
/// that back is a brief that opens by telling the model a file listing was the
/// last thing it was asked. [`recent_turns`] carries the rule; a session with no
/// typed `user` record at all renders no "It last asked" line, because the
/// lookup returns nothing and [`Continuity::text`] omits what it has not got.
///
/// **One read for all of them, of only the rows they cover.** Every turn is
/// in the same session, so one read fetches and decompresses each block row the
/// turns cover once, on the path with the tightest budget in the product. What
/// it cost comes back beside the quotes.
///
/// `turn_seq` order and never `ts` order: 31 of 62 sampled real transcripts
/// carry a record whose timestamp runs backwards, so the last
/// turn by clock is not the last turn of the conversation.
///
/// The text is cut through [`excerpt::of_record_with`] under the caller's
/// redaction, which is the same projection under the same filter that ingest
/// indexed these turns with and the search path reads them through - the brief
/// carries the turns truncated, and a second definition of what a turn said
/// would be a second thing to keep true. The query is empty because there is no
/// query here: the window then falls back to the head of the turn, which is what
/// the turn opened with.
fn last_exchange(
    conn: &Connection,
    session_key: &str,
    redaction: &Redaction,
) -> (Vec<Quote>, Option<Quote>, excerpt::Reads) {
    let mut reads = excerpt::Reads::default();
    let prompts = prompts_to_read(recent_turns(conn, session_key, "user", PROMPTS_LOOKED_AT));
    let reply = recent_turns(conn, session_key, "assistant", 1).pop();
    if prompts.is_empty() && reply.is_none() {
        return (Vec::new(), None, reads);
    }

    let wanted: Vec<(u64, u64)> = prompts
        .iter()
        .chain(&reply)
        .map(|&(_, offset, len)| (offset as u64, len as u64))
        .collect();
    let Some(read) = archive::ranges(conn, session_key, &wanted).ok().flatten() else {
        return (Vec::new(), None, reads);
    };
    reads.add(&read);

    let query = Query::parse("");
    let mut bytes = read.bytes.into_iter();
    let mut cut = |coordinates: &(i64, i64, i64)| -> Option<Quote> {
        let bytes = bytes.next()?.ok()?;
        let text = excerpt::of_record_scrubbed(&query, &bytes, redaction, super::neutralize);
        (!text.trim().is_empty()).then_some(Quote {
            turn_id: coordinates.0,
            text,
            shown: true,
        })
    };
    let prompts: Vec<Quote> = prompts.iter().filter_map(&mut cut).collect();
    let reply = reply.as_ref().and_then(&mut cut);
    (worth_quoting(prompts), reply, reads)
}

/// How many of a session's newest typed prompts get read to find three worth
/// quoting.
const PROMPTS_LOOKED_AT: i64 = 12;

/// The most prompt bytes one brief reads, so a run of huge pastes can't eat the
/// hook's 50 ms. The newest prompt is always read.
const PROMPT_READ_BYTES: i64 = 256 * 1024;

/// How many prompts a brief quotes.
const PROMPTS_QUOTED: usize = 3;

/// Shorter than this it's a reply to the last answer, not something asked.
const SHORTEST_ASK_CHARS: usize = 20;

/// The newest prompts, stopping once they've used up [`PROMPT_READ_BYTES`].
fn prompts_to_read(newest_first: Vec<(i64, i64, i64)>) -> Vec<(i64, i64, i64)> {
    let mut spent = 0;
    newest_first
        .into_iter()
        .enumerate()
        .take_while(|&(n, (_, _, len))| {
            spent += len;
            n == 0 || spent <= PROMPT_READ_BYTES
        })
        .map(|(_, turn)| turn)
        .collect()
}

/// Up to three prompts that say something, oldest first. A session of nothing
/// but short replies still gets its last one, because that's still what it said.
fn worth_quoting(newest_first: Vec<Quote>) -> Vec<Quote> {
    let (mut kept, short): (Vec<Quote>, Vec<Quote>) = newest_first
        .into_iter()
        .partition(|quote| says_something(&quote.text));
    if kept.is_empty() {
        kept.extend(short.into_iter().next());
    }
    kept.truncate(PROMPTS_QUOTED);
    kept.reverse();
    kept
}

/// Long enough to be an ask, and not one of the two things a person "typed"
/// that aren't: a `!` shell command, and the line an interrupt leaves behind.
fn says_something(text: &str) -> bool {
    chars(text) >= SHORTEST_ASK_CHARS
        && !text.starts_with("<bash-input>")
        && !text.starts_with("[Request interrupted")
}

/// `(id, stream_offset, byte_len)` of one session's newest turns of a record
/// type, newest first, skipping the `user` records nobody typed.
///
/// Both offsets address the UNCOMPRESSED session stream, never a block.
/// `UNIQUE (session_key, turn_seq)` covers the lookup. The id comes back too
/// because the prompt arm suppresses a turn the brief quoted and cannot
/// recognize one from its text.
///
/// **`is_typed IS NOT 0` and not `is_typed = 1`**, which is what lets one
/// statement serve all three callers. `IS NOT` is SQLite's null-safe
/// comparison, so a null passes: an `assistant` row carries null by
/// construction (`parse::record` classifies `user` records and nothing else).
/// For an evicted session the choice is unobservable -
/// `retention::apply` deletes its block rows, so [`last_exchange`] reads no bytes
/// and renders no quote either way.
///
/// One `AND` and no new index. Measured against the live 1.16 GB store
/// (450,834 turns), the lookup costs 0.002 ms and a variant forced to scan a
/// 2,465-turn session backwards without matching costs 0.362 ms, both planned
/// as `SEARCH turns USING INDEX sqlite_autoindex_turns_1`, against a 10 ms
/// wall; real scan depth is p50 27, p90 181, p99 404, max 520 rows.
fn recent_turns(
    conn: &Connection,
    session_key: &str,
    record_type: &str,
    limit: i64,
) -> Vec<(i64, i64, i64)> {
    let turns = || -> rusqlite::Result<Vec<(i64, i64, i64)>> {
        conn.prepare_cached(
            "SELECT id, stream_offset, byte_len FROM turns \
             WHERE session_key = ?1 AND record_type = ?2 AND is_typed IS NOT 0 \
             ORDER BY turn_seq DESC LIMIT ?3",
        )?
        .query_map(rusqlite::params![session_key, record_type, limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect()
    };
    turns().unwrap_or_default()
}

/// How many files a brief names before it says how many more there were.
const FILES_NAMED: usize = 8;

/// The longest a named file gets.
const FILE_CHARS: usize = 120;

/// The files the session called a writing tool on, newest first and relative to
/// the project. A call that failed still counts: the record says it was tried,
/// not that it worked. Files outside the project are scratch, so they're left out.
///
/// `CROSS JOIN` forces turns first. Left alone, SQLite starts from every `path`
/// entity in the store, and that's a second against this hook's 50 ms.
fn edited_files(conn: &Connection, row: &LastSession, redaction: &Redaction) -> Option<String> {
    let tools: serde_json::Value = WRITING_TOOLS.iter().copied().collect();
    let mut statement = conn
        .prepare_cached(
            "SELECT e.value_norm FROM turns t CROSS JOIN entities e ON e.turn_id = t.id \
             WHERE t.session_key = ?1 AND e.kind = 'path' \
             AND t.tool_name IN (SELECT value FROM json_each(?2)) \
             GROUP BY e.value_norm ORDER BY max(t.turn_seq) DESC, e.value_norm",
        )
        .ok()?;
    let paths = statement
        .query_map(rusqlite::params![row.session_key, tools.to_string()], |r| {
            r.get::<_, String>(0)
        })
        .ok()?;

    let mut roots: Vec<&str> = [&row.cwd, &row.pre_worktree, &row.project]
        .into_iter()
        .filter_map(|root| root.as_deref())
        .collect();
    roots.sort_by_key(|root| std::cmp::Reverse(root.len()));

    let mut files: Vec<String> = Vec::new();
    for path in paths.flatten() {
        let Some(relative) = inside_project(&path, &roots) else {
            continue;
        };
        let file = clip(
            &excerpt::flatten(&super::neutralize(&redaction.apply(relative))),
            FILE_CHARS,
        );
        if !files.contains(&file) {
            files.push(file);
        }
    }
    let hidden = files.len().saturating_sub(FILES_NAMED);
    files.truncate(FILES_NAMED);
    let named = files.join(", ");
    match hidden {
        _ if named.is_empty() => None,
        0 => Some(format!("{named}.")),
        _ => Some(format!("{named} and {hidden} more.")),
    }
}

/// `path` relative to the longest root it sits under. An absolute path under no
/// root is outside the project.
fn inside_project<'p>(path: &'p str, roots: &[&str]) -> Option<&'p str> {
    let relative = roots
        .iter()
        .find_map(|root| path.strip_prefix(root)?.strip_prefix('/'));
    match relative {
        Some(relative) => Some(relative),
        None => (!path.starts_with(['/', '~'])).then_some(path),
    }
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
