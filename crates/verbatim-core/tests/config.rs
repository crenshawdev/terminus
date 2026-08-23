//! Verbatim's own config: the roots it walks and the projects it never opens.
//!
//! The exclusion tests are the load-bearing half. D-09 fixes what the pre-open
//! test may match and D-07 is why it may not be a prefix match, so each case
//! below is one of those two decisions stated as an assertion.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use verbatim_core::config::{
    CaptureMode, Config, ResponseFormat, RetentionAction, Secret, CLAUDE_CONFIG_DIR_ENV,
    CONFIG_FILE_NAME, DEFAULT_BRIEF_CHARS, DEFAULT_CLAUDE_DIR, DEFAULT_PROMPT_CHARS,
    DEFAULT_SNAPSHOTS_KEPT, DEFAULT_SNAPSHOT_INTERVAL_HOURS, PROJECTS_SUBDIR, REDACTED,
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

/// The encoded name matches exactly, and an extension of it is resolved
/// against the filesystem rather than guessed at.
///
/// D-07 proved the encoding cannot tell `<repo>/research` from
/// `<repo>-research`, and D-09 asked for both a segment-boundary match and for
/// the hyphenated sibling to be spared - not both satisfiable in the encoded
/// space, so the ambiguity is resolved outside it. Real directories here, since
/// that is now what the answer depends on.
#[test]
fn an_extension_of_an_excluded_name_is_resolved_against_the_filesystem() {
    let tree = tempfile::tempdir().unwrap();
    let repo = tree.path().join("cadence");
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::create_dir_all(repo.join(".claude").join("worktrees").join("agent-a33a")).unwrap();
    // A sibling with no colliding child under the repo: `cadence/research` does
    // not exist, so this name resolves to exactly one real path.
    std::fs::create_dir_all(tree.path().join("cadence-research")).unwrap();

    let config = excluding(&[repo.to_str().unwrap()]);
    let encoded = |path: &Path| verbatim_core::config::encode(path.to_str().unwrap());

    assert!(config.excludes_encoded_dir(&encoded(&repo)));
    assert!(
        config.excludes_encoded_dir(&encoded(&repo.join("docs"))),
        "a real subdirectory of the excluded repo must not be opened"
    );
    assert!(
        config.excludes_encoded_dir(&encoded(
            &repo.join(".claude").join("worktrees").join("agent-a33a")
        )),
        "D-06 folds a worktree into its parent repo, so excluding the repo must \
         exclude the worktree before anything is opened"
    );
    assert!(
        !config.excludes_encoded_dir(&encoded(&tree.path().join("cadence-research"))),
        "a hyphenated sibling with no colliding child under the repo resolves to \
         one real path, and that path is not excluded"
    );
    assert!(!config.excludes_encoded_dir(&encoded(&tree.path().join("hindsight"))));

    // Nothing configured excludes nothing.
    assert!(!excluding(&[]).excludes_encoded_dir(&encoded(&repo)));
}

/// When a child and a hyphenated sibling both exist, they are ONE encoded name
/// and no test given only that name can separate them - so the pre-open test
/// excludes, and the sibling's sessions are the price of not reading the
/// child's. [`Config::excludes_path`] still sees two distinct paths and hides
/// only the excluded one, so nothing already archived is lost.
#[test]
fn a_child_and_a_sibling_that_collide_resolve_to_excluded() {
    let tree = tempfile::tempdir().unwrap();
    let repo = tree.path().join("cadence");
    std::fs::create_dir_all(repo.join("research")).unwrap();
    std::fs::create_dir_all(tree.path().join("cadence-research")).unwrap();

    let config = excluding(&[repo.to_str().unwrap()]);
    let child = verbatim_core::config::encode(repo.join("research").to_str().unwrap());
    let sibling =
        verbatim_core::config::encode(tree.path().join("cadence-research").to_str().unwrap());

    assert_eq!(child, sibling, "D-07: these are the same encoded name");
    assert!(config.excludes_encoded_dir(&child));

    assert!(config.excludes_path(&repo.join("research")));
    assert!(
        !config.excludes_path(&tree.path().join("cadence-research")),
        "the read-side test has real paths and keeps them apart"
    );
}

