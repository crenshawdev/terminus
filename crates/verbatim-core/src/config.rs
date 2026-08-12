//! Verbatim's own config: which transcript trees to walk, and which projects
//! never to look at.
//!
//! The file is `verbatim.toml` inside verbatim's config directory, and
//! `config.toml` in that same directory is deliberately never read (D-15). The
//! legacy tool wrote one - 184 bytes, `base_dir = "/data/verbatim"` - and
//! adopting it would point the new store at the legacy data directory whose
//! import `.planning/PROJECT.md` defers.
//!
//! A missing config file is not an error. It yields the defaults, which is the
//! state every user starts in.
//!
//! # Exclusion has two entry points, because its two callers know two different
//! things
//!
//! [`Config::excludes_encoded_dir`] answers **before any file is opened**. All
//! it has is the encoded project directory name, because `cwd` only exists on a
//! parsed record and reading a record means opening the file - which is exactly
//! what ING-08 forbids for an excluded project (D-22). It is therefore an exact
//! match on the encoded name, plus one fixed-literal worktree clause, and not a
//! prefix match: the encoding is provably lossy (D-07), so an
//! extension-tolerant rule cannot tell a child directory from a hyphenated
//! sibling and `-data-projects-cadence` would silently swallow
//! `-data-projects-cadence-research`.
//!
//! [`Config::excludes_path`] answers wherever a real filesystem path is
//! available, and there it is an ordinary subtree match on path components,
//! which is unambiguous. That is the read-side predicate (D-23).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// Verbatim's config file. `config.toml` beside it is the legacy tool's and is
/// never read (D-15).
pub const CONFIG_FILE_NAME: &str = "verbatim.toml";

/// Points verbatim's config directory somewhere else, the way
/// `VERBATIM_DATA_DIR` does for the store.
pub const CONFIG_DIR_ENV: &str = "VERBATIM_CONFIG_DIR";

/// Claude Code's own override for its config directory. Read as a **single**
/// directory (D-15): the design brief says roots accept a list, but the
/// separator convention there is Claude Code's and the variable is unset on the
/// development machine, so multi-root is supported through `verbatim.toml`
/// rather than by guessing somebody else's separator.
pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

/// The subdirectory of a Claude config directory that holds the transcripts.
pub const PROJECTS_SUBDIR: &str = "projects";

/// The default Claude config directory, relative to the user's home.
pub const DEFAULT_CLAUDE_DIR: &str = ".claude";

/// What a path separator encodes to under D-07's rule.
///
/// [`encode`] maps every non-alphanumeric character to this, so a subdirectory
/// of an encoded path is that path, this, and the rest - and so is a sibling
/// whose name merely contains a `-` or a `.`. Telling those two apart is what
/// [`descends_to`] is for.
const ENCODED_SEPARATOR: char = '-';

/// How many directory entries [`descends_to`] may examine before it gives up
/// and lets the caller fail safe.
///
/// The search is pruned to the branches whose encoding is still a prefix of the
/// candidate, so a real tree costs a handful of entries; this only bounds a
/// pathological one. Exhausting it is indistinguishable from an unreadable
/// directory on purpose - both mean "could not resolve", and both exclude.
const DIR_SCAN_BUDGET: usize = 4_096;

/// What `verbatim.toml` may contain.
///
/// Unknown keys are ignored rather than rejected: this file will grow across
/// phases, and a store written by a newer binary must not make an older one
/// refuse to start.
#[derive(Debug, Clone, Default, Deserialize)]
struct FileConfig {
    /// Claude config directories, each of whose `projects` subdirectory is a
    /// tree to walk. An explicit list wins over [`CLAUDE_CONFIG_DIR_ENV`].
    #[serde(default)]
    roots: Vec<String>,
    /// Project paths - real filesystem paths, as the user writes them - that
    /// are never read and never returned by a read path.
    #[serde(default)]
    exclude: Vec<String>,
}

/// The resolved config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    roots: Vec<PathBuf>,
    exclusions: Vec<String>,
    /// D-07's encoding of each exclusion, case-folded where the platform is,
    /// computed once so the pre-open test costs a comparison per project
    /// directory rather than a re-encode.
    encoded_exclusions: Vec<String>,
}

impl Config {
    /// Load from verbatim's config directory.
    pub fn load() -> Result<Config> {
        let dir = config_dir()?;
        Config::load_from(&dir)
    }

