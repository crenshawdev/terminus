//! Project identity: what a session belongs to, from the record's `cwd`.
//!
//! This module turns a `cwd` string into a project key and does nothing else.
//! It never touches the store, knows nothing about ingest, and every rule in it
//! is a function a unit test can call - which is the point, because project
//! identity is where the incumbent's four identity bugs live
//! (`.planning/PROJECT.md`).
//!
//! Two rules and one refusal.
//!
//! **Git only when the directory still exists (D-05).** If the `cwd` names a
//! directory that is still on disk, `git rev-parse --show-toplevel` runs in it
//! and its answer is the key. Otherwise - and whenever git is absent, the
//! directory is not a repository, git exits non-zero, or git takes longer than
//! [`GIT_BUDGET`] - the key degrades to the `cwd` itself, normalized as a
//! string. Degrading is the COMMON case and is not an error: of the 63 distinct
//! `cwd` values in the real corpus, 33 no longer exist and 13 of the surviving
//! 30 are not repositories, so git answers for 17 of 63. A null project for the
//! other 46 would drop half the archive out of every project-scoped search and
//! every resume brief.
//!
//! **Worktrees fold into their parent repo (D-06).** See [`worktree_parent`].
//! Both keys are kept, because a worktree directory that is later deleted must
//! not un-key a session that is already archived.
//!
//! **The encoded project directory name is never decoded (D-07).** Applying
//! `[^A-Za-z0-9] -> '-'` to a `cwd` reproduces its containing directory name
//! for 1,252 of 1,252 real files, so the encoding is exact and provably lossy:
//! `/data/code/jcrenshaw.dev` and `/data/code/jcrenshaw-dev` encode
//! identically. A decoder would merge two unrelated projects under one key, and
//! that merge is unrecoverable without a reindex. The encoded name is an input
//! to the pre-open exclusion test ([`crate::config::Config::excludes_encoded_dir`])
//! and to nothing else.
//!
//! Answers are memoized per `cwd` string for the lifetime of a [`Resolver`],
//! which is one ingest pass: 1,253 real transcripts carry 63 distinct `cwd`
//! values, and an unmemoized resolver would shell out to git a thousand times
//! for answers it already has.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long `git rev-parse` may take before the answer is abandoned.
///
/// A hung git in one project directory must not hang a pass over two thousand
/// transcripts, and degrading is a normal outcome here rather than a failure -
/// so a slow answer is simply not waited for.
pub const GIT_BUDGET: Duration = Duration::from_secs(2);

/// The path segments a worktree `cwd` carries between the repository and the
/// worktree's own name (D-06).
const WORKTREE_SEGMENTS: [&str; 2] = [".claude", "worktrees"];

/// One `cwd`, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// The project key: what `session_meta.project` holds.
    pub key: String,
    /// The key this `cwd` had **before** worktree folding, when folding
    /// happened at all (D-06, ING-05). Stored beside the folded key so a later
    /// deletion of the worktree directory cannot un-key an already-archived
    /// session, and so the read-side exclusion test can match either path.
    pub pre_worktree: Option<String>,
}

impl Project {
    /// Every path this session is keyed under: the resolved key, then the
    /// pre-mapping one when the two differ.
    ///
    /// The read-side exclusion test needs both, because either one alone leaks
    /// (D-23): a user may reasonably exclude the worktree path or the repository
    /// it folded into.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.key.as_str()).chain(self.pre_worktree.as_deref())
    }
}

/// Resolves `cwd` values to project keys, remembering what it has answered.
///
/// One per pass. The memo is what keeps the git calls proportional to the
/// number of distinct `cwd` values rather than to the number of transcripts.
#[derive(Debug, Default)]
pub struct Resolver {
    memo: HashMap<String, Project>,
    /// `None` until the first lookup, then the absolute path of `git` or
    /// `Some(None)` for "there is no git on this machine".
    git: Option<Option<PathBuf>>,
    git_invocations: usize,
}

impl Resolver {
    pub fn new() -> Resolver {
        Resolver::default()
    }

    /// The project key for a `cwd`, from the memo when it has been asked before.
    pub fn resolve(&mut self, cwd: &str) -> Project {
        if let Some(hit) = self.memo.get(cwd) {
            return hit.clone();
        }
        let project = self.resolve_uncached(cwd);
        self.memo.insert(cwd.to_owned(), project.clone());
        project
    }

    /// How many times this resolver has actually spawned git.
    ///
    /// Exposed because "the same `cwd` resolves twice without running git
    /// twice" is a statement about a number, and a memo asserted from the code
    /// rather than from a counter is a memo nobody checked.
    pub fn git_invocations(&self) -> usize {
        self.git_invocations
    }

