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
use verbatim_core::config::Config;
use verbatim_core::inject::{prompt, Payload};
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

    /// The one `path` entity value the fixture stored, read back out of the
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

    /// What a `UserPromptSubmit` in this project carries.
    fn payload<'a>(&self, cwd: &'a str, prompt: &'a str) -> Payload<'a> {
        Payload {
            session_id: Some("0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55"),
            transcript_path: None,
            cwd: Some(cwd),
            prompt: Some(prompt),
            source: None,
        }
    }

    /// The turns one prompt fires on, at the default config.
    fn fired(&self, cwd: &str, prompt: &str) -> prompt::Fired {
        prompt::select(
            &self.data_dir,
            &Config::default(),
            &self.payload(cwd, prompt),
        )
    }

    /// Archive one session of hand-written records through the ordinary ingest
    /// path, under a project directory of its own.
    fn archive(&self, name: &str, project: &str, records: Vec<serde_json::Value>) -> PathBuf {
        let cwd = self.root.join(project);
        let session = format!("{:0>8}-0000-4000-8000-000000000000", name.len());
        let mut body = String::new();
        for (n, content) in records.into_iter().enumerate() {
            let record = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": cwd.to_string_lossy(),
                "sessionId": session,
                "type": "assistant",
                "uuid": format!("{session}-{n}"),
                "timestamp": format!("2026-08-14T10:{:02}:{:02}.000Z", n / 60, n % 60),
                "requestId": format!("req_{n}"),
                "message": {"role": "assistant", "model": "claude-opus-5", "content": [content]},
            });
            body.push_str(&record.to_string());
            body.push('\n');
        }

        let path = self.work.join(name);
        std::fs::write(&path, body).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{name}: {other:?}"),
        }
        path
    }
}

/// A turn that says something and stores nothing: prose, which emits no
/// entity whatever it names (RCL-02).
fn prose(text: &str) -> serde_json::Value {
    serde_json::json!({"type": "text", "text": text})
}

/// A `Read` of one file: one `path` entity and the tool that opened it.
fn read(file: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_use",
        "id": format!("toolu_r{}", file.len()),
        "name": "Read",
        "input": {"file_path": file},
    })
}

/// An `Edit` of one file naming one symbol: two independent entities on one
/// turn, which is what INJ-03's co-occurrence condition is about.
fn edit(file: &str, symbol: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_use",
        "id": format!("toolu_e{}", file.len()),
        "name": "Edit",
        "input": {
            "file_path": file,
            "old_string": format!("    let handle = {symbol}(1);"),
            "new_string": "    let handle = replaced(1);",
        },
    })
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

/// The tool a turn called, or `None` for a turn that called none.
fn tool_of(conn: &Connection, turn_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT tool_name FROM turns WHERE id = ?1",
        [turn_id],
        |r| r.get(0),
    )
    .unwrap()
}

/// AC3 at the library seam: the prompt names the file relative and the turn
/// that stored it absolute comes back.
///
/// The fixture's prose turn names the same file in the same words, so this is
/// also the difference the phase is built on: the turn that DID the thing fires
/// and the turn that mentioned it does not.
#[test]
fn a_prompt_naming_a_stored_path_fires_on_the_turn_that_stored_it() {
    let bench = bench();
    let cwd = bench.project();

    let fired = bench.fired(&cwd, &format!("what changed in {RELATIVE}"));
    assert_eq!(fired.hits.len(), 1, "{fired:?}");
    assert_eq!(
        tool_of(&bench.conn(), fired.hits[0].turn_id).as_deref(),
        Some("Edit"),
        "the turn that fired is not the one that stored the path"
    );
    assert!(fired.hits[0].entity_match.is_some());
    assert!(
        !fired.hits[0].excerpt.is_empty(),
        "a turn injected with no text under it says nothing"
    );
    assert_eq!(
        fired.reads.blobs, 1,
        "one turn from one session is one blob: {:?}",
        fired.reads
    );

    // The same prompt with the path taken out of it (AC3's other half).
    let silent = bench.fired(&cwd, "what changed");
    assert!(silent.hits.is_empty(), "{silent:?}");
    assert_eq!(silent.reads.blobs, 0);
}

