//! RET-01..RET-03: what retention names, and what it then does to it.
//!
//! Every case runs against a real temp SQLite store. The evaluation instant is
//! passed in rather than read off the wall clock, which is the property the
//! shared evaluation exists to give (D-04) and is also what makes these
//! assertions about a fixed corpus rather than about the day they run.

#![cfg(feature = "testkit")]

use std::cell::Cell;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::config::{Config, CONFIG_FILE_NAME};
use verbatim_core::retention::{self, Applied, Selection, MAX_PER_PASS};
use verbatim_core::store::{Store, DB_FILE_NAME};
use verbatim_core::ingest::pass;
use verbatim_core::{ingest, testkit};

/// The instant every evaluation below measures from.
const NOW: &str = "2026-08-22T12:00:00.000Z";
/// Comfortably outside any window these tests configure.
const ANCIENT: &str = "2020-01-01T00:00:00.000Z";
/// Comfortably inside a 30-day window ending at [`NOW`].
const RECENT: &str = "2026-08-20T00:00:00.000Z";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    /// Where the rooted fixtures' `cwd` and stored paths point: somewhere this
    /// test owns, so a project key is whatever the test built.
    root: PathBuf,
    next: Cell<i64>,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    // Created here so `session_meta` exists before the first plant.
    Store::open(&data_dir).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        root,
        next: Cell::new(1),
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// A session row shaped like an archived one, plus the transcript file on
    /// disk that D-02's delete rule asks about.
    ///
    /// Planted rather than ingested: this half of the module reads
    /// `session_meta` and never a blob, so a fixture would only make the same
    /// assertion slower and less precise about the row it is asserting on.
    fn plant(
        &self,
        name: &str,
        project: &str,
        last_turn_at: Option<&str>,
        is_final: bool,
    ) -> String {
        let path = self.work.join(name);
        std::fs::write(&path, "{}\n").unwrap();
        let key = path.to_string_lossy().into_owned();
        let no = self.next.get();
        self.next.set(no + 1);

        let conn = self.conn();
        conn.execute(
            "INSERT INTO sessions (session_key, session_no, blob) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, no, vec![0u8; 8]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_meta
                (session_key, transcript_path, checksum, uncompressed_len,
                 last_turn_at, project, is_final)
             VALUES (?1, ?1, ?2, 8, ?3, ?4, ?5)",
            rusqlite::params![
                key,
                vec![0u8; 32],
                last_turn_at,
                project,
                is_final.then_some(1_i64)
            ],
        )
        .unwrap();
        key
    }

    /// What Claude Code's own `cleanupPeriodDays` does, in one line.
    fn remove_transcript(&self, key: &str) {
        std::fs::remove_file(Path::new(key)).unwrap();
    }

    fn mark_evicted(&self, key: &str) {
        self.conn()
            .execute(
                "UPDATE session_meta SET is_evicted = 1 WHERE session_key = ?1",
                [key],
            )
            .unwrap();
    }

    fn evaluate(&self, config: &Config) -> Selection {
        retention::evaluate(&self.conn(), config, NOW).expect("the evaluation must run")
    }
}

/// A config carrying nothing but this `verbatim.toml` body.
fn config(body: &str) -> Config {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(CONFIG_FILE_NAME), body).unwrap();
    Config::load_from(dir.path()).expect("a retention table is not a parse failure")
}

const EVICT_30: &str = "[retention]\naction = \"evict\"\nage_days = 30\n";
const DELETE_30: &str = "[retention]\naction = \"delete\"\nage_days = 30\n";

/// RET-01. Every session in the store predates any plausible default and the
/// off policy still names none of them - which is the whole of AC1 stated at
/// the level retention decides it.
#[test]
fn an_off_policy_names_nothing_however_old_the_store_is() {
    let bench = bench();
    for n in 0..5 {
        bench.plant(
            &format!("s{n}.jsonl"),
            "/data/code/alpha",
            Some(ANCIENT),
            true,
        );
    }

    for body in [
        "",
        "[injection]\nbrief_chars = 500\n",
        "[retention]\naction = \"evict\"\n",
        "[retention]\nage_days = 1\n",
    ] {
        let selection = bench.evaluate(&config(body));
        assert!(
            selection.is_empty(),
            "{body:?} named {:?} / {:?}",
            selection.evict,
            selection.delete
        );
        assert_eq!(selection.over, 0, "{body:?}");
        assert_eq!(selection.evaluated_at, NOW, "{body:?}");
    }
}

