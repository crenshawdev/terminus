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

/// What `/.claude/worktrees/` encodes to under D-07's rule.
///
/// A fixed literal segment, not an open-ended prefix extension: it is the one
/// clause [`Config::excludes_encoded_dir`] can tolerate without reintroducing
/// the ambiguity D-07 proved.
const WORKTREE_INFIX: &str = "--claude-worktrees-";

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
    /// Exact equality against the encoded exclusion, or that plus the fixed
    /// literal [`WORKTREE_INFIX`] and anything after it. The worktree clause is
    /// not optional: D-06 folds a `cwd` of `<repo>/.claude/worktrees/<name>`
    /// into `<repo>`, and that path shape encodes literally, so without the
    /// clause excluding `/data/code/cadence` would still walk and archive every
    /// one of its worktree sessions - and the read path would then be
    /// filtering them out afterwards, which is the read-then-filter behaviour
    /// ING-08 exists to forbid.
    ///
    /// Known and accepted imprecision: `/data/code/jcrenshaw.dev` and
    /// `/data/code/jcrenshaw-dev` encode identically, so they are the same test
    /// here. [`Config::excludes_path`] tells them apart wherever a real path is
    /// available.
    pub fn excludes_encoded_dir(&self, dir_name: &str) -> bool {
        let name = fold(dir_name);
        self.encoded_exclusions.iter().any(|encoded| {
            name == *encoded
                || name
                    .strip_prefix(encoded.as_str())
                    .is_some_and(|rest| rest.starts_with(WORKTREE_INFIX))
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
