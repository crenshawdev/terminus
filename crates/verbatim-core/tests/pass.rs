//! The tree pass: one bad file does not stop it, and an excluded project is
//! never opened.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::config::{Config, CONFIG_FILE_NAME};
use verbatim_core::discover;
use verbatim_core::ingest::pass::{self, PassOutcome, Summary};
use verbatim_core::retention::{Applied, MAX_PER_PASS};
use verbatim_core::store::{Store, DB_FILE_NAME};
use verbatim_core::{ingest, testkit};

const PROJECT: &str = "-data-projects-cadence";
const SESSION_DIR: &str = "33333333-3333-4333-8333-333333333333";

/// A data directory plus a Claude config directory whose `projects` tree the
/// pass walks. Nothing here ever resolves a real transcript root.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        claude_dir,
    }
}

impl Bench {
    fn projects(&self) -> PathBuf {
        self.claude_dir.join("projects")
    }

    fn project(&self, name: &str) -> PathBuf {
        let dir = self.projects().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Copy a fixture into a project directory under a transcript-shaped name.
    fn place(&self, project: &str, name: &str, fixture: &str) -> PathBuf {
        let dest = self.project(project).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    fn config(&self, exclusions: &[&str]) -> Config {
        Config::from_parts(
            vec![self.claude_dir.clone()],
            exclusions.iter().map(|e| (*e).to_owned()).collect(),
        )
    }

    fn pass(&self, exclusions: &[&str]) -> Summary {
        match pass::run_with(&self.data_dir, &self.config(exclusions)).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// A config over this bench's root, plus whatever `verbatim.toml` body the
    /// test needs. `Config::from_parts` cannot carry a `[retention]` table -
    /// nothing in this workspace builds one in memory, because nothing writes
    /// this file - so a retention test loads a real file like a user would.
    ///
    /// A TOML literal string for the root, so a Windows path is not read as a
    /// run of escapes.
    fn config_file(&self, body: &str) -> Config {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            format!("roots = ['{}']\n{body}", self.claude_dir.display()),
        )
        .unwrap();
        Config::load_from(dir.path()).unwrap()
    }

    fn pass_with(&self, config: &Config) -> Summary {
        match pass::run_with(&self.data_dir, config).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }
}

/// Every session's blob length and eviction flag, in ingest order.
fn blobs(conn: &Connection) -> Vec<(i64, Option<i64>)> {
    conn.prepare(
        "SELECT length(s.blob), m.is_evicted FROM sessions s
         JOIN session_meta m USING (session_key) ORDER BY s.session_no",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn uuid(n: u8) -> String {
    format!("{n:08x}-1111-4111-8111-111111111111")
}

fn key(conn: &Connection, path: &Path) -> Option<String> {
    conn.query_row(
        "SELECT session_key FROM sessions WHERE session_key = ?1",
        [path.to_str().unwrap()],
        |r| r.get(0),
    )
    .ok()
}

fn turns_for(conn: &Connection, path: &Path) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM turns WHERE session_key = ?1",
        [path.to_str().unwrap()],
        |r| r.get(0),
    )
    .unwrap()
}

/// D-12, at the scale it matters. Six transcripts, two of them damaged in the
/// two ways phase 1 made fatal, one pass: the other four archive and the
/// summary names exactly the two that did not.
///
/// Either failure propagating would let one damaged session wedge every future
/// ingest of every other transcript in the tree - the failure gate fix
/// `3e5d9ff` closed for `reindex`, which costs far more here.
#[test]
fn one_damaged_transcript_does_not_stop_the_rest_of_the_tree() {
    let bench = bench();

    // Two files that will be damaged, ingested first so they have a
    // `session_meta` row and a watermark to damage.
    let orphaned = bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );
    let shrunk = bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(2)),
        "session-large-record.jsonl",
    );
    for path in [&orphaned, &shrunk] {
        match ingest::run(&bench.data_dir, path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{other:?}"),
        }
    }

    // Damage one: archived, with its metadata row gone. `Existing::read`
    // refuses rather than rewriting the blob from the tail.
    bench
        .conn()
        .execute(
            "DELETE FROM session_meta WHERE session_key = ?1",
            [orphaned.to_str().unwrap()],
        )
        .unwrap();
    // Damage the other: shorter on disk than its stored watermark. `read_tail`
    // refuses rather than guessing which bytes are still ours.
    std::fs::write(&shrunk, b"{}\n").unwrap();

    // Four healthy transcripts, none of them ingested yet.
    let healthy = vec![
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(3)),
            "session-basic.jsonl",
        ),
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(4)),
            "session-continuation.jsonl",
        ),
        bench.place(
            PROJECT,
            &format!("{SESSION_DIR}/subagents/agent-a.jsonl"),
            "subagents/agent-alpha.jsonl",
        ),
        bench.place(
            PROJECT,
            &format!("{SESSION_DIR}/subagents/workflows/wf_x/agent-deep.jsonl"),
            "subagents/workflows/wf_demo/agent-deep.jsonl",
        ),
    ];

    let summary = bench.pass(&[]);
    assert_eq!(summary.files_walked, 6);
    assert_eq!(summary.files_committed, 4);

    let mut failed: Vec<PathBuf> = summary.failures.iter().map(|(p, _)| p.clone()).collect();
    failed.sort();
    let mut expected = vec![orphaned.clone(), shrunk.clone()];
    expected.sort();
    assert_eq!(failed, expected, "exactly the two damaged files");

    let reasons: Vec<&str> = summary.failures.iter().map(|(_, r)| r.as_str()).collect();
    assert!(
        reasons.iter().any(|r| r.contains("session_meta")),
        "the orphaned session's reason must name what is missing: {reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("watermark")),
        "the short file's reason must name the watermark: {reasons:?}"
    );

    let conn = bench.conn();
    for path in &healthy {
        assert_eq!(key(&conn, path).as_deref(), path.to_str());
        assert!(
            turns_for(&conn, path) > 0,
            "{} archived no turns",
            path.display()
        );
    }
}

