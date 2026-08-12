//! Finding every transcript beneath the configured roots.
//!
//! Three decisions shape everything here.
//!
//! **Filename, never record shape (D-16).** A `<uuid>.jsonl` directly inside a
//! project directory is a transcript, and an `agent-*.jsonl` anywhere below one
//! is a transcript. Nothing else is, and nothing else is opened to find out.
//! That is what keeps the four `journal.jsonl` files, the 816
//! `agent-*.meta.json` files, the `workflows/wf_*.json` files and the 131
//! `tool-results/` directories of `.txt` and `.md` out of the archive without a
//! single read.
//!
//! **Unbounded depth (D-16).** Sidecars sit at depth 4
//! (`<project>/<sessionId>/subagents/agent-*.jsonl`, 781 real files) *and* at
//! depth 6 (`.../subagents/workflows/wf_*/agent-*.jsonl`, 41 real files). A
//! depth limit silently drops those 41.
//!
//! **Canonicalize the root once, join onto it (D-17).** There are zero symlinks
//! inside the real tree while the root itself is a two-hop symlink chain
//! (`~/.claude -> /claude/.claude -> /data/claude/.claude`), so canonicalizing
//! the root alone yields exactly the session keys `ingest::run` produces
//! per-file today - at one syscall instead of two thousand.
//!
//! The walk is sequential and single-threaded, with no `walkdir`, no `glob` and
//! no `rayon` (D-18): bounded-parallelism backfill is phase 4's, and a walker
//! built now on one thread is what phase 4 fans out.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// What one walk found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovered {
    /// Every transcript, sorted, each path built by joining onto the canonical
    /// root so it equals the session key ingest will write.
    pub transcripts: Vec<PathBuf>,
    /// Directories that could not be listed, with why. Reported and skipped -
    /// one unreadable directory is not a reason to archive nothing.
    pub unreadable: Vec<(PathBuf, String)>,
    /// Project directories the config excluded. Listed for reporting only;
    /// nothing inside them was listed, and nothing inside them was opened.
    pub excluded: Vec<PathBuf>,
}

impl Discovered {
    fn absorb(&mut self, other: Discovered) {
        self.transcripts.extend(other.transcripts);
        self.unreadable.extend(other.unreadable);
        self.excluded.extend(other.excluded);
    }
}

/// Walk every transcript root the config resolved, in configured order.
pub fn discover(config: &Config) -> Discovered {
    let mut found = Discovered::default();
    for root in config.transcript_roots() {
        found.absorb(discover_root(&root, config));
    }
    found
}

/// Walk one transcript root - a Claude config directory's `projects` tree.
///
/// A root that does not exist yields nothing and is not reported: a user who
/// has never run Claude Code has no `projects` directory, and that is a normal
/// state rather than a failure to surface.
pub fn discover_root(root: &Path, config: &Config) -> Discovered {
    let mut found = Discovered::default();

    let canonical = match root.canonicalize() {
        Ok(canonical) => canonical,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return found,
        Err(e) => {
            found.unreadable.push((root.to_path_buf(), e.to_string()));
            return found;
        }
    };

    let entries = match entries(&canonical) {
        Ok(entries) => entries,
        Err(e) => {
            found.unreadable.push((canonical, e.to_string()));
            return found;
        }
    };

    for entry in entries {
        // Only a directory is a project. A stray file at root level is not a
        // transcript at project depth, because there is no project.
        if !entry.is_dir {
            continue;
        }
        // Before anything inside it is listed, let alone opened (D-22). All 822
        // real sidecar files sit under `<project>/<sessionId>/subagents/`, so
        // one directory-level skip covers them too.
        if config.excludes_encoded_dir(&entry.name) {
            found.excluded.push(entry.path);
            continue;
        }
        walk_project(&entry.path, &mut found);
    }

    // Sorted once, at the end: the traversal order is an implementation detail
    // and two passes over one tree must walk it identically.
    found.transcripts.sort();
    found
}

/// Everything inside one project directory.
///
/// The project's own level is where a `<uuid>.jsonl` counts; below it only
/// `agent-*.jsonl` does.
fn walk_project(project: &Path, found: &mut Discovered) {
    let listing = match entries(project) {
        Ok(listing) => listing,
        Err(e) => {
            found
                .unreadable
                .push((project.to_path_buf(), e.to_string()));
            return;
        }
    };

    // An explicit stack rather than recursion: the real tree is six deep, but
    // "unbounded" is the requirement and a hostile or symlinked tree must not
    // be able to exhaust the stack.
    let mut below: Vec<PathBuf> = Vec::new();
    for entry in listing {
        if entry.is_dir {
            below.push(entry.path);
        } else if is_transcript_name(&entry.name, true) {
            found.transcripts.push(entry.path);
        }
    }

    while let Some(dir) = below.pop() {
        let listing = match entries(&dir) {
            Ok(listing) => listing,
            Err(e) => {
                found.unreadable.push((dir, e.to_string()));
                continue;
            }
        };
        for entry in listing {
            if entry.is_dir {
                below.push(entry.path);
            } else if is_transcript_name(&entry.name, false) {
                found.transcripts.push(entry.path);
            }
        }
    }
}

