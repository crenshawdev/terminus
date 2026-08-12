//! Exclusion on the READ path (ING-08, D-23).
//!
//! The ingest half - never opening a file under an excluded project - lives in
//! `tests/pass.rs`. This file covers the half that is easy to leave out and
//! that `.planning/PROJECT.md` names the incumbent for: honoring exclusion on
//! write and ignoring it on read.
//!
//! Every test here archives with **no** exclusions configured and only then
//! excludes, because a session archived before its project was excluded is
//! exactly the case a flag written at ingest could never cover.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::config::{self, visible, Config};
use verbatim_core::ingest::pass::{self, PassOutcome};
use verbatim_core::store::DB_FILE_NAME;

struct Tree {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
}

fn tree() -> Tree {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Tree {
        _dir: dir,
        data_dir,
        claude_dir,
    }
}

impl Tree {
    /// A one-record transcript in the project directory `cwd` encodes to.
    fn transcript(&self, n: u8, cwd: &str) -> PathBuf {
        self.write(n, &config::encode(cwd), &record(n, Some(cwd)))
    }

    fn write(&self, n: u8, dir_name: &str, body: &str) -> PathBuf {
        let dir = self.claude_dir.join("projects").join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{n:08x}-1111-4111-8111-111111111111.jsonl"));
        std::fs::write(&path, body).unwrap();
        path.canonicalize().unwrap()
    }

    fn config(&self, exclusions: &[&str]) -> Config {
        Config::from_parts(
            vec![self.claude_dir.clone()],
            exclusions.iter().map(|e| (*e).to_owned()).collect(),
        )
    }

