//! Redaction at egress, keyed on the declared destination (PRIV-03, D-13,
//! D-16).
//!
//! # Two entry points, one rule set
//!
//! [`for_destination`] is the gate for whatever text its caller hands it: a
//! whole request body, one message's content on its way into one, a recall
//! excerpt on its way into an injected brief. It parses nothing and reads no
//! structure, so it has no opinion about which of those it was given, and every
//! rule below is written to hold on plain prose as well as on a document.
//! `local = true` hands the text back untouched, because a local provider is
//! not egress at all and filtering it would destroy detail for nothing;
//! anything else - including a `local` key the user forgot to write - hands
//! back a filtered copy.
//!
//! [`scrub`] is the error boundary, and it runs whatever the destination is.
//! That is the half of D-16 that survives the `local` distinction, and it
//! survives for a reason with a paper trail: `feedback`'s `Drained::discarded`
//! and `Labeled::notes` travel into `runs.error` because there is no log file,
//! `runs.error` is free text, and `verbatim status` prints that column. A 401
//! body echoing an API key would be durably stored and reprinted, which is
//! worse than a leak that scrolls past.
//!
//! Both go through [`redact`], so there is one rule set and not two that drift.
//!
//! # The rules, and why each one is here
//!
//! In the order the one rule set runs them, each with the marker it leaves.
//!
//! 1. **The credential this run resolved**, wherever it appears. The only exact
//!    rule; everything below it is a guess at shape.
//!    Marker: [`REDACTED_CREDENTIAL`].
//! 2. **PEM private key blocks**, whole. Marker: [`REDACTED_PRIVATE_KEY`].
//! 3. **A connection URL's userinfo** - the span between `://` and the `@` that
//!    closes it, with the scheme and the host left standing so a reader can see
//!    which machine was reached. Ahead of rule 4 deliberately: a userinfo name
//!    matching [`SECRET_NAMES`] is a `name: value` shape, and rule 4 would take
//!    the host away with the password. Marker: [`REDACTED_URL_USERINFO`].
//! 4. **Header shapes, ANYWHERE on a line** - `name: value` whose name matches
//!    [`SECRET_NAMES`]. `Authorization: Bearer ...` is the case this exists for,
//!    and "anywhere" is what makes it fire on a transcript turn, which arrives
//!    as one line behind a `turn_id=` prefix rather than as a bare header line.
//!    What it takes is bounded, so the rest of that turn survives.
//!    Marker: [`REDACTED`].
//! 5. **JSON string pairs** whose name matches [`SECRET_NAMES`].
//!    Marker: [`REDACTED`].
//! 6. **Assignment-shaped text** - `NAME=value` whose name matches
//!    [`SECRET_NAMES`]. Marker: [`REDACTED`].
//! 7. **Space-separated secret flags** - `--name value`, which rule 6 cannot see
//!    because it scans for `=`. Marker: [`REDACTED_FLAG_VALUE`].
//! 8. **Bare JSON Web Tokens** - an `eyJ`-prefixed base64url run. Nothing names
//!    it, so rules 4 to 7 have nothing to catch it by; it names itself instead,
//!    because `eyJ` is `{"` in base64url. Marker: [`REDACTED_JWT`].
//! 9. **GitHub tokens** - a run opening with one of [`GITHUB_TOKEN_PREFIXES`],
//!    also nameless in the wild. That list is a pinned 2026-08-30 snapshot with
//!    no in-repo source of truth and it will go stale: a prefix GitHub invents
//!    after that date is missed until someone edits the array.
//!    Marker: [`REDACTED_GITHUB_TOKEN`].
//!
//! Every one of them leaves a marker naming what went. A payload that came back
//! silently shorter would leave a reader unable to tell filtering from a
//! provider that returned less.
//!
//! # The direction to be wrong in
//!
//! The name test is a substring match and therefore OVER-matches: `monkey`
//! contains `key`. That is the tolerated failure, stated as one: a
//! name-substring false positive costs a caller some detail in one message,
//! while under-redaction costs a key rotation.
//!
//! Rules 8 and 9 over-match for a second reason and in the same direction.
//! Their length floors are set against this repo's short unrealistic sentinels
//! (`ghp_abc123XYZ`) rather than against real credentials, because
//! realistic-length values in a public repo trip GitHub push protection - and a
//! floor low enough to fire on a sentinel fires on more ordinary text than one
//! sized to a real token would. What that costs is measured rather than
//! guessed: over the 60 most recent real transcripts, 21 MB, runs carrying a
//! GitHub prefix occurred 0 times and bare `eyJ`-prefixed runs 4 times.
//!
//! # Egress only, never ingest
//!
//! `.planning/PROJECT.md` bars ingest-time redaction outright - it makes the
//! store lossy and destroys what it strips - and nothing in this module is
//! reachable from the ingest write path.
//! `tests/egress.rs::nothing_in_the_ingest_path_names_the_egress_filter` holds
//! that shut at the source level.

