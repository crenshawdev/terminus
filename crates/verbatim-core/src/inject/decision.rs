//! What one `UserPromptSubmit` decided, written down (FEED-01).
//!
//! One JSON file per prompt event, under a `decisions` subdirectory of the data
//! directory, drained into the `decisions` table by a later ingest pass. It
//! carries the spellings the prompt was asked about, every candidate the index
//! scored and what it matched on, the turns injected and what they cost, the
//! turns refused and why, and the threshold values that were in force.
//!
//! **A file and not a table (D-01).** The hook never writes SQLite. An ingest
//! or a backfill was measured holding the store for 49 s over the real corpus,
//! and a write on the prompt path would put the user behind exactly the machine
//! state that provoked it - while a read at worst returns nothing. So the prompt
//! path spends one `write` plus one `rename` on a thread the binary never joins,
//! and the pass that already holds the lock does the insert.
//!
//! **Every prompt writes one, including the ones that inject nothing (D-11).**
//! A prompt that named no path and no identifier-shaped token never opens the
//! store at all, and it still leaves a record: non-fires are where the miss data
//! lives (`DESIGN-BRIEF.md:272`), and precision computed only over the prompts
//! that fired has no denominator. Measured growth is bounded - median 2 user
//! prompts per session, p90 10, max 50 over 150 sampled transcripts.
//!
//! **Not [`super::state`].** That file is one session's disposable scratch,
//! capped, overwritten every prompt and thrown away; this one is durable, one
//! file per prompt, and joined against the transcripts that follow.
//!
//! **This is the one place injection reads a clock.** A decision anchors to its
//! session by `session_id` plus wall clock and never by a turn index (D-04),
//! because the injector deliberately never reads the live transcript and so
//! cannot know one. The clock is read as unix milliseconds and formatted by
//! SQLite at drain time, which keeps one spelling of a stored timestamp in the
//! product rather than growing a second one - and a date crate - on the cold
//! path.
//!
//! **Every failure is silence.** A refused session id, a directory that cannot
//! be created, a write that fails: all of them return `false` and cost the user
//! nothing. A record that cannot land must never become a prompt that cannot be
//! answered.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::state::Suppressed;
use crate::recall::search::MatchedEntity;

/// The subdirectory of the data directory these files live in.
pub const DIR_NAME: &str = "decisions";

/// The document shape this build writes, and the only one it reads.
///
/// A file carrying anything else reads as nothing and is deleted by the drain.
/// There is no migration: a decision is evidence about a build's retrieval
/// behaviour, and half-reading one written by a different build would be
/// evidence about neither.
const FORMAT: u32 = 1;

/// The shortest and longest a `session_id` may be and still name a file.
///
/// Mirrored from [`super::state`] rather than shared with it, deliberately: both
/// turn an untrusted payload field into a file name, and the allow-list is the
/// whole of that defence, so it is stated where it is applied.
const MIN_NAME: usize = 8;
const MAX_NAME: usize = 64;

/// Enough to step over a name a same-millisecond sibling already took. Past
/// that, something is wrong with the directory rather than with the name.
const ATTEMPTS: u32 = 64;

/// One turn the index offered, with everything the threshold judged it on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub turn_id: i64,
    pub relevance: f64,
    pub entity_score: f64,
    pub entity_count: usize,
    /// The distinct `(kind, value_norm)` pairs this candidate matched (D-05).
    /// Without them a replayed label change cannot be attributed to a path rule
    /// rather than a symbol rule, which is the question replay exists to answer.
    #[serde(default)]
    pub matched_on: Vec<MatchedEntity>,
}

/// One turn that was actually injected, and what it spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Injected {
    pub turn_id: i64,
    /// Characters, never tokens (D-12): the same proxy the budget is spent in.
    pub chars: usize,
}

/// The threshold values in force when this decision was taken (D-08).
///
/// Logged per decision because they are compile-time constants: they are not
/// configurable by design, so the only way a later analysis can know which
/// numbers produced a label is if the decision carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Thresholds {
    pub ranked: usize,
    pub compacted_ranked: usize,
    pub entity_rank: usize,
    pub co_occurring: usize,
    pub max_turns: usize,
    pub max_candidates: usize,
    /// The character budget actually applied, which is the configured one
    /// clamped to the hard ceiling.
    pub prompt_chars: usize,
}

