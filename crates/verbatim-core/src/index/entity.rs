//! RCL-02 and RCL-03: the exact-match entities a structured tool record leaves
//! behind.
//!
//! **Where they come from (D-02).** One record's own bytes and nothing else:
//! the `tool_use` and `tool_result` blocks of `message.content`, and the
//! top-level `toolUseResult` object. There is no join between a call and its
//! result - that would need a `tool_use_id` index and a second pass, and would
//! break the one-record-in / one-row-out shape that gives ingest and rebuild a
//! single seam. Nothing is read from prose, and nothing from `attachment`, even
//! though the text projection reads that one: D-02 names three subtrees.
//!
//! **What that buys.** An assistant sentence mentioning `src/main.rs` is a
//! guess about what happened; a `Read` whose `file_path` is `src/main.rs` is a
//! record of it. Exact-match recall over the second is worth something and over
//! the first is a keyword search that already exists in `turns_fts`.
//!
//! Nothing here rejects a value for being common (RCL-04). "Too common" changes
//! as the corpus grows and an index-time rejection is irreversible; commonness
//! is a query-time weighting problem.

use serde_json::Value;

use super::expand::case_components;
use super::text::floor_char_boundary;

/// A file or directory named by a structured tool field.
pub const PATH: &str = "path";
/// The program a `Bash` call ran.
pub const COMMAND: &str = "command";
/// A failure's normalized text (D-03, D-04).
pub const ERROR: &str = "error";
/// An identifier-shaped token out of a search pattern or an edit.
pub const SYMBOL: &str = "symbol";
/// The tool a record called.
pub const TOOL: &str = "tool";

/// Every kind, for a test or a report that needs the closed set.
pub const KINDS: [&str; 5] = [PATH, COMMAND, ERROR, SYMBOL, TOOL];

/// One extracted entity, before it becomes an `entities` row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entity {
    pub kind: &'static str,
    pub value: String,
}

/// The `tool_use` input keys that name a file outright.
const PATH_INPUTS: [&str; 3] = ["file_path", "path", "notebook_path"];

/// The recall tool whose input says what the model went looking for.
const RECALL_SEARCH: &str = "recall_search";

/// Is this the name of a `recall_search` call?
///
/// A suffix and not an equality: the harness registers an MCP tool under a
/// server-prefixed name (`mcp__verbatim__recall_search`), the bare name is what
/// a direct caller writes, and a build that matched only one of the two would
/// see none of the calls on a real machine. The separator is still required, so
/// a tool genuinely called `preflight_recall_search` matches and one called
/// `xrecall_search` does not.
pub fn is_recall_search(name: &str) -> bool {
    match name.strip_suffix(RECALL_SEARCH) {
        Some("") => true,
        Some(prefix) => prefix.ends_with('_'),
        None => false,
    }
}

/// How many entities one turn may emit, across every kind (RCL-04, D-15).
///
/// Measured over a 300-file sample of the real corpus: of the turns emitting any
/// entity at all, p50 emits 3, p90 emits 10, p99 emits 38 and the largest emits
/// 146. A cap of 48 therefore binds the tail and leaves the common turn
/// untouched - which is the point of capping at all, since the alternative is
/// one `git status --porcelain` result writing a row per changed file.
///
/// This is a bound on rows and **not** a stop-list. RCL-04 forbids rejecting an
/// entity at index time for being common - "too common" changes as the corpus
/// grows and the rejection is irreversible - and a value dropped by position
/// because its turn carried 150 of them is not a value rejected for what it is.
pub const MAX_ENTITIES_PER_TURN: usize = 48;

/// Every entity one turn record carries, deduplicated on `(kind, value)`, in the
/// order the record walk produces, and no more than
/// [`MAX_ENTITIES_PER_TURN`] of them.
///
/// The order is the whole reason a rebuild reproduces the same rows: it depends
/// on the record's bytes and on nothing about the store, so re-deriving the same
/// record twice writes the same list twice. The cap is applied in that same
/// order and never by frequency, recency or anything else the store knows -
/// a data-dependent cut would make the rebuild AC3 asserts depend on what else
/// was in the store at the time.
pub fn extract(record: &Value) -> Vec<Entity> {
    let mut out = Collector::default();

    if let Some(blocks) = record
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => out.tool_use(block),
                Some("tool_result") => out.tool_result(block),
                _ => {}
            }
        }
    }

    if let Some(result) = record.get("toolUseResult") {
        out.tool_use_result(result);
    }

    out.entities
}

#[derive(Default)]
struct Collector {
    entities: Vec<Entity>,
}

