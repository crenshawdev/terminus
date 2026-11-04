//! The `UserPromptSubmit` relevance injection.
//!
//! Silence is the answer this arm gives most of the time and the one it is
//! built to protect: three near-misses are worse than nothing at all, so a turn
//! is injected only on a structural match - a rank-1-to-3 exact entity, or two
//! independent entities co-occurring - and never on a BM25 score, which is not
//! comparable across queries.
//!
//! **What has already been given is never given again.** Three
//! suppressions, applied to the turns that passed the threshold and BEFORE the
//! cap of three, so a refused turn does not spend a slot: a turn an earlier
//! prompt of this session already injected, a turn of the session the user is
//! looking at, and a turn the resume brief quoted. Each is written into the
//! session's [`state`] file with its reason, which is the file a person reads
//! to see why a turn was not given.
//!
//! **What a prompt is asked about.** Not the sentence. A `Query` is
//! conjunctive by construction, so handing the whole prompt to
//! `MATCH` asks for a turn that repeats the prompt, which is silence for
//! anything a person would actually type. The candidates come from the
//! spellings a prompt names - its paths, resolved against the payload's `cwd`,
//! and its identifier-shaped tokens - each conjoined within itself and
//! disjoined across, which is the signal hierarchy injection ranks by: exact
//! matches on rare tokens, never a sentence-level similarity. A prompt naming
//! none of those never opens the store at all.
//!
//! **The decision is structural.** The search is a net and the threshold is
//! the judgement: a hit is injected when it matched at least one stored entity
//! AND sits in the top three of the ranked order, or when two independent
//! entities co-occur on it wherever it ranks. A hit the query reached only
//! through its text is never eligible, whatever its relevance - a score cutoff
//! is ruled out because bm25 scores are not comparable across queries, and free
//! text is the weakest signal in the hierarchy and the one that fires the most
//! false positives.
//!
//! **The one prompt after a compaction asks a different pool.** A
//! `SessionStart` whose `source` is `compact` leaves a flag in the session's
//! state file; the next prompt spends it, looks through a wider ranked window,
//! and keeps only the turns that just fell out of the model's context.
//! The threshold is NOT relaxed inside that pool and the suppressions still
//! apply - one definition of relevance for both paths - and the flag clears
//! only once a boundary row was actually there to read, so a prompt that beat
//! the ingest to it carries the debt forward rather than losing it.
//!
//! **Tool results are never injected.** A tool result is text Claude
//! read - a file, a web page, a command's output - and quoting it on a later
//! prompt hands it to the model as this project's own history, planted
//! instructions included. So the hits the threshold admits lose every `user`
//! turn the harness wrote (`turns.is_typed` = 0) before the suppressions and the
//! cap see them, and the step is [`quotable`], which `terminus replay` runs too.
//! The trade is accepted: error entities exist only on result turns, so a prompt
//! that pastes an error line no longer fires, and `recall_search` stays the way
//! back to those turns. A named file still fires, because a Read matches on its
//! tool-call turn as well as on its result.
//!
//! **What it hands over is framed.** A header line saying the block
//! is quoted history and not instructions, the turn lines between the
//! `inject` module's fixed tags, and the closing tag last. Every archived string
//! in the lines, the excerpt and the day, has any closing tag neutralized, and a
//! budget too small for the frame is an injection that does not happen.
//!
//! **Nothing decompresses a session until something has fired.** One excerpt
//! materializes a whole compressed session - p90 1.03 MB, max 10.1 MB over the
//! real corpus - so the search runs with excerpts off and the at most
//! three turns that survive the threshold get theirs afterwards.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;

use super::decision::{self, Candidate, Decision, Injected};
use super::state::{Reason, State, Suppressed};
use super::{compaction, Payload};
use crate::config::Config;
use crate::index::entity::{self, normalize_path};
use crate::observe::egress::Redaction;
use crate::recall::search::{self, Request};
use crate::recall::{excerpt, Hit, Query, Scope};

/// How many ranked hits the threshold is applied over.
///
/// Wider than the cap of three, so a hit that two entities corroborate can be
/// seen below the top three, and far narrower than `CANDIDATE_POOL`, which the
/// re-rank already fixes: ten rows of eight small columns, no blob touched.
pub const RANKED: usize = 10;

/// How many ranked hits the one prompt after a compaction looks through.
///
/// Wider than [`RANKED`] because that window is about to be cut down to one
/// session's dropped turns, and a window of ten would mostly return turns the
/// pool then discards. It costs what a wider window costs and no more: the
/// excerpt step is already skipped until the threshold has fired, so
/// these are small columns and no blob is touched. It is [`search::MAX_RESULTS`]
/// because that is the ceiling `search::run` clamps to anyway, and asking for
/// more would be a number that reads as a decision and is not one.
pub const COMPACTED_RANKED: usize = search::MAX_RESULTS;

/// The rank within which one matched entity is enough.
pub const ENTITY_RANK: usize = 3;

/// How many distinct entities make a hit eligible wherever it ranks.
///
/// Not a subset of the rank condition: corroboration by two independent facts
/// is evidence that bm25's ordering may have got this one wrong.
pub const CO_OCCURRING: usize = 2;

/// The most turns one prompt may ever be given.
pub const MAX_TURNS: usize = 3;

/// How many spellings one prompt may ask the archive about.
///
/// A pasted stack trace names dozens of files and identifiers, and every one
/// of them is another disjunct fts5 evaluates under a hard deadline. The
/// strongest signals are first - paths before symbols, in the order they were
/// typed - so the cut falls on the weakest.
pub const MAX_CANDIDATES: usize = 8;

/// The six numbers the firing decision is taken under.
///
/// **The live path constructs [`Thresholds::default`] and nothing else**, which
/// is exactly the six constants above: no [`Config`] field reaches it, no flag
/// detunes it and no `terminus.toml` key names it, because the precision-first
/// defaults are not detunable.
/// Three near-misses are worse than silence, and a threshold a user can
/// lower is a threshold a user lowers once and never raises.
///
/// What the type buys is `terminus replay`: one build can score the whole
/// logged history under a variant of these numbers and diff the labels it would
/// have produced, so a retrieval change is tested against history rather than
/// rebuilt and eyeballed. The values a decision was actually taken under travel
/// in the record ([`Thresholds::recorded`]), so a later analysis reads them off
/// the row rather than off whatever the constants say by then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// How many ranked hits the threshold is applied over - [`RANKED`].
    pub ranked: usize,
    /// The wider window the one prompt after a compaction looks through -
    /// [`COMPACTED_RANKED`].
    pub compacted_ranked: usize,
    /// The rank within which one matched entity is enough - [`ENTITY_RANK`].
    pub entity_rank: usize,
    /// How many distinct entities make a hit eligible at any rank -
    /// [`CO_OCCURRING`].
    pub co_occurring: usize,
    /// The most turns one prompt may ever be given - [`MAX_TURNS`].
    pub max_turns: usize,
    /// How many spellings one prompt may ask the archive about -
    /// [`MAX_CANDIDATES`].
    pub max_candidates: usize,
}