/// The shape the real corpus is actually in: the worktree is gone, its parent
/// `worktrees` directory survives empty, and the archived sessions are still
/// there under the deleted worktree's encoded name.
///
/// A live prefix with a dead leaf means the leaf was deleted, not that the name
/// belongs to some other path - so it stays excluded. Every worktree project
/// directory on the development machine is this case (all three
/// `.claude/worktrees` directories there are empty), which is why a search that
/// answered "not a child" here would reopen the ING-08 violation rather than
/// close it.
#[test]
fn a_deleted_worktree_under_a_live_repo_is_still_excluded() {
    let tree = tempfile::tempdir().unwrap();
    let repo = tree.path().join("cadence");
    // The worktree itself is NOT created - it was deleted, like every one of
    // them on the real machine.
    std::fs::create_dir_all(repo.join(".claude").join("worktrees")).unwrap();

    let config = excluding(&[repo.to_str().unwrap()]);
    let worktree = verbatim_core::config::encode(
        repo.join(".claude")
            .join("worktrees")
            .join("agent-a33a")
            .to_str()
            .unwrap(),
    );

    assert!(
        config.excludes_encoded_dir(&worktree),
        "the worktree directory is gone but its sessions are archived under this \
         name, and D-06 folds them into the excluded repo"
    );
}

/// A branch that could still lead to the name but cannot be followed leaves the
/// question open rather than answering it by not looking.
#[cfg(unix)]
#[test]
fn a_symlinked_branch_is_unresolved_rather_than_absent() {
    let tree = tempfile::tempdir().unwrap();
    let repo = tree.path().join("cadence");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(tree.path().join("elsewhere").join("vendor")).unwrap();
    std::os::unix::fs::symlink(tree.path().join("elsewhere"), repo.join("linked")).unwrap();

    let config = excluding(&[repo.to_str().unwrap()]);
    let through_link =
        verbatim_core::config::encode(repo.join("linked").join("vendor").to_str().unwrap());

    assert!(
        config.excludes_encoded_dir(&through_link),
        "the link is not followed, so whether this is under the excluded repo is \
         unknown - and unknown is not read"
    );
}