/// AC7: a prompt whose only match is free text fires on nothing, and pays for
/// nothing.
///
/// The candidate is real - `widgetFactory` is identifier-shaped and the search
/// returns the turn that says it - so this is the threshold refusing a hit it
/// found, not a query that found nothing. A score cutoff would have injected
/// it: it is the only hit there is, at rank 1.
#[test]
fn a_free_text_only_match_fires_on_nothing_and_reads_no_blob() {
    let bench = bench();
    let cwd = bench.project();
    bench.archive(
        "prose.jsonl",
        "project-alpha",
        vec![prose(
            "the widgetFactory helper is still the one we settled on",
        )],
    );

    let raw = "is widgetFactory still the one we settled on";
    let fired = bench.fired(&cwd, raw);
    assert!(fired.hits.is_empty(), "{fired:?}");
    assert_eq!(
        fired.reads.blobs, 0,
        "a prompt that injected nothing decompressed a session: {:?}",
        fired.reads
    );

    // The falsifying half: the search really does return that turn, so what
    // refused it is the threshold.
    let found = ranked(&bench, &cwd, raw, vec!["widgetFactory"]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].entity_match, None);
}

/// INJ-03's cap: four turns qualify and three are injected.
#[test]
fn at_most_three_turns_are_ever_injected() {
    let bench = bench();
    let cwd = bench.project();
    let spark = format!("{cwd}/crates/gizmo/spark.rs");
    bench.archive(
        "spark.jsonl",
        "project-alpha",
        (0..4).map(|_| read(&spark)).collect(),
    );

    let raw = "what happened to crates/gizmo/spark.rs";
    let qualified = ranked(&bench, &cwd, raw, vec![&spark, "crates/gizmo/spark.rs"]);
    assert_eq!(
        qualified
            .iter()
            .filter(|hit| hit.entity_match.is_some())
            .count(),
        4,
        "fewer than four turns qualify, so a cap of three proves nothing: {qualified:?}"
    );

    let fired = bench.fired(&cwd, raw);
    assert_eq!(fired.hits.len(), prompt::MAX_TURNS, "{fired:?}");
    assert_eq!(fired.reads.blobs, 1, "one session, one blob");
}

/// The ranked hits one prompt's candidates return, with no threshold applied.
///
/// The test's own copy of what `select` runs, so an assertion can be made about
/// where a turn RANKED as well as about whether it fired.
fn ranked(
    bench: &Bench,
    cwd: &str,
    raw: &str,
    spellings: Vec<&str>,
) -> Vec<verbatim_core::recall::Hit> {
    use verbatim_core::recall::{search, Query, Request, Scope};
    let request = Request::new(
        prompt::query_of(raw, Some(cwd)),
        Scope::Directory(cwd.into()),
    )
    .limit(10)
    .candidates(spellings.iter().map(|s| Query::parse(s)).collect())
    .excerpts(false);
    search::run(&bench.conn(), &Config::default(), &request)
        .unwrap()
        .hits
}

/// INJ-03's second condition, and the one that is not a subset of the first:
/// two independent entities on one turn fire it wherever that turn ranks, and
/// one entity below the top three does not.
///
/// The three turns above it are prose that repeats a word the prompt asks about
/// and that no tool record anywhere carries - which is what makes the case
/// observable at all: a rare term gives free text the highest relevance in the
/// order, and the corroborated turn beneath it is still the one worth having.
/// A score cutoff would have taken the three that say the word and left the one
/// that did the work.
#[test]
fn two_entities_on_one_turn_fire_from_below_the_top_three() {
    let bench = bench();
    let cwd = bench.project();
    let anchor = format!("{cwd}/src/deep/anchor.rs");
    // Named by the prompt and carried by nothing structural: free text only.
    let rumour = "zephyrLatch";
    // Named by the prompt and carried by the edits: an entity.
    let symbol = "anchorHandle";

    let mut records: Vec<serde_json::Value> = Vec::new();
    for n in 0..3 {
        records.push(prose(&format!(
            "{rumour} {rumour} {rumour} {rumour} {rumour} is what note {n} was about"
        )));
    }
    // The corroborated turn: this file AND that symbol, one turn.
    records.push(edit(&anchor, symbol));
    // Turns that make each of those two values common, so neither one's IDF
    // can carry the corroborated turn into the top three on rarity alone.
    for n in 0..20 {
        records.push(read(&anchor));
        records.push(edit(&format!("{cwd}/src/deep/other{n}.rs"), symbol));
    }
    bench.archive("anchor.jsonl", "project-alpha", records);

    let raw = format!("did {rumour} break {symbol} in src/deep/anchor.rs");
    let order = ranked(
        &bench,
        &cwd,
        &raw,
        vec![&anchor, "src/deep/anchor.rs", rumour, symbol],
    );
    for (rank, hit) in order.iter().enumerate() {
        println!(
            "rank {rank}: turn {} entities {} kind {:?} relevance {:.3} entity_score {:.3}",
            hit.turn_id, hit.entity_count, hit.entity_match, hit.relevance, hit.entity_score
        );
    }

    assert!(
        order.iter().take(3).all(|hit| hit.entity_match.is_none()),
        "the top three are not free-text-only, so nothing below them can fire"
    );

    let corroborated = order
        .iter()
        .position(|hit| hit.entity_count >= 2)
        .expect("no turn carries two matched entities");
    assert!(
        corroborated >= 3,
        "the corroborated turn is inside the top three, where the other \
         condition would have fired it anyway"
    );
    let single = order
        .iter()
        .enumerate()
        .find(|(rank, hit)| *rank >= 3 && hit.entity_count == 1)
        .map(|(_, hit)| hit.turn_id)
        .expect("no single-entity turn below the top three");

    let fired = bench.fired(&cwd, &raw);
    let injected: Vec<i64> = fired.hits.iter().map(|hit| hit.turn_id).collect();
    assert_eq!(
        injected,
        vec![order[corroborated].turn_id],
        "the corroborated turn is the whole of what should have fired"
    );
    assert!(!injected.contains(&single));
    assert_eq!(fired.reads.blobs, 1);
}