impl Default for Thresholds {
    /// The compiled-in values, and the only ones the live path ever uses.
    fn default() -> Self {
        Thresholds {
            ranked: RANKED,
            compacted_ranked: COMPACTED_RANKED,
            entity_rank: ENTITY_RANK,
            co_occurring: CO_OCCURRING,
            max_turns: MAX_TURNS,
            max_candidates: MAX_CANDIDATES,
        }
    }
}

impl Thresholds {
    /// These values as a decision records them, with the character budget that
    /// was actually applied.
    ///
    /// Off the value in force rather than off the constants, which is the whole
    /// reason the type exists: a record that re-read [`RANKED`] would claim the
    /// build's default even on a run that used something else, and every
    /// comparison against it afterwards would be wrong about which numbers
    /// produced which label.
    pub fn recorded(&self, prompt_chars: usize) -> decision::Thresholds {
        decision::Thresholds {
            ranked: self.ranked,
            compacted_ranked: self.compacted_ranked,
            entity_rank: self.entity_rank,
            co_occurring: self.co_occurring,
            max_turns: self.max_turns,
            max_candidates: self.max_candidates,
            prompt_chars,
        }
    }
}

/// The turns one prompt fired on, and what reading them cost.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Fired {
    /// At most [`MAX_TURNS`], in the ranked order. Empty is the ordinary
    /// answer.
    pub hits: Vec<Hit>,
    /// Block rows fetched and blocks decompressed to produce them. An
    /// instrument, not an answer: a prompt that fires on nothing must read
    /// none, and that is a claim about a number.
    pub reads: excerpt::Reads,
}

/// Render the injection for one `UserPromptSubmit`, or nothing.
///
/// Nothing is the ordinary answer and the one this arm exists to protect: the
/// hook then writes nothing at all and exits 0, which is what every prompt that
/// names something the archive has not seen gets.
///
/// **Every call leaves a decision record, unless the project is excluded.**
/// Including the ones that inject nothing, and including the ones that return
/// before the store is opened at all: non-fires are where the miss data lives,
/// and a precision computed only over the prompts that fired has no
/// denominator. A record holds the whole prompt, so an excluded project gets
/// none: exclusion means never read, and that covers the prompt too. The record is
/// completed HERE rather than inside [`selected`] because what one injection
/// cost is known only after [`render`] has clipped it, and it is written after
/// the state save, on the same thread the hook abandons - a write that fails
/// changes nothing about what is emitted. [`selected`] renders too, because
/// what the state remembers as injected is what the render showed.
pub fn user_prompt_submit(data_dir: &Path, config: &Config, payload: &Payload) -> Option<String> {
    if excluded(config, payload) {
        return None;
    }
    let mut decision = Decision::opened(
        payload.session_id,
        payload.cwd,
        payload.prompt.unwrap_or_default(),
    );
    let budget = budget(config);
    // The compiled-in values, constructed here and configurable nowhere.
    let thresholds = Thresholds::default();
    decision.thresholds = thresholds.recorded(budget);

    let text =
        selected(data_dir, config, payload, &thresholds, &mut decision).and_then(|(_, text)| text);

    decision.save(data_dir);
    text
}

/// Is this prompt in a project the config excludes?
///
/// Both coordinates, because they can disagree. The payload's `cwd` follows
/// Claude through a `cd`, while the transcript stays in the project directory
/// the session started in, which is what ingest goes by. So a session that
/// started in an excluded project and moved out of it still counts.
///
/// The transcript's folder is what gets resolved, because a session's first
/// prompt can arrive before its transcript exists. A folder that does not exist
/// yet says nothing, and `cwd` has already answered for that session.
fn excluded(config: &Config, payload: &Payload) -> bool {
    if config.exclusions().is_empty() {
        return false;
    }
    if payload
        .cwd
        .is_some_and(|cwd| config.excludes_path(Path::new(cwd)))
    {
        return true;
    }
    let Some(transcript) = payload.transcript_path.map(Path::new) else {
        return false;
    };
    let (Some(folder), Some(name)) = (transcript.parent(), transcript.file_name()) else {
        return false;
    };
    folder.canonicalize().is_ok_and(|folder| {
        crate::discover::excluded_project_of(config, &folder.join(name)).is_some()
    })
}

/// The character budget one injection actually gets: what `terminus.toml`
/// configures, clamped to the hard ceiling.
///
/// One clamp site, because the number is now reported as well as applied: a
/// decision record claiming a budget the render did not use would make every
/// replayed comparison against it wrong.
fn budget(config: &Config) -> usize {
    config.prompt_chars().min(MAX_PROMPT_CHARS)
}

/// Whether this arm's budget can hold the frame at all. The live arm and
/// `terminus replay` both ask here, so a budget too small for the frame injects
/// nothing in either.
pub(crate) fn frame_fits(config: &Config) -> bool {
    super::frame_fits(DESCRIPTION, budget(config))
}

/// The ceiling on one injection, whatever `terminus.toml` configures.
///
/// Bundle 2.1.237 persists a hook stdout longer than 10,000 characters to disk
/// and hands the model a reference to the file instead of the text - so past
/// that, injected context stops being context and becomes a path. The same
/// number bounds the brief, for the same reason and from the same measurement.
pub const MAX_PROMPT_CHARS: usize = 10_000;