/// AC5's ingest half. An excluded project records zero opens of anything
/// beneath it and adds no row to any table, while the sibling whose encoded
/// name merely extends the excluded one is ingested normally.
///
/// The counted-open log is queried by path prefix rather than by total, because
/// a total could reach the expected number by opening every excluded file and
/// skipping an equal number elsewhere.
#[test]
fn an_excluded_project_is_never_opened_and_adds_no_row() {
    let bench = bench();

    // The real tree the encoded project names describe. The pre-open test
    // resolves an extension of an excluded name against the filesystem, so the
    // repository has to exist for the sibling beside it to be told apart from a
    // subdirectory of it.
    let tmp = bench
        .projects()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let repo = tmp.join("data").join("projects").join("cadence");
    std::fs::create_dir_all(repo.join(".claude").join("worktrees").join("wt-a")).unwrap();
    std::fs::create_dir_all(tmp.join("data").join("projects").join("cadence-research")).unwrap();

    let encode = |path: &std::path::Path| verbatim_core::config::encode(path.to_str().unwrap());
    let project = encode(&repo);
    let sibling = encode(&tmp.join("data").join("projects").join("cadence-research"));
    let worktree = encode(&repo.join(".claude").join("worktrees").join("wt-a"));

    let excluded = vec![
        bench.place(
            &project,
            &format!("{}.jsonl", uuid(1)),
            "session-basic.jsonl",
        ),
        bench.place(
            &project,
            &format!("{SESSION_DIR}/subagents/agent-a.jsonl"),
            "subagents/agent-alpha.jsonl",
        ),
        bench.place(
            &worktree,
            &format!("{}.jsonl", uuid(2)),
            "session-continuation.jsonl",
        ),
    ];
    let kept = bench.place(
        &sibling,
        &format!("{}.jsonl", uuid(3)),
        "session-large-record.jsonl",
    );

    let summary = bench.pass(&[repo.to_str().unwrap()]);
    assert_eq!(summary.files_walked, 1, "only the sibling was walked");
    assert_eq!(summary.files_committed, 1);
    assert!(summary.failures.is_empty(), "{:?}", summary.failures);

    for project in [project.as_str(), worktree.as_str()] {
        let dir = bench.projects().join(project).canonicalize().unwrap();
        assert!(
            discover::opened::under(&dir).is_empty(),
            "{project} was opened: {:?}",
            discover::opened::under(&dir)
        );
    }

    let conn = bench.conn();
    for path in &excluded {
        let key = path.to_str().unwrap();
        for (table, column) in [
            ("sessions", "session_key"),
            ("session_meta", "session_key"),
            ("turns", "session_key"),
            ("watermarks", "transcript_path"),
        ] {
            let rows: i64 = conn
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE {column} = ?1"),
                    [key],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(rows, 0, "{table} holds a row for an excluded transcript");
        }
    }
    let fts: i64 = conn
        .query_row("SELECT count(*) FROM turns_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        fts,
        turns_for(&conn, &kept),
        "turns_fts holds rows the excluded projects contributed"
    );

    // The sibling is there in full, and was opened.
    assert_eq!(key(&conn, &kept).as_deref(), kept.to_str());
    assert!(turns_for(&conn, &kept) > 0);
    assert!(!discover::opened::under(&kept).is_empty());
}

