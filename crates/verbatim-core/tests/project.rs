//! Project identity from a `cwd` (ING-05, D-05, D-07).
//!
//! Degrading is the common case and it is asserted as a normal answer, not as
//! an error: 33 of the 63 distinct `cwd` values in the real corpus no longer
//! exist and 13 of the surviving 30 are not repositories, so a resolver that
//! answered `None` for those would drop half the archive out of every
//! project-scoped search.

use std::path::Path;
use std::process::Command;

use verbatim_core::project::{self, Resolver};

/// A real `git init` repository at `dir`, or `false` when there is no git.
fn git_init(dir: &Path) -> bool {
    if !project::git_available() {
        println!("skipped: no `git` on PATH");
        return false;
    }
    std::fs::create_dir_all(dir).unwrap();
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir)
        .status()
        .expect("spawn git init");
    assert!(status.success(), "git init failed in {}", dir.display());
    true
}

fn text(path: &Path) -> String {
    path.to_str().expect("test paths are UTF-8").to_owned()
}

/// D-05's git branch: a `cwd` inside a repository keys to the repository, and a
/// `cwd` two directories deeper keys to the same one.
#[test]
fn a_cwd_inside_a_repository_resolves_to_its_toplevel() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    if !git_init(&repo) {
        return;
    }
    let deep = repo.join("crates").join("core");
    std::fs::create_dir_all(&deep).unwrap();

    // git prints the real path, so the expectation is the canonical one.
    let toplevel = text(&repo.canonicalize().unwrap());

    let mut resolver = Resolver::new();
    assert_eq!(resolver.resolve(&text(&repo)).key, toplevel);
    assert_eq!(
        resolver.resolve(&text(&deep)).key,
        toplevel,
        "a subdirectory of a repository belongs to the repository"
    );
}

/// The 52% case: the directory is gone. The key is the `cwd` string itself, and
/// never `None` - a null project drops the session out of every read path.
#[test]
fn a_cwd_that_does_not_exist_resolves_to_itself() {
    let mut resolver = Resolver::new();
    let gone = "/data/code/deleted-months-ago";

    assert_eq!(resolver.resolve(gone).key, gone);
    assert_eq!(
        resolver.git_invocations(),
        0,
        "a directory that is gone must cost no git spawn"
    );
}

/// The other degrade: the directory exists and is not a repository.
#[test]
fn an_existing_non_repository_directory_resolves_to_itself() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("not-a-repo");
    std::fs::create_dir_all(&plain).unwrap();

    let mut resolver = Resolver::new();
    assert_eq!(resolver.resolve(&text(&plain)).key, text(&plain));
}

/// Memoization, measured rather than argued: 1,253 real transcripts carry 63
/// distinct `cwd` values, so an unmemoized resolver shells out a thousand times
/// for answers it already has.
#[test]
fn resolving_the_same_cwd_twice_runs_git_at_most_once() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    if !git_init(&repo) {
        return;
    }

    let mut resolver = Resolver::new();
    let first = resolver.resolve(&text(&repo));
    let after_one = resolver.git_invocations();
    assert_eq!(after_one, 1, "the first resolve must actually ask git");

    for _ in 0..5 {
        assert_eq!(resolver.resolve(&text(&repo)), first);
    }
    assert_eq!(
        resolver.git_invocations(),
        after_one,
        "a repeated cwd must be answered from the memo"
    );
}

/// The degrade path is memoized too, so a tree of 1,900 sessions under 33
/// deleted directories costs 33 `is_dir` checks and not 1,900.
#[test]
fn a_degraded_answer_is_memoized_as_well() {
    let mut resolver = Resolver::new();
    let gone = "/data/code/deleted-months-ago";
    assert_eq!(resolver.resolve(gone), resolver.resolve(gone));
}

/// The key is canonicalized by string and never by the filesystem: the common
/// case is a path that no longer exists, which `Path::canonicalize` cannot
/// answer for at all.
#[test]
fn a_cwd_is_normalized_as_a_string() {
    assert_eq!(
        project::normalize("/data//code/./verbatim/"),
        "/data/code/verbatim"
    );
    assert_eq!(
        project::normalize("/data/code/x/../verbatim"),
        "/data/code/verbatim"
    );

    let mut resolver = Resolver::new();
    assert_eq!(
        resolver.resolve("/data/code/gone/./sub/").key,
        "/data/code/gone/sub"
    );
}

/// D-07 from the other side: two `cwd` values that encode to the same project
/// directory name are two different project keys. Nothing ever runs the
/// encoding backwards.
#[test]
fn two_cwds_that_encode_identically_stay_two_keys() {
    let mut resolver = Resolver::new();
    let dotted = resolver.resolve("/x/a.b");
    let hyphenated = resolver.resolve("/x/a-b");

    assert_ne!(dotted.key, hyphenated.key);
    assert_eq!(
        verbatim_core::config::encode("/x/a.b"),
        verbatim_core::config::encode("/x/a-b"),
        "the encoding really is lossy, which is why it is never decoded"
    );
}