/// Is this file name a transcript, at this level?
///
/// `at_project_depth` is the only thing that distinguishes the two rules: a
/// `<uuid>.jsonl` names a session and only means that directly inside a project
/// directory, while `agent-*.jsonl` names a sidecar wherever it sits.
fn is_transcript_name(name: &str, at_project_depth: bool) -> bool {
    if is_agent_transcript(name) {
        return true;
    }
    at_project_depth && is_uuid_transcript(name)
}

fn is_agent_transcript(name: &str) -> bool {
    // `agent-*.meta.json` fails on the extension, which is the whole reason
    // D-04 can store those bytes without them ever becoming a session.
    name.starts_with("agent-") && name.ends_with(".jsonl")
}

fn is_uuid_transcript(name: &str) -> bool {
    name.strip_suffix(".jsonl").is_some_and(is_uuid)
}

/// `8-4-4-4-12` hex. 1,253 of 1,253 real top-level transcripts match it.
fn is_uuid(s: &str) -> bool {
    let mut groups = s.split('-');
    for width in [8usize, 4, 4, 4, 12] {
        match groups.next() {
            Some(group) if group.len() == width && group.bytes().all(|b| b.is_ascii_hexdigit()) => {
            }
            _ => return false,
        }
    }
    groups.next().is_none()
}

/// One directory entry, reduced to what the filter needs.
struct Entry {
    name: String,
    is_dir: bool,
    path: PathBuf,
}

/// A directory's entries, sorted by name, dot-entries included.
///
/// Sorted because two passes over one tree must walk it identically. Dot
/// directories are descended into: the tree lives under `~/.claude` and its
/// project directories are named for paths that frequently begin with a dot.
///
/// `file_type` is not followed through a symlink. A symlinked directory is
/// therefore not descended, which is what keeps the walk free of cycles; a
/// symlinked file is still a candidate. The real tree contains zero symlinks.
fn entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push(Entry {
            name,
            is_dir,
            path: entry.path(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The one way anything in this crate opens a transcript for reading.
///
/// One function, so AC5's "zero opens of any file under that directory" is a
/// number a test can read rather than a claim about the code. The shipped
/// binary takes no branch for it: [`opened::record`] compiles to nothing
/// without the `testkit` feature.
///
/// The path is recorded *before* the open is attempted, so a file that exists
/// and cannot be read still counts as an open. AC5 is about what verbatim
/// reaches for, not about what it succeeded in reading.
pub fn open_transcript(path: &Path) -> std::io::Result<std::fs::File> {
    opened::record(path);
    std::fs::File::open(path)
}

/// The counted-open log.
///
/// It records **paths** and not a count, and that is the point. "Zero opens of
/// any file under that directory" is not answerable from a scalar: a total
/// could reach the expected number by opening every excluded file and skipping
/// an equal number elsewhere. Tests query by path prefix, so the assertion is
/// attributable; a count is derivable from the log wherever one is wanted.
#[cfg(feature = "testkit")]
pub mod opened {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    static LOG: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    fn log() -> std::sync::MutexGuard<'static, Vec<PathBuf>> {
        // A test that panicked mid-assertion poisons the lock, and a poisoned
        // lock here would turn one failing test into every following test
        // failing for an unrelated reason.
        LOG.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn record(path: &Path) {
        log().push(path.to_path_buf());
    }

    /// Forget every recorded open. Call it at the start of a test that asserts
    /// on the log, never at the end: a test that failed leaves its evidence.
    pub fn reset() {
        log().clear();
    }

    /// Every path opened since the last [`reset`], in order.
    pub fn paths() -> Vec<PathBuf> {
        log().clone()
    }

    /// Every recorded open at or beneath `prefix`.
    pub fn under(prefix: &Path) -> Vec<PathBuf> {
        log()
            .iter()
            .filter(|p| p.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// How many opens were recorded in total.
    pub fn count() -> usize {
        log().len()
    }
}

#[cfg(not(feature = "testkit"))]
pub mod opened {
    use std::path::Path;

    #[inline(always)]
    pub(super) fn record(_path: &Path) {}
}