use std::borrow::Cow;

use crate::config::{Secret, REDACTED};

/// The name fragments that make a value a secret, matched case-insensitively
/// as substrings.
pub const SECRET_NAMES: &[&str] = &[
    "token",
    "password",
    "passwd",
    "secret",
    "key",
    "api",
    "bearer",
    // The header a session pastes verbatim more often than any other secret
    // shape after `Authorization` - measured 7 times over the 60 most recent
    // real transcripts, 21 MB, on 2026-08-30.
    "cookie",
    // Not in the phase plan's list and here anyway: it is the exact header the
    // one request this phase makes carries its credential in, and matching it
    // by the `bearer` in its value would depend on the scheme's spelling.
    "authorization",
];

/// What replaces the configured credential wherever it is found.
pub const REDACTED_CREDENTIAL: &str = "[redacted: the configured provider credential]";

/// What replaces a PEM private key block.
pub const REDACTED_PRIVATE_KEY: &str = "[redacted: a PEM private key block]";

/// What replaces a bare JSON Web Token.
///
/// Every marker the value-shape rules leave is ONE whitespace-free token,
/// deliberately: a marker can land exactly where a later rule looks for a
/// value, every value run in this file stops at whitespace, and a marker that
/// holds no whitespace is therefore replaced by itself. That is what makes
/// scrubbing twice the same as scrubbing once, so a test can attribute a catch
/// to the rule that made it.
pub const REDACTED_JWT: &str = "[redacted:a-json-web-token]";

/// What replaces a GitHub token. One whitespace-free token, for
/// [`REDACTED_JWT`]'s reason.
pub const REDACTED_GITHUB_TOKEN: &str = "[redacted:a-github-token]";

/// What replaces the userinfo a connection URL carries before its `@`. One
/// whitespace-free token, for [`REDACTED_JWT`]'s reason.
pub const REDACTED_URL_USERINFO: &str = "[redacted:connection-url-userinfo]";

/// What replaces the value of a space-separated secret flag. One
/// whitespace-free token, for [`REDACTED_JWT`]'s reason - and here that is not
/// a nicety: the value this rule takes IS the next whitespace-delimited run, so
/// a marker holding a space would be re-consumed and grown on every scrub.
pub const REDACTED_FLAG_VALUE: &str = "[redacted:a-command-line-flag-value]";

/// The prefixes GitHub issues its credentials under.
///
/// A PINNED list and a 2026-08-30 snapshot with no in-repo source of truth: it
/// WILL go stale, and a prefix GitHub invents after that date is missed until
/// someone edits this array. A generic `gh?_` shape was rejected because it
/// matches unrelated text.
pub const GITHUB_TOKEN_PREFIXES: &[&str] = &["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"];

