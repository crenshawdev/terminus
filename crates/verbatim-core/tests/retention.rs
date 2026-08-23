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
use verbatim_core::retention::{self, Selection, MAX_PER_PASS};
use verbatim_core::store::{Store, DB_FILE_NAME};

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
    next: Cell<i64>,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    // Created here so `session_meta` exists before the first plant.
    Store::open(&data_dir).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
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