/// An exclusion that cannot be resolved excludes: the answer is unknown and the
/// safe direction for a privacy boundary is not to read.
#[test]
fn an_unresolvable_exclusion_still_excludes() {
    let config = excluding(&["/data/projects/cadence"]);

    assert!(config.excludes_encoded_dir("-data-projects-cadence"));
    assert!(
        config.excludes_encoded_dir("-data-projects-cadence-research"),
        "the excluded directory does not exist, so whether this is a child of \
         it or a hyphenated sibling cannot be answered - and unresolved means \
         unopened"
    );
    assert!(
        !config.excludes_encoded_dir("-data-projects-hindsight"),
        "a name that does not extend the exclusion at all is never ambiguous"
    );
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

/// The two entry points read the same configured string, so they must agree
/// about which projects it names.
///
/// `excludes_path` compares components and has always been indifferent to how
/// a path is spelled; `excludes_encoded_dir` compares a string the encoding
/// produced, where every spelling is a different name and only one can equal a
/// real project directory's. Unnormalized, a trailing separator is the whole
/// ING-08 failure: the pre-open test never matches, so every file in the
/// project is opened and archived, and only the read path hides it afterwards.
#[test]
fn a_differently_spelled_exclusion_is_the_same_exclusion_to_both_tests() {
    for spelling in [
        "/data/projects/cadence/",
        "/data/projects//cadence",
        "/data/projects/./cadence",
    ] {
        let config = excluding(&[spelling]);
        assert!(
            config.excludes_encoded_dir("-data-projects-cadence"),
            "the pre-open test lost `{spelling}`, so the project would be read"
        );
        assert!(
            config.excludes_path(Path::new("/data/projects/cadence/sub")),
            "the read test lost `{spelling}`"
        );
        assert_eq!(
            config.exclusions(),
            ["/data/projects/cadence"],
            "`{spelling}` is reported to the user in the form both tests use"
        );
    }
}

/// An exclusion naming nothing is dropped rather than kept as an empty string,
/// which every encoded name has as a prefix.
#[test]
fn an_exclusion_that_names_nothing_is_dropped() {
    let config = excluding(&["", "/data/projects/cadence"]);

    assert_eq!(config.exclusions(), ["/data/projects/cadence"]);
    assert!(!config.excludes_encoded_dir("-data-projects-hindsight"));
    assert!(!config.excludes_path(Path::new("/data/projects/hindsight")));
}

/// Excluding the filesystem root means "read nothing", and both tests say so.
///
/// The encoded test needs this stated: root encodes to a bare separator, and
/// `-data` extends `-` with no second separator between them, because for root
/// the separator IS the encoding. `excludes_path` already reads `/` this way,
/// so without it the one exclusion that means "read nothing" would be the one
/// that reads everything and hides it afterwards.
#[test]
fn excluding_the_root_excludes_every_project() {
    let config = excluding(&["/"]);

    assert!(config.excludes_encoded_dir("-data-projects-cadence"));
    assert!(config.excludes_encoded_dir("-home-john--claude"));
    assert!(config.excludes_path(Path::new("/data/projects/cadence")));
}

/// `..` and a leading `~` are spellings a user writes, and neither predicate
/// can match either one left standing.
///
/// This is worse than the trailing separator above rather than the same bug: a
/// trailing separator at least still hid the sessions on the read path, while
/// an exclusion carrying `..` matches nothing anywhere, so the project is read,
/// archived and searchable with nothing anywhere reporting that the exclusion
/// did not take.
#[test]
fn an_exclusion_spelled_with_a_parent_or_a_tilde_still_names_its_project() {
    let config = excluding(&["/data/projects/../projects/cadence"]);
    assert_eq!(config.exclusions(), ["/data/projects/cadence"]);
    assert!(config.excludes_encoded_dir("-data-projects-cadence"));
    assert!(config.excludes_path(Path::new("/data/projects/cadence/sub")));

    let encoded = |path: &Path| verbatim_core::config::encode(path.to_str().unwrap());
    let home = home();
    let config = excluding(&["~/code/demo"]);
    assert_eq!(
        config.exclusions(),
        [home.join("code").join("demo").to_str().unwrap()]
    );
    assert!(config.excludes_encoded_dir(&encoded(&home.join("code").join("demo"))));
    assert!(config.excludes_path(&home.join("code").join("demo").join("sub")));

    // A relative exclusion names no project - a project directory encodes a
    // `cwd`, which is always absolute - so it is dropped rather than kept as an
    // exclusion that silently matches nothing.
    for relative in ["../cadence", "code/demo", "cadence", "~john/code/demo"] {
        assert!(
            excluding(&[relative]).exclusions().is_empty(),
            "`{relative}` was kept as an exclusion no path can match"
        );
    }
}

// ---------------------------------------------------------------------------
// The `[injection]` table (INJ-01, D-18)

/// Every key present: the file's numbers are the budgets, not the defaults.
#[test]
fn an_injection_table_resolves_to_the_numbers_it_names() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        "[injection]\nbrief_chars = 1200\nprompt_chars = 800\n",
    )]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    assert_eq!(config.brief_chars(), 1200);
    assert_eq!(config.prompt_chars(), 800);
}

/// The state every user starts in. A file with no `[injection]` table at all -
/// and the missing file the test above this section covers - resolves to the
/// documented defaults rather than to zero, which would be a config that
/// silently injects nothing.
#[test]
fn a_config_without_an_injection_table_resolves_to_the_defaults() {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, "exclude = [\"/data/private\"]\n")]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    assert_eq!(config.brief_chars(), DEFAULT_BRIEF_CHARS);
    assert_eq!(config.prompt_chars(), DEFAULT_PROMPT_CHARS);
    assert_eq!(
        Config::default().brief_chars(),
        DEFAULT_BRIEF_CHARS,
        "a config built with no file carries the same budgets as one read from a file with no table"
    );
    assert_eq!(Config::default().prompt_chars(), DEFAULT_PROMPT_CHARS);
}

/// One key configured leaves the other at its default, which is why each is an
/// `Option` on the way in rather than a `#[serde(default)]` zero.
#[test]
fn one_configured_budget_does_not_take_the_other_with_it() {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, "[injection]\nprompt_chars = 250\n")]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    assert_eq!(config.prompt_chars(), 250);
    assert_eq!(config.brief_chars(), DEFAULT_BRIEF_CHARS);
}