/// How many bytes a run needs after a [`GITHUB_TOKEN_PREFIXES`] prefix.
///
/// Loose on purpose. This repo's fixtures are short unrealistic sentinels
/// (`ghp_abc123XYZ`) because realistic-length values in a public repo risk
/// GitHub push protection, so the floor is sized against a sentinel rather than
/// against a real token. What the looseness costs is measured rather than
/// guessed: over the 60 most recent real transcripts, 21 MB, a run carrying one
/// of these prefixes occurred 0 times.
const MIN_GITHUB_TOKEN_BODY: usize = 6;

/// A bare `eyJ`-prefixed run shorter than this is not treated as a JWT.
///
/// Sized against sentinels for [`MIN_GITHUB_TOKEN_BODY`]'s reason. Measured
/// cost over the same sample: 4 bare `eyJ`-prefixed runs.
const MIN_JWT_CHARS: usize = 12;

/// A credential shorter than this is not replaced by rule 1.
///
/// The rule is a plain substring replacement, so a two-character credential
/// would blank two characters out of every word in the payload and leave a
/// reader with rubble. Nothing that short is a credential; if one ever is, the
/// shape rules below still catch it wherever it is labelled.
const MIN_CREDENTIAL_CHARS: usize = 4;

/// Filter text for its declared destination (D-13).
///
/// `local = true` is the only thing that returns the text unchanged, and it is
/// a DECLARATION about where the bytes end up rather than an observation about
/// the address. Absent or false means remote, so a user who forgets the key
/// pays the filter - which is the harmless direction to be wrong in.
pub fn for_destination<'a>(
    local: bool,
    credential: Option<&Secret>,
    text: &'a str,
) -> Cow<'a, str> {
    if local {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(redact(credential, text))
    }
}

/// Scrub a string that is about to become an error (D-16).
///
/// No destination argument, deliberately: an error travels to `runs.error` and
/// to stderr whatever the provider was, and those are local destinations that
/// keep the bytes.
pub fn scrub(credential: Option<&Secret>, text: &str) -> String {
    redact(credential, text)
}

/// The one rule set both entry points run.
fn redact(credential: Option<&Secret>, text: &str) -> String {
    let text = replace_credential(text, credential);
    let text = redact_pem_blocks(&text);
    // Before the header rule, not after: a userinfo username matching
    // `SECRET_NAMES` (`postgres://apiuser:pw@host/db`) is a `name: value` shape
    // to rule 4, which would take the host along with the password and leave a
    // reader unable to see which machine was reached.
    let text = redact_url_userinfo(&text);
    let text = redact_header_lines(&text);
    let text = redact_json_pairs(&text);
    let text = redact_assignments(&text);
    let text = redact_flag_values(&text);
    let text = redact_jwts(&text);
    redact_github_tokens(&text)
}

/// Rule 1: the exact value, wherever it is.
fn replace_credential(text: &str, credential: Option<&Secret>) -> String {
    match credential {
        Some(secret) if secret.expose().chars().count() >= MIN_CREDENTIAL_CHARS => {
            text.replace(secret.expose(), REDACTED_CREDENTIAL)
        }
        _ => text.to_owned(),
    }
}

