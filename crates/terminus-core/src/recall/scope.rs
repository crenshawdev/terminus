//! Two rules over one projection: which project a search is in, and which
//! projects it may never see.
//!
//! **Scoping.** The project a read command means by default is the one
//! the process is standing in, and it is found by a longest-prefix match of the
//! working directory against the project keys `session_meta` already holds -
//! never by resolving the directory afresh through [`crate::project::Resolver`].
//! Two reasons, and both are decisive. The resolver shells out to `git` with a
//! 2 s budget, and process spawn is the 10-30 ms the cold-start budget is made
//! of; and a rule that resolves independently can disagree with what ingest
//! wrote, at which point a scoped search inside a worktree returns zero hits -
//! the identity fragmentation the incumbent suffers from.
//! Matching against stored keys cannot disagree with them.
//!
//! **Exclusion.** Applied to `project` and to
//! `project_pre_worktree` both, because either alone leaks a worktree session,
//! and a session whose `project` is null stays visible because nothing can say
//! it is excluded. It stays in force under every scope, including `*`.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::config::{self, visible, Config};
use crate::error::Result;

/// The literal a caller passes to mean "every project".
pub const EVERY_PROJECT: &str = "*";

/// Which project's turns a search may return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The project this directory sits inside - the default, and the process's
    /// own working directory at the top of a read command.
    ///
    /// The directory is a value rather than something this module reads for
    /// itself: an MCP server's `cwd` and a terminal command's are the same
    /// question asked by two front ends, and a caller that has already resolved
    /// it should not have it resolved again.
    Directory(PathBuf),
    /// One project, named by any path inside it - the project key itself, a
    /// worktree path, or a subdirectory of either.
    Named(String),
    /// Every project. Exclusion is still in force.
    Everything,
}

impl Scope {
    /// The default scope: whatever project the process is standing in.
    pub fn current_directory() -> Result<Scope> {
        let dir =
            std::env::current_dir().map_err(|e| crate::error::Error::io(Path::new("."), e))?;
        Ok(Scope::Directory(dir))
    }

    /// The `project` argument a caller passed, as a scope.
    pub fn parse(value: &str) -> Scope {
        if value == EVERY_PROJECT {
            Scope::Everything
        } else {
            Scope::Named(value.to_owned())
        }
    }
}

/// Why a scoped search can return nothing at all.
///
/// A value rather than a formatted string, for the same reason
/// [`crate::Error::StoreNotFound`] is a variant: an empty result is answered
/// with a reason, and a caller must not have to match on message text to tell
/// one empty result from another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Nothing is archived, so there is no project to be standing in.
    NothingArchived,
    /// No archived project covers this directory. Said out loud rather than
    /// silently widening to every project: a user in an unindexed directory
    /// who gets the whole archive back reads it as a scoping bug, and a user
    /// who gets nothing back with no reason reads it as an empty archive.
    UnknownProject { named: String },
    /// The project in scope is one the config excludes.
    ProjectExcluded { project: String },
    /// The config excludes the filesystem root, so no read path may see
    /// anything at all.
    EverythingExcluded,
    /// A context window was asked about a turn the archive does not hold.
    NoSuchTurn { turn_id: i64 },
    /// The query reduced to no words that can be searched (`*`, `?!`). Said
    /// out loud because an empty result with no reason reads as an empty
    /// archive, to a model calling `recall_search` most of all.
    NothingSearchable,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::NothingArchived => write!(f, "no sessions are archived yet"),
            Reason::UnknownProject { named } => {
                write!(f, "no archived project contains {named}")
            }
            Reason::ProjectExcluded { project } => {
                write!(f, "{project} is excluded by config")
            }
            Reason::EverythingExcluded => {
                write!(f, "the configured exclusions cover the filesystem root")
            }
            Reason::NoSuchTurn { turn_id } => write!(f, "no turn {turn_id} is archived"),
            Reason::NothingSearchable => write!(f, "the query has no words that can be searched"),
        }
    }
}

/// A scope resolved against what the archive actually holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scoped {
    project: Option<String>,
    excluded_projects: Vec<String>,
    excluded_pre_worktree: Vec<String>,
    reason: Option<Reason>,
}

