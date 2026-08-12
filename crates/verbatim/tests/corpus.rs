//! AC1 / AC2 / D-24 / D-25: one real pass over the private corpus, measured.
//!
//! Everything else in this suite runs against seven synthetic fixtures. This
//! runs against the tree the product exists for - 1,896 files and ~988 MB when
//! CONTEXT measured it, growing daily - and it is the only test that can catch
//! the class of defect that only appears at that scale: a sidecar layout the
//! walk does not reach, a record shape the parser has never seen, a rebuild
//! that is fine over 3 MB and unacceptable over 988.
//!
//! It is gated on [`testkit::CORPUS_DIR_ENV`] and skips loudly when that is
//! unset, because the tree is private and cannot live in the repo.
//!
//! # Isolation, and why the config directory matters as much as the data one
//!
//! The run writes its store into a temporary data directory and never into the
//! user's. That much is obvious. The config directory is the part that is easy
//! to get wrong: `VERBATIM_CONFIG_DIR` must also point somewhere temporary and
//! empty, because the Claude-directory override replaces only the **default**
//! root. An explicit `roots` list in the developer's real `verbatim.toml` would
//! win over it, and once exclusions are configured - the feature this phase
//! ships - the run would walk his configured roots and skip his excluded
//! projects while this file's own walk counted the corpus tree, and the counts
//! would disagree for a reason nobody would find. The corpus variable must be
//! the only thing that decides what is walked.
//!
//! # Why the assertion is a set relation and not a count
//!
//! The corpus is **live**. The Claude Code session running this test appends to
//! its own transcript and spawns new `agent-*.jsonl` sidecars while the pass is
//! walking. A count taken before the pass races it and fails on correct
//! behaviour. So the tree is walked twice, before and after, and every
//! assertion is against the intersection - the files that were there for the
//! whole run - which stays true both as the corpus grows between runs and as it
//! grows during one.
//!
//! The walk here is deliberately **not** `discover::discover`. AC1 says every
//! `<uuid>.jsonl` and every `agent-*.jsonl` beneath the root at any depth, and
//! reusing the product's own filter to decide which files those are would make
//! the assertion agree with itself: a walk that missed the 41 sidecars at depth
//! 6 would miss them on both sides and pass.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use verbatim_core::blob;
use verbatim_core::store::{DB_FILE_NAME, DERIVED_SCHEMA, META_DERIVED_SCHEMA};
use verbatim_core::testkit::{self, LineSurvey};