/// D-11: only sessions the idle rule has already closed are eligible. Evicting
/// a live transcript's blob would fail the next pass's `blob::append` checksum
/// precondition and record that file as a failure on every pass thereafter.
#[test]
fn a_session_that_is_not_final_is_never_named_however_old_it_is() {
    let bench = bench();
    let live = bench.plant("live.jsonl", "/data/code/alpha", Some(ANCIENT), false);
    let closed = bench.plant("closed.jsonl", "/data/code/alpha", Some(ANCIENT), true);

    let selection = bench.evaluate(&config(EVICT_30));
    assert_eq!(selection.evict, vec![closed]);
    assert!(
        !selection.evict.contains(&live),
        "a session the idle rule has not closed was named"
    );
}

/// D-11 again, from the other side: a session with no `last_turn_at` is not
/// idle, it is unknown, and nothing is done to a session nothing is known about.
#[test]
fn a_session_with_no_last_turn_is_never_named() {
    let bench = bench();
    bench.plant("unknown.jsonl", "/data/code/alpha", None, true);

    assert!(bench.evaluate(&config(EVICT_30)).is_empty());
}

/// The age test itself: the window's inside is kept and its outside is named.
#[test]
fn the_age_window_decides_and_the_cutoff_comes_off_the_passed_instant() {
    let bench = bench();
    let recent = bench.plant("recent.jsonl", "/data/code/alpha", Some(RECENT), true);
    let old = bench.plant("old.jsonl", "/data/code/alpha", Some(ANCIENT), true);

    let selection = bench.evaluate(&config(EVICT_30));
    assert_eq!(selection.evict, vec![old.clone()]);
    assert!(!selection.evict.contains(&recent));

    // A wider window keeps the same session; a window of one day takes both.
    // The instant is the test's, so this is a fact about the rule and not about
    // the day the suite runs.
    let narrow = bench.evaluate(&config("[retention]\naction = \"evict\"\nage_days = 1\n"));
    assert_eq!(narrow.evict, vec![old, recent]);
}

/// D-02: `delete` is a garbage collector for what Claude Code's own
/// `cleanupPeriodDays` already removed. A session whose transcript is still on
/// disk is rediscovered and re-ingested from offset 0 on the next hook spawn,
/// so deleting it reclaims nothing measurable.
#[test]
fn delete_names_only_a_session_whose_transcript_is_gone() {
    let bench = bench();
    let present = bench.plant("present.jsonl", "/data/code/alpha", Some(ANCIENT), true);
    let gone = bench.plant("gone.jsonl", "/data/code/alpha", Some(ANCIENT), true);
    bench.remove_transcript(&gone);

    let selection = bench.evaluate(&config(DELETE_30));
    assert_eq!(selection.delete, vec![gone]);
    assert!(
        !selection.delete.contains(&present),
        "a session whose transcript is still on disk was named for deletion"
    );
    assert!(selection.evict.is_empty(), "a delete policy evicts nothing");
}

/// An evicted session is not offered for eviction again: a pass that reported
/// one every time would be reporting work that reclaims nothing.
#[test]
fn a_session_already_evicted_is_not_named_again() {
    let bench = bench();
    let key = bench.plant("done.jsonl", "/data/code/alpha", Some(ANCIENT), true);
    assert_eq!(bench.evaluate(&config(EVICT_30)).evict, vec![key.clone()]);

    bench.mark_evicted(&key);
    assert!(bench.evaluate(&config(EVICT_30)).is_empty());
}

/// D-01: the per-project rule governs its subtree, and the global table governs
/// everything else.
#[test]
fn a_per_project_rule_governs_its_subtree_and_the_global_table_the_rest() {
    let bench = bench();
    let scratch = bench.plant(
        "scratch.jsonl",
        "/data/code/scratch/sub",
        Some(ANCIENT),
        true,
    );
    let sibling = bench.plant(
        "sibling.jsonl",
        "/data/code/scratch-other",
        Some(ANCIENT),
        true,
    );
    bench.remove_transcript(&scratch);
    bench.remove_transcript(&sibling);

    let selection = bench.evaluate(&config(
        "[retention]\naction = \"evict\"\nage_days = 30\n\n\
         [retention.project.\"/data/code/scratch\"]\naction = \"delete\"\nage_days = 30\n",
    ));
    assert_eq!(selection.delete, vec![scratch]);
    assert_eq!(selection.evict, vec![sibling]);
}