impl Scoped {
    /// The `session_meta.project` value a search is restricted to, or `None`
    /// when every project is in scope.
    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    /// May a turn whose session carries these two keys be served?
    ///
    /// No when this scope names a project and the turn's is not exactly that
    /// one, an absent project included. No when either key is exactly one the
    /// exclusion lists hold. The same rule [`Scoped::push_filter`] writes in
    /// SQL, for the reads that take a turn id rather than a query.
    ///
    /// A caller answers a no as `NoSuchTurn` and names no path. Turn ids are
    /// densely enumerable (`session_no << 24 | turn_seq`), so a reason of its
    /// own would confirm the id exists, and one naming the project would say
    /// what an excluded project is called.
    pub(crate) fn admits(&self, project: Option<&str>, pre_worktree: Option<&str>) -> bool {
        if self.project.is_some() && project != self.project() {
            return false;
        }
        let hidden = |key: Option<&str>, excluded: &[String]| {
            key.is_some_and(|k| excluded.iter().any(|e| e == k))
        };
        !hidden(project, &self.excluded_projects)
            && !hidden(pre_worktree, &self.excluded_pre_worktree)
    }

    /// Restrict a query over `session_meta m` to this scope: its project when
    /// it names one, and never a session an exclusion covers.
    ///
    /// Writes only `AND` clauses, each starting with a space, so the caller's
    /// SQL is already inside a `WHERE`.
    ///
    /// Both exclusions are written even under a project. Exclusion holds on the
    /// read path as well as on ingest, and [`resolve`] only rejects a scope
    /// whose own project is excluded: a session of a visible project whose
    /// pre-worktree key is excluded is still in the table and is filtered here.
    ///
    /// Each list is one bound parameter, a JSON array read through
    /// `json_each`, so its length is not bounded by SQLite's variable limit
    /// (#108).
    ///
    /// `NOT IN` carries the null guard because SQL's `NULL NOT IN (...)` is
    /// NULL, which is not true, so an unguarded clause would drop every session
    /// whose key is null - the one session nothing can say is excluded.
    ///
    /// Nothing is written for an empty list, which excludes nothing. The skip
    /// is per list, and that is what keeps `m.project_pre_worktree` out of the
    /// SQL on a store older than that column, where its list is always empty.
    pub(crate) fn push_filter(&self, sql: &mut String, params: &mut Vec<Box<dyn rusqlite::ToSql>>) {
        if let Some(project) = self.project() {
            sql.push_str(" AND m.project = ?");
            params.push(Box::new(project.to_owned()));
        }
        for (column, excluded) in [
            ("m.project", &self.excluded_projects),
            ("m.project_pre_worktree", &self.excluded_pre_worktree),
        ] {
            if excluded.is_empty() {
                continue;
            }
            sql.push_str(&format!(
                " AND ({column} IS NULL OR {column} NOT IN (SELECT value FROM json_each(?)))"
            ));
            let excluded: serde_json::Value = excluded.iter().map(String::as_str).collect();
            params.push(Box::new(excluded.to_string()));
        }
    }

    /// Set when this scope can match nothing at all, whatever the query is.
    pub fn reason(&self) -> Option<&Reason> {
        self.reason.as_ref()
    }

    /// Can this scope return a hit? False whenever [`Scoped::reason`] is set.
    pub fn is_empty(&self) -> bool {
        self.reason.is_some()
    }

    fn nothing(reason: Reason) -> Scoped {
        Scoped {
            reason: Some(reason),
            ..Scoped::default()
        }
    }
}

/// Resolve a scope against the project keys the archive holds.
pub fn resolve(conn: &Connection, config: &Config, scope: &Scope) -> Result<Scoped> {
    resolve_with(config, scope, || visible::projects(conn, config))
}

