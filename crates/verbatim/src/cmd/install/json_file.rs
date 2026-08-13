//! One Claude Code settings file: read it, change exactly one thing in it, and
//! write it back looking like the file it was.
//!
//! Both of install's targets go through here, and so will `uninstall` and
//! `doctor`. AC4's "every other key byte-identical" and AC7's "restores it to
//! its pre-install bytes" are properties of this module and of nothing else.
//!
//! # Why not `serde_json::Value`
//!
//! `serde_json`'s object is a `BTreeMap`. A round trip through it re-sorts the
//! 29 top-level keys of `settings.json` and every one of the 86 `projects`
//! entries inside 240 KB of `.claude.json` - a diff of the whole file, offered
//! for confirmation, for a change of two lines. `serde_json`'s `preserve_order`
//! feature would fix that and is deliberately not enabled: it is a
//! workspace-wide feature that would silently reorder the keys of every `--json`
//! document the six commands in `docs/json-shapes.md` promise.
//!
//! So this module carries its own reader and writer. They are small because
//! they are faithful rather than clever: **every scalar is kept as the source
//! text it arrived as**. A number is the digits that were written, so
//! `1786647853145` does not come back as `1786647853145.0` and `1.0` does not
//! come back as `1`; a string is its literal with its escapes intact, so a
//! `é` stays a `é` and a path full of backslashes is re-emitted
//! exactly. Decoding happens only where a value is compared or read, and a
//! decode that fails costs a comparison rather than the file.
//!
//! Objects hold their members in a `Vec` in the order they were parsed. Two
//! space indentation, `": "` between key and value, `{}` and `[]` for the empty
//! ones: that is `JSON.stringify(value, null, 2)`, which is what wrote both
//! files.
//!
//! # Writing
//!
//! Read again immediately before writing (D-06). Claude Code rewrote
//! `.claude.json` five times inside six minutes on 2026-08-13, so the copy
//! install read a moment ago to render a diff is not the copy it may write
//! over. The edit is re-applied to whatever is on disk at that instant, then a
//! temporary file in the same directory is renamed into place.
//!
//! Symlinks are resolved first. `~/.claude.json` on this machine is a symlink
//! to `/claude/.claude.json` to `/data/claude/.claude.json`; renaming over the
//! link would replace the user's symlink with a regular file and orphan the real
//! one. The temporary file goes beside the *resolved* target, because a rename
//! across filesystems is not atomic and is not even always possible.
//!
//! A file that does not exist is created. A file that exists and is not valid
//! JSON is an operational failure that writes nothing - never a file to
//! overwrite, because whatever it is, it is not ours to lose.
//!
//! # What the user approves is what lands (INST-03)
//!
//! [`Document::apply`] writes the whole document back, re-rendered. For the two
//! real files that is a byte-for-byte round trip, because both are what
//! `JSON.stringify(value, null, 2)` produced. For a settings file that is
//! minified, four-space indented, or CRLF it is not: the write reformats
//! everything, not only the members install added.
//!
//! Of the two ways to close that gap - preserve the original bytes outside the
//! edited region, or show the rewrite honestly - this module does the second.
//! The diff is taken against [`Document::source`], the bytes on disk, so a file
//! whose formatting is not this renderer's shows its whole rewrite at the
//! confirmation and the user approves exactly what lands.
//!
//! What that does not cover: install still cannot make a small edit to a file
//! it did not format. The user's choice is to accept a re-rendered file or to
//! decline the install; there is no third answer, and a byte-preserving splice
//! is the change that would offer one. It also says nothing about a file that
//! changes between the diff and the write - `apply` re-reads and re-applies at
//! that instant (D-06), and its own no-op check is rendered against rendered, so
//! an install with nothing to add writes nothing and reformats nothing.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::cmd::Failure;

/// How deep a nested value may go before the reader gives up.
///
/// The reader is recursive, so this is what keeps a hostile file from
/// exhausting the stack. Real settings files nest four or five deep.
const MAX_DEPTH: usize = 128;

/// The suffix of the whole-file backup install writes.
///
/// Not `.backup`: Claude Code writes `.claude.json.backup` itself, and rotating
/// its own backups over install's would leave nothing to restore.
const BACKUP_SUFFIX: &str = ".verbatim-backup";

/// A JSON value that remembers what it looked like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    /// A string literal, a number, `true`, `false` or `null`, as source text.
    Scalar(String),
    Array(Vec<Json>),
    /// Members in the order they were read, which is the order they are written.
    Object(Vec<Member>),
}

