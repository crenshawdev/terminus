//! The shared credentials loader, and PRIV-02's refusal.
//!
//! # Three tiers, one order (D-14)
//!
//! A provider credential is looked for in exactly this order, and the first
//! hit wins:
//!
//! 1. **The process environment.** The variable's name is derived from the
//!    provider name in `verbatim.toml` - `name = "openrouter"` is read from
//!    `OPENROUTER_API_KEY` - by upper-casing it and replacing every character
//!    that is not an ASCII letter or digit with `_`, then appending
//!    `_API_KEY`. See [`env_var_for`]. This is the tier a CI runner and a
//!    one-off shell export land in.
//! 2. **`verbatim.toml`'s own `api_key`.** The product-config tier: a value,
//!    not a reference to one.
//! 3. **The shared file**, `~/.config/jcrenshaw/credentials.toml` on Unix and
//!    `%APPDATA%\jcrenshaw\credentials.toml` on Windows, as a table of provider
//!    namespaces:
//!
//!    ```toml
//!    [openrouter]
//!    api_key = "..."
//!    ```
//!
//!    It does not exist on this machine, and its absence is the common case
//!    rather than an error.
//!
//! Implemented here rather than consumed from a cross-product library because
//! no such library exists yet (D-14), and blocking phase 7 on building one
//! would block every observation.
//!
//! # It reads, and does nothing else
//!
//! No file is created, no directory is created, no permission is repaired and
//! no legacy file is migrated. A wrong mode is reported, by this module as a
//! refusal and by `verbatim doctor` as the `chmod` that fixes it; neither
//! changes it. A loader that repaired what it found would be a loader that
//! silently widened or narrowed a file shared with every other product on the
//! machine.
//!
//! # PRIV-02, and where it stops (D-15)
//!
//! On Unix a credentials file with any group or world bit set is refused, and
//! the refusal names the file and its octal mode. On Windows the ACL is not
//! checked by this build and the file is accepted with a caveat that
//! `verbatim doctor` states in words - the deferral is D-15's, not an oversight.
//!
//! Nothing here can put a credential value on a stream. The value is returned
//! inside [`Secret`], whose `Debug` and `Display` both render
//! [`crate::config::REDACTED`], and this module's own [`Error`] carries a path
//! and a mode and never a byte of a file's contents - including when the file
//! fails to parse, where only the parser's POSITION survives, because every
//! byte of this particular file is a credential.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::{Config, Secret};

/// The shared config directory's name, under whichever config root the
/// platform resolves. Every jcrenshawdev product reads the same one.
pub const SHARED_DIR_NAME: &str = "jcrenshaw";

/// The file inside it.
pub const CREDENTIALS_FILE_NAME: &str = "credentials.toml";

/// Points the shared config directory somewhere else, the way
/// `VERBATIM_CONFIG_DIR` does for verbatim's own.
///
/// It exists for the reason `config::CONFIG_DIR_ENV` exists and for one more:
/// without it every test in this file would read the developer's real
/// credentials, and the one that asserts a refusal would need to chmod it.
pub const SHARED_DIR_ENV: &str = "JCRENSHAW_CONFIG_DIR";

/// What went wrong looking a credential up.
///
/// Carries a path and a mode. It does not carry file contents, a parser
/// excerpt, an environment value or anything else that could be a credential -
/// this type is rendered into `runs.error` and onto stderr, and D-16 makes that
/// boundary the redaction boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// PRIV-02: the shared file is readable by somebody other than its owner.
    ///
    /// A refusal and not a warning. The alternative is reading a secret out of
    /// a file the user has effectively published, and then behaving as though
    /// it were private.
    TooOpen { path: PathBuf, mode: u32 },
    /// The shared file is there and could not be read or does not parse.
    ///
    /// `detail` is the reason WITHOUT the parser's rendering of the file: a
    /// TOML error normally quotes the offending line back, and in this file
    /// every line is a credential.
    Unreadable { path: PathBuf, detail: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooOpen { path, mode } => write!(
                f,
                "{} is mode {:03o}; a credentials file readable by group or world is refused",
                path.display(),
                mode & 0o777
            ),
            Error::Unreadable { path, detail } => {
                write!(f, "{} could not be read: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {}

/// What the shared file's permissions say (PRIV-02, D-15).
///
/// One answer computed in one place, because [`resolve`] has to act on it and
/// `verbatim doctor` has to report it, and a report that disagreed with the
/// refusal would be worse than no report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permissions {
    /// Nothing there. The ordinary state, not a problem: the file does not
    /// exist on this machine and most machines will never have one.
    Absent,
    /// Present, and its owner is the only one who can read it.
    Owner { mode: u32 },
    /// Present and readable by group or world. [`resolve`] refuses it.
    TooOpen { mode: u32 },
    /// Present, on a build that does not check the ACL (D-15).
    Unchecked,
    /// Present, and its metadata could not be read.
    Unreadable { detail: String },
}

/// Where the shared credentials file would be, if there were one.
///
/// `None` when no config root resolves at all - no override, no
/// `XDG_CONFIG_HOME` and no home directory. That is a state to report rather
/// than an error to raise: there is nowhere to look, so nothing was found.
pub fn path() -> Option<PathBuf> {
    Some(shared_dir()?.join(CREDENTIALS_FILE_NAME))
}

/// The shared config directory, resolved the way [`crate::config::config_dir`]
/// resolves verbatim's own: an override first, then the platform location.
///
/// The XDG arm is deliberate even though `DESIGN-BRIEF.md` spells the path as
/// `~/.config/jcrenshaw`: `XDG_CONFIG_HOME` IS `~/.config` unless the user
/// moved it, and a product that ignored the move would look somewhere the
/// user's other tools do not.
#[cfg(not(target_os = "windows"))]
fn shared_dir() -> Option<PathBuf> {
    if let Some(dir) = non_empty_var(SHARED_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    if let Some(xdg) = non_empty_var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join(SHARED_DIR_NAME));
    }
    Some(
        PathBuf::from(non_empty_var("HOME")?)
            .join(".config")
            .join(SHARED_DIR_NAME),
    )
}

#[cfg(target_os = "windows")]
fn shared_dir() -> Option<PathBuf> {
    if let Some(dir) = non_empty_var(SHARED_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    Some(PathBuf::from(non_empty_var("APPDATA")?).join(SHARED_DIR_NAME))
}

/// The environment variable a provider's credential may be given in.
///
/// `openrouter` -> `OPENROUTER_API_KEY`, `open-router` -> `OPEN_ROUTER_API_KEY`.
/// Every character that is not an ASCII letter or digit becomes `_`, which also
/// keeps a config-supplied provider name from spelling something exotic.
pub fn env_var_for(provider: &str) -> String {
    let mut name: String = provider
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    name.push_str("_API_KEY");
    name
}

/// Read the shared file's permissions without reading the file (PRIV-02).
#[cfg(unix)]
pub fn permissions(path: &Path) -> Permissions {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Permissions::Absent,
        Err(e) => Permissions::Unreadable {
            detail: e.to_string(),
        },
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            // Any group or world bit, not just the read bits: a file somebody
            // else can write is a file somebody else can replace with their
            // own key, and a directory bit here would mean this is not the
            // file at all.
            if mode & 0o077 != 0 {
                Permissions::TooOpen { mode }
            } else {
                Permissions::Owner { mode }
            }
        }
    }
}