    /// Load from a named config directory.
    ///
    /// Only `verbatim.toml` inside it is read. A `config.toml` sitting beside
    /// it is the legacy tool's file and is not this program's (D-15).
    pub fn load_from(config_dir: &Path) -> Result<Config> {
        let path = config_dir.join(CONFIG_FILE_NAME);
        let file = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<FileConfig>(&text).map_err(|source| {
                // The file exists and does not parse, so the user meant
                // something by it: naming the file and the position beats
                // falling back to defaults and walking the wrong tree.
                Error::ConfigParse {
                    path: path.clone(),
                    detail: source.to_string(),
                }
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
            Err(e) => return Err(Error::io(&path, e)),
        };
        Config::resolve(file)
    }

    /// A config built in memory, for callers that have no file. Test support
    /// and nothing else uses this today.
    pub fn from_parts(roots: Vec<PathBuf>, exclusions: Vec<String>) -> Config {
        let encoded_exclusions = exclusions.iter().map(|e| fold(&encode(e))).collect();
        Config {
            roots,
            exclusions,
            encoded_exclusions,
        }
    }

    fn resolve(file: FileConfig) -> Result<Config> {
        let roots = if !file.roots.is_empty() {
            file.roots.iter().map(PathBuf::from).collect()
        } else if let Some(dir) = non_empty_var(CLAUDE_CONFIG_DIR_ENV) {
            // Replaces the DEFAULT only. An explicit `roots` list is verbatim's
            // own configuration and outranks Claude Code's environment.
            vec![PathBuf::from(dir)]
        } else {
            vec![home_dir()?.join(DEFAULT_CLAUDE_DIR)]
        };
        Ok(Config::from_parts(roots, file.exclude))
    }

    /// The Claude config directories, in the order they were configured.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// The trees a pass walks: each root's `projects` subdirectory.
    pub fn transcript_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| r.join(PROJECTS_SUBDIR)).collect()
    }

    /// The excluded project paths, exactly as configured.
    pub fn exclusions(&self) -> &[String] {
        &self.exclusions
    }

    /// The pre-open test (D-09, D-22): is this encoded project directory name
    /// excluded?
    ///
    /// Exact equality against the encoded exclusion, or a name that extends it
    /// past an encoded separator and is confirmed against the filesystem to be
    /// a real subdirectory of the excluded path.
    ///
    /// The extension case is the ambiguity D-07 proved: `-` encodes the
    /// separator and also a literal `-` or `.`, so `-data-code-foo-bar` is
    /// `/data/code/foo/bar` and `/data/code/foo-bar` and
    /// `/data/code/foo.bar` at once. D-09 asked for a segment-boundary match
    /// and for `-data-projects-cadence-research` not to match
    /// `-data-projects-cadence`, which are not both satisfiable in the encoded
    /// space alone - so this resolves them outside it, by asking the filesystem
    /// which of those paths exists ([`descends_to`]). That reads directory
    /// entries and opens no transcript, which is what AC5 and D-22 constrain.
    ///
    /// Unresolvable means excluded: an excluded directory that is unreadable or
    /// gone, or a tree too large to search, leaves the question open, and the
    /// safe answer for an exclusion boundary is not to read. The cost of being
    /// wrong that way is a project that goes unarchived and can be archived
    /// later; the cost of the other way is bytes the user said never to read.
    ///
    /// This decides only whether to OPEN. [`Config::excludes_path`] is the
    /// exact test, and it is what decides whether anything is hidden.
    pub fn excludes_encoded_dir(&self, dir_name: &str) -> bool {
        let name = fold(dir_name);
        self.exclusions
            .iter()
            .zip(&self.encoded_exclusions)
            .any(|(excluded, encoded)| {
                if name == *encoded {
                    return true;
                }
                let extends = name
                    .strip_prefix(encoded.as_str())
                    .is_some_and(|rest| rest.starts_with(ENCODED_SEPARATOR));
                if !extends {
                    return false;
                }
                descends_to(Path::new(excluded), &name).unwrap_or(true)
            })
    }

    /// The read-side test (D-23): is this real path inside an excluded project?
    ///
    /// A subtree match on path components, so `/data/projects/cadence` covers
    /// `/data/projects/cadence/sub` and does not cover
    /// `/data/projects/cadence-research`. Components rather than string
    /// prefixes, because that is what makes the separator boundary exact and
    /// what makes a trailing slash in the config irrelevant.
    pub fn excludes_path(&self, path: &Path) -> bool {
        let candidate = components(path);
        self.exclusions.iter().any(|excluded| {
            let prefix = components(Path::new(excluded));
            !prefix.is_empty()
                && candidate.len() >= prefix.len()
                && candidate[..prefix.len()] == prefix[..]
        })
    }
}

