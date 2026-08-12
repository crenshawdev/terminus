//! Discovery over a tree built to the real layout.
//!
//! Every case here is one of D-16's measured facts: the two filename rules, the
//! depth-6 sidecar a bounded walk drops, and the four kinds of `.jsonl`-adjacent
//! file that must never become a session. Plus D-22's pre-open exclusion, whose
//! whole claim is that an excluded project is not even listed.

use std::path::{Path, PathBuf};

use verbatim_core::config::Config;
use verbatim_core::discover;

const PROJECT: &str = "-data-projects-cadence";

const UUID_A: &str = "11111111-1111-4111-8111-111111111111";
const UUID_B: &str = "22222222-2222-4222-8222-222222222222";
const SESSION_DIR: &str = "33333333-3333-4333-8333-333333333333";

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"{}\n").unwrap();
}

/// One project directory carrying every shape the real tree puts in one:
/// two top-level transcripts, a sidecar at depth 4, a sidecar at depth 6, and
/// the four kinds of file that sit beside them and are not transcripts.
fn populate(project: &Path) {
    touch(&project.join(format!("{UUID_A}.jsonl")));
    touch(&project.join(format!("{UUID_B}.jsonl")));
    touch(&project.join("not-a-uuid.jsonl"));
    touch(&project.join("agent-a.meta.json"));
    touch(&project.join("wf_x.json"));
    touch(&project.join("tool-results").join("out.txt"));

    let subagents = project.join(SESSION_DIR).join("subagents");
    touch(&subagents.join("agent-a.jsonl"));
    touch(&subagents.join("agent-a.meta.json"));
    let deep = subagents.join("workflows").join("wf_x");
    touch(&deep.join("agent-deep.jsonl"));
    touch(&deep.join("journal.jsonl"));
}

/// A transcript root - a `projects` directory - holding one populated project.
fn tree() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("claude").join("projects");
    populate(&root.join(PROJECT));
    (dir, root)
}

fn no_exclusions() -> Config {
    Config::from_parts(vec![], vec![])
}

fn relative(found: &[PathBuf], root: &Path) -> Vec<String> {
    found
        .iter()
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or_else(|_| panic!("{} is not under {}", p.display(), root.display()))
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

fn expected() -> Vec<String> {
    let mut want = vec![
        format!("{PROJECT}/{UUID_A}.jsonl"),
        format!("{PROJECT}/{UUID_B}.jsonl"),
        format!("{PROJECT}/{SESSION_DIR}/subagents/agent-a.jsonl"),
        format!("{PROJECT}/{SESSION_DIR}/subagents/workflows/wf_x/agent-deep.jsonl"),
    ];
    want.sort();
    want
}

/// D-16 in full: both filename rules, the depth-6 sidecar, and nothing else.
#[test]
fn discovery_returns_both_transcript_shapes_and_no_other_file() {
    let (_dir, root) = tree();
    let found = discover::discover_root(&root, &no_exclusions());

    assert!(found.unreadable.is_empty(), "{:?}", found.unreadable);
    assert_eq!(relative(&found.transcripts, &root), expected());

    // Named negatives, so a regression says which rule broke.
    let names: Vec<String> = relative(&found.transcripts, &root);
    for forbidden in [
        "not-a-uuid.jsonl",
        "agent-a.meta.json",
        "wf_x.json",
        "out.txt",
        "journal.jsonl",
    ] {
        assert!(
            !names.iter().any(|n| n.ends_with(forbidden)),
            "{forbidden} was discovered as a transcript: {names:?}"
        );
    }
}

/// The 41 real files a depth limit drops, stated on its own so a bounded walk
/// fails here and not three tasks downstream.
#[test]
fn the_depth_six_sidecar_is_reached() {
    let (_dir, root) = tree();
    let found = discover::discover_root(&root, &no_exclusions());
    assert!(
        relative(&found.transcripts, &root)
            .iter()
            .any(|p| p.ends_with("subagents/workflows/wf_x/agent-deep.jsonl")),
        "an unbounded walk is what reaches the depth-6 sidecar"
    );
}

/// Two passes over one tree must walk it identically.
#[test]
fn two_walks_of_one_tree_return_the_same_order() {
    let (_dir, root) = tree();
    let config = no_exclusions();
    let first = discover::discover_root(&root, &config);
    let second = discover::discover_root(&root, &config);
    assert_eq!(first.transcripts, second.transcripts);
    assert_eq!(first, second);
}

/// D-17. The root is canonicalized once and every entry is joined onto it, so a
/// root reached through a symlink still produces the session keys ingest writes.
#[cfg(unix)]
#[test]
fn every_path_begins_with_the_canonical_root_even_through_a_symlink() {
    let (dir, root) = tree();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&root, &link).unwrap();

    let canonical = root.canonicalize().unwrap();
    let found = discover::discover_root(&link, &no_exclusions());

    assert_eq!(relative(&found.transcripts, &canonical), expected());
    for path in &found.transcripts {
        assert!(
            path.starts_with(&canonical),
            "{} was not joined onto the canonical root",
            path.display()
        );
        assert!(
            !path.starts_with(&link),
            "{} still runs through the symlink",
            path.display()
        );
    }
}