/// One `"key": value` pair, holding the key twice: decoded for comparison, raw
/// for output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    key: String,
    raw_key: String,
    value: Json,
}

impl Json {
    pub fn object() -> Json {
        Json::Object(Vec::new())
    }

    pub fn array() -> Json {
        Json::Array(Vec::new())
    }

    /// A JSON string carrying `value`.
    pub fn string(value: &str) -> Json {
        Json::Scalar(encode(value))
    }

    /// A JSON number.
    pub fn number(value: i64) -> Json {
        Json::Scalar(value.to_string())
    }

    /// This value as text, when it is a string and its escapes decode.
    pub fn as_str(&self) -> Option<String> {
        match self {
            Json::Scalar(raw) if raw.starts_with('"') => decode(raw),
            _ => None,
        }
    }

    /// This value as an integer, when it is one.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Scalar(raw) => raw.parse().ok(),
            _ => None,
        }
    }

    /// The value at `key`, when this is an object that has one.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members
                .iter()
                .find(|member| member.key == key)
                .map(|member| &member.value),
            _ => None,
        }
    }

    /// This object's keys, in the order they will be written.
    ///
    /// Test support: key order is the property AC4 turns on, and asserting it
    /// through a rendering would be asserting the renderer instead. Nothing in
    /// the command needs it, so nothing outside a test compiles it.
    #[cfg(test)]
    pub fn keys(&self) -> Vec<&str> {
        match self {
            Json::Object(members) => members.iter().map(|member| member.key.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    /// The elements, when this is an array. An empty slice otherwise, so a
    /// caller walking a malformed file reads nothing rather than branching.
    pub fn items(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            _ => &[],
        }
    }

    /// The value at `key`, appended as `default` when the key is absent.
    ///
    /// Appended, never inserted in sorted position: a new key belongs at the
    /// end, which is where every editor and every `JSON.stringify` would put it,
    /// and where a diff shows it as one added block.
    pub fn entry(&mut self, key: &str, default: Json) -> Result<&mut Json, Failure> {
        let Json::Object(members) = self else {
            return Err(Failure::Operational(format!(
                "expected a JSON object to hold '{key}', found {}",
                self.kind()
            )));
        };
        let at = match members.iter().position(|member| member.key == key) {
            Some(at) => at,
            None => {
                members.push(Member {
                    key: key.to_owned(),
                    raw_key: encode(key),
                    value: default,
                });
                members.len() - 1
            }
        };
        Ok(&mut members[at].value)
    }

    /// Append to an array, or say what was there instead.
    pub fn push(&mut self, value: Json) -> Result<(), Failure> {
        match self {
            Json::Array(items) => {
                items.push(value);
                Ok(())
            }
            other => Err(Failure::Operational(format!(
                "expected a JSON array, found {}",
                other.kind()
            ))),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Json::Scalar(raw) if raw.starts_with('"') => "a string",
            Json::Scalar(_) => "a number or literal",
            Json::Array(_) => "an array",
            Json::Object(_) => "an object",
        }
    }

    fn write_to(&self, out: &mut String, depth: usize) {
        match self {
            Json::Scalar(raw) => out.push_str(raw),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (at, item) in items.iter().enumerate() {
                    if at > 0 {
                        out.push_str(",\n");
                    }
                    indent(out, depth + 1);
                    item.write_to(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            }
            Json::Object(members) => {
                if members.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (at, member) in members.iter().enumerate() {
                    if at > 0 {
                        out.push_str(",\n");
                    }
                    indent(out, depth + 1);
                    out.push_str(&member.raw_key);
                    out.push_str(": ");
                    member.value.write_to(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// One settings file on disk, and what install read out of it.
#[derive(Debug)]
pub struct Document {
    /// As install names it, symlinks and all - what the summary prints.
    path: PathBuf,
    /// What is actually written to. May be several links away from `path`.
    resolved: PathBuf,
    existed: bool,
    /// The bytes read, kept so the diff can be taken against the file itself
    /// rather than against this module's idea of how it should look.
    source: String,
    value: Json,
    trailing_newline: bool,
}

impl Document {
    /// Read `path`, or a fresh empty object when nothing is there.
    pub fn read(path: &Path) -> Result<Document, Failure> {
        let resolved = resolve(path);
        let (existed, text) = match std::fs::read_to_string(&resolved) {
            Ok(text) => (true, text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
            Err(e) => {
                return Err(Failure::Operational(format!(
                    "{} could not be read: {e}",
                    resolved.display()
                )))
            }
        };
        let (value, trailing_newline) = interpret(&text, &resolved)?;
        Ok(Document {
            path: path.to_owned(),
            resolved,
            existed,
            source: text,
            value,
            trailing_newline,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn value(&self) -> &Json {
        &self.value
    }

    /// The file's bytes as they are on disk, empty when there is no file.
    ///
    /// The left-hand side of every diff install shows. It is the file, not the
    /// re-rendered copy, because the re-rendered copy is what install is asking
    /// permission to write.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// This file as this module would write it, unchanged.
    pub fn rendered(&self) -> String {
        render(&self.value, self.trailing_newline)
    }

    /// Would writing `after` change this file at all?
    ///
    /// The same test [`Document::apply`] makes at the moment it writes, so a
    /// caller can decide whether to back the file up without applying twice.
    /// Note what it is *not*: a file whose formatting differs from this
    /// renderer's is not a reason to write. Install re-renders only a file it
    /// also has something to add to.
    pub fn would_write(&self, after: &str) -> bool {
        !self.existed || after != self.rendered()
    }

    /// Would writing re-render bytes install was not asked to change?
    ///
    /// True for a file this renderer did not produce - minified, four-space
    /// indented, CRLF. The diff shows that rewrite in full; this is what lets a
    /// caller say why it is there.
    pub fn reformats(&self) -> bool {
        self.existed && self.source != self.rendered()
    }

    /// What `edit` would make of this file, without writing anything.
    pub fn preview(
        &self,
        edit: &dyn Fn(&mut Json) -> Result<(), Failure>,
    ) -> Result<String, Failure> {
        let mut next = self.value.clone();
        edit(&mut next)?;
        Ok(render(&next, self.trailing_newline))
    }

    /// Copy the whole file beside itself, once and once only.
    ///
    /// An existing backup is returned untouched rather than refreshed: a second
    /// install must not back up the file it wrote itself, or the backup stops
    /// being the pre-install bytes uninstall restores (AC7). `None` means there
    /// was no file to copy, which is an ordinary first install.
    ///
    /// That rule makes the backup's integrity load-bearing, because there is no
    /// second chance at it. So the copy is written to a temporary file and only
    /// then given the backup's name: a crash, a full disk or a kill leaves
    /// either no backup or a whole one, never the truncated one a direct copy
    /// would leave permanently in place. The name is taken with [`link_once`],
    /// which cannot clobber a backup an earlier install made and cannot be
    /// raced by a second install the way `exists()` then copy could.
    pub fn backup(&self) -> Result<Option<PathBuf>, Failure> {
        if !self.existed {
            return Ok(None);
        }
        let name = match self.resolved.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => {
                return Err(Failure::Operational(format!(
                    "{} does not name a file",
                    self.resolved.display()
                )))
            }
        };
        let path = self
            .resolved
            .with_file_name(format!("{name}{BACKUP_SUFFIX}"));
        let dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));

        let (temporary, mut file) =
            super::create_temporary(dir, &format!(".{name}{BACKUP_SUFFIX}."))?;
        let outcome = copy_through(&self.resolved, &mut file, &temporary)
            .and_then(|()| link_once(&temporary, &path));
        // Either it has the backup's name now or it never will; either way this
        // one is not a file to leave beside the user's settings.
        let _ = std::fs::remove_file(&temporary);
        outcome.map(|()| Some(path))
    }

    /// Re-read, re-apply `edit`, and write only if that changed something.
    ///
    /// Returns whether anything was written. The read is deliberately not the
    /// one [`Document::read`] did: D-06's whole point is that the file may have
    /// moved under us between the diff and the answer.
    ///
    /// What lands is the whole document, re-rendered - not a splice into the
    /// original bytes. That is why the diff is taken against
    /// [`Document::source`]: a file this renderer did not format is rewritten in
    /// full, and the user has to have seen that before answering. See the module
    /// header for what that choice does not cover.
    pub fn apply(&self, edit: &dyn Fn(&mut Json) -> Result<(), Failure>) -> Result<bool, Failure> {
        let fresh = Document::read(&self.resolved)?;
        let before = fresh.rendered();
        let after = fresh.preview(edit)?;
        if fresh.existed && after == before {
            return Ok(false);
        }
        write_atomically(&fresh.resolved, &after)?;
        Ok(true)
    }
}

fn interpret(text: &str, path: &Path) -> Result<(Json, bool), Failure> {
    if text.trim().is_empty() {
        // A file Claude Code has not written yet, or one truncated to nothing.
        // An empty object is what an absent file means, and it is the only
        // reading that lets install create the file it needs.
        return Ok((Json::object(), true));
    }
    let value = parse(text).map_err(|detail| {
        Failure::Operational(format!(
            "{} is not valid JSON ({detail}); verbatim will not overwrite a settings file it \
             cannot read",
            path.display()
        ))
    })?;
    if !matches!(value, Json::Object(_)) {
        return Err(Failure::Operational(format!(
            "{} does not hold a JSON object; verbatim will not overwrite it",
            path.display()
        )));
    }
    if let Some(at) = duplicate_key(&value, "") {
        return Err(Failure::Operational(format!(
            "{} has more than one '{at}' key, and a repeated key has no one answer: \
             `JSON.parse` - which is how Claude Code reads this file - takes the last one, \
             and this reader takes the first. an entry written into the wrong copy would be \
             the copy Claude Code ignores, and install would report success with no hook \
             ever firing.\n  \
             verbatim will not rewrite a file whose keys are ambiguous. remove the duplicate \
             and run install again.",
            path.display()
        )));
    }
    Ok((value, text.ends_with('\n')))
}

/// Where the first repeated key is, if there is one, as a path like
/// `.hooks.SessionStart` or `.mcpServers`.
///
/// JSON permits duplicate keys and every consumer resolves them differently.
/// This module's own lookups take the first; Claude Code's `JSON.parse` takes
/// the last. Rather than pick a rule, install refuses the file - it is about to
/// rewrite the whole of it, and there is no reading of a duplicate under which
/// that is safe.
fn duplicate_key(value: &Json, at: &str) -> Option<String> {
    match value {
        Json::Object(members) => {
            let mut seen = std::collections::HashSet::with_capacity(members.len());
            for member in members {
                if !seen.insert(member.key.as_str()) {
                    return Some(format!("{at}.{}", member.key));
                }
            }
            members
                .iter()
                .find_map(|member| duplicate_key(&member.value, &format!("{at}.{}", member.key)))
        }
        Json::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| duplicate_key(item, &format!("{at}[{index}]"))),
        Json::Scalar(_) => None,
    }
}

fn render(value: &Json, trailing_newline: bool) -> String {
    let mut out = String::new();
    value.write_to(&mut out, 0);
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// The real file behind `path`, following every symlink on the way.
///
/// Falls back to resolving the parent and re-attaching the name, because
/// `canonicalize` fails on a file that does not exist yet and install has to be
/// able to create one.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => match std::fs::canonicalize(parent) {
            Ok(real) => real.join(name),
            Err(_) => path.to_owned(),
        },
        _ => path.to_owned(),
    }
}

fn write_atomically(resolved: &Path, contents: &str) -> Result<(), Failure> {
    let dir = resolved
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| {
        Failure::Operational(format!("{} could not be created: {e}", dir.display()))
    })?;

    let name = resolved
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings".to_owned());
    // Exclusively, and not with `File::create`: see `super::create_temporary`.
    let (temporary, file) = super::create_temporary(dir, &format!(".{name}.verbatim-"))?;

    let outcome = fill(file, &temporary, contents, resolved).and_then(|()| {
        std::fs::rename(&temporary, resolved).map_err(|e| {
            Failure::Operational(format!("{} could not be replaced: {e}", resolved.display()))
        })
    });
    if outcome.is_err() {
        // A refusal that left a stray file beside the user's settings would be
        // its own small mess.
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

fn fill(
    mut file: std::fs::File,
    temporary: &Path,
    contents: &str,
    target: &Path,
) -> Result<(), Failure> {
    file.write_all(contents.as_bytes()).map_err(|e| {
        Failure::Operational(format!("{} could not be written: {e}", temporary.display()))
    })?;
    file.sync_all().map_err(|e| {
        Failure::Operational(format!("{} could not be flushed: {e}", temporary.display()))
    })?;
    drop(file);
    // `settings.json` is 0600 on this machine. A fresh temporary file is 0644,
    // and renaming it into place would quietly widen the user's permissions.
    if let Ok(meta) = std::fs::metadata(target) {
        let _ = std::fs::set_permissions(temporary, meta.permissions());
    }
    Ok(())
}

/// The whole of `source` into an already-open temporary, permissions and all.
///
/// The permissions matter: `settings.json` is 0600 on this machine and can hold
/// an `env` block of credentials, and a fresh temporary file is 0644. A backup
/// that widened them would be its own small leak.
fn copy_through(source: &Path, file: &mut std::fs::File, temporary: &Path) -> Result<(), Failure> {
    let mut reading = std::fs::File::open(source).map_err(|e| {
        Failure::Operational(format!("{} could not be read: {e}", source.display()))
    })?;
    std::io::copy(&mut reading, file).map_err(|e| {
        Failure::Operational(format!(
            "{} could not be copied to {}: {e}",
            source.display(),
            temporary.display()
        ))
    })?;
    if let Ok(meta) = std::fs::metadata(source) {
        let _ = file.set_permissions(meta.permissions());
    }
    file.sync_all().map_err(|e| {
        Failure::Operational(format!("{} could not be flushed: {e}", temporary.display()))
    })?;
    Ok(())
}

/// Give a finished copy the backup's name, and only if nothing else has it.
///
/// `hard_link` is the one operation here that is both atomic and refuses to
/// clobber. `rename` would overwrite a backup an earlier install made - the
/// pre-install bytes AC7 restores - and an `exists()` test before it is a race
/// two concurrent installs both win.
///
/// A filesystem without hard links (FAT, some network mounts) falls back to the
/// rename behind an `exists()` check: still atomic, with the refusal to clobber
/// back down to best effort.
fn link_once(temporary: &Path, path: &Path) -> Result<(), Failure> {
    let linked = match std::fs::hard_link(temporary, path) {
        Ok(()) => return Ok(()),
        // A backup is already there. It is never refreshed: it holds the bytes
        // from before any install ran, and this copy does not.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => e,
    };
    if path.exists() {
        return Ok(());
    }
    std::fs::rename(temporary, path).map_err(|e| {
        Failure::Operational(format!(
            "{} could not be written: {e} (and it could not be linked: {linked})",
            path.display()
        ))
    })
}

/// The lines `edit` adds and removes, with a little context and nothing else.
///
/// INST-03 asks for the exact diff, and exact is the point: `.claude.json` is
/// 240 KB and `settings.json`'s `hooks` object already holds seven event keys,
/// so a user asked to approve a change has to be shown the change rather than
/// the neighbourhood it happened in.
///
/// Common leading and trailing lines are trimmed off both sides first, which
/// reduces both files to a handful of lines around the insertions. What is left
/// is aligned by [`align`]. The trim is what keeps that alignment cheap: it
/// runs on the block that differs, never on the file.
pub fn diff(before: &str, after: &str) -> String {
    const CONTEXT: usize = 2;
    /// Above this many differing lines on either side, the block is printed as
    /// removed-then-added rather than aligned. Install's own edits never come
    /// near it; this only bounds the cost of a file that changed under us.
    const ALIGN_LIMIT: usize = 512;

    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();

    let mut head = 0;
    while head < old.len() && head < new.len() && old[head] == new[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < old.len() - head
        && tail < new.len() - head
        && old[old.len() - 1 - tail] == new[new.len() - 1 - tail]
    {
        tail += 1;
    }

    let old_middle = &old[head..old.len() - tail];
    let new_middle = &new[head..new.len() - tail];
    if old_middle.is_empty() && new_middle.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for line in &old[head.saturating_sub(CONTEXT)..head] {
        line_into(&mut out, ' ', line);
    }
    if old_middle.len() > ALIGN_LIMIT || new_middle.len() > ALIGN_LIMIT {
        for line in old_middle {
            line_into(&mut out, '-', line);
        }
        for line in new_middle {
            line_into(&mut out, '+', line);
        }
    } else {
        for (mark, line) in align(old_middle, new_middle) {
            line_into(&mut out, mark, line);
        }
    }
    let resume = old.len() - tail;
    for line in &old[resume..(resume + CONTEXT).min(old.len())] {
        line_into(&mut out, ' ', line);
    }
    out
}

fn line_into(out: &mut String, mark: char, line: &str) {
    out.push_str("   ");
    out.push(mark);
    out.push(' ');
    out.push_str(line);
    out.push('\n');
}

/// Two blocks of lines as one sequence of kept, removed and added lines.
///
/// A longest common subsequence, which is the ordinary diff. Quadratic, and
/// deliberately so: its caller has already cut both sides down to the block
/// that differs, and an implementation with a name is worth more here than a
/// heuristic that gets an insertion beside an identical line wrong.
fn align<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(char, &'a str)> {
    let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            out.push((' ', old[i]));
            i += 1;
            j += 1;
        } else if common[i + 1][j] >= common[i][j + 1] {
            out.push(('-', old[i]));
            i += 1;
        } else {
            out.push(('+', new[j]));
            j += 1;
        }
    }
    out.extend(old[i..].iter().map(|line| ('-', *line)));
    out.extend(new[j..].iter().map(|line| ('+', *line)));
    out
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn parse(text: &str) -> Result<Json, String> {
    let mut reader = Reader {
        src: text,
        bytes: text.as_bytes(),
        at: 0,
        depth: 0,
    };
    reader.skip_space();
    let value = reader.value()?;
    reader.skip_space();
    if reader.at != reader.bytes.len() {
        return Err(format!("trailing input at byte {}", reader.at));
    }
    Ok(value)
}

struct Reader<'a> {
    src: &'a str,
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!("nested more than {MAX_DEPTH} deep"));
        }
        let value = match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Json::Scalar),
            Some(_) => self.literal(),
            None => Err("unexpected end of input".to_owned()),
        };
        self.depth -= 1;
        value
    }

    /// A string literal, returned as the source text it is, quotes included.
    fn string(&mut self) -> Result<String, String> {
        let start = self.at;
        self.at += 1;
        loop {
            let Some(byte) = self.peek() else {
                return Err(format!("unterminated string at byte {start}"));
            };
            self.at += 1;
            match byte {
                b'"' => return Ok(self.src[start..self.at].to_owned()),
                b'\\' => {
                    // Skip the escaped character whole. It is ASCII in valid
                    // JSON, but a stray `\é` must not leave the cursor inside a
                    // UTF-8 sequence, where the slice above would panic.
                    if self.at >= self.bytes.len() {
                        return Err(format!("unterminated string at byte {start}"));
                    }
                    self.at += 1;
                    while self.at < self.bytes.len() && !self.src.is_char_boundary(self.at) {
                        self.at += 1;
                    }
                }
                _ => {}
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.at += 1;
        let mut members = Vec::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return Err(format!("expected a key at byte {}", self.at));
            }
            let raw_key = self.string()?;
            // An undecodable key never matches a lookup, which is the right
            // answer for a key install does not own, and the raw form is what
            // gets written back either way.
            let key = decode(&raw_key).unwrap_or_else(|| raw_key.clone());
            self.skip_space();
            if self.peek() != Some(b':') {
                return Err(format!("expected ':' at byte {}", self.at));
            }
            self.at += 1;
            self.skip_space();
            let value = self.value()?;
            members.push(Member {
                key,
                raw_key,
                value,
            });
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.at)),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value()?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.at)),
            }
        }
    }

    fn literal(&mut self) -> Result<Json, String> {
        let start = self.at;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b'.') {
                self.at += 1;
            } else {
                break;
            }
        }
        let raw = &self.src[start..self.at];
        if matches!(raw, "true" | "false" | "null") || is_number(raw) {
            return Ok(Json::Scalar(raw.to_owned()));
        }
        Err(format!("unexpected token at byte {start}"))
    }
}

