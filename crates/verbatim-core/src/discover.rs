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
    /// Project directories the config excluded, plus any symlinked transcript
    /// that reached into one. Listed for reporting only; nothing named here was
    /// listed, and nothing named here was opened.
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
        walk_project(&entry.path, config, &mut found);
    }

    // Sorted once, at the end: the traversal order is an implementation detail
    // and two passes over one tree must walk it identically.
    found.transcripts.sort();
    found
}

/// The excluded project directory a named transcript sits inside, if any.
///
/// The walk above answers this at the project directory it is about to descend
/// into, and never looks again. `ingest::run` is handed one path with no walk
/// behind it, so it has to find that directory itself - and it has to, because
/// exclusion that holds only on the tree pass is exclusion a single
/// `verbatim ingest <path>` walks around, which is the read-then-filter ING-08
/// forbids.
///
/// A project directory is an entry directly inside a configured transcript
/// root, so that is where the name comes from when the transcript is under one.
/// When it is under none - the crash harness and the lock race both name files
/// in temporary directories - every ancestor's name is tested instead. That
/// cannot be more precise, and being imprecise toward "do not read" is the
/// direction [`Config::excludes_encoded_dir`] already chose.
///
/// Reads no transcript and opens nothing: directory names and, inside the
/// encoded test, directory entries.
pub fn excluded_project_of(config: &Config, transcript: &Path) -> Option<PathBuf> {
    // A real filesystem path is available here, so the exact test answers first
    // (D-23). The encoded tests below only ever see a project DIRECTORY name -
    // `-data-projects-cadence` - and a path that is literally inside the
    // excluded project, which is what a link resolves to and what a user names
    // by hand, matches none of them.
    if config.excludes_path(transcript) {
        return transcript.parent().map(Path::to_path_buf);
    }

    for root in config.transcript_roots() {
        let Ok(canonical) = root.canonicalize() else {
            continue;
        };
        let Ok(rest) = transcript.strip_prefix(&canonical) else {
            continue;
        };
        let Some(project) = rest.components().next() else {
            continue;
        };
        // Under a root, the project directory is the only candidate: whatever
        // the encoded test says about it is the answer, negative included.
        let name = project.as_os_str().to_string_lossy();
        return config
            .excludes_encoded_dir(&name)
            .then(|| canonical.join(project));
    }

    // The ancestor test below reads directory NAMES, and the filesystem root
    // has none - so the one exclusion that means "read nothing" is the one that
    // would slip through a transcript sitting directly in it.
    if config.excludes_everything() {
        return transcript.parent().map(Path::to_path_buf);
    }

    // `ancestors()` starts at the path itself, which is the transcript file.
    transcript
        .ancestors()
        .skip(1)
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| config.excludes_encoded_dir(&name.to_string_lossy()))
        })
        .map(Path::to_path_buf)
}

/// Everything inside one project directory.
///
/// The project's own level is where a `<uuid>.jsonl` counts; below it only
/// `agent-*.jsonl` does.
///
/// The config comes along for the symlinks. A project directory's exclusion is
/// decided from its name before this is called, which is sound for every entry
/// whose bytes are actually inside it - and a symlinked transcript's are not.
/// A `<uuid>.jsonl` link in an unexcluded project pointing at a transcript in
/// an excluded one is otherwise walked, opened and archived, because the name
/// that was tested is the link's.
fn walk_project(project: &Path, config: &Config, found: &mut Discovered) {
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
            keep_transcript(entry, config, found);
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
                keep_transcript(entry, config, found);
            }
        }
    }
}

/// Yield one transcript, unless it is a link into a project the config
/// excludes.
///
/// Only a link is resolved. Canonicalizing every transcript would be two
/// thousand syscalls for an answer D-17 already has - the root is canonical and
/// the walk joined onto it - and the real tree contains zero symlinks, so this
/// costs nothing there. A link that cannot be resolved is not followed: an
/// unanswerable question at an exclusion boundary is the one
/// [`Config::excludes_encoded_dir`] already answers "do not read".
fn keep_transcript(entry: Entry, config: &Config, found: &mut Discovered) {
    if !entry.is_symlink {
        found.transcripts.push(entry.path);
        return;
    }
    match entry.path.canonicalize() {
        Ok(target) => match excluded_project_of(config, &target) {
            Some(_) => found.excluded.push(entry.path),
            None => found.transcripts.push(entry.path),
        },
        Err(e) => found.unreadable.push((entry.path, e.to_string())),
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
    /// The entry is itself a link. `is_dir` is read WITHOUT following it, so a
    /// symlink to a directory is not descended (that is what keeps the walk
    /// free of cycles) and a symlink to a file still looks like a file - which
    /// is the one way a path outside this project directory can be reached
    /// from inside it.
    is_symlink: bool,
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
        let file_type = entry.file_type();
        let is_dir = file_type.as_ref().map(|t| t.is_dir()).unwrap_or(false);
        let is_symlink = file_type.as_ref().map(|t| t.is_symlink()).unwrap_or(true);
        out.push(Entry {
            name,
            is_dir,
            is_symlink,
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
