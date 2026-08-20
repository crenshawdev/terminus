//! The `UserPromptSubmit` relevance injection, against stores built by the
//! ordinary ingest path (INJ-03, AC3, AC7).
//!
//! The store is seeded from `session-edits.jsonl` through
//! `testkit::copy_rooted_fixture_into` under a root the test owns, never from a
//! fixture that hardcodes a path: what AC3 is about is a prompt naming a file
//! relative to the payload's `cwd` reaching the ABSOLUTE spelling a past
//! session stored, and both spellings have to be something this test built or
//! the assertion is true only on a checkout at one literal path.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::inject::prompt;
use verbatim_core::recall::EntityMatch;
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{ingest, testkit};

/// The fixture that stores an absolute path, and the relative spelling of that
/// same file beneath its project.
const FIXTURE: &str = "session-edits.jsonl";
const RELATIVE: &str = "crates/gizmo/lantern.rs";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    root: PathBuf,
}

/// A store holding the edits fixture, ingested the ordinary way.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();

    let path = testkit::copy_rooted_fixture_into(FIXTURE, &work, &root);
    match ingest::run(&data_dir, &path).unwrap() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("{FIXTURE}: {other:?}"),
    }

    Bench {
        _dir: dir,
        data_dir,
        work,
        root,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// The directory the fixture's `cwd` names: what a payload carries.
    fn project(&self) -> String {
        testkit::fixture_project(FIXTURE, &self.root)
            .to_string_lossy()
            .into_owned()
    }

    /// The one `path` entity value the archive holds, read back out of the
    /// store rather than composed here: what the query has to match is what
    /// ingest actually wrote.
    fn stored_path(&self) -> String {
        let values: Vec<String> = self
            .conn()
            .prepare("SELECT DISTINCT value_norm FROM entities WHERE kind = 'path'")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(values.len(), 1, "{values:?}");
        assert!(
            std::path::Path::new(&values[0]).is_absolute(),
            "{} is not absolute",
            values[0]
        );
        values.into_iter().next().unwrap()
    }
}

/// D-05, and the whole of AC3's premise: the user types the relative spelling
/// and the archive holds the absolute one.
///
/// The second half is the falsifying one. `Query::matches_entity` needs every
/// token of the stored value present in the query, so the same prompt without
/// the resolution step matches nothing at all - which is what this phase would
/// have shipped, silently, with every absolute-path fixture still green.
#[test]
fn a_relative_path_resolves_to_the_absolute_one_the_archive_stored() {
    let bench = bench();
    let stored = bench.stored_path();
    let raw = format!("what changed in {RELATIVE}");

    let resolved = prompt::query_of(&raw, Some(&bench.project()));
    assert!(
        resolved.matches_entity(&stored).is_some(),
        "{raw:?} against cwd {:?} did not reach {stored}",
        bench.project()
    );

    assert_eq!(
        verbatim_core::recall::Query::parse(&raw).matches_entity(&stored),
        None,
        "the unresolved prompt matched, so the resolution proves nothing"
    );

    // A payload with no `cwd` has nothing to resolve against and asks what the
    // user typed, rather than inventing a base directory.
    assert_eq!(prompt::query_of(&raw, None).matches_entity(&stored), None);
}

/// An already-absolute path is left alone, not joined onto the `cwd` again.
#[test]
fn an_absolute_path_in_a_prompt_still_matches() {
    let bench = bench();
    let stored = bench.stored_path();

    let raw = format!("what changed in {stored}");
    assert_eq!(
        prompt::query_of(&raw, Some(&bench.project())).matches_entity(&stored),
        Some(EntityMatch::Covered),
        "{raw:?}"
    );

    // And the bare path, with nothing else asked for, is the exact case.
    assert_eq!(
        prompt::query_of(&stored, Some(&bench.project())).matches_entity(&stored),
        Some(EntityMatch::Exact)
    );
}

/// The ordering against `MAX_QUERY_TOKENS`: the resolved spelling goes first,
/// so a pasted wall of prose cannot cost the path its tokens.
///
/// The control is the same tokens in the other order. A query is truncated at
/// the thirty-third DISTINCT token, so prose-then-path drops exactly what the
/// resolution added and matches nothing - which is what makes this an assertion
/// about the ordering rather than about the prompt.
#[test]
fn thirty_words_of_prose_ahead_of_the_path_do_not_cost_it_its_tokens() {
    let bench = bench();
    let stored = bench.stored_path();

    let prose: String = (0..34)
        .map(|n| format!("word{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let raw = format!("{prose} {RELATIVE}");

    let query = prompt::query_of(&raw, Some(&bench.project()));
    assert!(
        query.truncated(),
        "the prose is not long enough to truncate"
    );
    assert!(
        query.matches_entity(&stored).is_some(),
        "the path lost its tokens to the prose"
    );

    let trailing = verbatim_core::recall::Query::parse(&format!("{prose} {stored}"));
    assert!(trailing.truncated());
    assert_eq!(
        trailing.matches_entity(&stored),
        None,
        "the same tokens in the other order still matched, so the ordering \
         is not what the resolution is relying on"
    );
}

/// What is not a path: a bare word, and a URL.
///
/// Neither is joined onto the `cwd`. A word with no separator is as likely to
/// be a subcommand as a file, and a URL joined onto a working directory names
/// nothing on any disk.
#[test]
fn a_bare_word_and_a_url_are_not_paths_to_resolve() {
    let bench = bench();
    let cwd = bench.project();

    // `alpha` is a token of the cwd and of nothing else, so it appears in a
    // query only when something was joined onto that cwd.
    let folded = |raw: &str| -> Vec<String> {
        prompt::query_of(raw, Some(&cwd))
            .tokens()
            .iter()
            .map(|t| t.to_lowercase())
            .collect()
    };

    for raw in [
        "what changed in lantern",
        "see https://example.com/gizmo/x.rs",
    ] {
        let tokens = folded(raw);
        assert!(
            !tokens.contains(&"alpha".to_owned()),
            "{raw:?} produced a resolved spelling: {tokens:?}"
        );
    }

    // The control: a real relative path in the same position does resolve, so
    // the assertions above are about what was skipped and not about the check.
    assert!(folded(&format!("what changed in {RELATIVE}")).contains(&"alpha".to_owned()));

    // The fixture really was copied into the work directory the bench seeded
    // from, which is what makes `stored_path` a value ingest wrote.
    assert!(bench.work.join(FIXTURE).is_file());
}