/// The turns that fired, as the text the hook emits, inside `budget`.
///
/// Deterministic: the ranked order, stored timestamps at day resolution, no
/// clock read anywhere. Two runs of one prompt against an unchanged store
/// render the same bytes, which is the same property the resume brief keeps
/// and is worth as much here - an injection that changes while the archive does
/// not is one nobody can reason about.
/// `budget` is the effective one - see [`budget`] - and is not clamped again
/// here.
///
/// It also completes `decision`: the per-turn shares and the whole injection's
/// character count are produced by exactly this arithmetic and nowhere else, so
/// counting them anywhere but here would be a second, drifting account of what a
/// prompt spent.
fn render(hits: &[Hit], budget: usize, decision: &mut Decision) -> Option<String> {
    // The frame is a fixed cost and comes off first.
    let room = budget.saturating_sub(super::frame_cost(DESCRIPTION));

    // Every line shown is whole. When the ids, the dates and the marks alone
    // are over what the frame leaves, the last turn is left out and the rest
    // share again, down to none. Cutting the lines instead, which `frame`
    // would, cuts the last turn's id (#94).
    let (hits, texts, marked) = (1..=hits.len())
        .rev()
        .find_map(|kept| fitted(&hits[..kept], room))?;

    // `frame` cuts the lines and never the frame, and here the lines fit, so it
    // has nothing to cut. No frame, no injection, and nothing recorded as given.
    let text = super::frame(DESCRIPTION, &assemble(hits, &texts), budget)?;

    // Per turn, the text that turn contributed - not its share of the total.
    // The frame and the ids are the injection's and belong to no turn. A turn
    // shown as the mark alone, or left out, is not here: the model was not
    // shown it, and a later prompt must still be free to quote it.
    decision.injected = hits
        .iter()
        .zip(&texts)
        .zip(&marked)
        .filter(|(_, cut)| !**cut)
        .map(|((hit, text), _)| Injected {
            turn_id: hit.turn_id,
            chars: super::chars(text),
        })
        .collect();
    // The whole emitted string, frame included: what the model was actually
    // given, which the per-turn shares do not sum to.
    decision.chars_injected = super::chars(&text);
    Some(text)
}

/// `hits` with each turn's text and whether it is the mark alone, or `None`
/// when their lines cannot all be whole inside `room`.
fn fitted(hits: &[Hit], room: usize) -> Option<(&[Hit], Vec<String>, Vec<bool>)> {
    // What the lines cost with no text in them: the ids and the dates.
    // Measured rather than estimated, so the share below is what is actually
    // left.
    let empty: Vec<String> = hits.iter().map(|_| String::new()).collect();
    let bare = super::chars(&assemble(hits, &empty));

    // A share smaller than the text and no larger than the mark cannot hold a
    // marked cut: `clip` would hand back a few raw characters, and a share of
    // nothing would drop the turn. Either way the turn is shown as the mark,
    // the rule the brief follows (#94). The mark is a fixed cost of its line,
    // so each turn marked takes three characters off what the others share, and
    // that can mark another: the loop runs until no turn is added. It ends,
    // because a turn is marked at most once.
    let excerpts: Vec<&str> = hits.iter().map(|hit| hit.excerpt.trim()).collect();
    let mark = super::chars(excerpt::ELISION);
    let mut marked = vec![false; hits.len()];
    let share = loop {
        let cut = marked.iter().filter(|cut| **cut).count();
        let open = hits.len() - cut;
        if open == 0 {
            break 0;
        }
        let share = room.saturating_sub(bare + cut * mark) / open;
        if share > mark {
            break share;
        }
        for (cut, text) in marked.iter_mut().zip(&excerpts) {
            *cut = *cut || super::chars(text) > share;
        }
        if marked.iter().filter(|cut| **cut).count() == cut {
            break share;
        }
    };

    // Each open text is within the share, so only the ids, the dates and the
    // marks can be over.
    if bare + marked.iter().filter(|cut| **cut).count() * mark > room {
        return None;
    }

    let texts: Vec<String> = excerpts
        .iter()
        .zip(&marked)
        .map(|(text, cut)| {
            if *cut {
                excerpt::ELISION.to_owned()
            } else {
                super::clip(text, share)
            }
        })
        .collect();
    Some((hits, texts, marked))
}

/// What this arm says the block holds, ahead of [`super::QUOTED_HISTORY`] on
/// the header line.
const DESCRIPTION: &str =
    "Terminus recall - past turns from this project that match what was just asked.";

/// One line per turn, which is what goes between the tags.
///
/// The turn id is first on each line and unmissable, because it is the argument
/// `recall_get` and `terminus show` take: the model that wants the whole turn
/// rather than the sentence has to be able to ask for it. The day is archived
/// text like the excerpt and is neutralized like it.
fn assemble(hits: &[Hit], texts: &[String]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (hit, text) in hits.iter().zip(texts) {
        let mut line = format!("- turn {}", hit.turn_id);
        if let Some(day) = hit.ts.as_deref().map(day) {
            line.push_str(&format!(" ({})", super::neutralize(day)));
        }
        line.push_str(": ");
        line.push_str(text);
        lines.push(line);
    }
    lines.join("\n")
}

/// The day of a stored timestamp, at day resolution.
///
/// `get` and not `&ts[..10]`, for the reason `cmd::search::day` states: `len`
/// counts bytes, the slice needs a char boundary, and a stored `ts` is any JSON
/// string the transcript carried. A timestamp with a multi-byte character
/// across byte 10 panicked a whole command once already, and a panic here is a
/// prompt that emits nothing.
fn day(ts: &str) -> &str {
    ts.get(..10).unwrap_or(ts)
}

/// The turns one prompt fires on: the whole of the firing decision.
///
/// Every failure is an empty [`Fired`] rather than an error, for the reason the
/// module doc on [`super`] gives: a missing, unreadable, busy or outdated store
/// is "no context this time" and not something a prompt can be told about.
/// The record is filled and then dropped: this seam answers "what would fire",
/// and only [`user_prompt_submit`] - the arm the hook actually calls - persists
/// what was decided. One code path either way, so what a test observes here is
/// what a prompt logs there: it renders at the config's budget too, because the
/// turns the session remembers as injected are the ones the render showed, and
/// the state this leaves is the state the hook leaves.
pub fn select(data_dir: &Path, config: &Config, payload: &Payload) -> Fired {
    let mut decision = Decision::opened(
        payload.session_id,
        payload.cwd,
        payload.prompt.unwrap_or_default(),
    );
    selected(
        data_dir,
        config,
        payload,
        &Thresholds::default(),
        &mut decision,
    )
    .map(|(fired, _)| fired)
    .unwrap_or_default()
}