/// A pass over a tree it has already ingested walks every file and commits
/// none: nothing is past its watermark. The steady state of a hook-spawned run.
#[test]
fn a_second_pass_over_an_unchanged_tree_commits_nothing() {
    let bench = bench();
    bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );
    bench.place(
        PROJECT,
        &format!("{SESSION_DIR}/subagents/agent-a.jsonl"),
        "subagents/agent-alpha.jsonl",
    );

    let first = bench.pass(&[]);
    assert_eq!((first.files_walked, first.files_committed), (2, 2));

    let second = bench.pass(&[]);
    assert_eq!(second.files_walked, 2);
    assert_eq!(second.files_committed, 0);
    assert_eq!(second.files_unchanged(), 2);
    assert!(second.failures.is_empty());
}

/// The lock is taken once for the whole pass, not once per file.
#[test]
fn a_pass_reports_the_lock_rather_than_waiting_for_it() {
    let bench = bench();
    bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );

    let guard = match verbatim_core::ingest::lock::try_acquire(&bench.data_dir).unwrap() {
        verbatim_core::ingest::Attempt::Acquired(guard) => guard,
        other => panic!("the lock should have been free: {other:?}"),
    };
    assert_eq!(
        pass::run_with(&bench.data_dir, &bench.config(&[])).unwrap(),
        PassOutcome::LockHeld
    );
    drop(guard);

    let summary = bench.pass(&[]);
    assert_eq!(summary.files_committed, 1);
}

fn runs(conn: &Connection) -> Vec<(i64, i64, i64, i64, Option<String>)> {
    conn.prepare(
        "SELECT files_seen, files_committed, files_failed, turns_added, error
         FROM runs ORDER BY id",
    )
    .unwrap()
    .query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Five transcripts in, exactly one `runs` row out (D-10).
///
/// Phase 1's `record_run` inserts a literal `1` for `files_seen` from inside
/// the per-file transaction; reused unchanged by a tree walk it would write
/// roughly 2,071 rows per hook-triggered pass, and ING-09's "the last ingest
/// run" would name one file rather than the pass.
#[test]
fn one_pass_over_five_transcripts_writes_exactly_one_runs_row() {
    let bench = bench();
    for (n, fixture) in [
        "session-basic.jsonl",
        "session-large-record.jsonl",
        "session-continuation.jsonl",
        "subagents/agent-alpha.jsonl",
        "subagents/workflows/wf_demo/agent-deep.jsonl",
    ]
    .into_iter()
    .enumerate()
    {
        bench.place(PROJECT, &format!("{}.jsonl", uuid(n as u8 + 1)), fixture);
    }

    let summary = bench.pass(&[]);
    assert_eq!((summary.files_walked, summary.files_committed), (5, 5));

    let rows = runs(&bench.conn());
    assert_eq!(rows.len(), 1, "one row per pass, whatever the file count");
    let (seen, committed, failed, turns, error) = rows[0].clone();
    assert_eq!(seen, 5, "files_seen is what the pass walked");
    assert_eq!(committed, 5);
    assert_eq!(failed, 0);
    assert_eq!(turns as usize, summary.turns_added);
    assert_eq!(error, None, "a clean pass has nothing to say");

    // A second pass finds nothing new and still moves "the last ingest run".
    bench.pass(&[]);
    let rows = runs(&bench.conn());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].0, 5, "walked again");
    assert_eq!(rows[1].1, 0, "committed nothing");
}