impl Collector {
    fn push(&mut self, kind: &'static str, value: impl Into<String>) {
        // The cap counts what was KEPT, so a repeated value never consumes a
        // slot and a turn naming one file forty times still has room for the
        // tool that touched it.
        if self.entities.len() >= MAX_ENTITIES_PER_TURN {
            return;
        }
        let value = value.into();
        if value.is_empty() {
            return;
        }
        let entity = Entity { kind, value };
        if !self.entities.contains(&entity) {
            self.entities.push(entity);
        }
    }

    /// One `tool_use` block: the tool itself, then whatever its inputs name.
    fn tool_use(&mut self, block: &Value) {
        let Some(name) = block.get("name").and_then(Value::as_str) else {
            return;
        };
        // The same value `parse::record::tool_name` lifts into `turns.tool_name`
        // (D-16 measured at most one `tool_use` block per record), so a
        // `kind = 'tool'` filter and the column cannot disagree.
        self.push(TOOL, name);

        let Some(input) = block.get("input") else {
            return;
        };

        for key in PATH_INPUTS {
            if let Some(raw) = input.get(key).and_then(Value::as_str) {
                if let Some(path) = normalize_path(raw) {
                    self.push(PATH, path);
                }
            }
        }

        match name {
            "Bash" => {
                if let Some(command) = input.get("command").and_then(Value::as_str) {
                    if let Some(program) = program_of(command) {
                        self.push(COMMAND, program);
                    }
                    for word in path_words(command) {
                        self.push(PATH, word);
                    }
                }
            }
            // A search pattern is what the user was looking FOR, which is the
            // one free-text field whose contents are reliably identifiers.
            "Grep" => self.symbols(input.get("pattern")),
            // Both sides of an edit: the symbol that went and the one that came.
            "Edit" => {
                self.symbols(input.get("old_string"));
                self.symbols(input.get("new_string"));
            }
            _ if is_recall_search(name) => self.recall_search(input),
            _ => {}
        }
    }

    /// What a `recall_search` call went looking FOR (FEED-02, D-13).
    ///
    /// This is the one tool whose inputs are evidence about the injector rather
    /// than about the project: a model that asked recall for something is a
    /// model that did not already have it, which is the whole of how a decision
    /// gets labelled `miss`. The label join is SQL over `entities` and may not
    /// read the blob (D-07), so if the query never becomes rows there is nothing
    /// downstream that can say what was searched for.
    ///
    /// The `query` is treated exactly as a `Grep` `pattern` is - it is the same
    /// kind of field, free text that reliably holds identifiers - plus its
    /// path-shaped words, because a person asking recall about a file types the
    /// file. Each `paths` filter entry is a path outright.
    fn recall_search(&mut self, input: &Value) {
        let query = input.get("query");
        self.symbols(query);
        if let Some(text) = query.and_then(Value::as_str) {
            for word in path_words(text) {
                self.push(PATH, word);
            }
        }
        for entry in input
            .get("paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(path) = entry.as_str().and_then(normalize_path) {
                self.push(PATH, path);
            }
        }
    }

    /// The identifier-shaped tokens of a string field, and nothing else.
    ///
    /// `old_string` and a `Grep` `pattern` can both hold prose, so the shape
    /// test is what keeps RCL-02's "not from prose" true for a field that is not
    /// structurally guaranteed to hold a symbol. An internal case or underscore
    /// boundary admits `SearchManager` and `search_manager` and refuses every
    /// ordinary English word.
    fn symbols(&mut self, field: Option<&Value>) {
        let Some(text) = field.and_then(Value::as_str) else {
            return;
        };
        for token in identifier_tokens(text) {
            if is_identifier_shaped(token) {
                self.push(SYMBOL, token);
            }
        }
    }

    /// One `tool_result` block: half of D-03's error signal.
    ///
    /// `is_error: true` and nothing else - not the `interrupted` flag, which
    /// says a `Bash` call was cut short rather than that it failed, and not an
    /// exit code, because the transcript carries none.
    fn tool_result(&mut self, block: &Value) {
        if block.get("is_error").and_then(Value::as_bool) != Some(true) {
            return;
        }
        if let Some(value) = normalize_error(&result_text(block)) {
            self.push(ERROR, value);
        }
    }

    /// The top-level `toolUseResult`. Two path keys, not one, and a non-empty
    /// `stderr` as the other half of D-03's error signal.
    ///
    /// D-02's measurement counted `filePath` at the top level only (679 of that
    /// sample) and the code that followed it read only there. `Read` - the tool
    /// that names a file more often than any other - nests its path one level
    /// down at `file.filePath` instead, so the result-side turn of every `Read`
    /// emitted no path at all. Remeasured over a 400-file sample: 8,719
    /// `toolUseResult` objects carrying `file.filePath` 1,238 times against the
    /// top-level key's 1,171, so reading one of the two dropped slightly more
    /// than half of the result-side paths in the archive.
    ///
    /// Both are read and neither is preferred. A record carrying the same value
    /// twice is deduped by `push` on `(kind, value)` like any other repeat.
    fn tool_use_result(&mut self, result: &Value) {
        let nested = result
            .get("file")
            .and_then(|file| file.get("filePath"))
            .and_then(Value::as_str);
        for raw in [result.get("filePath").and_then(Value::as_str), nested]
            .into_iter()
            .flatten()
        {
            if let Some(path) = normalize_path(raw) {
                self.push(PATH, path);
            }
        }
        if let Some(stderr) = result.get("stderr").and_then(Value::as_str) {
            if let Some(value) = normalize_error(stderr) {
                self.push(ERROR, value);
            }
        }
    }
}

