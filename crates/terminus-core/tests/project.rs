//! Project identity from a `cwd` (ING-05, D-05, D-07).
//!
//! Degrading is the common case and it is asserted as a normal answer, not as
//! an error: 33 of the 63 distinct `cwd` values in the real corpus no longer
//! exist and 13 of the surviving 30 are not repositories, so a resolver that
//! answered `None` for those would drop half the archive out of every
//! project-scoped search.

use std::path::Path;
use std::process::Command;

use terminus_core::project::{self, Resolver};

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
        terminus_core::config::encode("/x/a.b"),
        terminus_core::config::encode("/x/a-b"),
        "the encoding really is lossy, which is why it is never decoded"
    );
}

// ---------------------------------------------------------------------------
// AC3, through a real pass: the worktree half and the encoding half.
// ---------------------------------------------------------------------------

use std::path::PathBuf;

use rusqlite::Connection;
use terminus_core::config::{self, Config};
use terminus_core::ingest::pass::{self, PassOutcome};
use terminus_core::store::DB_FILE_NAME;

/// A Claude config directory whose `projects` tree the pass walks, plus a data
/// directory. Nothing here resolves a real transcript root.
struct Tree {
    dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
}

fn tree() -> Tree {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Tree {
        dir,
        data_dir,
        claude_dir,
    }
}

impl Tree {
    /// A one-record transcript at `<projects>/<dir_name>/<n>.jsonl`, whose only
    /// record carries `cwd`.
    fn transcript(&self, dir_name: &str, n: u8, cwd: &str) -> PathBuf {
        let dir = self.claude_dir.join("projects").join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{n:08x}-1111-4111-8111-111111111111.jsonl"));
        std::fs::write(&path, record(n, cwd)).unwrap();
        path.canonicalize().unwrap()
    }

    fn append(&self, path: &Path, n: u8, cwd: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(record(n, cwd).as_bytes()).unwrap();
    }

    fn pass(&self) -> pass::Summary {
        let config = Config::from_parts(vec![self.claude_dir.clone()], Vec::new());
        match pass::run_with(&self.data_dir, &config).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// `(project, project_pre_worktree)` for one archived transcript.
    fn keys_of(&self, path: &Path) -> (Option<String>, Option<String>) {
        self.conn()
            .query_row(
                "SELECT project, project_pre_worktree FROM session_meta WHERE session_key = ?1",
                [path.to_str().unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
}

/// A turn record carrying a `cwd`, which is all these tests need from one.
fn record(n: u8, cwd: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"uuid\":\"cccccccc-0000-4000-8000-0000000000{n:02x}\",\
          \"timestamp\":\"2026-08-12T10:00:{n:02}.000Z\",\
          \"sessionId\":\"{n:08x}-2222-4222-8222-222222222222\",\"cwd\":{},\
          \"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hi\"}}]}}}}\n",
        serde_json::to_string(cwd).unwrap()
    )
}

/// AC3's worktree half. A repository `cwd` and a `cwd` under its
/// `.claude/worktrees/` fold to one project key, and both keys survive the
/// worktree directory being deleted afterwards (D-06, ING-05).
///
/// It has to be constructed rather than measured: all 13 real worktree `cwd`
/// values name deleted directories, so git answers for none of them.
#[test]
fn a_repo_and_its_worktree_land_on_one_project_key() {
    let tree = tree();
    let repo = tree.dir.path().join("repo");
    if !git_init(&repo) {
        return;
    }
    let worktree = repo.join(".claude").join("worktrees").join("wt-a");
    std::fs::create_dir_all(&worktree).unwrap();

    let toplevel = text(&repo.canonicalize().unwrap());
    let repo_cwd = text(&repo);
    let worktree_cwd = text(&worktree);

    let plain = tree.transcript(&config::encode(&repo_cwd), 1, &repo_cwd);
    let forked = tree.transcript(&config::encode(&worktree_cwd), 2, &worktree_cwd);

    let summary = tree.pass();
    assert_eq!((summary.files_walked, summary.files_committed), (2, 2));

    assert_eq!(tree.keys_of(&plain), (Some(toplevel.clone()), None));
    assert_eq!(
        tree.keys_of(&forked),
        (Some(toplevel.clone()), Some(worktree_cwd.clone())),
        "the worktree session keys to the repo and keeps the path it came from"
    );

    // The directory goes away and the session grows. A pass that re-derived
    // identity from the disk would now key this session somewhere else; the
    // stored keys are what stop that (D-06), and D-20 keeps the first `cwd`.
    std::fs::remove_dir_all(repo.join(".claude")).unwrap();
    tree.append(&forked, 3, "/somewhere/else/entirely");
    let second = tree.pass();
    assert_eq!(second.files_committed, 1, "the appended record committed");

    assert_eq!(tree.keys_of(&plain), (Some(toplevel.clone()), None));
    assert_eq!(
        tree.keys_of(&forked),
        (Some(toplevel), Some(worktree_cwd)),
        "a deleted worktree directory must not un-key an archived session"
    );
}

/// AC3's encoding half (D-07). Two `cwd` values that encode to the same project
/// directory name - and really do sit in one such directory - stay two keys.
#[test]
fn two_cwds_in_one_encoded_directory_stay_two_project_keys() {
    let tree = tree();
    let encoded = config::encode("/x/a.b");
    assert_eq!(encoded, config::encode("/x/a-b"));

    let dotted = tree.transcript(&encoded, 1, "/x/a.b");
    let hyphenated = tree.transcript(&encoded, 2, "/x/a-b");

    let summary = tree.pass();
    assert_eq!((summary.files_walked, summary.files_committed), (2, 2));

    assert_eq!(tree.keys_of(&dotted), (Some("/x/a.b".to_owned()), None));
    assert_eq!(tree.keys_of(&hyphenated), (Some("/x/a-b".to_owned()), None));
}

/// D-20: a transcript whose `cwd` changes mid-session keeps the first one. 19
/// of 1,253 real transcripts carry more than one distinct `cwd`.
#[test]
fn a_session_whose_cwd_changes_keeps_the_project_it_started_in() {
    let tree = tree();
    let first = "/data/code/first-home";
    let path = tree.transcript(&config::encode(first), 1, first);
    tree.pass();
    assert_eq!(tree.keys_of(&path).0.as_deref(), Some(first));

    tree.append(&path, 2, "/data/code/somewhere-else");
    let summary = tree.pass();
    assert_eq!(summary.files_committed, 1);
    assert_eq!(
        tree.keys_of(&path).0.as_deref(),
        Some(first),
        "a tail pass must not revise the project the first pass established"
    );
}

/// A transcript with no `cwd` at all gets a null project rather than a guess.
/// One real transcript is in exactly this state.
#[test]
fn a_transcript_with_no_cwd_gets_a_null_project() {
    let tree = tree();
    let dir = tree.claude_dir.join("projects").join("-no-cwd-anywhere");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("0000000f-1111-4111-8111-111111111111.jsonl");
    std::fs::write(
        &path,
        "{\"type\":\"user\",\"uuid\":\"dddddddd-0000-4000-8000-000000000001\",\
          \"timestamp\":\"2026-08-12T10:00:00.000Z\"}\n",
    )
    .unwrap();
    let path = path.canonicalize().unwrap();

    assert_eq!(tree.pass().files_committed, 1);
    assert_eq!(tree.keys_of(&path), (None, None));
}