/// JSON's number grammar, so `01`, `+1`, `.5`, `NaN` and `1e` are rejected
/// rather than round-tripped as text that no other parser would accept.
fn is_number(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut at = 0;
    if bytes.first() == Some(&b'-') {
        at += 1;
    }
    let digits = at;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    if at == digits || (bytes[digits] == b'0' && at - digits > 1) {
        return false;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let fraction = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == fraction {
            return false;
        }
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == exponent {
            return false;
        }
    }
    at == bytes.len()
}

/// A JSON string literal as the text it stands for, or `None` when its escapes
/// do not make one.
fn decode(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let high = hex4(&mut chars)?;
                let scalar = if (0xD800..0xDC00).contains(&high) {
                    if chars.next()? != '\\' || chars.next()? != 'u' {
                        return None;
                    }
                    let low = hex4(&mut chars)?;
                    if !(0xDC00..0xE000).contains(&low) {
                        return None;
                    }
                    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                } else {
                    high
                };
                out.push(char::from_u32(scalar)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn hex4(chars: &mut std::str::Chars) -> Option<u32> {
    let mut value = 0u32;
    for _ in 0..4 {
        value = value * 16 + chars.next()?.to_digit(16)?;
    }
    Some(value)
}

/// Text as a JSON string literal, escaped the way `JSON.stringify` escapes.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings file with everything that breaks a naive round trip in it:
    /// nested objects, an array of objects, unicode both raw and escaped, a
    /// path full of backslashes, an empty object and an empty array, a large
    /// integer, a float, and a key order no sort would produce.
    const FIXTURE: &str = r#"{
  "theme": "dark",
  "cleanupPeriodDays": 7,
  "env": {
    "CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS": "50000"
  },
  "attribution": {
    "commit": "",
    "pr": ""
  },
  "hooks": {
    "SessionStart": [],
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "$HOME/.claude/hooks/terse-answers.sh",
            "timeout": 5
          }
        ]
      }
    ]
  },
  "projects": {
    "C:\\Users\\jo\u00e9\\code": {
      "lastCost": 0.5,
      "createdAt": 1786647853145,
      "history": []
    },
    "/data/code/verbatim — ✓": {
      "allowedTools": [],
      "notes": {}
    }
  },
  "autoCompactEnabled": true,
  "voice": null
}
"#;

    fn seeded(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    /// The property everything else rests on: reading and writing a file this
    /// module did not create gives back the same bytes.
    #[test]
    fn a_real_shaped_file_round_trips_byte_for_byte() {
        let (_dir, path) = seeded(FIXTURE);
        let document = Document::read(&path).unwrap();
        assert_eq!(document.rendered(), FIXTURE);
    }

    /// AC4 in one test: adding a key changes that key and nothing else, and
    /// every other key keeps its value *and its position*.
    #[test]
    fn adding_a_key_leaves_every_other_key_where_it_was() {
        let (_dir, path) = seeded(FIXTURE);
        let document = Document::read(&path).unwrap();
        let before = document.value().keys().to_vec();

        document
            .apply(&|root: &mut Json| {
                root.entry("mcpServers", Json::object())?;
                Ok(())
            })
            .unwrap();

        let after = Document::read(&path).unwrap();
        let mut keys = after.value().keys();
        assert_eq!(keys, [before.as_slice(), &["mcpServers"]].concat());

        // `del(.mcpServers)` is byte-equal to the original.
        keys.pop();
        let mut stripped = after.value().clone();
        let Json::Object(members) = &mut stripped else {
            panic!("not an object")
        };
        members.retain(|member| member.key != "mcpServers");
        assert_eq!(render(&stripped, true), FIXTURE);
    }

    /// The symlink case that is real on this machine: `~/.claude.json` is a
    /// link two hops from the file it names. Renaming over the link would leave
    /// a regular file where the user's symlink was and orphan the real one.
    #[test]
    #[cfg(unix)]
    fn writing_through_a_symlink_updates_the_target_and_keeps_the_link() {
        let (dir, real) = seeded(FIXTURE);
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let document = Document::read(&link).unwrap();
        document
            .apply(&|root: &mut Json| {
                root.entry("mcpServers", Json::object())?;
                Ok(())
            })
            .unwrap();

        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the symlink was replaced by a regular file"
        );
        let written = std::fs::read_to_string(&real).unwrap();
        assert!(
            written.contains("\"mcpServers\""),
            "the target was not updated"
        );
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            real,
            "the link stopped pointing where it did"
        );
    }

    /// A second install must not overwrite the backup the first one made: that
    /// backup is the pre-install bytes, and it is what AC7 restores.
    #[test]
    fn a_second_backup_leaves_the_first_one_alone() {
        let (_dir, path) = seeded(FIXTURE);
        let first = Document::read(&path).unwrap().backup().unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), FIXTURE);

        std::fs::write(&path, r#"{"changed": true}"#).unwrap();
        let second = Document::read(&path).unwrap().backup().unwrap().unwrap();

        assert_eq!(second, first);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), FIXTURE);
    }

    /// The backup is the file, with the file's permissions, and the temporary
    /// it was written through is gone.
    ///
    /// 0600 is what `settings.json` is on this machine, and a fresh temporary
    /// file is 0644: a backup that widened them would be its own small leak.
    #[test]
    #[cfg(unix)]
    fn the_backup_carries_the_bytes_and_the_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, path) = seeded(FIXTURE);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let backup = Document::read(&path).unwrap().backup().unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), FIXTURE);
        assert_eq!(
            std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["settings.json", "settings.json.verbatim-backup"],
            "the temporary the backup was written through is still there"
        );
    }

    /// Nothing to back up when there is no file yet.
    #[test]
    fn a_file_that_does_not_exist_has_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(Document::read(&path).unwrap().backup().unwrap().is_none());
    }

    /// A file that exists and is not JSON is an operational failure that leaves
    /// the file exactly as it found it and no temporary file beside it.
    #[test]
    fn an_invalid_target_is_operational_and_leaves_nothing_behind() {
        let (dir, path) = seeded("{ \"a\": 1, oops }");
        match Document::read(&path) {
            Err(Failure::Operational(message)) => {
                assert!(message.contains("not valid JSON"), "{message}")
            }
            other => panic!("expected an operational failure, got {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ \"a\": 1, oops }"
        );
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy() != "settings.json")
            .collect();
        assert!(strays.is_empty(), "left behind {strays:?}");
    }

    /// An absent file is created, with the object install put in it.
    #[test]
    fn an_absent_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let document = Document::read(&path).unwrap();
        assert!(document
            .apply(&|root: &mut Json| {
                root.entry("hooks", Json::object())?;
                Ok(())
            })
            .unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"hooks\": {}\n}\n"
        );
    }

    /// Applying an edit that changes nothing writes nothing.
    #[test]
    fn an_edit_that_changes_nothing_does_not_write() {
        let (_dir, path) = seeded(FIXTURE);
        let document = Document::read(&path).unwrap();
        assert!(!document.apply(&|_: &mut Json| Ok(())).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FIXTURE);
    }

    /// A repeated key is refused, wherever it is, and the refusal says where.
    ///
    /// `entry` takes the first member with a key; `JSON.parse` takes the last.
    /// A file that makes those two disagree is one install must not rewrite.
    #[test]
    fn a_repeated_key_is_refused_and_located() {
        for (text, expected) in [
            (r#"{"hooks": {}, "hooks": {"SessionStart": []}}"#, ".hooks"),
            (
                r#"{"hooks": {"SessionStart": [], "SessionStart": []}}"#,
                ".hooks.SessionStart",
            ),
            (
                r#"{"hooks": {"SessionStart": [{"matcher": "", "matcher": ""}]}}"#,
                ".hooks.SessionStart[0].matcher",
            ),
        ] {
            let (_dir, path) = seeded(text);
            match Document::read(&path) {
                Err(Failure::Operational(message)) => assert!(
                    message.contains(&format!("more than one '{expected}' key")),
                    "{message}"
                ),
                other => panic!("expected an operational failure, got {other:?}"),
            }
        }
        // And a file that repeats nothing is still read.
        let (_dir, path) = seeded(FIXTURE);
        assert!(Document::read(&path).is_ok());
    }

    /// The reader rejects what is not JSON rather than round-tripping it as
    /// text some other parser would refuse.
    #[test]
    fn malformed_input_is_rejected() {
        for bad in [
            "{",
            "{\"a\": 1,}",
            "{\"a\" 1}",
            "{a: 1}",
            "[1, 2] extra",
            "{\"a\": 01}",
            "{\"a\": NaN}",
            "{\"a\": +1}",
            "{\"a\": 1e}",
            "{\"a\": \"unterminated}",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    /// The diff is the change, not the file.
    #[test]
    fn the_diff_shows_only_the_changed_block() {
        let (_dir, path) = seeded(FIXTURE);
        let document = Document::read(&path).unwrap();
        let after = document
            .preview(&|root: &mut Json| {
                root.entry("mcpServers", Json::object())?;
                Ok(())
            })
            .unwrap();
        let rendered = diff(&document.rendered(), &after);
        assert!(rendered.contains("+"), "{rendered}");
        assert!(rendered.contains("mcpServers"), "{rendered}");
        assert!(
            !rendered.contains("terse-answers"),
            "the diff carried a line nothing changed:\n{rendered}"
        );
        assert!(
            rendered.lines().count() < 10,
            "the diff is the whole file:\n{rendered}"
        );
        assert!(diff(&document.rendered(), &document.rendered()).is_empty());
    }

    /// Escapes survive the trip in both directions.
    #[test]
    fn strings_decode_and_encode() {
        assert_eq!(decode(r#""a\u00e9b""#).unwrap(), "aéb");
        assert_eq!(decode(r#""\ud83d\ude00""#).unwrap(), "😀");
        assert_eq!(decode(r#""C:\\Users""#).unwrap(), r"C:\Users");
        assert_eq!(decode(r#""\ud800""#), None);
        assert_eq!(encode("a\"b\\c\nd\u{1}"), r#""a\"b\\c\nd\u0001""#);
        assert_eq!(encode("é ✓"), "\"é ✓\"");
    }
}