/// A config whose `[injection] prompt_chars` is what the test says, loaded
/// through the file the binary loads one from - the budget has to reach the
/// render, and a struct built in memory would not prove that it does.
fn config_with(bench: &Bench, prompt_chars: usize) -> Config {
    let dir = bench.work.join(format!("config-{prompt_chars}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("verbatim.toml"),
        format!("[injection]\nprompt_chars = {prompt_chars}\n"),
    )
    .unwrap();
    Config::load_from(&dir).unwrap()
}

/// What the hook actually emits: the turn id, the day, the text - and the same
/// bytes twice against an unchanged store.
#[test]
fn the_injected_text_names_the_turn_and_the_day_and_repeats_itself() {
    let bench = bench();
    let cwd = bench.project();
    let raw = format!("what changed in {RELATIVE}");
    let payload = bench.payload(&cwd, &raw);

    let text = prompt::user_prompt_submit(&bench.data_dir, &Config::default(), &payload)
        .expect("the prompt names a path the archive stored");
    let fired = bench.fired(&cwd, &raw);
    assert!(
        text.contains(&fired.hits[0].turn_id.to_string()),
        "the id `recall_get` takes is not in the injection: {text}"
    );
    assert!(
        text.contains("2026-08-13"),
        "no day-resolution date: {text}"
    );
    assert!(
        !text.contains("09:00"),
        "a time of day is volatile text: {text}"
    );
    assert!(
        text.contains("lanternFlicker"),
        "the injected text is not the edit's own: {text}"
    );

    assert_eq!(
        prompt::user_prompt_submit(&bench.data_dir, &Config::default(), &payload),
        Some(text),
        "two runs against an unchanged store rendered different bytes"
    );

    // The other half of AC3, at the same seam.
    let bare = bench.payload(&cwd, "what changed");
    assert_eq!(
        prompt::user_prompt_submit(&bench.data_dir, &Config::default(), &bare),
        None
    );
}

/// D-16: the budget is enforced in characters, and it binds.
///
/// The small budget is the point. At the default of 4,000 nothing the fixture
/// produces is ever cut, so a test that only ran at the default would assert
/// that the budget was never reached rather than that it holds.
#[test]
fn the_injection_is_cut_to_the_configured_budget() {
    let bench = bench();
    let cwd = bench.project();
    let raw = format!("what changed in {RELATIVE}");
    let payload = bench.payload(&cwd, &raw);

    let full = prompt::user_prompt_submit(&bench.data_dir, &Config::default(), &payload).unwrap();
    for budget in [100, 90, 3] {
        let text =
            prompt::user_prompt_submit(&bench.data_dir, &config_with(&bench, budget), &payload)
                .unwrap();
        assert!(
            text.chars().count() <= budget,
            "{} characters against a budget of {budget}: {text}",
            text.chars().count()
        );
        assert!(text.chars().count() < full.chars().count());
    }
}