/// [`select`]'s body, written in the `?` its every step wants.
///
/// `decision` is filled as the decision is taken rather than reconstructed
/// afterwards: every `?` below is an exit that injected nothing, and what makes
/// those exits distinguishable in the log is how much of the record was filled
/// before one of them fired.
///
/// The text is what [`render`] made of the turns, or nothing when they did not
/// fit the frame. It is rendered here, ahead of the state save, because a turn
/// is remembered as injected only when the render showed it with text.
fn selected(
    data_dir: &Path,
    config: &Config,
    payload: &Payload,
    thresholds: &Thresholds,
    decision: &mut Decision,
) -> Option<(Fired, Option<String>)> {
    let prompt = payload.prompt.filter(|prompt| !prompt.trim().is_empty())?;
    let cwd = payload.cwd.filter(|cwd| !cwd.is_empty())?;

    // Before the store is opened, because a prompt naming nothing the archive
    // could match structurally is the common case, and the cheapest thing this
    // arm can do with it is nothing at all.
    let (spellings, candidates) = candidates(prompt, Some(cwd), thresholds.max_candidates);
    decision.spellings = spellings;
    if candidates.is_empty() {
        return None;
    }

    let store = super::open(data_dir)?;
    let conn = store.conn();
    // The bound a replay needs to score this prompt against the index as
    // it stood, taken on the connection that is already open. A record whose
    // prompt never reached here keeps a null watermark for the drain to stamp.
    decision.watermark_session_no = watermark(conn);

    let mut state = State::load(data_dir, payload.session_id);
    let before = state.clone();
    let current = current_session(payload);
    // What a compaction owes this session, or nothing. Derived before the
    // search because it is what decides how wide the search is.
    let dropped = owed(conn, &mut state, current.as_deref());
    if let Some(dropped) = &dropped {
        // The one prompt per compaction that asks the compacted pool. Recorded
        // because it changes both the window and the pool, so a decision taken
        // under it is not comparable to the ordinary ones.
        decision.compacted = true;
        decision.dropped = dropped.iter().copied().collect();
    }

    let request = Request::new(query_of(prompt, Some(cwd)), Scope::Directory(cwd.into()))
        .limit(if dropped.is_some() {
            thresholds.compacted_ranked
        } else {
            thresholds.ranked
        })
        .candidates(candidates)
        .excerpts(false);
    // The scope is the payload's `cwd` and `search::run` resolves it,
    // which is also where an excluded project becomes an empty result.
    let response = match search::run(conn, config, &request) {
        Ok(response) => response.hits,
        // Not a `?`: the flag may have just been consumed, and losing that
        // write would offer the same compaction's turns again on the next
        // prompt.
        Err(_) => Vec::new(),
    };

    // Every hit the ranked window returned, before the threshold judges any of
    // them: a decision that logged only what fired could never say what was
    // there to fire on, which is the whole of what a replay re-scores.
    decision.candidates = response.iter().map(scored).collect();

    let threshold = eligible(response, thresholds);
    // Tool results out, after the threshold so no rank is renumbered and before
    // the cap so a tool result never spends a slot.
    let quoted = quotable(threshold);
    let pooled = within(quoted, dropped.as_ref());
    let mut hits = surviving(
        &mut state,
        pooled,
        current.as_deref(),
        dropped.as_ref(),
        &mut decision.suppressed,
        thresholds.max_turns,
    );

    // Now, and only now, is a session worth decompressing - and only for the
    // turns that survived both the threshold and the suppressions, since a
    // suppressed turn is not injected and its blob is not read. Nor is one
    // when the budget cannot hold the frame: [`render`] would emit
    // nothing, and a turn the model was never shown must not be remembered as
    // injected, the rule `brief::quoted` follows too.
    let fired = if hits.is_empty() || !frame_fits(config) {
        None
    } else {
        // The knob governs this surface too: the per-prompt injection is one of
        // the surfaces `[privacy] redact_recall` covers. The decision is
        // resolved at this entry point, which already holds the `&Config`, and
        // never inside `excerpt::of_record`/`attach`, which read no config at
        // all. Filtering it is not optional. What this path writes goes onto
        // hook stdout as the model's next context, which is exactly the
        // destination `egress::for_model_context` exists for; leaving it raw
        // would mean a user who set `[privacy] redact_recall` still has every
        // secret in a matched turn copied verbatim into the model on the next
        // prompt, with the four filtered surfaces making it look as though the
        // knob held. The other half is upstream: this path and
        // `feedback::replay` both build their `Request` with
        // `.excerpts(false)`, so `search::run` never attaches for either of
        // them and this call is the only one that can. With the knob off
        // `Redaction::of` is `Redaction::none`, so the default path is
        // byte-identical to what it was and still borrows. The closing-tag
        // neutralizer rides in redaction's position, and only on this call:
        // search, context and get render unchanged.
        match excerpt::attach_scrubbed(
            conn,
            &request.query,
            &mut hits,
            &Redaction::of(config),
            super::neutralize,
        ) {
            Ok(reads) => {
                let text = render(&hits, budget(config), decision);
                // What the render showed with text and nothing else: a turn
                // shown as the mark alone is not in `decision.injected`, and a
                // later prompt in this session can still quote it (#94).
                for shown in &decision.injected {
                    state.record_injected(shown.turn_id);
                }
                Some((Fired { hits, reads }, text))
            }
            // The excerpt is what makes an injection worth reading, so a blob
            // that will not open is silence rather than a line of turn ids -
            // and nothing was injected, so nothing is remembered as injected.
            Err(_) => None,
        }
    };

    // After the outcome is known and whatever it was: the prompt that injected
    // nothing because everything was suppressed is exactly the one a person
    // reads this file for. Written only when this prompt changed it, so a
    // session whose every prompt fires on nothing acquires no file at all.
    if state != before {
        state.save(data_dir, payload.session_id);
    }
    fired
}

/// One ranked hit, as a decision records it.
///
/// Everything the threshold reads and nothing it does not: the excerpt is
/// deliberately absent, because the search runs with excerpts off until
/// something has fired and a record carrying transcript text would be a
/// second copy of the archive in the data directory.
fn scored(hit: &Hit) -> Candidate {
    Candidate {
        turn_id: hit.turn_id,
        relevance: hit.relevance,
        entity_score: hit.entity_score,
        entity_count: hit.entity_count,
        matched_on: hit.matched_on.clone(),
    }
}