/// The Windows arm: present or absent, and the ACL unexamined (D-15).
#[cfg(not(unix))]
pub fn permissions(path: &Path) -> Permissions {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Permissions::Absent,
        Err(e) => Permissions::Unreadable {
            detail: e.to_string(),
        },
        Ok(_) => Permissions::Unchecked,
    }
}

/// The credential for the configured provider, or `None` if there is none.
///
/// Tiers in D-14's order; see the module comment. An absent shared file yields
/// `None` and not an error, and so does a shared file that has no section for
/// this provider.
///
/// The file is only opened when the two tiers above it came up empty, so a key
/// supplied in the environment is not held hostage by the mode of a file
/// nothing read.
pub fn resolve(config: &Config) -> Result<Option<Secret>, Error> {
    if let Some(provider) = config.provider_name() {
        if let Some(value) = non_empty_var(&env_var_for(provider)) {
            // Lossy rather than refused: a credential arriving through the
            // environment with a stray non-UTF-8 byte is a value the user
            // meant, and the alternative is an error message about encoding
            // that would be tempted to show the bytes.
            return Ok(Some(Secret::new(value.to_string_lossy().into_owned())));
        }
    }
    if let Some(key) = config.provider_api_key() {
        return Ok(Some(key.clone()));
    }
    let Some(provider) = config.provider_name() else {
        // Without a namespace there is no section to read, and reading the
        // whole file to guess one would be reading credentials for products
        // that are not this one.
        return Ok(None);
    };
    from_shared_file(provider)
}

/// Tier three: the shared file's section for this provider.
fn from_shared_file(provider: &str) -> Result<Option<Secret>, Error> {
    let Some(path) = path() else {
        return Ok(None);
    };
    match permissions(&path) {
        Permissions::Absent => return Ok(None),
        Permissions::TooOpen { mode } => return Err(Error::TooOpen { path, mode }),
        Permissions::Unreadable { detail } => return Err(Error::Unreadable { path, detail }),
        Permissions::Owner { .. } | Permissions::Unchecked => {}
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // Checked above and gone by now, or never really there: a race with
        // whoever owns this shared file is not a failure of this program.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error::Unreadable {
                path,
                detail: e.to_string(),
            })
        }
    };
    let file: SharedFile = toml::from_str(&text).map_err(|e| Error::Unreadable {
        path,
        detail: position_only(&e.to_string()),
    })?;

    Ok(file
        .0
        .get(provider)
        .and_then(|section| section.api_key.as_deref())
        .filter(|value| !value.is_empty())
        .map(Secret::new))
}

/// The shared file: a table of provider namespaces and nothing else.
///
/// Unknown keys inside a section are ignored, the same rule `FileConfig`
/// documents, and for a stronger reason here: this file is written by every
/// jcrenshawdev product and verbatim is one reader among several.
#[derive(Debug, Deserialize)]
struct SharedFile(std::collections::BTreeMap<String, Section>);

#[derive(Debug, Deserialize)]
struct Section {
    api_key: Option<String>,
}

/// A TOML error reduced to its first line - the position - with the parser's
/// excerpt of the file dropped.
///
/// Unconditional here, unlike `config`'s equivalent, and that is the whole
/// difference between the two files: `verbatim.toml` has one secret-bearing key
/// among many, and every line of this one is a secret.
fn position_only(detail: &str) -> String {
    detail
        .lines()
        .next()
        .unwrap_or("it is not valid TOML")
        .to_owned()
}

/// An environment variable set to the empty string is unset, the rule
/// `config::non_empty_var` states.
fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}