/// Rule 2: `-----BEGIN ... PRIVATE KEY-----` through its `-----END ...-----`.
///
/// An unterminated block is redacted to the end of the text. A private key that
/// was cut off mid-way is still most of a private key.
fn redact_pem_blocks(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    const FENCE: &str = "-----";

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let Some(label_end) = after.find(FENCE) else {
            break;
        };
        if !after[..label_end]
            .to_ascii_uppercase()
            .contains("PRIVATE KEY")
        {
            // A certificate, a public key, anything else: not a secret, and
            // cutting it would remove evidence the reader may need.
            let keep = start + BEGIN.len() + label_end + FENCE.len();
            out.push_str(&rest[..keep]);
            rest = &rest[keep..];
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(REDACTED_PRIVATE_KEY);
        const END: &str = "-----END ";
        let body = &after[label_end + FENCE.len()..];
        rest = match body.find(END) {
            Some(end) => {
                let after_end = end + END.len();
                match body[after_end..].find(FENCE) {
                    Some(tail) => &body[after_end + tail + FENCE.len()..],
                    // An END line with no closing fence: nothing after it can
                    // be trusted to be outside the block.
                    None => "",
                }
            }
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Rule 3: the userinfo a connection URL carries before its `@`.
///
/// The scheme and everything from the `@` onward stay, so a reader can still
/// see which host was reached - which is the whole reason this is its own rule
/// rather than a header match. Nothing here assumes the surrounding text is a
/// request body.
fn redact_url_userinfo(text: &str) -> String {
    const SEP: &str = "://";
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while let Some(offset) = text[at..].find(SEP) {
        let start = at + offset + SEP.len();
        let mut scan = start;
        while scan < bytes.len() && bytes[scan] != b'@' && is_userinfo_byte(bytes[scan]) {
            scan += 1;
        }
        if scan > start && bytes.get(scan) == Some(&b'@') {
            out.push_str(&text[copied..start]);
            out.push_str(REDACTED_URL_USERINFO);
            copied = scan;
        }
        at = scan.max(start);
    }
    out.push_str(&text[copied..]);
    out
}

/// Can this byte appear in a URL's userinfo?
///
/// The `@` that closes the userinfo is the caller's own stop. These are the
/// others, and they are what make this a rule rather than a wildcard: an
/// authority ends at `/`, `?`, `#`, whitespace or a quote, and a scan that ran
/// past them would read `https://example.com/path@thing` as a credential.
fn is_userinfo_byte(b: u8) -> bool {
    !(b.is_ascii_whitespace() || matches!(b, b'/' | b'?' | b'#' | b'"' | b'\'' | b'`' | b'\\'))
}

/// Rule 4: a header shape ANYWHERE on a line, whose name is a secret name.
///
/// The name is the run of ASCII letters, digits, `-` and `_` that ends at a
/// colon, and every colon on the line is tested rather than only the first.
/// Transcript text is why: `judgment::transcript` writes each turn as
/// `turn_id=<id> <record_type>: <said>` and collapses the turn to one line, so
/// the first colon on every line sits behind a prefix carrying a space and an
/// `=`. A rule that could only ever test that colon was inert on exactly the
/// text this filter exists to guard.
///
/// A name whose preceding byte is a double quote is left to rule 5:
/// `  "api_key": "x",` is a JSON pair, and taking it here would swallow the
/// line's trailing comma and hand the endpoint a document that is not JSON.
///
/// What goes is BOUNDED - an optional auth scheme word plus one value run -
/// rather than the rest of the line. A whole turn is one line, so running to
/// end of line would erase the rest of that turn from the prompt while leaving
/// its `turn_id=` anchor standing, and the model can still anchor a claim to a
/// turn it was shown a fragment of.
fn redact_header_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // `split_inclusive` keeps each line's own newline on it, so nothing has to
    // be re-added and a text with no trailing newline round-trips unchanged.
    for line in text.split_inclusive('\n') {
        // The line ending is held back and re-attached, so a CRLF payload
        // keeps its CRs: this runs over text that may already be an HTTP
        // request, where a lost \r is a protocol error rather than cosmetics.
        let ending_len = if line.ends_with("\r\n") {
            2
        } else {
            usize::from(line.ends_with('\n'))
        };
        let (body, ending) = line.split_at(line.len() - ending_len);
        redact_header_spans(body, &mut out);
        out.push_str(ending);
    }
    out
}

/// Every `name: value` shape on one line's body, appended to `out`.
fn redact_header_spans(line: &str, out: &mut String) {
    let bytes = line.as_bytes();
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b':' {
            at += 1;
            continue;
        }
        let colon = at;
        let mut name_start = colon;
        while name_start > copied && is_name_byte(bytes[name_start - 1]) {
            name_start -= 1;
        }
        // An empty run, a quoted name (rule 5's case) or an ordinary word: not
        // a header, and the colon is just punctuation.
        if name_start == colon
            || (name_start > 0 && bytes[name_start - 1] == b'"')
            || !is_secret_name(&line[name_start..colon])
        {
            at = colon + 1;
            continue;
        }
        let span_start = skip_blanks(bytes, colon + 1);
        let first_end = value_run_end(bytes, span_start);
        // `Authorization: Bearer <value>` has to lose the scheme word and the
        // value together, or the shape is still legible enough to say what kind
        // of credential was sent and where the rest of it went.
        let span_end = if first_end > span_start && is_auth_scheme(&line[span_start..first_end]) {
            let next_start = skip_blanks(bytes, first_end);
            let next_end = value_run_end(bytes, next_start);
            if next_end > next_start {
                next_end
            } else {
                first_end
            }
        } else {
            first_end
        };
        if span_end == span_start {
            at = colon + 1;
            continue;
        }
        out.push_str(&line[copied..=colon]);
        out.push(' ');
        out.push_str(REDACTED);
        copied = span_end;
        at = span_end;
    }
    out.push_str(&line[copied..]);
}

/// Is this byte part of a header name?
fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// The first index at or after `from` that is not a space or a tab.
///
/// Spaces and tabs only, never the whole whitespace class: a caller has already
/// split the line and holds its ending back, and a lone `\r` in the middle of a
/// body is content rather than a separator.
fn skip_blanks(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
        at += 1;
    }
    at
}