    fn resolve_uncached(&mut self, cwd: &str) -> Project {
        let normalized = normalize(cwd);

        // The worktree rule runs BEFORE the on-disk git branch (D-06): the
        // worktree directory may still exist while the parent repository is
        // what the session belongs to. The repository then goes through the
        // same rule as any other `cwd`, so a repo `cwd` and a worktree `cwd`
        // beneath it land on one key rather than on two spellings of it.
        if let Some(repo) = worktree_parent(&normalized) {
            return Project {
                key: self.repo_or_itself(&repo),
                pre_worktree: Some(normalized),
            };
        }

        Project {
            key: self.repo_or_itself(&normalized),
            pre_worktree: None,
        }
    }

    /// The git toplevel for a directory that still exists, or the path itself.
    fn repo_or_itself(&mut self, path: &str) -> String {
        self.git_toplevel(path).unwrap_or_else(|| path.to_owned())
    }

    fn git_toplevel(&mut self, dir: &str) -> Option<String> {
        let dir = Path::new(dir);
        // D-05's gate. A `cwd` whose directory is gone gets no git call at all:
        // git would answer for the process's own directory or fail, and either
        // costs a spawn for an answer that cannot be right.
        if !dir.is_dir() {
            return None;
        }
        let program = self.git_program()?.to_path_buf();
        self.git_invocations += 1;
        git_toplevel_in(&program, dir)
    }

    fn git_program(&mut self) -> Option<&Path> {
        self.git
            .get_or_insert_with(git_program)
            .as_deref()
            .map(Path::new)
    }
}

/// Is there a `git` on this machine?
///
/// For tests, which skip their git-backed cases rather than failing on an image
/// that has no git.
pub fn git_available() -> bool {
    git_program().is_some()
}

/// `git`'s absolute path, found by scanning `PATH` ourselves.
///
/// An absolute program and no shell: the hook path spawns this, and a bare
/// program name resolved by the OS is the Windows PATH-probing failure class
/// the design retires (`.planning/PROJECT.md`).
fn git_program() -> Option<PathBuf> {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Run `git rev-parse --show-toplevel` in `dir`, giving up after [`GIT_BUDGET`].
///
/// Every failure is the same answer - `None`, meaning "degrade" - because a
/// directory that is not a repository, a git that is not installed and a git
/// that hangs all leave the caller with exactly the same thing to do.
///
/// The output is read after the child has exited, which is safe only because
/// `rev-parse --show-toplevel` prints one short line and cannot fill the pipe
/// buffer. `stderr` goes to null: "not a git repository" is an expected answer
/// here, not a message to put in front of a user.
fn git_toplevel_in(program: &Path, dir: &Path) -> Option<String> {
    let mut child = Command::new(program)
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + GIT_BUDGET;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return None,
        }
    }

    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// The repository a worktree `cwd` belongs to (D-06).
///
/// A path of the form `<repo>/.claude/worktrees/<name>`, with or without
/// further path below it, maps to `<repo>`. This is a path-shaped rule and not
/// `git rev-parse --git-common-dir`, and that is measured rather than
/// preferred: every worktree `cwd` in the real corpus has exactly that nested
/// form and all 13 of them are deleted, so git returns nothing for any of them.
///
/// Getting it wrong splits six `cadence`, six `hindsight` and six `assistant`
/// worktrees into seven project keys each - the identity fragmentation
/// `.planning/PROJECT.md` cites the incumbent for.
///
/// The **last** occurrence of the pair wins, so a worktree checked out inside
/// another worktree keys to the nearer repository rather than the outermost.
pub fn worktree_parent(cwd: &str) -> Option<String> {
    let parts: Vec<String> = Path::new(cwd)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    // `<repo>/.claude/worktrees/<name>` needs at least one component before the
    // pair and one after it, so the pair starts no earlier than index 1 and no
    // later than three from the end.
    let at = (1..parts.len().saturating_sub(2))
        .rev()
        .find(|i| parts[*i] == WORKTREE_SEGMENTS[0] && parts[i + 1] == WORKTREE_SEGMENTS[1])?;

    let mut repo = PathBuf::new();
    for part in &parts[..at] {
        repo.push(part);
    }
    Some(repo.to_string_lossy().into_owned())
}

/// A `cwd` canonicalized **by string**: no filesystem access, no symlink
/// resolution.
///
/// Redundant separators, `.` components and a trailing separator disappear, and
/// `..` pops the component before it. Nothing here touches the disk, because
/// half the real `cwd` values name directories that no longer exist and a
/// resolver that needed them to exist would have no key for those sessions at
/// all.
pub fn normalize(cwd: &str) -> String {
    let mut out = PathBuf::new();
    for component in Path::new(cwd).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    let normalized = out.to_string_lossy().into_owned();
    if normalized.is_empty() {
        cwd.to_owned()
    } else {
        normalized
    }
}
