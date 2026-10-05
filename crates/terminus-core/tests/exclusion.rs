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
use terminus_core::config::{self, visible, Config};
use terminus_core::ingest::pass::{self, PassOutcome};
use terminus_core::recall::{search, Query, Request, Response, Scope, MAX_RESULTS};
use terminus_core::store::DB_FILE_NAME;

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

    /// One search over this tree, run the way a read command runs it.
    ///
    /// The read path this file is about now has two halves - listing sessions
    /// and searching turns - and D-21 routes both through `config::visible`.
    /// A search that honored exclusion by a rule of its own is the drift the
    /// module note forbids, so the search is asserted here beside the listing.
    fn found(&self, exclusions: &[&str], scope: Scope, raw: &str) -> Response {
        search::run(
            &self.conn(),
            &self.config(exclusions),
            &Request::new(Query::parse(raw), scope).limit(MAX_RESULTS),
        )
        .unwrap()
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

/// The distinct projects a search answered from, in sorted order.
fn projects_of(response: &Response) -> Vec<String> {
    let mut out: Vec<String> = response
        .hits
        .iter()
        .map(|hit| hit.project.clone().unwrap_or_default())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The search half of the same retroactive rule: a project excluded after its
/// sessions were archived stops answering, under the default scope and under
/// `*` alike.
#[test]
fn an_excluded_project_disappears_from_search_too() {
    let tree = tree();
    tree.transcript(1, "/data/projects/alpha");
    tree.transcript(2, "/data/projects/beta");
    tree.transcript(3, "/data/projects/alpha-research");
    assert_eq!(tree.ingest_everything().files_committed, 3);

    // Scoped to one project, nothing excluded: only that project answers,
    // though every session in the tree carries the queried word.
    let scoped = tree.found(&[], Scope::parse("/data/projects/alpha"), "hi");
    assert_eq!(
        projects_of(&scoped),
        vec!["/data/projects/alpha".to_owned()]
    );
    assert!(
        !projects_of(&scoped).contains(&"/data/projects/alpha-research".to_owned()),
        "a project that merely shares leading segments is a different project"
    );

    // `*` is every project, and this is the control for what exclusion removes.
    let everything = tree.found(&[], Scope::Everything, "hi");
    assert_eq!(
        projects_of(&everything),
        vec![
            "/data/projects/alpha".to_owned(),
            "/data/projects/alpha-research".to_owned(),
            "/data/projects/beta".to_owned(),
        ]
    );

    // Excluded afterwards: gone from `*` as well as from its own scope.
    let excluded = ["/data/projects/alpha"];
    let after = tree.found(&excluded, Scope::Everything, "hi");
    assert_eq!(
        projects_of(&after),
        vec![
            "/data/projects/alpha-research".to_owned(),
            "/data/projects/beta".to_owned(),
        ]
    );

    let scoped_after = tree.found(&excluded, Scope::parse("/data/projects/alpha"), "hi");
    assert!(scoped_after.hits.is_empty());
    assert!(
        scoped_after.reason.is_some(),
        "an excluded scope is an empty result with a reason, not a silent one"
    );
}

/// D-23 from the worktree side, on the search path: a session folded into its
/// parent repo is still hidden by excluding the worktree path alone.
///
/// `project` alone could not answer this - it holds the repo for both sessions -
/// so a search filtering on one column would return the worktree session's
/// turns after the user excluded exactly that directory.
#[test]
fn a_worktree_session_is_hidden_from_search_under_either_key() {
    let tree = tree();
    let repo_cwd = "/data/projects/beta";
    let worktree_cwd = "/data/projects/beta/.claude/worktrees/wt-a";
    let plain = tree.transcript(1, repo_cwd);
    let forked = tree.transcript(2, worktree_cwd);
    assert_eq!(tree.ingest_everything().files_committed, 2);

    // Both sessions carry the same `project`, which is the point.
    let both = tree.found(&[], Scope::Everything, "hi");
    let keys: Vec<String> = both.hits.iter().map(|h| h.session_key.clone()).collect();
    assert!(
        keys.contains(&key(&plain)) && keys.contains(&key(&forked)),
        "{keys:?}"
    );
    assert_eq!(projects_of(&both), vec![repo_cwd.to_owned()]);

    // Excluding the worktree path hides only the worktree session's turns.
    let after = tree.found(&[worktree_cwd], Scope::Everything, "hi");
    let keys: Vec<String> = after.hits.iter().map(|h| h.session_key.clone()).collect();
    assert_eq!(keys, vec![key(&plain)]);

    // Excluding the repository hides both, under the other key.
    assert!(tree
        .found(&[repo_cwd], Scope::Everything, "hi")
        .hits
        .is_empty());
}