/// The highest `sessions.session_no` this store holds, or `None`.
///
/// A monotone bound. `session_no` is assigned in ingest order and survives
/// a rebuild, which is what makes it answer "was this row indexed yet" -
/// `turns.ts` is transcript time and cannot. `None` for an empty archive and for
/// a query that failed: a bound nothing can state is better left for the drain
/// to stamp than guessed at.
fn watermark(conn: &Connection) -> Option<i64> {
    conn.query_row("SELECT max(session_no) FROM sessions", [], |r| r.get(0))
        .ok()
        .flatten()
}

/// The firing threshold, applied over the ranked order.
///
/// The rank is the position in that order and nothing else - `recall::search`'s
/// order is total on purpose, so two runs over an unchanged store agree about
/// which turns were in the top three.
///
/// **The cap is not applied here.** The suppressions run between the two
/// (see [`surviving`]), so that a turn this session has already been given does
/// not consume one of the three slots on its way to being refused.
///
/// Public because `terminus replay` applies exactly this judgement to the hits
/// a logged prompt would get back today under a different [`Thresholds`]: two
/// definitions of "eligible" would make a replayed diff a statement about the
/// replay engine rather than about the change under test.
pub fn eligible(hits: Vec<Hit>, thresholds: &Thresholds) -> Vec<Hit> {
    hits.into_iter()
        .enumerate()
        .filter(|(rank, hit)| {
            // A free-text-only hit is never eligible, whatever its relevance.
            hit.entity_match.is_some()
                && (*rank < thresholds.entity_rank || hit.entity_count >= thresholds.co_occurring)
        })
        .map(|(_, hit)| hit)
        .collect()
}

/// `hits` without the tool results, in the ranked order.
///
/// A tool result is a `user` row with `turns.is_typed` = 0. Not keyed on
/// [`Hit::record_type`], which is `user` for a typed prompt too, nor on the
/// `toolUseResult` key, which some result records do not carry. A null
/// `is_typed` is left alone: nothing derived it, so nothing can say it was a
/// tool result.
///
/// Read off [`Hit::is_typed`], which the search projects with every hit, so
/// this runs no query and cannot fail.
///
/// Public for the reason [`eligible`] is: `terminus replay` must drop exactly
/// the turns the hook drops.
pub fn quotable(hits: Vec<Hit>) -> Vec<Hit> {
    hits.into_iter()
        .filter(|hit| !(hit.record_type == "user" && hit.is_typed == Some(false)))
        .collect()
}

/// The eligible turns that are actually injected, once `refused` is taken out
/// and the cap applied.
///
/// The live path reaches the same answer through [`surviving`], which derives
/// its refusals from the session's [`State`] file as it goes. `terminus replay`
/// cannot: those files are one session's disposable scratch, deleted or stale
/// long before a replay runs, and re-deriving a refusal would attribute
/// unreproducible session state to the rule change under test. So it re-applies
/// the refusals the decision itself recorded and calls this.
///
/// The cap falls AFTER the refusals here for the reason it does there: a turn
/// that was refused must not have spent one of the slots on its way out.
pub fn capped(hits: Vec<Hit>, refused: &BTreeSet<i64>, max_turns: usize) -> Vec<Hit> {
    hits.into_iter()
        .filter(|hit| !refused.contains(&hit.turn_id))
        .take(max_turns)
        .collect()
}

/// What a compaction owes this session, consuming the flag that says so.
///
/// `None` on the ordinary path, which is every prompt but one per compaction:
/// no flag, no query, no cost. `None` too when the flag is set and the answer
/// cannot be had yet - a payload naming no transcript this archive knows, or a
/// boundary row the ingest that is racing this prompt has not committed. Those
/// leave the flag SET, which is the whole of why it is a flag: the debt carries
/// to the following prompt instead of being lost to the race.
///
/// `Some` means a boundary row was there and the compaction is accounted for,
/// so the flag clears - even when the set is empty, and even if nothing that
/// follows passes the threshold. A compaction is one prompt's worth of debt,
/// not a standing mode the session never leaves.
fn owed(conn: &Connection, state: &mut State, current: Option<&str>) -> Option<BTreeSet<i64>> {
    if !state.compaction_owed {
        return None;
    }
    let dropped = compaction::dropped(conn, current?)?;
    state.compaction_owed = false;
    Some(dropped)
}

/// The turns of `hits` the compaction pool admits.
///
/// Everything, on the ordinary path. On the one prompt after a compaction, only
/// what fell out of the model's context - and applied AFTER the threshold, on
/// purpose: filtering first would renumber the ranks, and a turn that placed
/// twentieth overall would arrive at the threshold looking like a rank-1 hit.
/// One definition of relevance for both paths, and "never on a
/// free-text-only match" is not conditioned on a compaction having happened.
///
/// Public for the reason [`eligible`] is: `terminus replay` filters a
/// compacted decision's hits to its stored dropped set through this, rather
/// than through a second copy that could drift from what the prompt did.
pub fn within(hits: Vec<Hit>, dropped: Option<&BTreeSet<i64>>) -> Vec<Hit> {
    let Some(dropped) = dropped else {
        return hits;
    };
    hits.into_iter()
        .filter(|hit| dropped.contains(&hit.turn_id))
        .collect()
}

/// The turns that pass the suppressions, capped at [`MAX_TURNS`], recording
/// every refusal in `state`.
///
/// The reasons are checked in the order `state::Reason` declares them and the
/// first one that answers is the one recorded: they can overlap - the brief's
/// own session can be the one the user is looking at after a resume - and a
/// turn refused twice for two reasons would say no more than a turn refused
/// once. `recorded` collects THIS prompt's refusals for the decision record.
/// The session state file keeps its own list and is not the same thing: that
/// one is cumulative across the session and capped at a hundred, so reading it
/// back would attribute another prompt's refusals to this one and silently drop
/// the oldest.
fn surviving(
    state: &mut State,
    hits: Vec<Hit>,
    current: Option<&str>,
    dropped: Option<&BTreeSet<i64>>,
    recorded: &mut Vec<Suppressed>,
    max_turns: usize,
) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for hit in hits {
        match refusal(state, &hit, current, dropped) {
            Some(reason) => {
                state.record_suppressed(hit.turn_id, reason);
                recorded.push(Suppressed {
                    turn_id: hit.turn_id,
                    reason,
                });
            }
            None => out.push(hit),
        }
        if out.len() == max_turns {
            break;
        }
    }
    out
}