/// D-22 and D-09 together: the excluded project yields nothing, and the sibling
/// whose encoded name merely extends it is walked in full.
///
/// The excluded repository is built for real, because the pre-open test
/// resolves an extension of an excluded name against the filesystem: with no
/// `<repo>/research` on disk, the sibling's encoded name has exactly one real
/// path behind it and that path is not excluded. An excluded path that does not
/// exist cannot answer the question and excludes the sibling too, which is what
/// `an_unresolvable_exclusion_still_excludes` in the config tests pins.
#[test]
fn an_excluded_project_yields_nothing_while_its_hyphenated_sibling_is_walked() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("claude").join("projects");

    // The real tree the encoded names above describe.
    let repo = dir.path().join("data").join("projects").join("cadence");
    std::fs::create_dir_all(repo.join(".claude").join("worktrees").join("wt-a")).unwrap();
    std::fs::create_dir_all(
        dir.path()
            .join("data")
            .join("projects")
            .join("cadence-research"),
    )
    .unwrap();

    let encoded = |path: &Path| verbatim_core::config::encode(path.to_str().unwrap());
    let project = encoded(&repo);
    let sibling_name = encoded(
        &dir.path()
            .join("data")
            .join("projects")
            .join("cadence-research"),
    );
    let worktree_name = encoded(&repo.join(".claude").join("worktrees").join("wt-a"));

    populate(&root.join(&project));
    populate(&root.join(&sibling_name));
    populate(&root.join(&worktree_name));

    let config = Config::from_parts(vec![], vec![repo.to_str().unwrap().to_owned()]);
    let found = discover::discover_root(&root, &config);
    let names = relative(&found.transcripts, &root);

    assert!(
        !names.iter().any(|n| n.starts_with(&format!("{project}/"))),
        "the excluded project was walked: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with(&format!("{worktree_name}/"))),
        "D-06 folds a worktree into its repo, so excluding the repo excludes it: {names:?}"
    );

    let sibling: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with(&format!("{sibling_name}/")))
        .map(|n| n[sibling_name.len() + 1..].to_owned())
        .collect();
    assert_eq!(
        sibling,
        expected()
            .iter()
            .map(|p| p[PROJECT.len() + 1..].to_owned())
            .collect::<Vec<_>>(),
        "the hyphenated sibling must be walked in full"
    );

    assert_eq!(found.transcripts.len(), 4, "only the sibling contributes");
    assert_eq!(
        found.excluded.len(),
        2,
        "both excluded directories reported"
    );
}

/// A root that has never existed is the state of a machine that has never run
/// Claude Code. It yields nothing and reports nothing.
#[test]
fn a_missing_root_is_empty_rather_than_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let found = discover::discover_root(&dir.path().join("nope"), &no_exclusions());
    assert_eq!(found, discover::Discovered::default());
}

/// An unreadable directory is named and skipped: one of them is not a reason to
/// archive nothing.
#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_reported_and_the_rest_is_still_walked() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, root) = tree();
    let locked = root.join(PROJECT).join(SESSION_DIR).join("subagents");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let found = discover::discover_root(&root, &no_exclusions());
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    if found.unreadable.is_empty() {
        // Running as root defeats the mode bits; the assertion below would then
        // be about nothing.
        println!("skipped: this process can read a 0000 directory");
        return;
    }
    assert_eq!(found.unreadable[0].0, locked);
    assert_eq!(
        relative(&found.transcripts, &root),
        vec![
            format!("{PROJECT}/{UUID_A}.jsonl"),
            format!("{PROJECT}/{UUID_B}.jsonl"),
        ],
        "the readable half of the project must still be discovered"
    );
}

/// The counted-open helper records the path, which is what makes AC5's "zero
/// opens under that directory" attributable rather than aggregate.
#[cfg(feature = "testkit")]
#[test]
fn opening_a_transcript_records_its_path() {
    let (_dir, root) = tree();
    let path = root.join(PROJECT).join(format!("{UUID_A}.jsonl"));

    discover::opened::reset();
    assert!(discover::opened::under(&root).is_empty());

    discover::open_transcript(&path).expect("the fixture opens");
    assert_eq!(discover::opened::under(&root), vec![path.clone()]);

    // A failed open still counts: AC5 is about what verbatim reaches for.
    let missing = root.join(PROJECT).join("no-such-file.jsonl");
    assert!(discover::open_transcript(&missing).is_err());
    assert_eq!(discover::opened::under(&root), vec![path, missing]);
}