/// One prompt's decision.
///
/// `format` is private and has no `serde` default, exactly as [`super::state`]
/// does it: a document that does not spell it is not one of ours, and the
/// missing field is what makes the read discard it rather than read half of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    format: u32,
    /// The Claude Code session, which is also what names the file.
    pub session_id: Option<String>,
    /// Wall clock, unix milliseconds, read once when the prompt arrived. Half of
    /// D-04's anchor; the drain formats it into the stored timestamp shape.
    pub at_ms: i64,
    pub cwd: Option<String>,
    /// The prompt as submitted. Stored whole: the archive already holds every
    /// prompt verbatim, so a truncated copy here would be a second, lossier
    /// record of something the store has losslessly.
    pub prompt: String,
    /// Max `sessions.session_no` when the decision was taken (D-10), or `None`
    /// when the prompt never opened the store - the drain stamps it then.
    pub watermark_session_no: Option<i64>,
    /// The spellings the prompt was asked about, strongest first. Empty is the
    /// common case and the one D-11 exists for.
    #[serde(default)]
    pub spellings: Vec<String>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub injected: Vec<Injected>,
    /// What the rendered injection cost in characters, which is the clipped
    /// total and not the sum of the turns' shares.
    #[serde(default)]
    pub chars_injected: usize,
    /// The refusals of THIS prompt, with their reasons - never the session
    /// state file's cumulative, capped list.
    #[serde(default)]
    pub suppressed: Vec<Suppressed>,
    /// Whether the compacted pool was in force (INJ-05), and what it admitted.
    #[serde(default)]
    pub compacted: bool,
    #[serde(default)]
    pub dropped: Vec<i64>,
    #[serde(default)]
    pub thresholds: Thresholds,
}

impl Decision {
    /// A record of a prompt that has just arrived, stamped with the clock.
    pub fn opened(session_id: Option<&str>, cwd: Option<&str>, prompt: &str) -> Decision {
        Decision {
            format: FORMAT,
            session_id: session_id.map(str::to_owned),
            at_ms: now_ms(),
            cwd: cwd.map(str::to_owned),
            prompt: prompt.to_owned(),
            watermark_session_no: None,
            spellings: Vec::new(),
            candidates: Vec::new(),
            injected: Vec::new(),
            chars_injected: 0,
            suppressed: Vec::new(),
            compacted: false,
            dropped: Vec::new(),
            thresholds: Thresholds::default(),
        }
    }

    /// Write this record, reporting whether it landed.
    ///
    /// `false` covers a refused `session_id`, a data directory that cannot be
    /// created and a write that failed. None of them is worth failing a prompt
    /// over, and all of them cost one missing row in a log nothing decides on
    /// yet.
    pub fn save(&self, data_dir: &Path) -> bool {
        write_file(&data_dir.join(DIR_NAME), self).is_some()
    }
}

/// One file the drain found: its path, and the record it holds if it is one.
///
/// The path travels even when the parse failed, because the caller's job is to
/// empty this directory: a file nobody can read is a file to delete, and a
/// reader that returned only the good records would leave it there forever.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub path: PathBuf,
    pub decision: Option<Decision>,
}

/// Every decision file under `data_dir`, in a stable order.
///
/// An empty vector for a directory that is not there, which is every machine
/// before its first prompt. Only files are yielded: a subdirectory somebody
/// dropped in here is left alone rather than deleted by the drain.
pub fn read_all(data_dir: &Path) -> Vec<Found> {
    let dir = data_dir.join(DIR_NAME);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut found: Vec<Found> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        // A `.tmp` left by a write that was cut off at the hook's deadline is
        // not a record and never will be: skipping it here and deleting it in
        // the drain are two different answers, and this is the one that cannot
        // race a write still in flight.
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let decision = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Decision>(&bytes).ok())
            .filter(|decision| decision.format == FORMAT);
        found.push(Found { path, decision });
    }
    // Ingest order rather than directory order: the file names carry the clock,
    // so this is the order the prompts happened in, and `read_dir` guarantees no
    // order at all.
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

/// Wall clock as unix milliseconds, or 0 for a clock set before the epoch.
///
/// Zero rather than an error: a machine whose clock is that wrong produces a
/// decision whose anchor is useless, and a prompt that fails is worse than a row
/// that cannot be labelled.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