/// D-14. A pass that dies mid-walk still leaves one `runs` row carrying its
/// error - with no log file by design, that row is the only place the failure
/// can be seen.
#[test]
fn a_pass_that_dies_mid_walk_still_leaves_its_runs_row() {
    let bench = bench();
    for n in 1..=4u8 {
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(n)),
            "session-basic.jsonl",
        );
    }

    // The fault lives in this bench's own data directory, so it cannot reach a
    // test running beside it.
    std::fs::create_dir_all(&bench.data_dir).unwrap();
    std::fs::write(
        bench
            .data_dir
            .join(verbatim_core::ingest::fault::PASS_FAIL_AFTER_FILE),
        b"2",
    )
    .unwrap();

    let err = pass::run_with(&bench.data_dir, &bench.config(&[]))
        .expect_err("the armed fault must fail the pass");
    assert!(err.to_string().contains("pass fault"), "{err}");

    let rows = runs(&bench.conn());
    assert_eq!(rows.len(), 1, "the dead pass wrote no row");
    let (seen, committed, _, _, error) = rows[0].clone();
    assert_eq!(seen, 2, "it walked two files before it died");
    assert_eq!(committed, 2);
    let error = error.expect("a failed pass must carry its error");
    assert!(error.contains("pass failed"), "{error}");
    assert!(error.contains("pass fault"), "{error}");

    // And the two files it did commit are archived: the row is written after
    // the per-file transactions, not instead of them.
    let sessions: i64 = bench
        .conn()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 2);
}

/// A tree with one damaged file: one row, its error naming that path, and a
/// committed count one short of the walked count.
#[test]
fn a_damaged_file_shows_up_in_the_passs_own_runs_row() {
    let bench = bench();
    let shrunk = bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );
    match ingest::run(&bench.data_dir, &shrunk).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("{other:?}"),
    }
    std::fs::write(&shrunk, b"{}\n").unwrap();
    bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(2)),
        "session-continuation.jsonl",
    );

    let before = runs(&bench.conn()).len();
    let summary = bench.pass(&[]);
    assert_eq!(summary.failures.len(), 1);

    let rows = runs(&bench.conn());
    assert_eq!(rows.len(), before + 1, "one row for the pass");
    let (seen, committed, failed, _, error) = rows.last().unwrap().clone();
    assert_eq!(seen, 2);
    assert_eq!(committed, seen - 1, "one file short of what was walked");
    assert_eq!(failed, 1);
    let error = error.expect("a skipped file must be named");
    assert!(error.contains(shrunk.to_str().unwrap()), "{error}");
    assert!(error.contains("watermark"), "{error}");
}

/// The exclusion the pass honors is not one `verbatim ingest <path>` walks
/// around.
///
/// The tree pass decides exclusion at the project directory it declines to
/// descend into, and the single-file entry point has no walk behind it to
/// decide anything. Left to discovery alone, naming the file by hand - which is
/// the shape phase 4's hooks call - reads and archives a project the user said
/// never to read, and only the read path hides it afterwards. That is
/// read-then-filter, which ING-08 forbids.
#[test]
fn naming_an_excluded_transcript_by_hand_reads_nothing() {
    let bench = bench();
    let path = bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );

    let config = bench.config(&["/data/projects/cadence"]);
    assert_eq!(
        ingest::run_with(&bench.data_dir, &path, &config).unwrap(),
        ingest::Outcome::Excluded(bench.projects().canonicalize().unwrap().join(PROJECT)),
    );

    assert!(
        discover::opened::under(&path).is_empty(),
        "the transcript was opened: {:?}",
        discover::opened::under(&path)
    );
    assert!(
        !bench.data_dir.join(DB_FILE_NAME).exists(),
        "an excluded transcript wrote a store"
    );

    // The same call with nothing excluded is the control: the file is a real
    // transcript and the refusal above is the exclusion, not the fixture.
    match ingest::run_with(&bench.data_dir, &path, &bench.config(&[])).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("the same transcript did not commit unexcluded: {other:?}"),
    }
    assert!(!discover::opened::under(&path).is_empty());
}