/// The documented rule holds one level down: this file grows every phase, and a
/// key a later build writes must not make this one refuse to start.
#[test]
fn an_unknown_key_inside_the_injection_table_is_ignored_rather_than_rejected() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        "[injection]\nbrief_chars = 900\nsomething_phase_9_writes = \"whatever\"\n",
    )]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect("an unknown injection key is not an error")
    });

    assert_eq!(config.brief_chars(), 900);
    assert_eq!(config.prompt_chars(), DEFAULT_PROMPT_CHARS);
}

// ---------------------------------------------------------------------------
// The `[provider]` table (OBS-05, D-05, D-10, D-13)

/// A value distinctive enough that finding any part of it in a rendered string
/// is unambiguous evidence of a leak rather than a coincidence.
const KEY: &str = "sk-VERBATIMTESTKEY-9f3a1c-do-not-log";

/// The state every user starts in, and the one OBS-02 requires: a config with
/// no `[provider]` table asks no model anything and holds no credential.
#[test]
fn a_config_without_a_provider_table_is_not_enabled_and_has_no_key() {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, "exclude = [\"/data/private\"]\n")]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    assert!(!config.provider_enabled());
    assert!(config.provider_api_key().is_none());
    assert_eq!(config.provider_base_url(), None);
    assert_eq!(config.provider_model(), None);
    assert_eq!(config.provider_name(), None);
    assert_eq!(config.provider_daily_token_budget(), None);
    assert!(
        !config.provider_local(),
        "an absent `local` means remote and therefore filtered (D-13)"
    );
    assert!(
        !Config::default().provider_enabled(),
        "a config built with no file at all agrees with one read from a file with no table"
    );
}

/// One key configured leaves the rest alone, which is why every key is an
/// `Option` on the way in rather than a `#[serde(default)]` zero.
#[test]
fn a_provider_table_naming_only_a_model_leaves_the_other_keys_at_their_defaults() {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, "[provider]\nmodel = \"qwen3:8b\"\n")]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    assert_eq!(config.provider_model(), Some("qwen3:8b"));
    assert!(
        !config.provider_enabled(),
        "naming a model is not asking for judgment"
    );
    assert!(
        !config.provider_local(),
        "a table that does not declare its destination is remote (D-13)"
    );
    assert_eq!(config.provider_base_url(), None);
    assert_eq!(config.provider_name(), None);
    assert!(config.provider_api_key().is_none());
    assert_eq!(config.provider_daily_token_budget(), None);
}

/// Every key at once, so the accessors are proven to read the keys they name
/// rather than each other.
#[test]
fn a_full_provider_table_resolves_to_the_values_it_names() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        &format!(
            "[provider]\n\
             enabled = true\n\
             base_url = \"http://localhost:11434/v1/\"\n\
             model = \"qwen3:8b\"\n\
             name = \"ollama\"\n\
             api_key = \"{KEY}\"\n\
             local = true\n\
             daily_token_budget = 50000\n"
        ),
    )]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    assert!(config.provider_enabled());
    assert_eq!(
        config.provider_base_url(),
        Some("http://localhost:11434/v1/")
    );
    assert_eq!(config.provider_model(), Some("qwen3:8b"));
    assert_eq!(config.provider_name(), Some("ollama"));
    assert!(config.provider_local());
    assert_eq!(config.provider_daily_token_budget(), Some(50_000));
    assert_eq!(
        config.provider_api_key().map(Secret::expose),
        Some(KEY),
        "the one accessor that reaches the value has to reach it"
    );
    assert_eq!(
        config.provider_response_format(),
        ResponseFormat::JsonSchema,
        "a table naming no response_format asks for the default mode"
    );
}