/// The end of an unquoted value run starting at `from`.
///
/// It stops at whitespace, at a quote and at a backslash for rule 6's reason:
/// this filter runs over text that may sit INSIDE a JSON string, where the byte
/// after the value is the string's closing quote, and running past it would
/// hand the endpoint a document that is not JSON.
fn value_run_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < bytes.len() {
        let b = bytes[at];
        if b.is_ascii_whitespace() || b == b'"' || b == b'\'' || b == b'\\' {
            break;
        }
        at += 1;
    }
    at
}

/// Is this word an auth scheme, and therefore part of the value that follows?
fn is_auth_scheme(word: &str) -> bool {
    const SCHEMES: &[&str] = &["bearer", "basic", "digest", "token", "negotiate", "ntlm"];
    SCHEMES
        .iter()
        .any(|scheme| word.eq_ignore_ascii_case(scheme))
}

/// Rule 5: `"name": "value"` where the name is a secret name.
///
/// Byte indices throughout, and every slice boundary is an ASCII quote or a
/// position already copied from, so a payload holding multi-byte text is cut in
/// the right places rather than panicking on one.
fn redact_json_pairs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'"' {
            at += 1;
            continue;
        }
        let Some(name_end) = quoted_end(bytes, at) else {
            break;
        };
        let name = &text[at + 1..name_end];
        let mut cursor = skip_space(bytes, name_end + 1);
        if cursor >= bytes.len() || bytes[cursor] != b':' || !is_secret_name(name) {
            at = name_end + 1;
            continue;
        }
        cursor = skip_space(bytes, cursor + 1);
        if bytes.get(cursor) == Some(&b'"') {
            if let Some(value_end) = quoted_end(bytes, cursor) {
                out.push_str(&text[copied..=cursor]);
                out.push_str(REDACTED);
                out.push('"');
                copied = value_end + 1;
                at = value_end + 1;
                continue;
            }
        }
        // A non-string value - a number, a null, an object - has nothing shaped
        // like a credential in it to take out.
        at = name_end + 1;
    }
    out.push_str(&text[copied..]);
    out
}

/// The first index at or after `from` that is not ASCII whitespace.
fn skip_space(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// The index of the closing quote of the string starting at `open`.
fn quoted_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut at = open + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return Some(at),
            _ => at += 1,
        }
    }
    None
}