    /// Archive the tree with nothing excluded. Exclusion arrives afterwards.
    fn ingest_everything(&self) -> pass::Summary {
        match pass::run_with(&self.data_dir, &self.config(&[])).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn visible_keys(&self, exclusions: &[&str]) -> Vec<String> {
        visible::sessions(&self.conn(), &self.config(exclusions))
            .unwrap()
            .into_iter()
            .map(|s| s.session_key)
            .collect()
    }

    fn counts(&self, exclusions: &[&str]) -> visible::Counts {
        visible::counts(&self.conn(), &self.config(exclusions)).unwrap()
    }

    /// The row is still in `session_meta` - exclusion hides, it does not delete.
    fn archived(&self, path: &Path) -> bool {
        let rows: i64 = self
            .conn()
            .query_row(
                "SELECT count(*) FROM session_meta WHERE session_key = ?1",
                [path.to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        rows == 1
    }
}

fn record(n: u8, cwd: Option<&str>) -> String {
    let cwd = match cwd {
        Some(cwd) => format!("\"cwd\":{},", serde_json::to_string(cwd).unwrap()),
        None => String::new(),
    };
    format!(
        "{{\"type\":\"user\",\"uuid\":\"cccccccc-0000-4000-8000-0000000000{n:02x}\",\
          \"timestamp\":\"2026-08-12T10:00:{n:02}.000Z\",\
          \"sessionId\":\"{n:08x}-2222-4222-8222-222222222222\",{cwd}\
          \"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hi\"}}]}}}}\n"
    )
}

fn key(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

/// The case a per-session flag could never cover: three projects archived with
/// no exclusions, then one excluded. The counts drop by exactly that project's
/// share, its sessions leave the listing, and its rows are still in the store.
#[test]
fn a_project_excluded_after_ingest_disappears_from_the_read_path() {
    let tree = tree();
    let alpha = tree.transcript(1, "/data/projects/alpha");
    let beta = tree.transcript(2, "/data/projects/beta");
    let sibling = tree.transcript(3, "/data/projects/alpha-research");

    assert_eq!(tree.ingest_everything().files_committed, 3);

    let before = tree.counts(&[]);
    assert_eq!(before.sessions, 3);
    assert!(before.turns >= 3);

    let after = tree.counts(&["/data/projects/alpha"]);
    let alpha_turns = before.turns - after.turns;
    assert_eq!(after.sessions, 2, "one project's sessions are hidden");
    assert_eq!(alpha_turns, 1, "exactly that project's turns went with it");
    assert!(after.watermarks < before.watermarks);

    let visible = tree.visible_keys(&["/data/projects/alpha"]);
    assert!(!visible.contains(&key(&alpha)));
    assert!(visible.contains(&key(&beta)));
    assert!(
        visible.contains(&key(&sibling)),
        "a project that merely shares leading segments stays visible"
    );

    // Hidden, not deleted: the archive is what the exclusion is applied to.
    assert!(tree.archived(&alpha));
}

/// D-23 from the worktree side: either of a session's two project keys may be
/// the one the user excluded, so the predicate is applied to both.
#[test]
fn a_worktree_session_is_hidden_under_either_of_its_two_keys() {
    let tree = tree();
    let repo_cwd = "/data/projects/beta";
    let worktree_cwd = "/data/projects/beta/.claude/worktrees/wt-a";
    let plain = tree.transcript(1, repo_cwd);
    let forked = tree.transcript(2, worktree_cwd);

    assert_eq!(tree.ingest_everything().files_committed, 2);
    let (project, pre): (Option<String>, Option<String>) = tree
        .conn()
        .query_row(
            "SELECT project, project_pre_worktree FROM session_meta WHERE session_key = ?1",
            [key(&forked)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(project.as_deref(), Some(repo_cwd), "it folded to the repo");
    assert_eq!(pre.as_deref(), Some(worktree_cwd));

    // Excluding the worktree path hides only the worktree session - the folded
    // `project` column alone could not have answered this.
    let visible = tree.visible_keys(&[worktree_cwd]);
    assert_eq!(visible, vec![key(&plain)]);

    // Excluding the repository hides both, because the worktree path is
    // beneath it and the subtree match covers it.
    assert!(tree.visible_keys(&[repo_cwd]).is_empty());
}

/// One real transcript carries no `cwd` at all. Nothing can say it is excluded,
/// so it stays visible under any exclusion list.
#[test]
fn a_session_with_a_null_project_stays_visible() {
    let tree = tree();
    let nameless = tree.write(1, "-no-cwd-anywhere", &record(1, None));
    tree.transcript(2, "/data/projects/alpha");

    assert_eq!(tree.ingest_everything().files_committed, 2);
    assert_eq!(
        tree.conn()
            .query_row(
                "SELECT project FROM session_meta WHERE session_key = ?1",
                [key(&nameless)],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );

    for exclusions in [
        vec!["/data/projects/alpha"],
        vec!["/"],
        vec!["/data/projects/alpha", "/data/projects/beta"],
    ] {
        assert!(
            tree.visible_keys(&exclusions).contains(&key(&nameless)),
            "a null project was hidden by {exclusions:?}"
        );
    }
}

/// A session archived without its `session_meta` row is damaged, not excluded:
/// the listing still shows it, so `verify` has something to report against.
#[test]
fn a_session_missing_its_metadata_row_is_still_listed() {
    let tree = tree();
    let orphan = tree.transcript(1, "/data/projects/alpha");
    assert_eq!(tree.ingest_everything().files_committed, 1);
    tree.conn()
        .execute(
            "DELETE FROM session_meta WHERE session_key = ?1",
            [key(&orphan)],
        )
        .unwrap();

    assert_eq!(tree.visible_keys(&[]), vec![key(&orphan)]);
    assert_eq!(tree.counts(&["/data/projects/alpha"]).sessions, 1);
}

/// With nothing excluded the boundary is a plain listing: every archived
/// session, in ingest order.
#[test]
fn with_no_exclusions_every_session_is_visible() {
    let tree = tree();
    let paths: Vec<PathBuf> = (1..=3)
        .map(|n| tree.transcript(n, &format!("/data/projects/p{n}")))
        .collect();
    assert_eq!(tree.ingest_everything().files_committed, 3);

    assert_eq!(
        tree.visible_keys(&[]),
        paths.iter().map(|p| key(p)).collect::<Vec<_>>()
    );
    let counts = tree.counts(&[]);
    assert_eq!(counts.sessions, 3);
    assert_eq!(counts.watermarks, 3);
    assert!(counts.watermark_bytes > 0);
}