/// D-09's mode key, and why it exists: `deepseek-chat` answers 400 `This
/// response_format type is unavailable now` to `json_schema` however
/// well-formed, and takes `json_object`. Measured 2026-08-22.
#[test]
fn the_response_format_key_selects_the_mode_and_defaults_to_json_schema() {
    for (written, expected) in [
        ("json_schema", ResponseFormat::JsonSchema),
        ("json_object", ResponseFormat::JsonObject),
        ("JSON_OBJECT", ResponseFormat::JsonObject),
        ("  json_object  ", ResponseFormat::JsonObject),
        ("none", ResponseFormat::None),
        // An unrecognized VALUE falls back the way an unrecognized KEY is
        // ignored: this file grows every phase and a typo must not stop a
        // store from opening.
        ("tool_calling", ResponseFormat::JsonSchema),
        ("", ResponseFormat::JsonSchema),
    ] {
        let dir = config_dir_holding(&[(
            CONFIG_FILE_NAME,
            &format!("[provider]\nenabled = true\nresponse_format = \"{written}\"\n"),
        )]);
        let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
            Config::load_from(dir.path()).unwrap()
        });

        assert_eq!(
            config.provider_response_format(),
            expected,
            "response_format = {written:?}"
        );
    }
}

/// The documented rule holds one level down: this file grows every phase, and a
/// key a later build writes must not make this one refuse to start.
#[test]
fn an_unknown_key_inside_the_provider_table_is_ignored_rather_than_rejected() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        "[provider]\nmodel = \"qwen3:8b\"\nsomething_phase_9_writes = \"whatever\"\n",
    )]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect("an unknown provider key is not an error")
    });

    assert_eq!(config.provider_model(), Some("qwen3:8b"));
}

/// PRIV-01: `Config` derives `Debug`, and nothing that derive can reach may
/// render the key.
#[test]
fn a_config_carrying_an_api_key_formats_with_no_byte_of_it() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        &format!("[provider]\nenabled = true\napi_key = \"{KEY}\"\n"),
    )]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    // The falsifying half: the key really is in there, so the assertions below
    // are about a formatter and not about an empty config.
    assert_eq!(config.provider_api_key().map(Secret::expose), Some(KEY));

    let rendered = format!("{config:?}");
    for fragment in [KEY, "VERBATIMTESTKEY", "9f3a1c"] {
        assert!(
            !rendered.contains(fragment),
            "the formatted config carries {fragment:?}: {rendered}"
        );
    }
    assert!(
        rendered.contains(REDACTED),
        "the formatted config does not say a credential was withheld: {rendered}"
    );

    let secret = config.provider_api_key().unwrap();
    assert_eq!(format!("{secret:?}"), REDACTED);
    assert_eq!(format!("{secret}"), REDACTED);
}

/// The other way a config file can put a key on a stream: forget the quotes and
/// let the TOML parser echo the line back (PRIV-01).
#[test]
fn a_malformed_api_key_line_is_not_echoed_by_the_parse_error() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        // Unquoted, which is a syntax error, and the parser's rendering of it
        // points a caret at the value.
        &format!("[provider]\napi_key = {KEY}\n"),
    )]);
    let error = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect_err("an unquoted value does not parse")
    });

    let rendered = error.to_string();
    for fragment in [KEY, "VERBATIMTESTKEY", "9f3a1c"] {
        assert!(
            !rendered.contains(fragment),
            "the parse error echoed {fragment:?}: {rendered}"
        );
    }
    assert!(
        rendered.contains("line 2"),
        "the parse error withheld the position too, so it names nothing findable: {rendered}"
    );
    assert!(
        matches!(error, Error::ConfigParse { .. }),
        "an unparseable config is still a parse error: {error:?}"
    );
}

/// An empty string is not a value, in a config file as in an environment
/// variable: `api_key = ""` must not outrank a real key in the shared file, and
/// `base_url = ""` must not become a request against the empty string.
#[test]
fn an_empty_provider_string_is_the_same_as_an_absent_one() {
    let dir = config_dir_holding(&[(
        CONFIG_FILE_NAME,
        "[provider]\nenabled = true\nbase_url = \"\"\napi_key = \"\"\nname = \"\"\n",
    )]);
    let config = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).unwrap()
    });

    assert!(config.provider_enabled());
    assert_eq!(config.provider_base_url(), None);
    assert_eq!(config.provider_name(), None);
    assert!(config.provider_api_key().is_none());
}

// RET-01 and D-01: retention is off unless `verbatim.toml` says otherwise, and
// a per-project rule is matched through the same subtree test the exclusions
// use rather than by string equality against `session_meta.project`.

/// Load a config out of a directory holding just this `verbatim.toml` body.
fn retention_config(body: &str) -> Config {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, body)]);
    with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect("a retention table is not a parse failure")
    })
}

