//! The four derived recall projections, pinned byte for byte against a golden
//! captured before this phase changed anything (phase 4 AC2).
//!
//! **Captured before the first product edit, deliberately.** The criterion this
//! file exists for is that the default - the new config boolean absent - is
//! today's behaviour, and "today" has no in-repo source of truth until it is
//! written down. A golden captured after the first change would bake that
//! change in and assert nothing at all, so this file lands as the phase's first
//! commit and its own commit touches no `src` path.
//!
//! **One document, not four assertions.** All four projections - the search
//! excerpt, the context window's turn texts, the `recall_get` body and the
//! `SessionStart` brief - go into a single deterministic document, so a diff
//! shows which projection moved and how, next to the three that did not.
//!
//! **Nothing here names the knob or the filter.** The whole value of the
//! comparison is that it is written against the call signatures that existed
//! before the phase; a golden that reached for the new config key would be
//! measuring the change against itself.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use terminus_core::config::Config;
use terminus_core::inject::{brief, Payload};
use terminus_core::recall::{context, get, search, Query, Request, Scope};
use terminus_core::store::DB_FILE_NAME;
use terminus_core::{ingest, testkit};

/// Setting this rewrites the golden instead of comparing against it.
///
/// A regeneration is always a deliberate act and always visible in the diff:
/// the test cannot quietly re-baseline itself on a run that would otherwise
/// have failed, which is the whole failure mode a self-updating golden has.
const REGENERATE_ENV: &str = "TERMINUS_REGENERATE_RECALL_GOLDEN";

/// The checked-in document, relative to `crates/terminus-core/tests`.
const GOLDEN: &str = "goldens/recall-projections.txt";

/// The two scoped queries the document is built from.
///
/// `project-delta` is `session-secrets.jsonl`'s project, so its excerpts carry
/// the planted credential sentinels verbatim - which is what makes this golden
/// able to show a filter turning on. `project-alpha` is the ordinary prose
/// corpus every other recall test is written against, and it is here as the
/// control: nothing in it is credential-shaped, so its projections must not
/// move whatever the filter does.
const QUERIES: &[(&str, &str)] = &[
    ("project-delta", "gateway"),
    ("project-alpha", "retry budget"),
];

/// How many turns of context the window section captures on each side.
const CONTEXT_SIDE: usize = 2;

struct Bench {
    dir: tempfile::TempDir,
    data_dir: PathBuf,
    root: PathBuf,
}

