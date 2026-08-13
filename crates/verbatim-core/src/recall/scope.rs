//! Two rules over one projection: which project a search is in, and which
//! projects it may never see.
//!
//! **Scoping (D-12).** The project a read command means by default is the one
//! the process is standing in, and it is found by a longest-prefix match of the
//! working directory against the project keys `session_meta` already holds -
//! never by resolving the directory afresh through [`crate::project::Resolver`].
//! Two reasons, and both are decisive. The resolver shells out to `git` with a
//! 2 s budget, and process spawn is the 10-30 ms the cold-start budget is made
//! of; and a rule that resolves independently can disagree with what ingest
//! wrote, at which point a scoped search inside a worktree returns zero hits -
//! the identity fragmentation `.planning/PROJECT.md` names the incumbent for.
//! Matching against stored keys cannot disagree with them.
//!
//! **Exclusion (D-21, D-23).** Applied to `project` and to
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
/// [`crate::Error::StoreNotFound`] is a variant: RCL-10 renders an empty result
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

    /// Distinct `session_meta.project` values the config excludes.
    pub fn excluded_projects(&self) -> &[String] {
        &self.excluded_projects
    }

    /// Distinct `session_meta.project_pre_worktree` values the config excludes.
    pub fn excluded_pre_worktree(&self) -> &[String] {
        &self.excluded_pre_worktree
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
    // The short circuit D-21 asks for. An exclusion of the filesystem root is
    // "read nothing", and answering it with a query would be asking the store
    // for rows it may not return.
    if config.excludes_everything() {
        return Ok(Scoped::nothing(Reason::EverythingExcluded));
    }

    let keys = visible::projects(conn, config)?;

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
/// own, which is the fragmentation phase 2 D-06 folded the two keys together to
/// prevent.
///
/// Ties are broken by the projection's own `ORDER BY`, which is over the stored
/// values, so two runs against an unchanged store resolve to the same project.
fn longest_prefix(keys: &[visible::ProjectKeys], path: &Path) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for row in keys {
        let Some(project) = row.project.as_deref() else {
            // Nothing to scope to: a session with no `cwd` is reachable only
            // through `*`.
            continue;
        };
        for key in [row.project.as_deref(), row.project_pre_worktree.as_deref()]
            .into_iter()
            .flatten()
        {
            let Some(depth) = config::covers(Path::new(key), path) else {
                continue;
            };
            if best.is_none_or(|(deepest, _)| depth > deepest) {
                best = Some((depth, project));
            }
        }
    }
    best.map(|(_, project)| project.to_owned())
}
