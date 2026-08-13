//! `verbatim sessions`: every session a read path may see (RCL-05).
//!
//! **Through `config::visible::sessions`, not through PLAN-2's projection.**
//! D-21 keeps the hot query paths on a project-only projection because the
//! per-session `count(*)` costs ~7 ms on a real store and search cannot spend
//! that before it starts. This is a listing: the counts ARE what it is for, it
//! runs when a person asks for it, and going through the module every read path
//! is required to use is what makes the exclusion gate one gate rather than two
//! that can disagree.
//!
//! Exclusion is applied retroactively, which is the whole of ING-08's read half:
//! no flag is written at ingest, the predicate is re-applied on every read, and
//! the case that matters is precisely the session archived *before* its project
//! was excluded.

use std::collections::BTreeMap;

use verbatim_core::config::visible;
use verbatim_core::recall::scope;

use super::json::Document;
use super::read::{self, Opened};
use super::Failure;

const COMMAND: &str = "sessions";

pub fn run(args: Args) -> Result<(), Failure> {
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        Opened::Nothing(reason) => return read::empty(document(&[]), &reason, args.json),
    };
    let conn = reader.store().conn();
    let config = reader.config();

    let scoped = scope::resolve(conn, config, &read::scope(args.project.as_deref())?)?;
    if let Some(reason) = scoped.reason() {
        return read::empty(document(&[]), &reason.to_string(), args.json);
    }

    // Inlined rather than a helper taking a connection: `rusqlite` is
    // deliberately not named in this crate (the house rule `status` states), and
    // a function signature is where the type would have to be written.
    let details: BTreeMap<String, Detail> = {
        let mut statement = op(conn.prepare(
            "SELECT session_key, branch, first_turn_at, last_turn_at,
                    parent_session_key IS NOT NULL, is_evicted
             FROM session_meta",
        ))?;
        let rows = op(statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Detail {
                    branch: row.get(1)?,
                    first_turn_at: row.get(2)?,
                    last_turn_at: row.get(3)?,
                    sidecar: row.get::<_, i64>(4)? != 0,
                    evicted: row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                },
            ))
        }))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (key, detail) = op(row)?;
            out.insert(key, detail);
        }
        out
    };
    let mut listed: Vec<Listing> = Vec::new();
    for session in visible::sessions(conn, config)? {
        // The scope narrows what is listed; `visible::sessions` has already
        // decided what may be listed at all.
        if let Some(wanted) = scoped.project() {
            if session.project.as_deref() != Some(wanted) {
                continue;
            }
        }
        let detail = details
            .get(&session.session_key)
            .cloned()
            .unwrap_or_default();
        if !args.window.covers(&detail) {
            continue;
        }
        listed.push(Listing { session, detail });
        if listed.len() == args.limit {
            break;
        }
    }

    if args.json {
        document(&listed.iter().map(value).collect::<Vec<_>>()).emit();
        return Ok(());
    }

    for listing in &listed {
        println!(
            "{}  {}  {} turn(s)  {}  {}{}{}",
            day(listing.detail.first_turn_at.as_deref()),
            file_name(&listing.session.session_key),
            listing.session.turns,
            listing.session.project.as_deref().unwrap_or("(no project)"),
            listing.detail.branch.as_deref().unwrap_or("(no branch)"),
            if listing.detail.sidecar {
                "  (subagent)"
            } else {
                ""
            },
            if listing.detail.evicted {
                "  (evicted)"
            } else {
                ""
            },
        );
    }

    // Commentary on stderr the way `status` reports its own, so stdout stays one
    // parseable line per session.
    eprintln!("{} session(s)", listed.len());
    Ok(())
}

/// One listed session: what the exclusion gate returned, plus what only
/// `session_meta` holds.
struct Listing {
    session: visible::Session,
    detail: Detail,
}