/// D-07's encoding: every character outside `[A-Za-z0-9]` becomes `-`.
///
/// Applying this to a transcript's first `cwd` reproduces its containing
/// directory name for 1,252 of 1,252 real files, so the encoding is exact - and
/// exactly lossy, which is why nothing ever runs it backwards.
pub fn encode(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Path comparison is case-insensitive on Windows and macOS and case-sensitive
/// elsewhere (`.planning/PROJECT.md` constraints).
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn fold(s: &str) -> String {
    s.to_lowercase()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn fold(s: &str) -> String {
    s.to_owned()
}

/// Does some real directory beneath `root` encode to `name`?
///
/// `Some(true)` when one does - `name` is a subdirectory of `root` and the
/// caller excludes it. `Some(false)` only when the search was COMPLETE and
/// nothing on the way to `name` exists at all, which is what a path that merely
/// encodes the same way looks like; the caller leaves that one alone. `None`
/// whenever the question could not be answered, and the caller treats `None` as
/// excluded.
///
/// The walk descends only into directories whose own encoding is still a prefix
/// of `name`, so it follows the one branch that can match rather than the tree.
///
/// **A partial match is unresolved, not a negative.** Descending a branch means
/// the leading components of `name` are real directories under `root`; failing
/// to find the leaf under them means the leaf is GONE, not that `name` names
/// something else. Worktrees are this case and are the majority of it: a
/// worktree directory is deleted when the worktree is, while `<repo>/.claude/
/// worktrees` survives empty, so every archived worktree session's project
/// directory has a live prefix and a dead leaf. Answering `Some(false)` there
/// would open exactly the transcripts D-06 folds into the excluded repo.
///
/// Unreadable entries and symlinks are unresolved for the same reason. A
/// symlinked branch is not followed - a link loop would not terminate - so when
/// one could still lead to `name` the answer is `None` rather than a negative
/// reached by not looking.
fn descends_to(root: &Path, name: &str) -> Option<bool> {
    if !root.is_dir() {
        return None;
    }
    let mut budget = DIR_SCAN_BUDGET;
    let mut stack = vec![root.to_path_buf()];
    let mut descended = false;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()? {
            budget = budget.checked_sub(1)?;
            let entry = entry.ok()?;
            let child = entry.path();
            let encoded = fold(&encode(&child.to_string_lossy()));
            let leads_to_name = name
                .strip_prefix(encoded.as_str())
                .is_some_and(|rest| rest.starts_with(ENCODED_SEPARATOR));
            if !encoded.eq(name) && !leads_to_name {
                continue;
            }
            // Only entries that could still be `name` are typed, so an
            // unreadable type on an unrelated file never decides anything.
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() {
                return None;
            }
            if !file_type.is_dir() {
                continue;
            }
            if encoded == name {
                return Some(true);
            }
            descended = true;
            stack.push(child);
        }
    }
    (!descended).then_some(false)
}

/// A path as its comparable components, folded for the platform.
fn components(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| fold(&c.as_os_str().to_string_lossy()))
        .collect()
}

/// Verbatim's config directory, resolved the way `store::data_dir` resolves the
/// data directory: an environment override first, then the platform location.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_var(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    platform_config_dir()
}

#[cfg(not(target_os = "windows"))]
fn platform_config_dir() -> Result<PathBuf> {
    if let Some(xdg) = non_empty_var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("verbatim"));
    }
    Ok(home_dir()?.join(".config").join("verbatim"))
}

#[cfg(target_os = "windows")]
fn platform_config_dir() -> Result<PathBuf> {
    let appdata = non_empty_var("APPDATA").ok_or_else(|| Error::ConfigUnresolved {
        detail: format!("neither {CONFIG_DIR_ENV} nor APPDATA is set"),
    })?;
    Ok(PathBuf::from(appdata).join("verbatim"))
}