/// A `tool_result` block's own text: a string, or the `text` blocks of an array.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// How many bytes of normalized error text one entity value may carry.
///
/// An entity value is a key an exact-match lookup compares whole, so a stderr
/// dump of a failing test suite is not one value worth keeping - it is one value
/// that will never recur, stored per turn. The head of a failure is the part
/// that repeats; the whole of it stays in the blob either way.
pub const MAX_ERROR_BYTES: usize = 512;

/// What a UUID normalizes to.
pub const UUID: &str = "<uuid>";
/// What an ISO-8601 timestamp normalizes to.
pub const TIMESTAMP: &str = "<ts>";
/// What a `0x` hex address normalizes to.
pub const ADDRESS: &str = "<addr>";
/// What a `:line` or `:line:col` suffix normalizes to, colon included.
pub const LINE_COL: &str = ":<line>";

/// One error's text reduced to the value two occurrences of the same failure
/// share (D-04).
///
/// Four substitutions and no more: UUIDs, ISO-8601 timestamps, `0x` addresses
/// and `:line:col` suffixes. **Bare integers are deliberately kept.** Stripping
/// them was measured and scores worse on the property the rule exists for: over
/// a 400-file sample the four rules leave 46 error values recurring across
/// sessions, and adding bare integers leaves 39 - the aggressive rule merges
/// failures that are genuinely different faster than it merges ones that are the
/// same.
///
/// Whitespace runs collapse to one space, so a value is single-line and two
/// occurrences that differ only in wrapping or trailing space are one value.
///
/// `None` when nothing is left, which is how an empty `stderr` - the ordinary
/// case on a successful call - contributes no entity.
pub fn normalize_error(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = String::new();
    let mut chunk = 0;
    let mut i = 0;

    while i < bytes.len() {
        // A match is only ever attempted at an ASCII byte, so `i` and `chunk`
        // are always char boundaries and the slices below cannot split a
        // character.
        match placeholder_at(bytes, i) {
            Some((len, placeholder)) => {
                push_collapsed(&mut out, &raw[chunk..i]);
                out.push_str(placeholder);
                i += len;
                chunk = i;
            }
            None => i += 1,
        }
    }
    push_collapsed(&mut out, &raw[chunk..]);

    let trimmed = out.trim_end();
    let take = floor_char_boundary(trimmed, MAX_ERROR_BYTES.min(trimmed.len()));
    (take > 0).then(|| trimmed[..take].to_owned())
}