/// RET-01's off state, from both directions it can be reached: no file at all,
/// and a file that configures something else entirely. Neither may select a
/// session, and a store whose every session predates any plausible default has
/// to come out of a pass unchanged because of this.
#[test]
fn a_config_with_no_retention_table_selects_nothing() {
    let missing = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(config_dir_holding(&[]).path()).unwrap()
    });
    let unrelated = retention_config("[injection]\nbrief_chars = 500\n");

    for (name, config) in [("no file", &missing), ("no table", &unrelated)] {
        assert!(
            config.retention_selects_nothing(),
            "{name}: retention is on with nothing configuring it"
        );
        let policy = config.retention_for(Some("/data/code/scratch"));
        assert_eq!(policy.action, RetentionAction::Keep, "{name}");
        assert_eq!(policy.age_days, 0, "{name}");
        assert!(policy.selects_nothing(), "{name}");
    }
}

/// D-01: the per-project key is a subtree, matched through `covers`. The
/// hyphenated sibling is the case a string prefix would get wrong, and it is
/// the same one D-07 proved the encoded exclusion test cannot decide.
#[test]
fn a_project_rule_covers_the_subtree_and_stops_at_a_segment_boundary() {
    let config = retention_config(
        "[retention.project.\"/data/code/scratch\"]\naction = \"evict\"\nage_days = 30\n",
    );

    assert!(!config.retention_selects_nothing());

    let inside = config.retention_for(Some("/data/code/scratch/sub"));
    assert_eq!(inside.action, RetentionAction::Evict);
    assert_eq!(inside.age_days, 30);

    let itself = config.retention_for(Some("/data/code/scratch"));
    assert_eq!(
        itself.action,
        RetentionAction::Evict,
        "the key covers itself"
    );

    for elsewhere in ["/data/code/scratch-other", "/data/code", "/elsewhere"] {
        let policy = config.retention_for(Some(elsewhere));
        assert_eq!(
            policy.action,
            RetentionAction::Keep,
            "{elsewhere} took a rule written for /data/code/scratch"
        );
        assert!(policy.selects_nothing(), "{elsewhere}");
    }

    assert!(
        config.retention_for(None).selects_nothing(),
        "a session with no project key falls back to the global table, which is off here"
    );
}

/// Two keys cover the same session and the deeper one decides, which is the
/// rule `recall::scope` already applies to project keys. A user who names a
/// tree and then carves one project out of it means the carve-out.
#[test]
fn the_deepest_configured_project_key_wins() {
    let config = retention_config(
        "[retention]\naction = \"evict\"\nage_days = 400\n\n         [retention.project.\"/data/code\"]\naction = \"evict\"\nage_days = 90\n\n         [retention.project.\"/data/code/scratch\"]\naction = \"delete\"\nage_days = 7\n",
    );

    let deep = config.retention_for(Some("/data/code/scratch/sub"));
    assert_eq!(deep.action, RetentionAction::Delete);
    assert_eq!(deep.age_days, 7);

    let shallow = config.retention_for(Some("/data/code/other"));
    assert_eq!(shallow.action, RetentionAction::Evict);
    assert_eq!(shallow.age_days, 90);

    let global = config.retention_for(Some("/elsewhere"));
    assert_eq!(global.age_days, 400, "nothing covers this one");
}

/// A typo in `action` resolves to keep rather than failing the load, under the
/// same rule `ResponseFormat::parse` states - and here the alternative is worse
/// than a wrong mode: an action nobody meant must never be a deletion.
#[test]
fn an_unrecognized_action_resolves_to_keep_rather_than_failing_the_load() {
    let config = retention_config("[retention]\naction = \"nonsense\"\nage_days = 30\n");

    let policy = config.retention_for(Some("/data/code/scratch"));
    assert_eq!(policy.action, RetentionAction::Keep);
    assert_eq!(policy.age_days, 30, "the age it did understand is kept");
    assert!(
        policy.selects_nothing(),
        "keep names no session however old it is"
    );
    assert!(config.retention_selects_nothing());
}