/// Why this turn may not be injected, or `None` if it may.
///
/// "Visible in this session" means in this session AND not known to have fallen
/// out of the model's context. Without that second half the suppressions would
/// refuse the whole compacted pool: every turn a compaction dropped belongs to
/// the session the user is looking at, which is exactly why it is worth
/// offering back. On the ordinary path no dropped set is derived - that is a
/// query per prompt for a fact almost every prompt has no use for - so nothing
/// is exempt there.
fn refusal(
    state: &State,
    hit: &Hit,
    current: Option<&str>,
    dropped: Option<&BTreeSet<i64>>,
) -> Option<Reason> {
    if state.was_injected(hit.turn_id) {
        return Some(Reason::AlreadyInjected);
    }
    let fell_out = dropped.is_some_and(|dropped| dropped.contains(&hit.turn_id));
    if current == Some(hit.session_key.as_str()) && !fell_out {
        return Some(Reason::VisibleInSession);
    }
    if state.was_briefed(hit.turn_id) {
        return Some(Reason::CarriedByBrief);
    }
    None
}

/// The `session_key` of the session the user is looking at, or nothing.
///
/// The archive's own key for the payload's `transcript_path`: canonicalized,
/// then through `ingest::path_key`, the one function ingest keys a session
/// with. Keyed any other way, the compaction lookup finds no boundary and the
/// live session's own turns are injected back into it.
///
/// `None` for a payload with no `transcript_path`, and for a path that will not
/// canonicalize - a session whose file Claude Code has not created yet, or one
/// on a filesystem that has stopped answering. Both suppress nothing, which is
/// one turn possibly repeated rather than a prompt that fails.
///
/// **The live transcript is never read.** The question is which session a
/// candidate belongs to, not what is in it, and reading the file would cost the
/// measured p90 of 1.03 MB per prompt to learn something a string comparison
/// already answers. The archive lags the file by whatever the last ingest
/// missed, so a turn written seconds ago may not be keyed yet: that is the
/// small duplication window at the head of a session, and it is accepted.
fn current_session(payload: &Payload) -> Option<String> {
    let canonical = Path::new(payload.transcript_path?).canonicalize().ok()?;
    crate::ingest::path_key(&canonical).ok()
}

/// The spellings this prompt asks the archive about, strongest first, each with
/// the query it became.
///
/// A path counts twice: as the absolute spelling the archive almost certainly
/// stored and as the relative one the user typed, because the two are
/// different token sets and the second is what a turn quoting the short form
/// carries. An identifier-shaped token is the hierarchy's third signal and
/// is judged by exactly the rule the extractor judges a `Grep` pattern by, so a
/// prompt cannot ask about `the` or `should`.
///
/// The spellings travel beside the queries because the decision record logs
/// them: a `Query` keeps only its tokens, and "what this prompt was asked
/// about" reads as the text the extraction produced rather than as a token
/// list.
///
/// Public because `terminus replay` re-runs exactly this extraction over a
/// stored prompt and `cwd`. `max_candidates` is [`Thresholds::max_candidates`],
/// which is [`MAX_CANDIDATES`] on the live path and whatever a replay was asked
/// for on that one.
pub fn candidates(
    prompt: &str,
    cwd: Option<&str>,
    max_candidates: usize,
) -> (Vec<String>, Vec<Query>) {
    let mut spellings: Vec<String> = Vec::new();
    for relative in relative_paths(prompt) {
        if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
            spellings.push(join(cwd, &relative));
        }
        spellings.push(relative);
    }
    for absolute in absolute_paths(prompt) {
        spellings.push(absolute);
    }
    for token in entity::identifier_tokens(prompt) {
        if entity::is_identifier_shaped(token) {
            spellings.push(token.to_owned());
        }
    }

    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut asked: Vec<String> = Vec::new();
    let mut out: Vec<Query> = Vec::new();
    for spelling in spellings {
        let query = Query::parse(&spelling);
        if query.is_empty() {
            continue;
        }
        // Deduplicated on the TOKENS and not on the spelling: two spellings of
        // one path differing in a separator are one question to the index, and
        // asking it twice costs a disjunct and returns the same rows.
        if seen.insert(query.tokens().to_vec()) {
            asked.push(spelling);
            out.push(query);
        }
        if out.len() == max_candidates {
            break;
        }
    }
    (asked, out)
}

/// The query one prompt becomes, with every relative path in it resolved
/// against the payload's `cwd`.
///
/// **Why the resolution has to happen at all.** A stored `path` entity is
/// almost always absolute - 1,029 absolute against 2 relative over 120 sampled
/// real transcripts - while a user types the relative spelling, and
/// [`Query::matches_entity`] requires EVERY token of the stored value to be
/// present in the query. So `/home/u/proj/crates/x.rs` cannot be matched by a
/// query of `crates/x.rs` however obviously the two name one file, and a prompt
/// naming a file by its relative path would meet a permanent silence that every
/// test written against an absolute-path fixture still passes.
///
/// **The resolved spelling is added, never substituted.** The prose is still
/// there, because the query is also what weighs and excerpts the turns that
/// come back, and because the user's own spelling is what a turn quoting the
/// relative path carries.
///
/// **The resolved spellings go FIRST.** [`crate::recall::MAX_QUERY_TOKENS`]
/// drops the thirty-third distinct token onward, and a pasted stack trace ahead
/// of the path would otherwise drop exactly the tokens this exists to add.
///
/// **No filesystem is touched.** 52% of the real corpus's `cwd` directories no
/// longer exist, so a path canonicalized today would key differently tomorrow
/// and one file would key two ways across one archive. This is string joining,
/// and `..` is left in place: token containment does not care, since the tokens
/// of the true path are a subset of the tokens of the joined one either way.
pub fn query_of(prompt: &str, cwd: Option<&str>) -> Query {
    Query::parse(&resolved(prompt, cwd))
}

/// The prompt with the resolved spellings prepended, or the prompt itself.
///
/// A payload with no `cwd` asks what the user typed: there is nothing to
/// resolve against, and inventing a base would be inventing a file.
fn resolved(prompt: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return prompt.to_owned();
    };

    let mut out = String::new();
    for relative in relative_paths(prompt) {
        out.push_str(&join(cwd, &relative));
        out.push(' ');
    }
    if out.is_empty() {
        return prompt.to_owned();
    }
    out.push_str(prompt);
    out
}