/// A store with every transcript fixture ingested, built the way
/// `tests/recall.rs` builds one: the ordinary `ingest::run` path over the whole
/// fixture corpus, with the rooted fixtures' `cwd` pointed at a root the test
/// owns.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();

    for fixture in testkit::TRANSCRIPT_FIXTURES {
        let rooted = testkit::ROOTED_FIXTURES.iter().any(|(f, _)| f == fixture);
        let path = if rooted {
            testkit::copy_rooted_fixture_into(fixture, &work, &root)
        } else {
            testkit::copy_fixture_into(fixture, &work)
        };
        match ingest::run(&data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{fixture}: {other:?}"),
        }
    }

    Bench {
        dir,
        data_dir,
        root,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn project(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

/// The whole document, as this build renders it.
fn capture(bench: &Bench) -> String {
    let conn = bench.conn();
    // The config the criterion is about: no file, so the new table is absent
    // and every projection is whatever the default does.
    let config = Config::default();

    let mut out = String::new();
    out.push_str(HEADER);

    for (project, raw) in QUERIES {
        let scope = Scope::Named(bench.project(project).to_string_lossy().into_owned());
        let request = Request::new(Query::parse(raw), scope.clone());
        let response = search::run(&conn, &config, &request).unwrap();

        out.push_str(&format!("\n== search {project} {raw:?}\n"));
        if let Some(reason) = &response.reason {
            out.push_str(&format!("reason: {reason:?}\n"));
        }
        for hit in &response.hits {
            out.push_str(&format!(
                "-- hit turn_id={} record_type={} session_key={}\n{}\n",
                hit.turn_id, hit.record_type, hit.session_key, hit.excerpt
            ));
        }

        // The anchor for the other two projections: the top hit of this same
        // search, so the document has one turn it says three things about.
        let Some(anchor) = response.hits.first().map(|hit| hit.turn_id) else {
            out.push_str("-- no anchor: this query matched nothing\n");
            continue;
        };

        let window = context::window(&conn, &config, &scope, anchor, CONTEXT_SIDE, CONTEXT_SIDE)
            .unwrap();
        out.push_str(&format!("\n== context {project} anchor={anchor}\n"));
        for turn in &window.turns {
            out.push_str(&format!(
                "-- turn turn_id={} turn_seq={} record_type={} anchor={}\n{}\n",
                turn.turn_id, turn.turn_seq, turn.record_type, turn.is_anchor, turn.text
            ));
        }

        let fetched = get::records(&conn, &config, &scope, &[anchor]).unwrap();
        out.push_str(&format!("\n== get {project} anchor={anchor}\n"));
        for record in &fetched.records {
            let body = record
                .body
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            out.push_str(&format!(
                "-- record turn_id={} record_type={}\n{body}\n",
                record.turn_id, record.record_type
            ));
        }

        let cwd = bench.project(project);
        let payload = Payload {
            session_id: Some("0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55"),
            transcript_path: None,
            cwd: cwd.to_str(),
            prompt: None,
            source: Some("startup"),
        };
        let rendered = brief::session_start(&bench.data_dir, &config, &payload);
        out.push_str(&format!("\n== brief {project}\n"));
        out.push_str(rendered.as_deref().unwrap_or("(no brief)"));
        out.push('\n');
    }

    rooted(&out, bench.dir.path())
}

/// What every capture opens with, so a reader of the checked-in file knows what
/// it is and how it is meant to change.
const HEADER: &str = "\
# The four derived recall projections, captured from a store the test builds.
#
# Regenerate deliberately, never as a fix for a failing run:
#   TERMINUS_REGENERATE_RECALL_GOLDEN=1 cargo test -p terminus-core \\
#     --features testkit --test recall_golden
#
# Every temporary path is written back to the fixture root token, because the
# session keys and the brief's project label carry the tempdir and a golden
# holding it would differ on every run for a reason that is not the product.
";

/// The captured text with the tempdir written back to
/// [`testkit::FIXTURE_ROOT_TOKEN`].
///
/// The escaped spelling first and the plain one second: a `cwd` inside an
/// archived JSON body carries a Windows root with its separators doubled, and
/// replacing the plain form first would leave a stray backslash behind.
fn rooted(text: &str, dir: &Path) -> String {
    let raw = dir.to_string_lossy().into_owned();
    let escaped = raw.replace('\\', "\\\\");
    let substituted = text
        .replace(&escaped, testkit::FIXTURE_ROOT_TOKEN)
        .replace(&raw, testkit::FIXTURE_ROOT_TOKEN);
    forward_slashes(&substituted)
}

/// Separators normalized to `/` inside every token-rooted path, and nowhere
/// else.
///
/// One document for every platform, and the substitution above only takes the
/// root away: what follows it is still spelled with whatever separator the
/// platform uses. Bounded to the run that follows the token rather than applied
/// to the whole document on purpose - an archived record body is JSON, where a
/// backslash is an escape and rewriting one would corrupt the very bytes this
/// golden exists to pin.
fn forward_slashes(text: &str) -> String {
    let token = testkit::FIXTURE_ROOT_TOKEN;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(token) {
        out.push_str(&rest[..at + token.len()]);
        rest = &rest[at + token.len()..];

        let mut chars = rest.char_indices().peekable();
        let mut end = rest.len();
        while let Some((index, c)) = chars.next() {
            match c {
                '\\' => {
                    out.push('/');
                    // A doubled separator is one separator that was escaped for
                    // JSON, so it collapses to one `/` rather than to two.
                    if chars.peek().map(|(_, next)| *next) == Some('\\') {
                        chars.next();
                    }
                }
                // Whitespace and the delimiters a path can sit inside end the
                // run; anything else is still part of it.
                c if c.is_whitespace() || c == '"' || c == ',' => {
                    end = index;
                    break;
                }
                c => out.push(c),
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(GOLDEN)
}

/// The first line that differs, with both sides, and how many lines each has.
fn diff(expected: &str, actual: &str) -> String {
    for (n, (want, got)) in expected.lines().zip(actual.lines()).enumerate() {
        if want != got {
            return format!(
                "line {}:\n  golden: {want:?}\n  actual: {got:?}",
                n + 1
            );
        }
    }
    format!(
        "no line differs; the golden has {} lines and this build produced {}",
        expected.lines().count(),
        actual.lines().count()
    )
}

#[test]
fn the_four_projections_are_what_the_golden_says() {
    let bench = bench();
    let actual = capture(&bench);
    let path = golden_path();

    if std::env::var_os(REGENERATE_ENV).is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        panic!(
            "regenerated {} - unset {REGENERATE_ENV} and run again to compare",
            path.display()
        );
    }

    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(
        expected == actual,
        "the recall projections moved against {}\n{}",
        path.display(),
        diff(&expected, &actual)
    );
}

/// The golden is only worth comparing against if it visibly holds text a filter
/// would have to change.
///
/// Asserted here rather than left to a `grep` in a plan: a golden captured from
/// a corpus with nothing credential-shaped in it would be byte-identical in
/// both settings for the wrong reason, and every later assertion about the
/// default would still pass.
#[test]
fn the_golden_carries_an_unfiltered_sentinel() {
    let golden = std::fs::read_to_string(golden_path()).unwrap();
    assert!(
        golden.contains("sk-VBEGRESS-authz-9f2"),
        "the golden holds no planted credential, so it cannot show a filter turning on"
    );
}