#[cfg(not(target_os = "windows"))]
fn home_dir() -> Result<PathBuf> {
    non_empty_var("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::ConfigUnresolved {
            detail: "HOME is not set".into(),
        })
}

#[cfg(target_os = "windows")]
fn home_dir() -> Result<PathBuf> {
    non_empty_var("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| Error::ConfigUnresolved {
            detail: "USERPROFILE is not set".into(),
        })
}

/// An environment variable set to the empty string is treated as unset: an
/// empty path would resolve to the process's current directory.
fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// The read side of ING-08: the one way a read path lists sessions.
///
/// **Every future read path is required to go through this module** rather than
/// querying `session_meta` directly. Phase 3's search, phase 5's injection and
/// the MCP tools all reuse it; a query that reaches past it is how an excluded
/// project becomes visible again.
///
/// The failure this exists to prevent is named in `.planning/PROJECT.md`: the
/// incumbent honors exclusion on write and ignores it on read. Verbatim honors
/// it on both, and honors it **retroactively** - which is why no per-session
/// flag is written at ingest (D-23). A flag would say what was true when the
/// session was archived, and the case that matters is precisely the session
/// archived *before* its project was excluded. The predicate is re-applied on
/// every read instead.
///
/// It is applied to `session_meta.project` **and** to
/// `session_meta.project_pre_worktree`, because either one alone leaks: a
/// worktree session carries the folded parent repo in `project` and the
/// worktree path in the pre-mapping column, and a user may reasonably exclude
/// either path.
///
/// A session whose `project` is null - one real transcript carries no `cwd` at
/// all - is visible, because nothing can say it is excluded.
pub mod visible {
    use rusqlite::Connection;

    use super::Config;
    use crate::error::Result;

    /// One session a read path may see.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Session {
        /// The canonical transcript path, which is what `sessions` is keyed on.
        pub session_key: String,
        pub session_no: i64,
        pub project: Option<String>,
        pub project_pre_worktree: Option<String>,
        /// Turn rows this session contributed.
        pub turns: i64,
        /// The byte offset ingest will resume from, when one is recorded.
        pub watermark: Option<i64>,
    }

    /// What the visible sessions add up to.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct Counts {
        pub sessions: i64,
        pub turns: i64,
        pub watermarks: i64,
        /// Bytes those watermarks cover.
        pub watermark_bytes: i64,
    }

    /// Every session the config does not exclude, in ingest order.
    ///
    /// A `LEFT JOIN`, so a session archived without a `session_meta` row is
    /// still listed rather than silently dropped: `verbatim verify` is what
    /// reports that damage, and a read path that hid it would hide the evidence.
    pub fn sessions(conn: &Connection, config: &Config) -> Result<Vec<Session>> {
        let mut statement = conn.prepare(
            "SELECT s.session_key, s.session_no, m.project, m.project_pre_worktree,
                    (SELECT count(*) FROM turns t WHERE t.session_key = s.session_key),
                    (SELECT w.byte_offset FROM watermarks w
                      WHERE w.transcript_path = s.session_key)
             FROM sessions s LEFT JOIN session_meta m USING (session_key)
             ORDER BY s.session_no",
        )?;
        let rows = statement.query_map([], |r| {
            Ok(Session {
                session_key: r.get(0)?,
                session_no: r.get(1)?,
                project: r.get(2)?,
                project_pre_worktree: r.get(3)?,
                turns: r.get(4)?,
                watermark: r.get(5)?,
            })
        })?;

        let mut out = Vec::new();
        for session in rows {
            let session = session?;
            if !is_excluded(config, &session) {
                out.push(session);
            }
        }
        Ok(out)
    }

    /// The counts `verbatim status` prints, over exactly the visible sessions.
    pub fn counts(conn: &Connection, config: &Config) -> Result<Counts> {
        let mut counts = Counts::default();
        for session in sessions(conn, config)? {
            counts.sessions += 1;
            counts.turns += session.turns;
            if let Some(offset) = session.watermark {
                counts.watermarks += 1;
                counts.watermark_bytes += offset;
            }
        }
        Ok(counts)
    }

    /// Is this session inside an excluded project, under either of its keys?
    pub fn is_excluded(config: &Config, session: &Session) -> bool {
        [&session.project, &session.project_pre_worktree]
            .into_iter()
            .flatten()
            .any(|key| config.excludes_path(std::path::Path::new(key)))
    }
}