/// Rule 6: `NAME=value` where the name is a secret name.
///
/// The value runs to the closing quote when it is quoted and to the next
/// whitespace when it is not - which is how a shell export, a `.env` line and a
/// process command line all spell it.
fn redact_assignments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut copied = 0;
    while at < bytes.len() {
        if bytes[at] != b'=' {
            at += 1;
            continue;
        }
        let name_start = text[copied..at]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
            .map(|i| copied + i + 1)
            .unwrap_or(copied);
        if name_start == at || !is_secret_name(&text[name_start..at]) {
            at += 1;
            continue;
        }
        let value_start = at + 1;
        let value_end = match bytes.get(value_start) {
            Some(&quote @ (b'"' | b'\'')) => {
                match text[value_start + 1..].find(quote as char) {
                    Some(offset) => value_start + 1 + offset + 1,
                    // Unterminated: to the end, because half a quoted secret is
                    // still a secret.
                    None => bytes.len(),
                }
            }
            // Whitespace ends an unquoted value, and so does a quote or a
            // backslash. That second half is load-bearing rather than tidy:
            // this filter MAY be handed text that sits inside a JSON string,
            // where the character after the value is the string's closing
            // quote. Running to the next space there would swallow the quote
            // and the comma after it and hand the endpoint a document that is
            // not JSON.
            _ => text[value_start..]
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
                .map(|offset| value_start + offset)
                .unwrap_or(bytes.len()),
        };
        if value_end == value_start {
            at += 1;
            continue;
        }
        out.push_str(&text[copied..value_start]);
        out.push_str(REDACTED);
        copied = value_end;
        at = value_end;
    }
    out.push_str(&text[copied..]);
    out
}

/// Rule 7: `--name value` where the flag's name is a secret name.
///
/// Rule 6 scans for `=` and its name run admits `-`, so `--token=x` is already
/// its case; the space-separated spelling is not, and it was measured 5 times
/// over the 60 most recent real transcripts, 21 MB. The value stops at the same
/// quote and backslash bytes rule 6 stops at, for the reason stated there: this
/// text may sit inside a JSON string, and running to the next space would
/// swallow the string's closing quote and the comma after it.
fn redact_flag_values(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        // A flag begins at a `-` that begins a token, so `x--token` is not one.
        if bytes[at] != b'-' || (at > 0 && is_flag_name_byte(bytes[at - 1])) {
            at += 1;
            continue;
        }
        let mut name_end = at;
        while name_end < bytes.len() && is_flag_name_byte(bytes[name_end]) {
            name_end += 1;
        }
        let name = &text[at..name_end];
        // A bare `--` names nothing, and a name run followed by anything but a
        // blank is either rule 6's `--token=x` or not a flag at all.
        if !name.trim_start_matches('-').is_empty()
            && is_secret_name(name)
            && matches!(bytes.get(name_end), Some(b' ' | b'\t'))
        {
            let value_start = skip_blanks(bytes, name_end);
            let value_end = value_run_end(bytes, value_start);
            if value_end > value_start {
                out.push_str(&text[copied..value_start]);
                out.push_str(REDACTED_FLAG_VALUE);
                copied = value_end;
                at = value_end;
                continue;
            }
        }
        at = name_end.max(at + 1);
    }
    out.push_str(&text[copied..]);
    out
}

/// Is this byte part of a flag's name? The same run rule 6 reads a name by.
fn is_flag_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'
}

/// Rule 8: a bare JSON Web Token, with no name beside it to catch it by.
///
/// A JWT says what it is: `eyJ` is `{"` in base64url, so a run opening with it
/// is a serialized JSON header and very little else. Nothing here assumes the
/// surrounding text is a request body - phase 4 runs this same set over recall
/// excerpts and brief windows.
fn redact_jwts(text: &str) -> String {
    redact_runs(text, is_jwt_byte, is_jwt, REDACTED_JWT)
}

/// Is this byte part of a base64url run, dotted segments included?
fn is_jwt_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'
}