/// A transcript outside every configured root is still tested, by name, at each
/// of its ancestors.
///
/// There is no project directory to identify when nothing is under a root, and
/// the crash harness and the lock race both name files in temporary
/// directories. Imprecise toward "do not read" is the direction the pre-open
/// test already chose.
#[test]
fn an_excluded_transcript_under_no_configured_root_is_still_refused() {
    let bench = bench();
    let outside = bench.data_dir.parent().unwrap().join("elsewhere");
    let project = outside.join(PROJECT);
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{}.jsonl", uuid(1)));
    std::fs::copy(testkit::fixture_path("session-basic.jsonl"), &path).unwrap();
    let path = path.canonicalize().unwrap();

    let config = bench.config(&["/data/projects/cadence"]);
    assert!(matches!(
        ingest::run_with(&bench.data_dir, &path, &config).unwrap(),
        ingest::Outcome::Excluded(_)
    ));
    assert!(discover::opened::under(&path).is_empty());
}

/// An exclusion spelled with a trailing separator is the same exclusion.
///
/// It is `excludes_path`'s spelling either way, so before normalization this
/// pass archived the project in full while every read path hid it - the exact
/// write-then-hide split `.planning/PROJECT.md` names the incumbent for.
#[test]
fn an_exclusion_with_a_trailing_separator_is_honored_before_the_open() {
    let bench = bench();
    let path = bench.place(
        PROJECT,
        &format!("{}.jsonl", uuid(1)),
        "session-basic.jsonl",
    );

    let summary = bench.pass(&["/data/projects/cadence/"]);
    assert_eq!(summary.files_walked, 0, "the excluded project was walked");
    assert_eq!(
        summary.excluded,
        vec![bench.projects().canonicalize().unwrap().join(PROJECT)]
    );
    assert!(
        discover::opened::under(&path).is_empty(),
        "the transcript was opened: {:?}",
        discover::opened::under(&path)
    );
}

// RET-01 and RET-03: retention runs as a bounded step at the end of every pass,
// and reports through `runs.error` because there is no log file.

const EVICT: &str = "[retention]\naction = \"evict\"\nage_days = 1\n";
/// Comfortably outside any window these tests configure.
const AGED: &str = "2020-01-01T00:00:00.000Z";

/// Close and age every archived session, which is what a corpus older than any
/// plausible default looks like from `session_meta`.
fn age_everything(conn: &Connection) {
    conn.execute(
        "UPDATE session_meta SET is_final = 1, last_turn_at = ?1",
        [AGED],
    )
    .unwrap();
}

/// AC1. The corpus predates any default, nothing configures retention, and the
/// pass leaves every blob where it was and `runs.error` null. A note here would
/// be a pass telling a user about a feature they never turned on.
#[test]
fn a_pass_with_no_retention_configured_touches_nothing_and_says_nothing() {
    let bench = bench();
    for n in 1..=3 {
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(n)),
            "session-basic.jsonl",
        );
    }
    bench.pass(&[]);
    age_everything(&bench.conn());

    let summary = bench.pass(&[]);
    assert_eq!(summary.retention, Applied::default());
    assert!(summary.retention.lines().is_empty());

    for (len, evicted) in blobs(&bench.conn()) {
        assert!(len > 0, "a blob was emptied with retention off");
        assert_eq!(
            evicted, None,
            "a session was marked evicted with retention off"
        );
    }
    assert_eq!(
        runs(&bench.conn()).last().unwrap().4,
        None,
        "a pass with nothing to say must leave runs.error null"
    );
}