/// Text appended with every whitespace run collapsed to a single space.
fn push_collapsed(out: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_whitespace() {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

/// The variable part starting at `at`, if one does: its byte length and what it
/// normalizes to.
///
/// Timestamp before line-col, because `09:14:22` inside `2026-08-12T09:14:22Z`
/// is a time and not a source position.
fn placeholder_at(bytes: &[u8], at: usize) -> Option<(usize, &'static str)> {
    if let Some(len) = timestamp_at(bytes, at) {
        return Some((len, TIMESTAMP));
    }
    if let Some(len) = uuid_at(bytes, at) {
        return Some((len, UUID));
    }
    if let Some(len) = address_at(bytes, at) {
        return Some((len, ADDRESS));
    }
    line_col_at(bytes, at).map(|len| (len, LINE_COL))
}

/// `NNNN-NN-NNTNN:NN:NN`, with the fractional seconds and the zone offset both
/// optional - the corpus writes `.NNNZ` and a future producer may not.
fn timestamp_at(bytes: &[u8], at: usize) -> Option<usize> {
    if !starts_token(bytes, at) {
        return None;
    }
    let mut i = at;
    i = digits(bytes, i, 4)?;
    i = byte(bytes, i, b'-')?;
    i = digits(bytes, i, 2)?;
    i = byte(bytes, i, b'-')?;
    i = digits(bytes, i, 2)?;
    i = byte(bytes, i, b'T')?;
    i = digits(bytes, i, 2)?;
    i = byte(bytes, i, b':')?;
    i = digits(bytes, i, 2)?;
    i = byte(bytes, i, b':')?;
    i = digits(bytes, i, 2)?;
    if let Some(next) = byte(bytes, i, b'.') {
        i = digits_run(bytes, next)?;
    }
    match bytes.get(i) {
        Some(b'Z') => i += 1,
        Some(b'+') | Some(b'-') => {
            let mut zone = digits(bytes, i + 1, 2)?;
            if let Some(next) = byte(bytes, zone, b':') {
                zone = digits(bytes, next, 2)?;
            }
            i = zone;
        }
        _ => {}
    }
    ends_token(bytes, i).then_some(i - at)
}

/// The 8-4-4-4-12 hex form, which is the only one the transcript and the tools
/// under it write.
fn uuid_at(bytes: &[u8], at: usize) -> Option<usize> {
    if !starts_token(bytes, at) {
        return None;
    }
    let mut i = at;
    for (n, group) in [8, 4, 4, 4, 12].into_iter().enumerate() {
        if n > 0 {
            i = byte(bytes, i, b'-')?;
        }
        i = hex(bytes, i, group)?;
    }
    ends_token(bytes, i).then_some(i - at)
}

/// `0x` followed by hex digits.
fn address_at(bytes: &[u8], at: usize) -> Option<usize> {
    if !starts_token(bytes, at) || bytes.get(at) != Some(&b'0') || bytes.get(at + 1) != Some(&b'x')
    {
        return None;
    }
    let mut i = at + 2;
    while bytes.get(i).is_some_and(u8::is_ascii_hexdigit) {
        i += 1;
    }
    if i == at + 2 {
        return None;
    }
    ends_token(bytes, i).then_some(i - at)
}

/// `:line` or `:line:col`, attached to the token before it.
///
/// The attachment is the test that keeps this off a bare number: a source
/// position follows a file name (`reader.rs:214:9`), so the byte before the
/// colon has to be part of a token.
fn line_col_at(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b':') {
        return None;
    }
    if !bytes
        .get(at.wrapping_sub(1))
        .is_some_and(u8::is_ascii_alphanumeric)
    {
        return None;
    }
    let mut i = digits_run(bytes, at + 1)?;
    if let Some(next) = byte(bytes, i, b':') {
        if let Some(end) = digits_run(bytes, next) {
            i = end;
        }
    }
    ends_token(bytes, i).then_some(i - at)
}

/// Is `at` the start of a token rather than the middle of one? Keeps a match
/// from landing on the tail of a longer word.
fn starts_token(bytes: &[u8], at: usize) -> bool {
    !bytes
        .get(at.wrapping_sub(1))
        .is_some_and(u8::is_ascii_alphanumeric)
}

/// Is `at` past the end of a token?
fn ends_token(bytes: &[u8], at: usize) -> bool {
    !bytes.get(at).is_some_and(u8::is_ascii_alphanumeric)
}

fn byte(bytes: &[u8], at: usize, want: u8) -> Option<usize> {
    (bytes.get(at) == Some(&want)).then_some(at + 1)
}

fn digits(bytes: &[u8], at: usize, count: usize) -> Option<usize> {
    run(bytes, at, count, u8::is_ascii_digit)
}

fn hex(bytes: &[u8], at: usize, count: usize) -> Option<usize> {
    run(bytes, at, count, u8::is_ascii_hexdigit)
}

fn run(bytes: &[u8], at: usize, count: usize, class: fn(&u8) -> bool) -> Option<usize> {
    (0..count)
        .all(|n| bytes.get(at + n).is_some_and(class))
        .then_some(at + count)
}

/// One or more digits, however many.
fn digits_run(bytes: &[u8], at: usize) -> Option<usize> {
    let mut i = at;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    (i > at).then_some(i)
}

/// A path as the record wrote it, minus surrounding quotes and a trailing
/// `:line:col`.
///
/// Never canonicalized against the filesystem. 52% of the real corpus's `cwd`
/// directories no longer exist (phase 2 D-05), so a path that resolved today
/// would normalize differently tomorrow and the same file would key two ways
/// across one archive.
pub fn normalize_path(raw: &str) -> Option<String> {
    let trimmed = trim_quotes(raw.trim());
    let stripped = strip_line_col(trimmed);
    (!stripped.is_empty()).then(|| stripped.to_owned())
}

/// The program a shell command ran: the first argv word that is not a
/// `NAME=value` assignment, reduced to its basename.
///
/// The basename is what makes the entity worth having. `cargo`, `/usr/bin/cargo`
/// and `~/.cargo/bin/cargo` are one command to anybody searching for what they
/// ran, and three values to anybody who kept the spelling.
pub fn program_of(command: &str) -> Option<String> {
    for word in command.split_whitespace() {
        let word = trim_quotes(word);
        if word.is_empty() {
            continue;
        }
        // `FOO=bar cmd`: an assignment prefix, not the program. Tested before
        // the path split, because the value may itself contain a separator.
        if is_assignment(word) {
            continue;
        }
        let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
        return (!base.is_empty()).then(|| base.to_owned());
    }
    None
}

/// The path-shaped words of a shell command line.
///
/// Path-shaped means "carries a separator": a bare word in argv is as likely to
/// be a subcommand or a flag value as a file, and guessing wrong fills the
/// exact-match table with words that are not paths, which is the one thing that
/// table is for. A leading `-` is a flag whatever follows it.
///
/// **Whitespace alone does not find those words.** A command line is shell
/// syntax, so a path arrives welded to the punctuation around it: `cd /a/b;
/// make` gives `/a/b;`, `cargo test 2>/dev/null` gives `2>/dev/null`, and each
/// is stored as a key that a lookup for the real path cannot match. Splitting
/// every word on the shell's own control characters is what separates the path
/// from the syntax around it. Measured over a 400-file sample: 9,816 of 21,061
/// path-shaped words (46.6%) carried one of these characters, so this was the
/// common case and not the tail.
///
/// A surviving word is still dropped when it carries an expansion, a glob or a
/// URL scheme. `${ROOT}/x`, `src/*.rs` and `https://example.com/x` each name
/// something other than one file on disk, and none of the three is a key an
/// exact-match lookup will ever be handed. This is not RCL-04's forbidden
/// rejection-for-being-common: a glob or an unexpanded variable is not a path
/// that happens to be popular, it is not a path.
pub fn path_words(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .flat_map(|word| word.split(SHELL_CONTROL))
        .map(|word| word.trim_matches(QUOTES))
        .filter(|word| !word.starts_with('-') && !is_assignment(word))
        .filter(|word| word.contains('/') || word.contains('\\'))
        .filter(|word| !word.contains(NOT_A_PATH) && !word.contains("://"))
        .filter_map(normalize_path)
        .collect()
}

/// Shell syntax a path arrives welded to: the separators, the redirections and
/// the grouping. Splitting on these is what leaves the path behind.
const SHELL_CONTROL: [char; 7] = [';', '&', '|', '<', '>', '(', ')'];

/// What makes a word name something other than one file on disk: an expansion,
/// a glob, or a brace list.
const NOT_A_PATH: [char; 6] = ['$', '`', '*', '?', '{', '}'];

/// A quote is shell syntax at either end of a word, whether or not its partner
/// survived the split - `git commit -m "fix /a/b"` leaves `/a/b"` behind, and
/// the trailing byte is not part of the file's name.
const QUOTES: [char; 2] = ['"', '\''];

/// Is this token shaped like an identifier rather than like a word?
///
/// An internal case boundary (`searchManager`, `SearchManager`) or an internal
/// underscore with content on both sides (`search_manager`). `Search` alone is
/// not one, and neither is `the`.
pub fn is_identifier_shaped(token: &str) -> bool {
    if case_components(token).len() > 1 {
        return true;
    }
    let mut parts = token.split('_');
    let first = parts.next().unwrap_or_default();
    !first.is_empty() && parts.clone().count() > 0 && parts.all(|part| !part.is_empty())
}

/// Maximal runs of identifier characters. Distinct from
/// [`super::expand::separator_components`] on exactly one point: `_` is kept, so
/// `search_manager` survives to be judged as one token.
pub fn identifier_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
}

fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

fn trim_quotes(word: &str) -> &str {
    let bytes = word.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        return &word[1..word.len() - 1];
    }
    word
}

/// Drop a trailing `:line` or `:line:col`, which names a position inside a file
/// rather than a different file.
fn strip_line_col(path: &str) -> &str {
    let once = strip_trailing_number(path);
    strip_trailing_number(once)
}

fn strip_trailing_number(path: &str) -> &str {
    match path.rsplit_once(':') {
        Some((head, tail))
            if !head.is_empty() && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => path,
    }
}