/// Both halves have to be set. An action with no age names every session ever
/// archived, so an absent, zero or negative `age_days` is the off state - and a
/// negative one is clamped rather than rejected, because a config that refuses
/// to load stops every command including the ones that would show the mistake.
#[test]
fn an_action_without_a_positive_age_selects_nothing() {
    for body in [
        "[retention]\naction = \"delete\"\n",
        "[retention]\naction = \"delete\"\nage_days = 0\n",
        "[retention]\naction = \"delete\"\nage_days = -30\n",
    ] {
        let config = retention_config(body);
        let policy = config.retention_for(Some("/data/code/scratch"));
        assert_eq!(policy.action, RetentionAction::Delete, "{body:?}");
        assert_eq!(policy.age_days, 0, "{body:?}");
        assert!(policy.selects_nothing(), "{body:?}");
        assert!(config.retention_selects_nothing(), "{body:?}");
    }
}

/// D-01: a project key is spelled by a human and normalized by the same
/// function the exclusions go through, so a trailing separator, a `..` or a
/// `~` still names its project - and a key that names nothing at all is
/// dropped rather than kept as a prefix of every path there is.
#[test]
fn a_project_key_is_normalized_the_way_an_exclusion_is() {
    let config = retention_config(
        "[retention.project.\"/data/code/../code/scratch/\"]\naction = \"evict\"\nage_days = 5\n\n         [retention.project.\"code/relative\"]\naction = \"delete\"\nage_days = 5\n",
    );

    assert_eq!(
        config.retention_for(Some("/data/code/scratch/sub")).action,
        RetentionAction::Evict,
        "a key spelled with a parent component and a trailing separator names its project"
    );
    assert!(
        config
            .retention_for(Some("/anything/at/all"))
            .selects_nothing(),
        "a relative key matches no absolute project key and must not become one that matches all"
    );
}

// ---------------------------------------------------------------------------
// The `[snapshot]` table (STOR-06, D-15)

fn snapshot_config(body: &str) -> Config {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, body)]);
    with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect("a snapshot table is not a parse failure")
    })
}

/// The state every user starts in, and the one place in this file where that
/// state is ON. Every other block here - judgment, retention - defaults to off,
/// so this default is the one worth an assertion of its own.
#[test]
fn a_config_with_no_snapshot_table_still_takes_snapshots_daily() {
    let missing = config_dir_holding(&[]);
    let resolved = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(missing.path()).unwrap()
    });

    for (name, config) in [
        ("no verbatim.toml at all", resolved),
        (
            "a file with no [snapshot] table",
            snapshot_config("exclude = []\n"),
        ),
        ("a config built with no file", Config::default()),
    ] {
        assert!(config.snapshots_enabled(), "{name}: snapshots are off");
        assert_eq!(
            config.snapshot_interval_hours(),
            DEFAULT_SNAPSHOT_INTERVAL_HOURS,
            "{name}"
        );
        assert_eq!(config.snapshots_kept(), DEFAULT_SNAPSHOTS_KEPT, "{name}");
    }
}

/// Every key present: the file's numbers, not the defaults.
#[test]
fn a_snapshot_table_resolves_to_the_numbers_it_names() {
    let config = snapshot_config("[snapshot]\nenabled = false\ninterval_hours = 6\nkeep = 10\n");

    assert!(!config.snapshots_enabled());
    assert_eq!(config.snapshot_interval_hours(), 6);
    assert_eq!(config.snapshots_kept(), 10);
}

/// One key configured leaves the others where STOR-06 put them, which is why
/// each is an `Option` on the way in rather than a `#[serde(default)]` zero.
#[test]
fn one_configured_snapshot_key_does_not_take_the_others_with_it() {
    let config = snapshot_config("[snapshot]\nkeep = 1\n");

    assert!(config.snapshots_enabled());
    assert_eq!(
        config.snapshot_interval_hours(),
        DEFAULT_SNAPSHOT_INTERVAL_HOURS
    );
    assert_eq!(config.snapshots_kept(), 1);
}

/// Neither zero is taken literally, because both spell a disk filling up.
/// `interval_hours = 0` means a snapshot on every prompt, and `keep = 0` means
/// writing an archive-sized file and deleting it in the same step - so both
/// fall back, and turning snapshots off is the `enabled` key.
#[test]
fn a_zero_interval_or_a_zero_keep_falls_back_rather_than_writing_per_prompt() {
    let config = snapshot_config("[snapshot]\ninterval_hours = 0\nkeep = 0\n");

    assert_eq!(
        config.snapshot_interval_hours(),
        DEFAULT_SNAPSHOT_INTERVAL_HOURS
    );
    assert_eq!(config.snapshots_kept(), 1);
}