/// [`resolve`] over keys the caller reads, so every rule after the read exists once.
///
/// `read` runs only after the exclusion short circuit, as the read in `resolve` does.
pub(crate) fn resolve_with(
    config: &Config,
    scope: &Scope,
    read: impl FnOnce() -> Result<Vec<visible::ProjectKeys>>,
) -> Result<Scoped> {
    // The short circuit for excluding everything. An exclusion of the
    // filesystem root is "read nothing", and answering it with a query would be
    // asking the store for rows it may not return.
    if config.excludes_everything() {
        return Ok(Scoped::nothing(Reason::EverythingExcluded));
    }

    let keys = read()?;

    let mut excluded_projects: Vec<String> = keys
        .iter()
        .filter(|k| k.project_excluded)
        .filter_map(|k| k.project.clone())
        .collect();
    excluded_projects.sort_unstable();
    excluded_projects.dedup();

    let mut excluded_pre_worktree: Vec<String> = keys
        .iter()
        .filter(|k| k.pre_worktree_excluded)
        .filter_map(|k| k.project_pre_worktree.clone())
        .collect();
    excluded_pre_worktree.sort_unstable();
    excluded_pre_worktree.dedup();

    let named = match scope {
        Scope::Everything => {
            return Ok(Scoped {
                project: None,
                excluded_projects,
                excluded_pre_worktree,
                reason: None,
            })
        }
        Scope::Directory(dir) => dir.to_string_lossy().into_owned(),
        Scope::Named(name) => name.clone(),
    };

    if keys.is_empty() {
        return Ok(Scoped::nothing(Reason::NothingArchived));
    }

    let Some(project) = longest_prefix(&keys, Path::new(&named)) else {
        return Ok(Scoped::nothing(Reason::UnknownProject { named }));
    };

    if config.excludes_path(Path::new(&project)) {
        return Ok(Scoped::nothing(Reason::ProjectExcluded { project }));
    }

    Ok(Scoped {
        project: Some(project),
        excluded_projects,
        excluded_pre_worktree,
        reason: None,
    })
}

/// The `project` of the stored key that covers `path` most specifically.
///
/// Both stored keys are candidates and both resolve to the row's `project`. A
/// hit on `project_pre_worktree` resolving to the worktree path instead would
/// show only the sessions run from inside the worktree and hide the repository's
/// own, which is the fragmentation folding the two keys together exists to
/// prevent.
///
/// **At equal depth the folded key wins.** One directory is routinely both one
/// row's `project` and another row's `project_pre_worktree`: that is the
/// ordinary state of a machine that ingested from a worktree both before and
/// after the two keys were folded together. Both hits then score the same
/// depth, and breaking that tie by the projection's `ORDER BY` resolves to
/// whichever project string sorted first - which is not the caller's own, so
/// the search returns another project's turns and hides theirs. Deterministic,
/// and deterministically wrong. Ranking a direct `project` hit above a
/// `project_pre_worktree` hit resolves it to the row that actually names the
/// directory the caller is standing in.
///
/// Ties that remain - the same rank at the same depth - still fall to the
/// projection's `ORDER BY`, which is over the stored values, so two runs
/// against an unchanged store resolve to the same project.
fn longest_prefix(keys: &[visible::ProjectKeys], path: &Path) -> Option<String> {
    /// A hit on the row's own `project`, which outranks a pre-folding hit.
    const FOLDED: u8 = 1;
    /// A hit on `project_pre_worktree`, which resolves to the row's `project`.
    const PRE_WORKTREE: u8 = 0;

    let mut best: Option<(usize, u8, &str)> = None;
    for row in keys {
        let Some(project) = row.project.as_deref() else {
            // Nothing to scope to: a session with no `cwd` is reachable only
            // through `*`.
            continue;
        };
        let candidates = [
            (FOLDED, row.project.as_deref()),
            (PRE_WORKTREE, row.project_pre_worktree.as_deref()),
        ];
        for (rank, key) in candidates
            .into_iter()
            .filter_map(|(rank, key)| key.map(|key| (rank, key)))
        {
            let Some(depth) = config::covers(Path::new(key), path) else {
                continue;
            };
            if best.is_none_or(|(deepest, best_rank, _)| (depth, rank) > (deepest, best_rank)) {
                best = Some((depth, rank, project));
            }
        }
    }
    best.map(|(_, _, project)| project.to_owned())
}
