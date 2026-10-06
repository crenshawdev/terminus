//! PRIV-02 over terminus's own config file, at the process boundary (AC5).
//!
//! `terminus.toml` carrying `provider.api_key` is a credentials file whatever
//! else is in it, so at `0644` the key is refused where it is consumed - and
//! the refusal has to name that file and its mode without putting a byte of the
//! key on a stream. The three ways the same wide file must still WORK are here
//! too: with the key removed, with the key supplied through the environment,
//! and at `0600`.
//!
//! Spawns rather than library calls, because the assertion is about what
//! reaches stdout, stderr and `runs.error` - three destinations the library
//! tests cannot see. The harness is a deliberate copy of
//! `tests/observations.rs`'s, for the reason that file gives about its own.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use rusqlite::Connection;
use terminus_core::config::CONFIG_FILE_NAME;
use terminus_core::store::DB_FILE_NAME;

/// A value nothing else on this machine can produce by accident, so any part of
/// it in an output is a leak rather than a coincidence.
const KEY: &str = "sk-TERMINUSCONFIGMODE-9c31f7-do-not-log";

/// The fragments no stream may carry. A formatter that printed the first
/// characters of the key would pass an equality check and still be a leak.
const FRAGMENTS: [&str; 3] = [KEY, "TERMINUSCONFIGMODE", "9c31f7"];

/// The provider namespace, and the variable tier 1 reads it from.
const PROVIDER: &str = "openrouter";
const PROVIDER_ENV: &str = "OPENROUTER_API_KEY";

/// Every directory a spawned `terminus` may touch, all of them temporary.
///
/// The shared credentials directory is pinned and left empty for the reason
/// `tests/doctor.rs` states: the loader falls back to `XDG_CONFIG_HOME`, so
/// without this a run would reach the developer's real
/// `~/.config/jcrenshaw/credentials.toml`.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    shared_dir: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    let shared_dir = dir.path().join("shared");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::create_dir_all(&shared_dir).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        config_dir,
        claude_dir,
        shared_dir,
    }
}

impl Bench {
    fn config_path(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE_NAME)
    }

    /// Run the real binary with every directory it reads pinned, and with the
    /// provider variable cleared: a developer with one exported would otherwise
    /// see tier 1 answer and the refusal never fire.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("spawn terminus")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_terminus"));
        command
            .args(args)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("JCRENSHAW_CONFIG_DIR", &self.shared_dir)
            .env_remove(PROVIDER_ENV);
        command
    }

    /// Write `terminus.toml` at `mode`, with judgment on and the endpoint
    /// pointed at a port nothing listens on: this file must never produce a
    /// request, and if it ever does the connection fails rather than reaching
    /// somewhere real.
    fn write_config(&self, with_key: bool, mode: u32) {
        let mut body = format!(
            "[provider]\nenabled = true\nname = \"{PROVIDER}\"\n\
             base_url = \"http://127.0.0.1:1/\"\nmodel = \"qwen3:8b\"\n"
        );
        if with_key {
            body.push_str(&format!("api_key = \"{KEY}\"\n"));
        }
        std::fs::write(self.config_path(), body).unwrap();
        std::fs::set_permissions(self.config_path(), std::fs::Permissions::from_mode(mode))
            .unwrap();
    }

    /// Every `runs.error` the store holds, joined. The judgment step's notes go
    /// to stderr rather than here (D-07), and this asserts that a refusal did
    /// not find some other way in.
    fn run_errors(&self) -> String {
        let conn = Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap();
        let mut statement = conn
            .prepare("SELECT coalesce(error, '') FROM runs")
            .unwrap();
        let rows: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.join("\n")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The whole point: no destination carries any part of the key.
fn assert_no_key_reaches(bench: &Bench, out: &Output, what: &str) {
    let errors = bench.run_errors();
    for (channel, rendered) in [
        ("stdout", stdout(out)),
        ("stderr", stderr(out)),
        ("runs.error", errors),
    ] {
        for fragment in FRAGMENTS {
            assert!(
                !rendered.contains(fragment),
                "{what}: {channel} carries {fragment:?}:\n{rendered}"
            );
        }
    }
    // The falsifying half: the file really does hold the key, so the
    // assertions above are about the binary and not about an empty config.
    assert!(std::fs::read_to_string(bench.config_path())
        .unwrap()
        .contains(KEY));
}

/// AC5: the refusal names the file and the mode, leaks nothing, and does not
/// fail the ingest - the judgment step returns notes (OBS-04).
#[test]
fn a_group_readable_config_carrying_a_key_is_refused_by_name_and_leaks_nothing() {
    let bench = bench();
    bench.write_config(true, 0o644);

    let out = bench.run(&["ingest"]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let refusal = stderr(&out);
    assert!(
        refusal.contains(&bench.config_path().display().to_string()),
        "the refusal does not name the file: {refusal}"
    );
    assert!(
        refusal.contains("mode 644"),
        "the refusal does not name the mode: {refusal}"
    );
    assert!(
        refusal.contains("readable beyond its owner"),
        "the refusal does not say what is wrong: {refusal}"
    );
    // Not the shared file's wording: a user with a wide `terminus.toml` must
    // not go looking for a credentials file they may not have (D-14).
    assert!(
        !refusal.contains("credentials file"),
        "the refusal points at the shared file: {refusal}"
    );
    assert_no_key_reaches(&bench, &out, "a refused 0644 config");
}

/// The falsifying half, and the state a correct machine is in: the same file at
/// `0600` is accepted, so nothing is refused and nothing is said.
#[test]
fn the_same_file_at_owner_only_is_accepted_without_a_word() {
    let bench = bench();
    bench.write_config(true, 0o600);

    let out = bench.run(&["ingest"]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let said = stderr(&out);
    assert!(
        !said.contains("readable beyond its owner"),
        "a 0600 config was refused: {said}"
    );
    assert!(
        !said.contains(&bench.config_path().display().to_string()),
        "a 0600 config was named in a complaint: {said}"
    );
    assert_no_key_reaches(&bench, &out, "an accepted 0600 config");
}

/// D-07's two arms, at the process boundary: a wide file with no key in it, and
/// a wide file whose key the environment outranks. Neither reaches the tier the
/// refusal lives in, so both run.
#[test]
fn a_wide_config_still_loads_with_no_key_or_with_the_key_in_the_environment() {
    let bench = bench();

    bench.write_config(false, 0o644);
    let status = bench.run(&["status"]);
    assert_eq!(
        status.status.code(),
        Some(0),
        "a 0644 config with no key stopped a read: {}",
        stderr(&status)
    );
    assert!(
        !stderr(&status).contains("readable beyond its owner"),
        "a keyless 0644 config was refused: {}",
        stderr(&status)
    );

    bench.write_config(true, 0o644);
    let out = bench
        .command(&["ingest"])
        .env(PROVIDER_ENV, "sk-from-the-environment-not-the-file")
        .output()
        .expect("spawn terminus");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        !stderr(&out).contains("readable beyond its owner"),
        "the file's mode was held against a key that came from the environment: {}",
        stderr(&out)
    );
    assert_no_key_reaches(&bench, &out, "a 0644 config the environment outranks");
}