/// The path-shaped words of a prompt, in the order they were typed and without
/// repeats.
///
/// Path-shaped means it survives [`normalize_path`] - which trims the quotes
/// and the trailing `:line:col` a user pastes - and carries a separator. A bare
/// word is as likely to be a subcommand as a file, which is the same judgement
/// `entity::path_words` makes about a shell command line.
fn path_words(prompt: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in prompt.split_whitespace() {
        // A URL is not a file, and joining one onto a working directory
        // produces a spelling that names nothing at all.
        if word.contains("://") {
            continue;
        }
        let Some(path) = normalize_path(word) else {
            continue;
        };
        if !path.contains('/') && !path.contains('\\') {
            continue;
        }
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// The path-shaped words that are not already absolute: the ones a `cwd` says
/// something about.
fn relative_paths(prompt: &str) -> Vec<String> {
    path_words(prompt)
        .into_iter()
        .filter(|path| !Path::new(path).is_absolute())
        .collect()
}

/// The path-shaped words that already name a file outright.
fn absolute_paths(prompt: &str) -> Vec<String> {
    path_words(prompt)
        .into_iter()
        .filter(|path| Path::new(path).is_absolute())
        .collect()
}

/// `cwd` and `relative`, joined with one separator between them.
///
/// The separator is a forward slash whatever the platform: the value is a query
/// string and never a path this process opens, and the tokenizer splits on both
/// separators anyway (`index::expand::separator_components`), so the spelling
/// changes nothing about what matches.
fn join(cwd: &str, relative: &str) -> String {
    let base = cwd.trim_end_matches(['/', '\\']);
    let relative = relative.trim_start_matches("./").trim_start_matches(".\\");
    format!("{base}/{relative}")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::recall::EntityMatch;

    /// One ranked hit that matched a stored entity.
    ///
    /// Everything [`eligible`] reads and nothing it does not: a hit's rank is
    /// its index in the slice, so what decides are `entity_match` and
    /// `entity_count`.
    fn hit(turn_id: i64, entity_count: usize) -> Hit {
        Hit {
            turn_id,
            session_key: "/s.jsonl".into(),
            record_type: "assistant".into(),
            ts: Some("2026-08-13T09:00:00.000Z".into()),
            project: Some("/code/verbatim".into()),
            sidechain: false,
            entity_score: 1.0,
            entity_match: Some(EntityMatch::Exact),
            entity_count,
            matched_on: Vec::new(),
            relevance: 1.0,
            excerpt: String::new(),
            is_typed: None,
        }
    }

    fn ids(hits: &[Hit]) -> Vec<i64> {
        hits.iter().map(|hit| hit.turn_id).collect()
    }

    /// A hit of `record_type` whose `is_typed` is `is_typed`.
    fn typed(turn_id: i64, record_type: &str, is_typed: Option<bool>) -> Hit {
        Hit {
            record_type: record_type.into(),
            is_typed,
            ..hit(turn_id, 1)
        }
    }

    /// Only a `user` hit the harness wrote is dropped. A `user`
    /// hit whose `is_typed` is null is kept, because nothing can say it was a
    /// tool result, and so is a typed `user` hit and every non-`user` hit. The
    /// ranked order survives the filter.
    #[test]
    fn quotable_drops_only_a_user_hit_the_harness_wrote() {
        let hits = vec![
            typed(1, "user", None),
            typed(2, "user", Some(false)),
            typed(3, "user", Some(true)),
            typed(4, "assistant", Some(false)),
            typed(5, "assistant", None),
        ];
        assert_eq!(ids(&quotable(hits)), vec![1, 3, 4, 5]);
    }

    /// The point of a `Thresholds` value: the same ranked input fires
    /// differently under an overridden rank threshold, and exactly as before
    /// under the default.
    ///
    /// The default arm is the load-bearing half. It is what says the live path
    /// did not move when the six constants became a value type, and it fails the
    /// moment [`Thresholds::default`] stops being the compiled-in numbers.
    #[test]
    fn a_widened_rank_threshold_admits_a_hit_the_default_refuses() {
        let ranked: Vec<Hit> = (1..=5).map(|n| hit(n, 1)).collect();

        // Ranks 4 and 5 are out: one matched entity is enough only inside the
        // top three, and one entity is fewer than CO_OCCURRING.
        assert_eq!(
            ids(&eligible(ranked.clone(), &Thresholds::default())),
            vec![1, 2, 3]
        );

        let widened = Thresholds {
            entity_rank: 5,
            ..Thresholds::default()
        };
        assert_eq!(
            ids(&eligible(ranked.clone(), &widened)),
            vec![1, 2, 3, 4, 5]
        );

        // The other condition is not a subset of the rank one: two co-occurring
        // entities admit a hit wherever it sits, which is why rank 4 here is
        // eligible under the DEFAULT.
        let mut corroborated = ranked.clone();
        corroborated[3].entity_count = 2;
        assert_eq!(
            ids(&eligible(corroborated, &Thresholds::default())),
            vec![1, 2, 3, 4]
        );

        // A free-text-only hit is never eligible, whatever the thresholds say.
        let mut prose = ranked;
        prose[0].entity_match = None;
        assert_eq!(ids(&eligible(prose, &widened)), vec![2, 3, 4, 5]);
    }

    /// The cap falls after the refusals, so a refused turn does not spend a
    /// slot - the property [`surviving`] holds on the live path.
    #[test]
    fn a_refused_turn_does_not_spend_one_of_the_slots() {
        let admitted: Vec<Hit> = (1..=5).map(|n| hit(n, 1)).collect();
        let refused: BTreeSet<i64> = [1, 2].into_iter().collect();

        assert_eq!(
            ids(&capped(admitted.clone(), &refused, MAX_TURNS)),
            vec![3, 4, 5]
        );
        assert_eq!(
            ids(&capped(admitted, &BTreeSet::new(), MAX_TURNS)),
            vec![1, 2, 3]
        );
    }

    /// One hit whose excerpt is `text`.
    fn said(turn_id: i64, text: &str) -> Hit {
        Hit {
            excerpt: text.into(),
            ..hit(turn_id, 1)
        }
    }

    /// The budget at which [`render`] leaves `spare` characters for the texts:
    /// the frame, then the lines with nothing in them, then `spare`. Built from
    /// the pieces `render` measures, so a share is a number a test can choose.
    fn budget_for(hits: &[Hit], spare: usize) -> usize {
        let empty: Vec<String> = hits.iter().map(|_| String::new()).collect();
        super::super::frame_cost(DESCRIPTION) + super::super::chars(&assemble(hits, &empty)) + spare
    }

    /// What `render` emits at `budget`, and the text it put on each turn's line.
    fn rendered(hits: &[Hit], budget: usize) -> (String, Vec<String>, Decision) {
        let mut decision = Decision::opened(None, None, "");
        let text = render(hits, budget, &mut decision).expect("the frame fits");
        let lines = text
            .lines()
            .filter(|line| line.starts_with("- turn "))
            .map(|line| {
                line.split_once(": ")
                    .map_or("", |(_, text)| text)
                    .to_owned()
            })
            .collect();
        (text, lines, decision)
    }

    /// A longer excerpt with a share of 3 or less is the mark alone (#94). The
    /// share here is just what is spared per turn, so the mark fits it exactly.
    #[test]
    fn a_cut_the_share_cannot_hold_is_shown_as_the_mark() {
        let hits = [said(1, "abcdefgh")];
        let (_, lines, decision) = rendered(&hits, budget_for(&hits, 3));
        assert_eq!(lines, vec!["..."]);
        assert!(decision.injected.is_empty(), "a mark alone was recorded");

        // Less than the mark to spare leaves a lone turn no whole line, so it
        // is left out, and there is nothing to inject.
        for spare in 0..3 {
            let mut decision = Decision::opened(None, None, "");
            let text = render(&hits, budget_for(&hits, spare), &mut decision);
            assert_eq!(text, None, "spare {spare}");
            assert!(decision.injected.is_empty(), "spare {spare}");
        }

        // Several turns, each sharing 3.
        let hits = [
            said(1, "abcdefgh"),
            said(2, "ijklmnop"),
            said(3, "qrstuvwx"),
        ];
        let (_, lines, decision) = rendered(&hits, budget_for(&hits, 9));
        assert_eq!(lines, vec!["...", "...", "..."]);
        assert!(decision.injected.is_empty());
    }

    /// A share of 0 to 2 beside a turn that needs nothing: the long one is the
    /// mark, the short one is what it is, and the sum fits.
    #[test]
    fn a_mark_takes_its_three_characters_from_the_turns_beside_it() {
        // Spare 4 over two turns is a share of 2; the mark leaves 1 for the
        // empty one.
        let hits = [said(1, "abcdefgh"), said(2, "")];
        let (text, lines, decision) = rendered(&hits, budget_for(&hits, 4));
        assert_eq!(lines, vec!["...", ""]);
        assert_eq!(text.chars().count(), budget_for(&hits, 3));
        assert_eq!(
            decision
                .injected
                .iter()
                .map(|i| i.turn_id)
                .collect::<Vec<_>>(),
            vec![2]
        );

        // A turn that fit the first share and no longer fits what the mark
        // leaves is marked too. Two marks do not fit 4 spare characters, so
        // the last turn is left out and the first has its line to itself.
        let hits = [said(1, "abcdefgh"), said(2, "ab")];
        let (_, lines, _) = rendered(&hits, budget_for(&hits, 5));
        assert_eq!(lines, vec!["...", "ab"]);
        let (_, lines, decision) = rendered(&hits, budget_for(&hits, 4));
        assert_eq!(lines, vec!["abcdefgh"]);
        assert_eq!(
            decision
                .injected
                .iter()
                .map(|i| i.turn_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    /// An excerpt that fits its share whole prints as it is, at every share a
    /// mark would otherwise be wanted, and the empty one is one of them.
    #[test]
    fn an_excerpt_that_fits_its_share_prints_whole() {
        for (text, share) in [("", 0), ("a", 1), ("ab", 2), ("abc", 3), ("abcd", 4)] {
            let hits = [said(1, text)];
            let (_, lines, decision) = rendered(&hits, budget_for(&hits, share));
            assert_eq!(lines, vec![text], "share {share}");
            assert_eq!(decision.injected.len(), 1, "share {share}");
        }
    }

    /// From a share of 4 the cut is `clip`'s marked cut, and the turn is
    /// remembered.
    #[test]
    fn a_share_of_four_is_cut_with_the_mark_as_before() {
        let hits = [said(1, "abcdefgh")];
        let (_, lines, decision) = rendered(&hits, budget_for(&hits, 4));
        assert_eq!(lines, vec!["a..."]);
        assert_eq!(decision.injected.len(), 1);
    }

    /// At every budget from the frame's cost to the whole injection, whatever
    /// the mix of long, short and empty turns, the text is within the budget
    /// and every line in it is whole: the leading turns in order, each with its
    /// id and day, then its text whole or ending in the mark.
    #[test]
    fn the_text_never_exceeds_the_budget_at_any_budget() {
        let mixes = [
            vec![
                said(1, "abcdefghijkl"),
                said(2, "abcdefghijkl"),
                said(3, "abcdefghijkl"),
            ],
            vec![said(1, "abcdefghijkl"), said(2, ""), said(3, "ab")],
            vec![said(1, "abc"), said(2, "abcdefghijkl")],
            vec![said(1, "abcdefghijkl")],
        ];
        for hits in &mixes {
            let frame = super::super::frame_cost(DESCRIPTION);
            let full = budget_for(hits, 12 * hits.len());
            for budget in frame + 1..=full {
                let mut decision = Decision::opened(None, None, "");
                let Some(text) = render(hits, budget, &mut decision) else {
                    assert!(decision.injected.is_empty(), "budget {budget}");
                    continue;
                };
                assert!(
                    text.chars().count() <= budget,
                    "{} characters against a budget of {budget}: {text}",
                    text.chars().count()
                );
                let lines: Vec<&str> = text
                    .lines()
                    .filter(|line| line.starts_with("- turn "))
                    .collect();
                assert!(!lines.is_empty(), "budget {budget}: {text}");
                for (line, hit) in lines.iter().zip(hits) {
                    let said = line.strip_prefix(&format!("- turn {} (2026-08-13): ", hit.turn_id));
                    assert!(
                        said.is_some_and(|said| said == hit.excerpt || said.ends_with("...")),
                        "budget {budget}: {line:?}"
                    );
                }
            }
        }
    }

    /// The record's copy comes off the value in force, never off the constants.
    #[test]
    fn a_record_carries_the_values_that_were_actually_applied() {
        let overridden = Thresholds {
            entity_rank: 5,
            max_candidates: 2,
            ..Thresholds::default()
        };
        let recorded = overridden.recorded(4_000);

        assert_eq!(recorded.entity_rank, 5);
        assert_eq!(recorded.max_candidates, 2);
        assert_eq!(recorded.ranked, RANKED);
        assert_eq!(recorded.prompt_chars, 4_000);
    }
}