/// AC1, AC2, D-24 and D-25 against the real tree, in one pass.
///
/// One test and not six: the pass over ~988 MB is the expensive part and every
/// assertion below is about the store that one pass produced.
#[test]
fn a_pass_over_the_real_corpus_archives_every_transcript_and_nothing_else() {
    let Some(corpus) = testkit::corpus_dir() else {
        return;
    };
    let projects = corpus
        .join("projects")
        .canonicalize()
        .expect("the corpus `projects` directory must resolve");
    println!("corpus: {}", projects.display());

    let dir = tempfile::tempdir().expect("a temporary data directory");
    let data_dir = dir.path().join("data");
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    assert!(
        !config_dir.join("verbatim.toml").exists(),
        "the config override must resolve no config file, or the developer's own \
         roots and exclusions decide what this run walks"
    );

    // Before the pass.
    let before = transcripts(&projects);
    assert!(
        before.len() > 100,
        "{} transcripts is not a corpus; is {} pointing at the right tree?",
        before.len(),
        testkit::CORPUS_DIR_ENV
    );

    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_verbatim"))
        .arg("ingest")
        .env("VERBATIM_DATA_DIR", &data_dir)
        .env("VERBATIM_CONFIG_DIR", &config_dir)
        .env("CLAUDE_CONFIG_DIR", &corpus)
        .output()
        .expect("spawn verbatim");
    let pass_time = started.elapsed();
    assert_eq!(
        out.status.code(),
        Some(0),
        "the pass did not exit 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // After it. The intersection is the file set that existed for the whole run.
    let after = transcripts(&projects);
    let stable: BTreeSet<PathBuf> = before.intersection(&after).cloned().collect();
    println!(
        "walked {} transcripts before the pass, {} after, {} stable across it; \
         pass took {pass_time:?}",
        before.len(),
        after.len(),
        stable.len()
    );
    assert!(!stable.is_empty(), "no transcript survived the pass");

    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    let sessions: BTreeSet<String> = conn
        .prepare("SELECT session_key FROM sessions")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    println!("archived {} sessions", sessions.len());

    // --- AC1, the positive half: every stable transcript has a session. ---
    //
    // With one documented exception, asserted by name rather than allowed for
    // by a slack count: `ingest_locked` returns `UpToDate` and writes no
    // `sessions` row when the file holds no complete line, which a transcript
    // created moments before the pass legitimately is.
    let mut excused: Vec<PathBuf> = Vec::new();
    let mut missing: Vec<PathBuf> = Vec::new();
    for path in &stable {
        if sessions.contains(path.to_str().expect("a corpus path is UTF-8")) {
            continue;
        }
        match std::fs::read(path) {
            Ok(bytes) if !bytes.contains(&b'\n') => excused.push(path.clone()),
            _ => missing.push(path.clone()),
        }
    }
    for path in &excused {
        println!(
            "excused (no complete line at pass time): {}",
            path.display()
        );
    }
    assert!(
        missing.is_empty(),
        "{} transcript(s) present for the whole pass were not archived: {:?}",
        missing.len(),
        &missing[..missing.len().min(10)]
    );

    // --- AC1, the negative half: nothing else became a session. ---
    for key in &sessions {
        let path = Path::new(key);
        assert!(
            path.starts_with(&projects),
            "a session key outside the corpus tree: {key}"
        );
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert_ne!(name, "journal.jsonl", "a journal became a session: {key}");
        assert!(
            !name.ends_with(".meta.json"),
            "a sidecar's meta file became a session: {key}"
        );
        assert!(
            name.ends_with(".jsonl"),
            "a non-`.jsonl` file became a session: {key}"
        );
        assert!(
            !path.components().any(|c| c.as_os_str() == "tool-results"),
            "a tool-results file became a session: {key}"
        );
        assert!(
            is_transcript(&projects, path),
            "a session key that is not a transcript by AC1's own rule: {key}"
        );
    }

    // --- Zero unparseable lines, read out of the archive itself. ---
    //
    // `Record::parse` keeps a non-JSON line as a record rather than erroring,
    // by design, so the product cannot make this claim for the test: the blobs
    // are decompressed and every line is handed to `serde_json`.
    let mut survey = LineSurvey::default();
    let mut statement = conn
        .prepare("SELECT session_key, blob FROM sessions ORDER BY session_key")
        .unwrap();
    let mut rows = statement.query([]).unwrap();
    let mut bytes_read = 0u64;
    while let Some(row) = rows.next().unwrap() {
        let key: String = row.get(0).unwrap();
        let bytes: Vec<u8> = row.get(1).unwrap();
        let stream = blob::read_all(&bytes)
            .unwrap_or_else(|e| panic!("{key}: the archived blob will not decompress: {e}"));
        bytes_read += stream.len() as u64;
        survey.absorb(&key, &stream);
    }
    println!(
        "{} lines over {:.1} MB of archived stream, {} untyped",
        survey.lines,
        bytes_read as f64 / (1024.0 * 1024.0),
        survey.untyped
    );
    assert_eq!(
        survey.unparseable_total,
        0,
        "unparseable archived lines (first {}): {:?}",
        survey.unparseable.len(),
        survey.unparseable
    );

    // D-25: counted and printed, never asserted as a closed set. 16 distinct
    // top-level types were measured including `pr-link` and `frame-link`, the
    // set grows upstream, and an exact assertion fails on data that is correct.
    println!("{} distinct record types:", survey.types.len());
    let mut histogram: Vec<(&String, &usize)> = survey.types.iter().collect();
    histogram.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (kind, count) in histogram {
        println!("  {count:>8}  {kind}");
    }

    // --- AC2: lineage, on the same store. ---
    let continued: i64 = conn
        .query_row(
            "SELECT count(*) FROM session_meta WHERE continues_from IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let self_linked: i64 = conn
        .query_row(
            "SELECT count(*) FROM session_meta
             WHERE continues_from IS NOT NULL AND continues_from = session_id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("continues_from: {continued} linked, {self_linked} self-linked");
    assert!(
        continued > 0,
        "no session continues another; 1.2% of the real corpus does, so this is a \
         lineage that stopped resolving rather than a corpus that stopped forking"
    );
    assert_eq!(
        self_linked, 0,
        "a session claims to continue itself (D-01), which is the link that would \
         make a resume brief cite the session it is opening"
    );

    // --- AC3: the entities that pass derived, over the real tree. ---
    //
    // Every number here is printed and none is asserted: the corpus is live and
    // grows during the run, so a count or a ratio would fail on data that is
    // correct. What is asserted is a set relation - every kind present, nothing
    // over the cap, every `paths` row backed by its entity - and the printed
    // distribution is what a later phase tunes the cap and the query weights
    // against.
    let per_kind: Vec<(String, i64)> = conn
        .prepare("SELECT kind, count(*) FROM entities GROUP BY kind ORDER BY kind")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    println!("entities by kind:");
    for (kind, count) in &per_kind {
        println!("  {count:>8}  {kind}");
    }
    for kind in verbatim_core::index::KINDS {
        let count = per_kind
            .iter()
            .find(|(name, _)| name == kind)
            .map(|(_, count)| *count)
            .unwrap_or(0);
        assert!(
            count > 0,
            "no `{kind}` entity in the whole corpus - the rule for that kind \
             fires on synthetic fixtures and on nothing real"
        );
    }

    // The per-turn distribution D-15 set the cap from, remeasured on this tree.
    let mut counts: Vec<i64> = conn
        .prepare("SELECT count(*) FROM entities GROUP BY turn_id")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    counts.sort_unstable();
    assert!(!counts.is_empty(), "the corpus produced no entity at all");
    let at = |q: f64| counts[((counts.len() - 1) as f64 * q) as usize];
    println!(
        "entities per emitting turn over {} turns: p50 {}, p90 {}, p99 {}, max {} (cap {})",
        counts.len(),
        at(0.50),
        at(0.90),
        at(0.99),
        counts[counts.len() - 1],
        verbatim_core::index::MAX_ENTITIES_PER_TURN,
    );
    assert!(
        counts[counts.len() - 1] <= verbatim_core::index::MAX_ENTITIES_PER_TURN as i64,
        "a turn carries more entities than the cap"
    );

    // `paths` is the lookup table for "which turns touched this file", so a row
    // in it without the entity it came from would mean the two tables describe
    // different sets and a path search and an entity search disagree.
    let path_rows: i64 = conn
        .query_row("SELECT count(*) FROM paths", [], |r| r.get(0))
        .unwrap();
    println!("{path_rows} path rows");
    assert!(path_rows > 0, "no path entity survived the real corpus");
    let unbacked: i64 = conn
        .query_row(
            "SELECT count(*) FROM paths p
             WHERE NOT EXISTS (
                 SELECT 1 FROM entities e
                 WHERE e.turn_id = p.turn_id AND e.kind = 'path' AND e.value_norm = p.path
             )",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unbacked, 0, "`paths` rows without a matching `path` entity");

    drop(rows);
    drop(statement);
    drop(conn);

    // --- D-24: the rebuild a bumped DERIVED_SCHEMA triggers, timed. ---
    //
    // Named in D-24 as the scale consequence to plan around: on the real corpus
    // this rebuilds every session's derived rows from the blobs, inside the
    // ingest lock, before any walk begins. Its cost is the number that decides
    // whether a store-format bump is a hiccup or an outage.
    let rebuild_time = time_a_forced_rebuild(&data_dir);
    println!(
        "derived rebuild of {} sessions after a DERIVED_SCHEMA bump: {rebuild_time:?}",
        sessions.len()
    );
}

/// Roll the store's derived schema back one, then time the open that repairs it.
fn time_a_forced_rebuild(data_dir: &Path) -> Duration {
    {
        let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
        let changed = conn
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = ?2",
                rusqlite::params![(DERIVED_SCHEMA - 1).to_string(), META_DERIVED_SCHEMA],
            )
            .unwrap();
        assert_eq!(changed, 1, "the store must carry a derived schema version");
    }
    // `open_up_to_date` and not `Store::open`: opening a store is a read, and
    // `Store::open` deliberately only REPORTS that a rebuild is due
    // (`Store::rebuild_required`) rather than performing one. Timing it would
    // time an open that does nothing and call the number a rebuild.
    let started = Instant::now();
    let store = verbatim_core::reindex::open_up_to_date(data_dir)
        .expect("an older store opens by rebuilding");
    let elapsed = started.elapsed();

    assert!(
        store.rebuild_required().is_none(),
        "the rebuild must leave the store stamped up to date"
    );
    drop(store);
    elapsed
}

/// Every transcript beneath `projects`, by AC1's own rule.
fn transcripts(projects: &Path) -> BTreeSet<PathBuf> {
    walk_files(projects)
        .into_iter()
        .filter(|path| is_transcript(projects, path))
        .collect()
}

/// AC1's rule, read independently of `discover`: an `agent-*.jsonl` at any
/// depth, or a `<uuid>.jsonl` directly inside a project directory.
fn is_transcript(projects: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(projects) else {
        return false;
    };
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with("agent-") && name.ends_with(".jsonl") {
        return true;
    }
    // Project depth: `<project>/<file>`, and nothing deeper.
    let at_project_depth = relative.components().count() == 2;
    at_project_depth && name.strip_suffix(".jsonl").is_some_and(is_uuid)
}

/// `8-4-4-4-12` hex.
fn is_uuid(s: &str) -> bool {
    let mut groups = s.split('-');
    for width in [8usize, 4, 4, 4, 12] {
        match groups.next() {
            Some(g) if g.len() == width && g.bytes().all(|b| b.is_ascii_hexdigit()) => {}
            _ => return false,
        }
    }
    groups.next().is_none()
}

/// Every regular file beneath `root`, at any depth, dot entries included.
///
/// An explicit stack rather than recursion, and `file_type` not followed
/// through a symlink, for the same reasons `discover` gives: unbounded depth is
/// the requirement, and a symlinked directory would be a cycle.
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            // A directory that vanished or cannot be listed is not a failure of
            // this walk: the tree is live, and the intersection is what the
            // assertions rest on.
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                dirs.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    files
}

/// The skip path, as a test of its own: with the variable unset the suite must
/// stay green and say why, because CI has no corpus and never will.
///
/// It also pins the contract of the helper the gate uses, so a rename that
/// silently made every corpus run a no-op would fail here.
#[test]
fn the_corpus_gate_is_a_skip_and_not_a_failure() {
    match std::env::var_os(testkit::CORPUS_DIR_ENV) {
        Some(value) if !value.is_empty() => {
            let dir = testkit::corpus_dir().expect("the variable is set, so this resolves");
            assert!(dir.join("projects").is_dir());
        }
        _ => assert!(
            testkit::corpus_dir().is_none(),
            "with {} unset there is no corpus to resolve",
            testkit::CORPUS_DIR_ENV
        ),
    }
    // Unused otherwise, and the histogram print is the only other reader.
    let mut survey = LineSurvey::default();
    survey.absorb("synthetic", b"{\"type\":\"user\"}\nnot json\n{\"no\":1}\n");
    assert_eq!(survey.lines, 3);
    assert_eq!(survey.unparseable_total, 1);
    assert_eq!(survey.untyped, 1);
    assert_eq!(survey.types.get("user"), Some(&1));
}