/// ING-08: exclusion means never read, on the ingest path and the read path
/// both, and a deletion is the most extreme thing that could be done to bytes a
/// user said not to look at. Passed over, and counted rather than silently
/// dropped - a policy that reclaims nothing has to be able to say why.
#[test]
fn a_session_in_an_excluded_project_is_passed_over_and_counted() {
    let bench = bench();
    let hidden = bench.plant("hidden.jsonl", "/data/code/secret", Some(ANCIENT), true);
    let open = bench.plant("open.jsonl", "/data/code/alpha", Some(ANCIENT), true);

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(CONFIG_FILE_NAME),
        "exclude = [\"/data/code/secret\"]\n\
         [retention]\naction = \"evict\"\nage_days = 30\n",
    )
    .unwrap();
    let config = Config::load_from(dir.path()).unwrap();

    let selection = retention::evaluate(&bench.conn(), &config, NOW).unwrap();
    assert_eq!(selection.evict, vec![open]);
    assert!(!selection.evict.contains(&hidden));
    assert_eq!(selection.excluded, 1);
}

/// The bound. One evaluation names at most `MAX_PER_PASS` per action, oldest
/// first, and says how many it left - which is what lets a reader tell "there
/// is nothing left to do" from "the rest is coming next pass".
#[test]
fn the_bound_takes_the_oldest_and_counts_what_it_left() {
    let bench = bench();
    let over = 5;
    let mut planted = Vec::new();
    for n in 0..MAX_PER_PASS + over {
        // Ordered timestamps, so "oldest first" is checkable and not incidental.
        let stamp = format!("2020-01-01T00:00:{:02}.000Z", n % 60);
        let day = format!("2020-01-{:02}T00:00:00.000Z", (n / 60) + 1);
        let last = if n < 60 { stamp } else { day };
        planted.push((
            last.clone(),
            bench.plant(
                &format!("s{n}.jsonl"),
                "/data/code/alpha",
                Some(&last),
                true,
            ),
        ));
    }
    planted.sort();

    let selection = bench.evaluate(&config(EVICT_30));
    assert_eq!(selection.evict.len(), MAX_PER_PASS);
    assert_eq!(selection.over, over);
    let expected: Vec<String> = planted
        .into_iter()
        .map(|(_, key)| key)
        .take(MAX_PER_PASS)
        .collect();
    assert_eq!(selection.evict, expected, "the bound must take the oldest");
}

// The mutating half. These cases ingest real fixtures rather than planting
// rows, because what they assert on is the derived tables an eviction has to
// leave standing and a deletion has to take with it.