/// The documented rule holds one level down here too.
#[test]
fn an_unknown_key_inside_the_snapshot_table_is_ignored_rather_than_rejected() {
    let config = snapshot_config("[snapshot]\nkeep = 5\nsomething_phase_9_writes = true\n");

    assert_eq!(config.snapshots_kept(), 5);
    assert!(config.snapshots_enabled());
}

// ---------------------------------------------------------------------------
// The `[capture]` table (ING-07, D-05)

fn capture_config(body: &str) -> Config {
    let dir = config_dir_holding(&[(CONFIG_FILE_NAME, body)]);
    with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(dir.path()).expect("a capture table is not a parse failure")
    })
}

/// The state every user starts in. `full` is the only mode the byte-for-byte
/// invariant is a statement about, so every way of arriving with nothing
/// configured has to land on it.
#[test]
fn every_unconfigured_route_resolves_to_full_capture() {
    let missing = config_dir_holding(&[]);
    let no_file = with_var(CLAUDE_CONFIG_DIR_ENV, None, || {
        Config::load_from(missing.path()).expect("a missing config file is not an error")
    });

    for (name, config) in [
        ("no verbatim.toml at all", no_file),
        (
            "a file with no [capture] table",
            capture_config("exclude = []\n"),
        ),
        ("an empty [capture] table", capture_config("[capture]\n")),
        (
            "an empty mode string",
            capture_config("[capture]\nmode = \"\"\n"),
        ),
        ("a config built with no file", Config::default()),
        (
            "a config built from parts",
            Config::from_parts(Vec::new(), Vec::new()),
        ),
    ] {
        assert_eq!(
            config.capture_mode(),
            CaptureMode::Full,
            "{name}: the default must store every byte"
        );
        assert!(config.capture_mode().is_full(), "{name}");
    }
}

/// The two reduced modes, and the spellings the column stores.
#[test]
fn the_mode_key_selects_the_capture_mode() {
    for (written, expected) in [
        ("full", CaptureMode::Full),
        ("lean", CaptureMode::Lean),
        ("minimal", CaptureMode::Minimal),
        // Same rule the other tables follow: trimmed and case-folded, so a
        // hand-edited file is not held to an exact spelling.
        ("  LEAN  ", CaptureMode::Lean),
        ("Minimal", CaptureMode::Minimal),
    ] {
        let config = capture_config(&format!("[capture]\nmode = \"{written}\"\n"));
        assert_eq!(config.capture_mode(), expected, "mode = {written:?}");
        assert_eq!(config.capture_mode().as_str(), expected.as_str());
    }
}

/// A typo must not silently start throwing away tool output, and must not stop
/// the store from opening either. It resolves to `full`, the same rule
/// `ResponseFormat::parse` and `RetentionAction::parse` state.
#[test]
fn an_unrecognized_capture_mode_resolves_to_full_rather_than_failing_the_load() {
    for written in ["leen", "none", "off", "everything", "MINIMAL_"] {
        let config = capture_config(&format!("[capture]\nmode = \"{written}\"\n"));
        assert_eq!(
            config.capture_mode(),
            CaptureMode::Full,
            "mode = {written:?} must resolve to full, not to an elision"
        );
    }
}

/// The documented rule holds one level down here too.
#[test]
fn an_unknown_key_inside_the_capture_table_is_ignored_rather_than_rejected() {
    let config = capture_config("[capture]\nmode = \"lean\"\nsomething_phase_9_writes = 3\n");

    assert_eq!(config.capture_mode(), CaptureMode::Lean);
}

/// The spellings round-trip: what `session_meta.capture_mode` stores is what
/// `verbatim.toml` accepts, so a column value can be pasted back into the file.
#[test]
fn the_stored_spelling_is_a_configurable_one() {
    for mode in [CaptureMode::Full, CaptureMode::Lean, CaptureMode::Minimal] {
        let config = capture_config(&format!("[capture]\nmode = \"{}\"\n", mode.as_str()));
        assert_eq!(config.capture_mode(), mode);
    }
}