/// Write `decision` into `dir` through a temporary in the same directory.
///
/// The same tmp+rename shape [`super::state`] uses and for the same reason: this
/// runs on a thread the binary never joins, so process exit takes it wherever it
/// got to, and a rename within a directory is atomic - the drain sees a whole
/// document or no file at all, never a prefix.
///
/// No `fsync`. The rename buys the atomicity a reader cares about; durability
/// across a power cut would cost a disk flush on the path where the user is
/// waiting, to save one row of a log that is already only a sample of behaviour.
fn write_file(dir: &Path, decision: &Decision) -> Option<PathBuf> {
    // Before any filesystem call, exactly as `state::path` does it: the session
    // id arrives on an untrusted payload and becomes part of a file name, so a
    // value carrying a separator, a `..` or a NUL must reach no `create_dir_all`
    // and no `join` at all.
    let stem = file_stem(decision.session_id.as_deref()?)?;
    std::fs::create_dir_all(dir).ok()?;
    let bytes = serde_json::to_vec(decision).ok()?;

    let (temporary, mut file) = create_temporary(dir)?;
    let written = file.write_all(&bytes).and_then(|()| file.flush());
    // Closed before the rename: a Windows rename over an open handle fails, and
    // this runs on every prompt.
    drop(file);
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return None;
    }

    // One session submits many prompts, so the name has to be unique per EVENT
    // and not per session. The target is reserved with `create_new` rather than
    // probed with `exists`, so two prompts of one session landing in the same
    // millisecond cannot both pick the same name and lose one record.
    let stamp = decision.at_ms;
    let pid = std::process::id();
    for attempt in 0..ATTEMPTS {
        let target = dir.join(format!("{stem}-{pid}-{stamp}-{attempt}.json"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(reserved) => {
                drop(reserved);
                if std::fs::rename(&temporary, &target).is_ok() {
                    return Some(target);
                }
                // The reservation is ours and holds nothing; leaving it would
                // give the drain an empty file to report as malformed.
                let _ = std::fs::remove_file(&target);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => break,
        }
    }
    let _ = std::fs::remove_file(&temporary);
    None
}

/// The part of a file name a `session_id` may contribute, when it is one.
///
/// ASCII hex digits and dashes at a plausible uuid length, and nothing else: an
/// allow-list rather than a scan for dangerous spellings, because the list of
/// dangerous spellings is per-platform and open-ended (`..`, `/`, `\`, `:`, a
/// NUL, a trailing dot, `CON`) while the list of characters a session id is made
/// of is closed.
fn file_stem(session_id: &str) -> Option<String> {
    let plausible = (MIN_NAME..=MAX_NAME).contains(&session_id.len());
    let shaped = session_id
        .bytes()
        .all(|b| b.is_ascii_hexdigit() || b == b'-');
    (plausible && shaped).then(|| session_id.to_owned())
}

/// A new file in `dir`, created exclusively.
///
/// `create_new` and not `File::create`: `O_EXCL` fails on anything already at
/// the name rather than following it and truncating it, so a stale symlink left
/// by a killed run cannot turn this write into a truncation of whatever it
/// points at.
///
/// The name can never collide with a record's own: it starts with a dot and ends
/// in `.tmp`, and [`file_stem`] admits neither character.
fn create_temporary(dir: &Path) -> Option<(PathBuf, std::fs::File)> {
    let pid = std::process::id();
    for attempt in 0..ATTEMPTS {
        let path = dir.join(format!(".decision-{pid}-{attempt}.tmp"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Some((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::inject::state::Reason;

    const SESSION: &str = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";

    /// A record with every field populated, so a round trip that lost one is
    /// visible.
    fn full() -> Decision {
        let mut decision = Decision::opened(Some(SESSION), Some("/code/verbatim"), "where is x.rs");
        decision.watermark_session_no = Some(11);
        decision.spellings = vec!["/code/verbatim/x.rs".into(), "x.rs".into()];
        decision.candidates = vec![Candidate {
            turn_id: 42,
            relevance: 1.75,
            entity_score: 0.5,
            entity_count: 1,
            matched_on: vec![MatchedEntity {
                kind: "path".into(),
                value: "/code/verbatim/x.rs".into(),
            }],
        }];
        decision.injected = vec![Injected {
            turn_id: 42,
            chars: 120,
        }];
        decision.chars_injected = 137;
        decision.suppressed = vec![Suppressed {
            turn_id: 43,
            reason: Reason::AlreadyInjected,
        }];
        decision.compacted = true;
        decision.dropped = vec![41, 42];
        decision.thresholds = Thresholds {
            ranked: 10,
            compacted_ranked: 50,
            entity_rank: 3,
            co_occurring: 2,
            max_turns: 3,
            max_candidates: 8,
            prompt_chars: 2000,
        };
        decision
    }

    #[test]
    fn a_record_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let decision = full();
        assert!(decision.save(dir.path()), "the record did not land");

        let found = read_all(dir.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].decision.as_ref(), Some(&decision));
        assert!(found[0].path.starts_with(dir.path().join(DIR_NAME)));
    }

    /// D-11's shape: a prompt that never opened the store still writes a record,
    /// and it is a record of nothing having happened.
    #[test]
    fn a_non_fire_is_a_record_like_any_other() {
        let dir = tempfile::tempdir().unwrap();
        let decision =
            Decision::opened(Some(SESSION), Some("/code/verbatim"), "what were we doing");
        assert!(decision.save(dir.path()));

        let found = read_all(dir.path());
        let read = found[0].decision.as_ref().unwrap();
        assert!(read.spellings.is_empty());
        assert!(read.candidates.is_empty());
        assert!(read.injected.is_empty());
        assert_eq!(read.chars_injected, 0);
        assert_eq!(read.watermark_session_no, None);
    }

    /// The session id is untrusted payload text, so the refusal happens before
    /// any path is built - asserted on the filesystem, because the failure this
    /// prevents is a file somewhere else, not a message.
    #[test]
    fn a_hostile_session_id_reaches_no_filesystem_call() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");

        for hostile in [
            "../evil",
            "../../../../etc/passwd",
            "a/b/c/d/e/f/g",
            "a\\b\\c\\d\\e\\f",
            "0e5e6a1e\0truncated-here-0000000000",
            "0e5e6a1e:9f2b:4c7a:8d31:6b4f2a9c1d55",
            "short",
            &"f".repeat(MAX_NAME + 1),
        ] {
            let decision = Decision::opened(Some(hostile), Some("/code/verbatim"), "hello");
            assert!(
                !decision.save(&data_dir),
                "{hostile:?} was accepted as a file name"
            );
        }
        // A record with no session id at all is the same refusal.
        assert!(!Decision::opened(None, None, "hello").save(&data_dir));

        assert!(
            !data_dir.exists(),
            "a refused session id still created the data directory"
        );
    }

    /// One session submits many prompts, and each is its own record: a name
    /// keyed on the session alone would keep only the last one.
    #[test]
    fn two_prompts_of_one_session_land_as_two_files() {
        let dir = tempfile::tempdir().unwrap();
        for prompt in ["first", "second"] {
            assert!(Decision::opened(Some(SESSION), Some("/code"), prompt).save(dir.path()));
        }

        let found = read_all(dir.path());
        assert_eq!(found.len(), 2, "{found:?}");
        let prompts: Vec<String> = found
            .iter()
            .map(|f| f.decision.as_ref().unwrap().prompt.clone())
            .collect();
        assert!(prompts.contains(&"first".to_owned()));
        assert!(prompts.contains(&"second".to_owned()));
    }

    /// Bytes this build does not recognize read as nothing - and are still
    /// reported, because the drain's job is to empty the directory.
    #[test]
    fn a_document_this_build_did_not_write_reads_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let decisions = dir.path().join(DIR_NAME);
        std::fs::create_dir_all(&decisions).unwrap();

        // Not JSON; JSON that is not an object of ours; and a document of ours
        // from a format this build does not write.
        std::fs::write(decisions.join("a.json"), b"{not json at all").unwrap();
        std::fs::write(decisions.join("b.json"), br#"{"hello": "world"}"#).unwrap();
        let mut aged = serde_json::to_value(full()).unwrap();
        aged["format"] = serde_json::json!(FORMAT + 1);
        std::fs::write(decisions.join("c.json"), aged.to_string()).unwrap();

        let found = read_all(dir.path());
        assert_eq!(found.len(), 3, "{found:?}");
        for entry in &found {
            assert_eq!(entry.decision, None, "{entry:?} parsed as a record");
        }
    }

    /// A `.tmp` from a write cut off at the hook's deadline is not a record and
    /// is not offered as one.
    #[test]
    fn a_half_written_temporary_is_not_a_record() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Decision::opened(Some(SESSION), Some("/code"), "hello").save(dir.path()));
        std::fs::write(dir.path().join(DIR_NAME).join(".decision-1-0.tmp"), b"{").unwrap();

        let found = read_all(dir.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].decision.is_some());
    }
}