impl Bench {
    /// Archive one transcript that fills every table a deletion has to empty.
    ///
    /// Three fixtures in one file, and each is there for a table: the basic
    /// session carries the searchable token and the entities, the rooted edits
    /// session is the only one storing an absolute path (so `paths` has a row),
    /// and the appended boundary line is what gives `compaction_boundaries`
    /// one. Answers its session key.
    fn archive(&self, name: &str) -> String {
        let edits =
            testkit::copy_rooted_fixture_into("session-edits.jsonl", &self.work, &self.root);
        let mut bytes = testkit::fixture_bytes("session-basic.jsonl");
        bytes.extend_from_slice(&std::fs::read(&edits).unwrap());
        bytes.extend_from_slice(&testkit::boundary_line());
        bytes.push(b'\n');
        let path = self.work.join(name);
        std::fs::write(&path, bytes).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{name}: {other:?}"),
        }
        path.canonicalize().unwrap().to_string_lossy().into_owned()
    }

    fn apply(&self, selection: &Selection) -> Applied {
        let mut store = Store::open(&self.data_dir).unwrap();
        retention::apply(&mut store, selection)
    }

    fn count(&self, sql: &str, key: &str) -> i64 {
        self.conn().query_row(sql, [key], |r| r.get(0)).unwrap()
    }

    /// How many rows each table holds for this session.
    fn rows(&self, key: &str) -> Vec<(&'static str, i64)> {
        vec![
            ("sessions", self.count("SELECT count(*) FROM sessions WHERE session_key = ?1", key)),
            ("session_meta", self.count("SELECT count(*) FROM session_meta WHERE session_key = ?1", key)),
            ("turns", self.count("SELECT count(*) FROM turns WHERE session_key = ?1", key)),
            ("entities", self.count("SELECT count(*) FROM entities WHERE turn_id IN (SELECT id FROM turns WHERE session_key = ?1)", key)),
            ("paths", self.count("SELECT count(*) FROM paths WHERE turn_id IN (SELECT id FROM turns WHERE session_key = ?1)", key)),
            ("compaction_boundaries", self.count("SELECT count(*) FROM compaction_boundaries WHERE turn_id IN (SELECT id FROM turns WHERE session_key = ?1)", key)),
            ("observations", self.count("SELECT count(*) FROM observations WHERE session_key = ?1", key)),
            ("watermarks", self.count("SELECT count(*) FROM watermarks WHERE transcript_path = ?1", key)),
        ]
    }

    /// The one thing a deletion destroys that no rebuild reproduces.
    fn plant_observation(&self, key: &str) {
        self.conn()
            .execute(
                "INSERT INTO observations (session_key, session_id, generated_at, mechanical)
                 VALUES (?1, 'sid', '2026-08-22T00:00:00.000Z', '{}')",
                [key],
            )
            .unwrap();
    }

    fn matches(&self, token: &str) -> i64 {
        self.conn()
            .query_row(
                "SELECT count(*) FROM turns_fts WHERE turns_fts MATCH ?1",
                [token],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// RET-02: the row, the metadata and every derived row survive an eviction.
/// Only the bytes go - which is what lets `verbatim sessions` still list it and
/// a search still match its turns.
#[test]
fn an_eviction_empties_the_blob_and_leaves_everything_else_standing() {
    let bench = bench();
    let key = bench.archive("evicted.jsonl");
    let before = bench.rows(&key);
    let matched = bench.matches(testkit::UNIQUE_TOKEN);
    assert!(
        matched > 0,
        "the fixture has to be searchable to start with"
    );

    let applied = bench.apply(&Selection {
        evict: vec![key.clone()],
        ..Selection::default()
    });
    assert_eq!(applied.evicted, vec![key.clone()]);
    assert!(applied.notes.is_empty(), "{:?}", applied.notes);

    let conn = bench.conn();
    let (len, evicted): (i64, Option<i64>) = conn
        .query_row(
            "SELECT length(s.blob), m.is_evicted FROM sessions s
             JOIN session_meta m USING (session_key) WHERE s.session_key = ?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        len, 0,
        "the blob must be emptied, never null and never dropped"
    );
    assert_eq!(evicted, Some(1));
    assert_eq!(
        bench.rows(&key),
        before,
        "an eviction touched a derived row"
    );
    assert_eq!(bench.matches(testkit::UNIQUE_TOKEN), matched);

    // The lowered-watermark sweep compares these two, so leaving them alone is
    // what stops every later pass from "repairing" this session forever.
    let (checksum_len, uncompressed): (i64, i64) = conn
        .query_row(
            "SELECT length(checksum), uncompressed_len FROM session_meta WHERE session_key = ?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(checksum_len, 32);
    assert!(
        uncompressed > 0,
        "uncompressed_len was lowered by the eviction"
    );
}

/// A deletion takes the session and everything keyed on it, including the row
/// nothing can rebuild.
#[test]
fn a_deletion_removes_every_row_the_session_owns() {
    let bench = bench();
    let key = bench.archive("doomed.jsonl");
    bench.plant_observation(&key);
    for (table, count) in bench.rows(&key) {
        assert!(count > 0, "{table} has nothing to delete");
    }
    assert!(bench.matches(testkit::UNIQUE_TOKEN) > 0);

    let applied = bench.apply(&Selection {
        delete: vec![key.clone()],
        ..Selection::default()
    });
    assert_eq!(applied.deleted, vec![key.clone()]);
    assert_eq!(applied.observations_lost, 1);
    assert!(applied.notes.is_empty(), "{:?}", applied.notes);
    assert!(
        applied.lines().iter().any(|l| l.contains("observation")),
        "the loss of a paid model call has to be said out loud: {:?}",
        applied.lines()
    );

    for (table, count) in bench.rows(&key) {
        assert_eq!(
            count, 0,
            "{table} still holds a row for the deleted session"
        );
    }
    assert_eq!(
        bench.matches(testkit::UNIQUE_TOKEN),
        0,
        "a search still matches a turn of the deleted session"
    );
}

/// FEED-03: the replay history outlives the session it was recorded against.
/// `decisions` records prompt-time state no blob ever held, so a deletion that
/// took it would silently delete what `verbatim replay` and `stats` are
/// computed over.
#[test]
fn a_decision_naming_the_deleted_session_survives_it() {
    let bench = bench();
    let key = bench.archive("doomed.jsonl");
    let session_id: String = bench
        .conn()
        .query_row(
            "SELECT session_id FROM session_meta WHERE session_key = ?1",
            [&key],
            |r| r.get(0),
        )
        .unwrap();
    bench
        .conn()
        .execute(
            "INSERT INTO decisions (session_id, ts, prompt, chars_injected)
             VALUES (?1, '2026-08-22T00:00:00.000Z', 'what did we decide', 0)",
            [&session_id],
        )
        .unwrap();

    bench.apply(&Selection {
        delete: vec![key.clone()],
        ..Selection::default()
    });

    let decisions: i64 = bench
        .conn()
        .query_row(
            "SELECT count(*) FROM decisions WHERE session_id = ?1",
            [&session_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(decisions, 1, "the replay history went with the session");
}

/// Retention must not be able to wedge on one damaged row, the way `pass::walk`
/// cannot wedge on one damaged transcript. The fault is a trigger rather than a
/// test hook: it makes SQLite refuse a real statement, so the rollback under
/// test is the real one.
#[test]
fn a_failure_on_one_session_does_not_stop_the_next() {
    let bench = bench();
    let doomed = bench.archive("doomed.jsonl");
    let fine = bench.archive("fine.jsonl");
    bench.plant_observation(&doomed);
    bench.plant_observation(&fine);
    bench
        .conn()
        .execute_batch(&format!(
            "CREATE TRIGGER retention_fault BEFORE DELETE ON sessions
               WHEN old.session_key = '{}'
               BEGIN SELECT RAISE(ABORT, 'planted fault'); END;",
            doomed.replace('\'', "''")
        ))
        .unwrap();

    let applied = bench.apply(&Selection {
        delete: vec![doomed.clone(), fine.clone()],
        ..Selection::default()
    });

    assert_eq!(
        applied.deleted,
        vec![fine.clone()],
        "the second session was skipped"
    );
    assert_eq!(applied.notes.len(), 1);
    assert!(
        applied.notes[0].starts_with(&doomed) && applied.notes[0].contains("planted fault"),
        "the failure has to name the session it happened to: {:?}",
        applied.notes
    );

    for (table, count) in bench.rows(&doomed) {
        assert!(count > 0, "{table} lost a row despite the deletion failing");
    }
    for (table, count) in bench.rows(&fine) {
        assert_eq!(
            count, 0,
            "{table} still holds a row for the deleted session"
        );
    }
}

// D-03 on the ingest side. An evicted session's blob was emptied on purpose, so
// `blob::append` has no header to parse out of it: left to the ordinary path
// the transcript becomes a per-file failure on every pass forever, and writing
// its tail as a FRESH blob instead would resurrect a session retention
// deliberately emptied while leaving every stored `turns.stream_offset`
// pointing into bytes that are no longer there. The only safe answer is to do
// nothing at all, which is what these two cases assert - one through the
// single-file entry point and one through a whole pass.

impl Bench {
    fn watermark(&self, key: &str) -> Option<i64> {
        self.conn()
            .query_row(
                "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
                [key],
                |r| r.get(0),
            )
            .ok()
    }

    fn blob_len(&self, key: &str) -> i64 {
        self.count("SELECT length(blob) FROM sessions WHERE session_key = ?1", key)
    }

    fn is_evicted(&self, key: &str) -> Option<i64> {
        self.conn()
            .query_row(
                "SELECT is_evicted FROM session_meta WHERE session_key = ?1",
                [key],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// Append one more complete record to a transcript, the way Claude Code does.
fn grow(path: &Path) {
    use std::io::Write;

    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(&testkit::boundary_line()).unwrap();
    file.write_all(b"\n").unwrap();
}

/// The single-file entry point, over a transcript that GREW after its session
/// was evicted: up to date, and not one byte written anywhere.
///
/// The growth is the whole point. A file that has not changed reaches
/// `Prepared { work: None }` by the ordinary route and would pass this test
/// with no arm at all.
#[test]
fn re_ingesting_an_evicted_transcript_that_grew_is_up_to_date_and_writes_nothing() {
    let bench = bench();
    let key = bench.archive("evicted.jsonl");
    let applied = bench.apply(&Selection {
        evict: vec![key.clone()],
        ..Selection::default()
    });
    assert_eq!(applied.evicted, vec![key.clone()]);

    let rows = bench.rows(&key);
    let watermark = bench.watermark(&key).expect("the archive left a watermark");
    let digest = testkit::archive_digest(&bench.conn());

    let path = PathBuf::from(&key);
    let size_before = std::fs::metadata(&path).unwrap().len();
    grow(&path);
    assert!(
        std::fs::metadata(&path).unwrap().len() > size_before,
        "the transcript has to have grown, or the arm is never reached"
    );

    let outcome = ingest::run_with(&bench.data_dir, &path, &Config::default()).unwrap();
    assert_eq!(outcome, ingest::Outcome::UpToDate);

    assert_eq!(bench.blob_len(&key), 0, "the emptied blob was written back");
    assert_eq!(bench.is_evicted(&key), Some(1), "the session was un-evicted");
    assert_eq!(
        bench.watermark(&key),
        Some(watermark),
        "the watermark moved over bytes no blob holds"
    );
    assert_eq!(bench.rows(&key), rows, "a derived row moved");
    assert_eq!(
        testkit::archive_digest(&bench.conn()),
        digest,
        "the archive is not byte for byte what it was"
    );
}

// The same fact through a whole pass, where the question is what the walk
// RECORDS: a file it can do nothing with must not be a per-file failure, or
// every pass for the rest of the store's life reports one.

const PROJECT: &str = "-data-projects-cadence";

/// A Claude config directory whose `projects` tree a pass walks. Nothing here
/// ever resolves a real transcript root.
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
    fn place(&self, name: &str) -> PathBuf {
        let dest = self.claude_dir.join("projects").join(PROJECT).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path("session-basic.jsonl"), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    fn pass(&self) -> pass::Summary {
        let config = Config::from_parts(vec![self.claude_dir.clone()], Vec::new());
        match pass::run_with(&self.data_dir, &config).unwrap() {
            pass::PassOutcome::Ran(summary) => summary,
            pass::PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn watermark(&self, path: &Path) -> i64 {
        self.conn()
            .query_row(
                "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
                [path.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// A pass whose tree holds an evicted session's grown transcript records no
/// failure for it, moves nothing of it, and still commits every other file.
///
/// The unevicted sibling is what keeps this from being vacuous: it grew by the
/// same bytes on the same pass, so a pass that committed nothing at all would
/// fail here rather than looking like the arm working.
#[test]
fn a_pass_records_no_failure_for_an_evicted_session_whose_transcript_grew() {
    let tree = tree();
    let evicted_path = tree.place("11111111-1111-4111-8111-111111111111.jsonl");
    let control_path = tree.place("22222222-2222-4222-8222-222222222222.jsonl");

    let first = tree.pass();
    assert_eq!(first.files_committed, 2);
    assert!(first.failures.is_empty(), "{:?}", first.failures);

    let evicted = evicted_path.to_string_lossy().into_owned();
    let mut store = Store::open(&tree.data_dir).unwrap();
    let applied = retention::apply(
        &mut store,
        &Selection {
            evict: vec![evicted.clone()],
            ..Selection::default()
        },
    );
    drop(store);
    assert_eq!(applied.evicted, vec![evicted.clone()]);

    let watermark = tree.watermark(&evicted_path);
    let control_watermark = tree.watermark(&control_path);
    grow(&evicted_path);
    grow(&control_path);

    let second = tree.pass();
    assert!(
        second.failures.is_empty(),
        "an evicted session became a per-file failure: {:?}",
        second.failures
    );
    assert_eq!(
        second.files_walked, 2,
        "the evicted transcript is still discovered and still walked"
    );
    assert_eq!(
        second.files_committed, 1,
        "exactly the unevicted sibling had its tail archived"
    );
    assert!(second.turns_added > 0, "the sibling's tail added no turn");

    let (len, flag): (i64, Option<i64>) = tree
        .conn()
        .query_row(
            "SELECT length(s.blob), m.is_evicted FROM sessions s
             JOIN session_meta m USING (session_key) WHERE s.session_key = ?1",
            [&evicted],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(len, 0, "the pass wrote bytes into an emptied blob");
    assert_eq!(flag, Some(1), "the pass un-evicted the session");
    assert_eq!(
        tree.watermark(&evicted_path),
        watermark,
        "the watermark moved over bytes no blob holds"
    );
    assert!(
        tree.watermark(&control_path) > control_watermark,
        "the sibling's watermark did not move, so this pass proved nothing"
    );

    // And it does not become a failure on the pass after that either: the state
    // is stable, not merely quiet once.
    let third = tree.pass();
    assert!(third.failures.is_empty(), "{:?}", third.failures);
}
