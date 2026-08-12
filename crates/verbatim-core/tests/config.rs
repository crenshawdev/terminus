//! Verbatim's own config: the roots it walks and the projects it never opens.
//!
//! The exclusion tests are the load-bearing half. D-09 fixes what the pre-open
//! test may match and D-07 is why it may not be a prefix match, so each case
//! below is one of those two decisions stated as an assertion.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use verbatim_core::config::{
    Config, CLAUDE_CONFIG_DIR_ENV, CONFIG_FILE_NAME, DEFAULT_CLAUDE_DIR, PROJECTS_SUBDIR,
};
use verbatim_core::Error;

/// `std::env::set_var` is process-global and the harness is threaded, so every
/// test that touches the environment holds this.
static ENV: Mutex<()> = Mutex::new(());

/// Set a variable, run the closure, put the variable back exactly as it was.
fn with_var<T>(name: &str, value: Option<&str>, body: impl FnOnce() -> T) -> T {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var_os(name);
    match value {
        Some(v) => std::env::set_var(name, v),
        None => std::env::remove_var(name),
    }
    let out = body();
    match previous {
        Some(v) => std::env::set_var(name, v),
        None => std::env::remove_var(name),
    }
    out
}

fn config_dir_holding(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in files {
        std::fs::write(dir.path().join(name), body).unwrap();
    }
    dir
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is set on this platform"))
}

/// The state every user starts in: no file at all, one root, `~/.claude`.
#[test]
fn a_missing_config_file_yields_the_default_single_root() {
    let dir = config_dir_holding(&[]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect("a missing config file is not an error")
    });

    assert_eq!(config.roots(), [home().join(DEFAULT_CLAUDE_DIR)]);
    assert_eq!(
        config.transcript_roots(),
        vec![home().join(DEFAULT_CLAUDE_DIR).join(PROJECTS_SUBDIR)],
        "the tree walked for a root is its projects subdirectory"
    );
    assert!(config.exclusions().is_empty());
}

/// D-15: `CLAUDE_CONFIG_DIR` is read as a single directory, and it replaces the
/// default. An empty value is not a directory and must not become one.
#[test]
fn claude_config_dir_replaces_the_default_and_an_empty_value_does_not() {
    let dir = config_dir_holding(&[]);
    let elsewhere = tempfile::tempdir().unwrap();

    let overridden = with_var(
        CLAUDE_CONFIG_DIR_ENV,
        Some(elsewhere.path().to_str().unwrap()),
        || Config::load_from(dir.path()).unwrap(),
    );
    assert_eq!(overridden.roots(), [elsewhere.path().to_path_buf()]);

    let empty = with_var(CLAUDE_CONFIG_DIR_ENV, Some(""), || {
        Config::load_from(dir.path()).unwrap()
    });
    assert_eq!(
        empty.roots(),
        [home().join(DEFAULT_CLAUDE_DIR)],
        "an empty CLAUDE_CONFIG_DIR resolves to the process's current directory"
    );
}

/// Roots are a list, and the order is the user's.
#[test]
fn a_config_naming_two_roots_yields_both_in_order() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        "roots = [\"/one/.claude\", \"/two/.claude\"]\n",
    )]);

    // Set even so: an explicit list is verbatim's own configuration and
    // outranks Claude Code's environment.
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, Some("/ignored"), || {
        Config::load_from(dir.path()).unwrap()
    });

    assert_eq!(
        config.roots(),
        [PathBuf::from("/one/.claude"), PathBuf::from("/two/.claude")]
    );
}

/// D-15's whole point. The legacy tool's `config.toml` sits in this exact
/// directory on the development machine holding `base_dir = "/data/verbatim"`,
/// and adopting it would point the new store at the legacy data directory whose
/// import PROJECT.md defers.
#[test]
fn the_legacy_config_toml_beside_ours_is_never_read() {
    let dir = config_dir_holding(&[("config.toml", "base_dir = \"/data/verbatim\"\n")]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect("a legacy config.toml is not ours to fail on")
    });

    assert_eq!(config.roots(), [home().join(DEFAULT_CLAUDE_DIR)]);
    for root in config.roots() {
        assert!(
            !root.starts_with("/data/verbatim"),
            "the legacy base_dir was adopted: {}",
            root.display()
        );
    }
    for root in config.transcript_roots() {
        assert!(!root.starts_with("/data/verbatim"));
    }
}

/// A file the user wrote that verbatim cannot read must not fall back to
/// defaults: that walks a tree nobody asked for and honors no exclusion.
#[test]
fn a_config_file_that_does_not_parse_is_named_rather_than_ignored() {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, "roots = [\"/one\"\nexclude = ]\n")]);
    let err = Config::load_from(dir.path()).expect_err("malformed TOML must be refused");
    assert!(matches!(err, Error::ConfigParse { .. }), "{err:?}");

    let message = err.to_string();
    assert!(
        message.contains(CONFIG_FILE_NAME),
        "the message must name the file: {message}"
    );
    // The parser's own position, so the user can find the line.
    assert!(
        message.contains('2') || message.contains("line"),
        "the message must carry the parse position: {message}"
    );
}

fn excluding(paths: &[&str]) -> Config {
    Config::from_parts(vec![], paths.iter().map(|p| (*p).to_owned()).collect())
}

/// D-09 exactly: the encoded name matches, its worktree directories match, and
/// a hyphenated sibling does not.
#[test]
fn the_encoded_test_matches_the_project_and_its_worktrees_and_nothing_else() {
    let config = excluding(&["/data/projects/cadence"]);

    assert!(config.excludes_encoded_dir("-data-projects-cadence"));
    assert!(
        config.excludes_encoded_dir("-data-projects-cadence--claude-worktrees-agent-a33a"),
        "D-06 folds a worktree into its parent repo, so excluding the repo must \
         exclude the worktree before anything is opened"
    );
    assert!(
        !config.excludes_encoded_dir("-data-projects-cadence-research"),
        "D-07 proved the encoding is lossy here, so an extension-tolerant rule \
         cannot tell a child directory from a hyphenated sibling"
    );
    assert!(!config.excludes_encoded_dir("-data-projects-hindsight"));

    // Nothing configured excludes nothing.
    assert!(!excluding(&[]).excludes_encoded_dir("-data-projects-cadence"));
}

/// The read-side test keeps full subtree semantics, because a real path has no
/// ambiguity to protect against.
#[test]
fn the_path_test_covers_the_subtree_and_stops_at_a_segment_boundary() {
    let config = excluding(&["/data/projects/cadence"]);

    assert!(config.excludes_path(Path::new("/data/projects/cadence")));
    assert!(config.excludes_path(Path::new("/data/projects/cadence/sub")));
    assert!(config.excludes_path(Path::new("/data/projects/cadence/.claude/worktrees/wt-a")));
    assert!(
        !config.excludes_path(Path::new("/data/projects/cadence-research")),
        "a hyphenated sibling is a different project"
    );
    assert!(!config.excludes_path(Path::new("/data/projects")));

    // A trailing separator in the config is not a different exclusion.
    assert!(excluding(&["/data/projects/cadence/"])
        .excludes_path(Path::new("/data/projects/cadence/sub")));
}
