//! Owner-only on disk at the process boundary (PRIV-02, PRIV-04).
//!
//! The library test in `terminus-core` proves what `Store::open` does to a
//! directory it is handed. This one proves what the shipped binary does to a
//! data directory an earlier build left readable by group and world: one
//! writable command repairs it, a second one changes nothing, `verify` still
//! passes over it, and the read commands - `doctor`, `search`, `show` - leave
//! every mode and every file exactly as they found them (D-05).
//!
//! Regular files only for the ctime comparison. SQLite creates `-wal` and
//! `-shm` on every open and removes them again when the last connection closes,
//! which moves the data directory's own ctime on every single run and has
//! nothing to do with whether a `chmod` happened.

#![cfg(unix)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

const PROJECT: &str = "-data-projects-cadence";

/// The same isolation every other binary test uses: a spawn that set only
/// `TERMINUS_DATA_DIR` would resolve the developer's real config and walk the
/// live `~/.claude` tree.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    work: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let config_dir = root.join("config");
    let claude_dir = root.join("claude");
    let work = root.join("work");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    Bench {
        _dir: dir,
        data_dir: root.join("data"),
        config_dir,
        claude_dir,
        work,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_terminus"))
            .args(args)
            .current_dir(&self.work)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn terminus")
    }

    fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap_or_else(|e| panic!("{} is not there: {e}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

fn set(path: &Path, bits: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap();
}

/// Ctime AND its nanoseconds: a chmod and the stat that follows it can land in
/// the same second, so the whole-second field alone would let a repeated chmod
/// through.
fn ctimes(paths: &[PathBuf]) -> Vec<(PathBuf, i64, i64)> {
    paths
        .iter()
        .map(|p| {
            let m = std::fs::symlink_metadata(p).unwrap();
            (p.clone(), m.ctime(), m.ctime_nsec())
        })
        .collect()
}

/// Every path whose mode is not the one asked for, named in ONE message.
///
/// A per-path `assert_eq!` would stop at the first offender, which is always
/// the data directory, and leave the reader of a failing run guessing whether
/// `terminus.db` was repaired or not - the single most useful fact in the
/// message.
fn wrong_modes<'a>(paths: impl Iterator<Item = (&'a PathBuf, u32)>) -> Vec<String> {
    paths
        .filter_map(|(path, want)| {
            let found = mode(path);
            (found != want).then(|| format!("{} is {found:o}, want {want:o}", path.display()))
        })
        .collect()
}

fn assert_unmoved(before: &[(PathBuf, i64, i64)], after_what: &str) {
    for (path, ctime, nsec) in before {
        let m = std::fs::symlink_metadata(path).unwrap();
        assert_eq!(
            (m.ctime(), m.ctime_nsec()),
            (*ctime, *nsec),
            "{} was chmod'd by {after_what}",
            path.display()
        );
    }
}

/// AC3 and AC4 in one pass over one store, because both are claims about the
/// same widened directory and re-ingesting for the second would only make the
/// two harder to compare.
#[test]
fn one_writable_command_repairs_a_wide_store_and_no_read_command_touches_one() {
    let bench = bench();

    let name = "00000001-1111-4111-8111-111111111111.jsonl";
    let dest = bench.claude_dir.join("projects").join(PROJECT).join(name);
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::copy(testkit::fixture_path("session-basic.jsonl"), &dest).unwrap();
    bench.ok(&["ingest"]);

    let data = bench.data_dir.clone();
    // The injection scratch and the decision log, hand-written: both are
    // written by the hook path rather than by any command, and what this test
    // is about is their modes, not their contents.
    for (dir, file) in [("injection", "one-session.json"), ("decisions", "one.json")] {
        let dir = data.join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), b"{}").unwrap();
    }

    // The first pass takes a snapshot (STOR-06), so `snapshots/` is there with
    // a file in it whose name only the run that wrote it knows.
    let snapshots = data.join(terminus_core::store::snapshot::DIR_NAME);
    let snapshot: PathBuf = std::fs::read_dir(&snapshots)
        .expect("the first ingest takes a snapshot")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file())
        .expect("a snapshot file");

    // A turn id to hand `show` later, taken now: reading it through a second
    // SQLite connection after the ctimes are recorded would checkpoint the WAL
    // and move `terminus.db` for reasons that are not a chmod.
    let hits = bench.ok(&["search", "--project", "*", "--json", "Bash"]);
    let turn_id = serde_json::from_slice::<serde_json::Value>(&hits.stdout).unwrap()["data"]
        ["hits"][0]["turn_id"]
        .as_i64()
        .expect("a hit to show")
        .to_string();

    let directories: Vec<PathBuf> = vec![
        data.clone(),
        snapshots.clone(),
        data.join("injection"),
        data.join("decisions"),
    ];
    let files: Vec<PathBuf> = vec![
        data.join(DB_FILE_NAME),
        data.join("LOCK"),
        snapshot.clone(),
        data.join("injection").join("one-session.json"),
        data.join("decisions").join("one.json"),
    ];

    // What a build before this phase left behind, reproduced exactly: 755 on
    // every directory, 644 on every file.
    let widen = || {
        for dir in &directories {
            set(dir, 0o755);
        }
        for file in &files {
            assert!(file.is_file(), "{} is missing", file.display());
            set(file, 0o644);
        }
    };

    widen();
    bench.ok(&["status", "--json"]);

    let wrong = wrong_modes(
        directories
            .iter()
            .map(|d| (d, 0o700))
            .chain(files.iter().map(|f| (f, 0o600))),
    );
    assert!(
        wrong.is_empty(),
        "still readable by group or world after a writable open: {wrong:?}"
    );
    // The sidecars are SQLite's and are gone once the process that made them
    // exits; when one survives - a crashed run, a hot copy - it is on the list
    // too, and it is created at the mode of a `terminus.db` this repair has
    // already narrowed.
    for suffix in ["-wal", "-shm"] {
        let sidecar = data.join(format!("{DB_FILE_NAME}{suffix}"));
        if sidecar.exists() {
            assert_eq!(mode(&sidecar), 0o600, "{}", sidecar.display());
        }
    }

    // D-06: the repair is compare-then-chmod, so the second run finds every bit
    // already right and issues no chmod at all.
    let before = ctimes(&files);
    bench.ok(&["status", "--json"]);
    assert_unmoved(&before, "a second writable open");

    // AC3's last clause: the repaired store is still a store.
    bench.ok(&["verify"]);

    // AC4: a read reports on a store, it does not repair one (D-05).
    widen();
    let wide = ctimes(&files);
    // Not `ok`: `doctor` reports a Problem and exits 1 on a bench with nothing
    // installed into Claude Code, which is the truth about this store and none
    // of this test's business. What it may not do is change a mode.
    let doctor = bench.run(&["doctor", "--json"]);
    assert!(doctor.status.code().is_some(), "doctor did not run");
    bench.ok(&["search", "--project", "*", "--json", "Bash"]);
    bench.ok(&["show", "--project", "*", "--json", &turn_id]);

    let changed = wrong_modes(
        directories
            .iter()
            .map(|d| (d, 0o755))
            .chain(files.iter().map(|f| (f, 0o644))),
    );
    assert!(changed.is_empty(), "a read command repaired: {changed:?}");
    assert_unmoved(&wide, "a read command");
}