/// The columns a listing shows that `visible::Session` does not carry.
///
/// A second read rather than a widened projection: `config::visible` is the
/// exclusion gate every read path shares, and widening its row for one command's
/// display columns would put this command's needs in every other command's
/// query.
#[derive(Debug, Clone, Default)]
struct Detail {
    branch: Option<String>,
    first_turn_at: Option<String>,
    last_turn_at: Option<String>,
    /// D-07: a sidecar is a session whose `parent_session_key` is non-null.
    sidecar: bool,
    evicted: bool,
}

/// A store error is operational (exit 1), never misuse.
fn op<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> Result<T, Failure> {
    result.map_err(|e| Failure::Operational(e.to_string()))
}

/// The time window a listing is narrowed to.
///
/// A session is a span, not an instant, so the test is overlap: a session is in
/// the window when it was still going after `since` and had already started by
/// `until`. Comparing one end only would hide a session that ran across the
/// boundary, which is the session a user asking "what was I doing that week"
/// most wants.
#[derive(Debug, Default)]
pub struct Window {
    since: Option<String>,
    until: Option<String>,
}

impl Window {
    fn covers(&self, detail: &Detail) -> bool {
        if let Some(since) = &self.since {
            match &detail.last_turn_at {
                Some(last) if last.as_str() >= since.as_str() => {}
                // A session with no timestamps is outside every window, for the
                // reason a turn with none is: it cannot say when it happened, so
                // it cannot be shown as evidence of when something did.
                _ => return false,
            }
        }
        if let Some(until) = &self.until {
            match &detail.first_turn_at {
                Some(first) if first.as_str() <= until.as_str() => {}
                _ => return false,
            }
        }
        true
    }
}

fn value(listing: &Listing) -> serde_json::Value {
    serde_json::json!({
        "session_key": listing.session.session_key,
        "session_no": listing.session.session_no,
        "project": listing.session.project,
        "project_pre_worktree": listing.session.project_pre_worktree,
        "branch": listing.detail.branch,
        "first_turn_at": listing.detail.first_turn_at,
        "last_turn_at": listing.detail.last_turn_at,
        "turns": listing.session.turns,
        "watermark": listing.session.watermark,
        "sidecar": listing.detail.sidecar,
        "evicted": listing.detail.evicted,
    })
}

fn document(sessions: &[serde_json::Value]) -> Document {
    Document::new(COMMAND).field("sessions", sessions.to_vec())
}

/// A transcript timestamp at day resolution (D-23: one shape, UTC only).
fn day(ts: Option<&str>) -> &str {
    match ts {
        Some(ts) if ts.len() >= 10 => &ts[..10],
        Some(ts) => ts,
        None => "(no time)",
    }
}

fn file_name(session_key: &str) -> &str {
    session_key
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(session_key)
}

/// The parsed command line for `sessions`.
#[derive(Debug)]
pub struct Args {
    pub project: Option<String>,
    pub window: Window,
    /// How many to list. A listing has no natural page, so the default is all of
    /// them: `verbatim sessions | wc -l` is a question a user has.
    pub limit: usize,
    pub json: bool,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut project = None;
    let mut window = Window::default();
    let mut limit = usize::MAX;
    let mut json = false;

    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long("project") => project = Some(super::value(parser, "project")?),
            Long("since") => {
                window.since = Some(super::time_bound("since", &super::value(parser, "since")?)?)
            }
            Long("until") => {
                window.until = Some(super::time_bound("until", &super::value(parser, "until")?)?)
            }
            Long("limit") => {
                let raw = super::value(parser, "limit")?;
                limit = raw
                    .parse::<usize>()
                    .map_err(|_| Failure::Misuse(format!("--limit {raw:?} is not a number")))?;
                if limit == 0 {
                    return Err(Failure::Misuse("--limit 0 asks for no sessions".into()));
                }
            }
            Long(super::JSON_FLAG) => json = true,
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    Ok(Args {
        project,
        window,
        limit,
        json,
    })
}