fn is_jwt(run: &str) -> bool {
    run.len() >= MIN_JWT_CHARS && run.starts_with("eyJ")
}

/// Rule 9: a GitHub token, keyed on [`GITHUB_TOKEN_PREFIXES`].
///
/// Also nameless in the wild: a token pasted into a turn arrives on its own.
/// Nothing here assumes the surrounding text is a request body.
fn redact_github_tokens(text: &str) -> String {
    redact_runs(
        text,
        is_credential_byte,
        is_github_token,
        REDACTED_GITHUB_TOKEN,
    )
}

/// Is this byte part of an opaque credential run?
fn is_credential_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn is_github_token(run: &str) -> bool {
    GITHUB_TOKEN_PREFIXES
        .iter()
        .any(|prefix| run.starts_with(prefix) && run.len() >= prefix.len() + MIN_GITHUB_TOKEN_BODY)
}

/// Replace every maximal run of `in_run` bytes that `is_secret` accepts.
///
/// The run has to be WHOLE: it begins where a byte that is not part of one
/// ends, so a prefix that turns up mid-word is not a match. `in_run` accepts
/// ASCII only, which is what makes every slice boundary here a character
/// boundary in text holding multi-byte prose.
fn redact_runs(
    text: &str,
    in_run: fn(u8) -> bool,
    is_secret: fn(&str) -> bool,
    marker: &str,
) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        if !in_run(bytes[at]) || (at > 0 && in_run(bytes[at - 1])) {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && in_run(bytes[at]) {
            at += 1;
        }
        if is_secret(&text[start..at]) {
            out.push_str(&text[copied..start]);
            out.push_str(marker);
            copied = at;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Does this name mark its value as a secret?
fn is_secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|fragment| name.contains(fragment))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A turn as `judgment::transcript` writes it: one line, prefixed
    /// `turn_id=<id> <record_type>: `, with the header shape sitting mid-line
    /// behind that prefix.
    #[test]
    fn a_header_shape_mid_line_loses_its_scheme_and_its_value() {
        let turn = "turn_id=42 user: I set Authorization: Bearer sk-PLANTED-1 and it worked";

        let scrubbed = scrub(None, turn);

        assert!(
            !scrubbed.contains("sk-PLANTED-1"),
            "the value survived a mid-line header: {scrubbed}"
        );
        assert!(
            !scrubbed.contains("Bearer"),
            "the scheme word survived without its value: {scrubbed}"
        );
        assert!(
            scrubbed.contains("Authorization: [redacted]"),
            "the header went without saying so: {scrubbed}"
        );
        assert!(
            scrubbed.contains("turn_id=42"),
            "the turn lost the anchor a claim is made against: {scrubbed}"
        );
        assert!(
            scrubbed.contains("and it worked"),
            "the rest of the turn went with the header value: {scrubbed}"
        );
    }

    /// `cookie` is a secret name, and the span taken is bounded: a turn is one
    /// line, so everything after the cookie value has to still be there.
    #[test]
    fn a_cookie_header_takes_its_value_and_leaves_the_rest_of_the_turn() {
        let turn = "turn_id=7 user: Cookie: sid-PLANTED-2; the rest of the turn";

        let scrubbed = scrub(None, turn);

        assert!(
            !scrubbed.contains("sid-PLANTED-2"),
            "the cookie value survived: {scrubbed}"
        );
        assert!(
            scrubbed.contains("the rest of the turn"),
            "the rule ran to end of line and ate the turn: {scrubbed}"
        );
        assert!(
            scrubbed.contains("turn_id=7"),
            "the turn lost its anchor: {scrubbed}"
        );
    }

    /// Neither shape carries a name, so neither is catchable by rules 3, 4 or
    /// 5. Both sentinels sit in plain prose with no `:`, no `=` and no quoted
    /// pair anywhere near them, so the rule under test is the only thing that
    /// can be what caught them.
    #[test]
    fn a_bare_jwt_and_a_bare_github_token_each_go_under_their_own_marker() {
        let prose =
            "the log line read eyJQTEFOVEVEXzMz then stopped, and ghp_abc123XYZ sat beside it";

        let scrubbed = scrub(None, prose);

        assert!(
            !scrubbed.contains("eyJQTEFOVEVEXzMz"),
            "the bare JWT survived: {scrubbed}"
        );
        assert!(
            !scrubbed.contains("ghp_abc123XYZ"),
            "the bare GitHub token survived: {scrubbed}"
        );
        assert!(
            scrubbed.contains(REDACTED_JWT),
            "the JWT went into an unlabelled hole: {scrubbed}"
        );
        assert!(
            scrubbed.contains(REDACTED_GITHUB_TOKEN),
            "the GitHub token went into an unlabelled hole: {scrubbed}"
        );
        assert!(
            scrubbed.contains("sat beside it"),
            "the rules took the prose around the values: {scrubbed}"
        );
        // Both markers are inert against the whole set, which is what lets a
        // test say which rule made a catch.
        assert_eq!(
            scrub(None, &scrubbed),
            scrubbed,
            "a second scrub changed the markers the first one left"
        );
    }

    /// The host has to survive: a reader who cannot see which machine was
    /// reached cannot tell a leak from a typo.
    #[test]
    fn a_connection_url_loses_its_userinfo_and_keeps_its_host() {
        let turn =
            "turn_id=11 user: it connects to postgres://user:pw-PLANTED-3@db.example.invalid/app";

        let scrubbed = scrub(None, turn);

        assert!(
            !scrubbed.contains("pw-PLANTED-3"),
            "the userinfo password survived: {scrubbed}"
        );
        assert!(
            scrubbed.contains("db.example.invalid/app"),
            "the rule took the host with the credential: {scrubbed}"
        );
        assert!(
            scrubbed.contains(REDACTED_URL_USERINFO),
            "the userinfo went into an unlabelled hole: {scrubbed}"
        );
    }

    /// An `@` in a path is not a credential, and a URL with no userinfo at all
    /// is the shape this rule sees most often.
    #[test]
    fn an_at_sign_in_a_url_path_is_not_userinfo() {
        for plain in [
            "https://example.com/path@thing",
            "https://example.invalid/keys",
        ] {
            assert_eq!(scrub(None, plain), plain, "changed: {plain}");
        }
    }

    /// Rule 6 catches `--token=x` already. The space-separated spelling is this
    /// rule's, and what follows the value has to still be there.
    #[test]
    fn a_space_separated_secret_flag_loses_its_value_and_nothing_else() {
        let turn = "turn_id=12 user: gh auth login --token tok-PLANTED-4 --scopes repo";

        let scrubbed = scrub(None, turn);

        assert!(
            !scrubbed.contains("tok-PLANTED-4"),
            "the flag value survived: {scrubbed}"
        );
        assert!(
            scrubbed.contains("--scopes repo"),
            "the rule ran past the value: {scrubbed}"
        );
        assert!(
            scrubbed.contains(REDACTED_FLAG_VALUE),
            "the flag value went into an unlabelled hole: {scrubbed}"
        );
        assert_eq!(
            scrub(None, &scrubbed),
            scrubbed,
            "a second scrub re-consumed the marker the first one left"
        );
    }

    /// The prefix `judgment::transcript` writes is itself a `name: value` shape
    /// (`user: ...`), and it must not be one this rule takes.
    #[test]
    fn an_ordinary_turn_prefix_and_ordinary_prose_come_back_unchanged() {
        for plain in [
            "turn_id=3 assistant: the ratio was 3:1 and the build passed",
            "a line with a colon: and an = sign",
            "{\"model\":\"qwen3:8b\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}",
        ] {
            assert_eq!(scrub(None, plain), plain, "changed: {plain}");
        }
    }
}