/// RET-02 through the pass: an evict policy over an aged corpus empties the
/// blobs and writes what it did into the only textual channel this product has.
#[test]
fn a_pass_with_an_evict_policy_evicts_the_aged_sessions_and_says_so() {
    let bench = bench();
    for n in 1..=2 {
        bench.place(
            PROJECT,
            &format!("{}.jsonl", uuid(n)),
            "session-basic.jsonl",
        );
    }
    bench.pass(&[]);
    age_everything(&bench.conn());

    let summary = bench.pass_with(&bench.config_file(EVICT));
    assert_eq!(summary.retention.evicted.len(), 2);
    assert_eq!(summary.retention.deleted, Vec::<String>::new());

    for (len, evicted) in blobs(&bench.conn()) {
        assert_eq!(len, 0);
        assert_eq!(evicted, Some(1));
    }
    let error = runs(&bench.conn())
        .last()
        .unwrap()
        .4
        .clone()
        .expect("an eviction has to be reported");
    assert!(
        error.contains("retention evicted 2 session(s)"),
        "the note must say what it did: {error}"
    );

    // And nothing is offered twice: the next pass has nothing left to report.
    let again = bench.pass_with(&bench.config_file(EVICT));
    assert!(again.retention.is_silent(), "{:?}", again.retention.lines());
    assert_eq!(runs(&bench.conn()).last().unwrap().4, None);
}

/// RET-03's bound, and the half of it that matters to a reader: a truncated
/// pass says how many it left, so "there is nothing left to do" is
/// distinguishable from "the rest is coming next pass". The hook spawn is the
/// scheduler, so the next pass is the next prompt.
#[test]
fn the_bounded_step_leaves_the_rest_for_the_next_pass_and_says_how_many() {
    let bench = bench();
    let over = 3;
    let total = MAX_PER_PASS + over;
    {
        // Planted rather than placed: what is under test is the bound and the
        // note, and a hundred real transcripts would only make the same
        // assertion slower.
        let store = Store::open(&bench.data_dir).unwrap();
        let conn = store.conn();
        for n in 0..total {
            let key = format!("/planted/{n}.jsonl");
            conn.execute(
                "INSERT INTO sessions (session_key, session_no, blob) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, n as i64 + 1, vec![0u8; 16]],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO session_meta
                    (session_key, transcript_path, checksum, uncompressed_len,
                     last_turn_at, project, is_final)
                 VALUES (?1, ?1, ?2, 16, ?3, '/data/code/alpha', 1)",
                rusqlite::params![key, vec![0u8; 32], AGED],
            )
            .unwrap();
            // So `observe_new` has nothing to say about a planted blob it
            // cannot decompress: this test is about retention's note alone.
            conn.execute(
                "INSERT INTO observations (session_key, generated_at, mechanical)
                 VALUES (?1, ?2, '{}')",
                rusqlite::params![key, AGED],
            )
            .unwrap();
        }
    }

    let first = bench.pass_with(&bench.config_file(EVICT));
    assert_eq!(first.retention.evicted.len(), MAX_PER_PASS);
    assert_eq!(first.retention.over, over);
    let error = runs(&bench.conn()).last().unwrap().4.clone().unwrap();
    assert!(
        error.contains(&format!("{over} more session(s) are waiting on retention")),
        "a truncated pass must say what it left: {error}"
    );

    let second = bench.pass_with(&bench.config_file(EVICT));
    assert_eq!(second.retention.evicted.len(), over);
    assert_eq!(second.retention.over, 0);
    let error = runs(&bench.conn()).last().unwrap().4.clone().unwrap();
    assert!(
        !error.contains("waiting on retention"),
        "the second pass had nothing left and said otherwise: {error}"
    );

    let evicted = blobs(&bench.conn());
    assert_eq!(evicted.len(), total);
    assert!(evicted
        .iter()
        .all(|(len, flag)| *len == 0 && *flag == Some(1)));
}
