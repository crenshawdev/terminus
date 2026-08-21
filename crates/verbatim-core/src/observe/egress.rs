//! Redaction at egress, keyed on the declared destination (PRIV-03, D-13,
//! D-16).
//!
//! # Two entry points, one rule set
//!
//! [`for_destination`] is the request body's gate. `local = true` hands the
//! text back untouched, because a local provider is not egress at all and
//! filtering it would destroy detail for nothing; anything else - including a
//! `local` key the user forgot to write - hands back a filtered copy.
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
//! 1. **The credential this run resolved**, wherever it appears. The only exact
//!    rule; everything below it is a guess at shape.
//! 2. **PEM private key blocks**, whole.
//! 3. **Header-shaped lines** - a bare `name: value` line whose name matches
//!    [`SECRET_NAMES`]. `Authorization: Bearer ...` is the case this exists for.
//! 4. **JSON string pairs** whose name matches [`SECRET_NAMES`].
//! 5. **Assignment-shaped text** - `NAME=value` whose name matches
//!    [`SECRET_NAMES`].
//!
//! Every one of them leaves a marker naming what went. A payload that came back
//! silently shorter would leave a reader unable to tell filtering from a
//! provider that returned less.
//!
//! The name test is a substring match and therefore over-matches: `monkey`
//! contains `key`. That is the direction to be wrong in. Over-redaction costs a
//! caller some detail in a message; under-redaction costs a key rotation.
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
    // Not in the phase plan's list and here anyway: it is the exact header the
    // one request this phase makes carries its credential in, and matching it
    // by the `bearer` in its value would depend on the scheme's spelling.
    "authorization",
];

/// What replaces the configured credential wherever it is found.
pub const REDACTED_CREDENTIAL: &str = "[redacted: the configured provider credential]";

/// What replaces a PEM private key block.
pub const REDACTED_PRIVATE_KEY: &str = "[redacted: a PEM private key block]";

/// A credential shorter than this is not replaced by rule 1.
///
/// The rule is a plain substring replacement, so a two-character credential
/// would blank two characters out of every word in the payload and leave a
/// reader with rubble. Nothing that short is a credential; if one ever is, the
/// shape rules below still catch it wherever it is labelled.
const MIN_CREDENTIAL_CHARS: usize = 4;

/// Filter a request body for its declared destination (D-13).
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
    let text = redact_header_lines(&text);
    let text = redact_json_pairs(&text);
    redact_assignments(&text)
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

/// Rule 3: a header-shaped line whose name is a secret name.
///
/// "Header-shaped" is deliberately narrow - the name before the colon must be a
/// bare token of letters, digits and hyphens. A pretty-printed JSON line
/// (`  "api_key": "x",`) has quotes in that position and is left to rule 4,
/// which can replace the value without taking the line's trailing comma with
/// it.
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
        match header_name(body) {
            Some(name_end) if is_secret_name(body[..name_end].trim()) => {
                out.push_str(&body[..=name_end]);
                out.push(' ');
                out.push_str(REDACTED);
                out.push_str(ending);
            }
            _ => out.push_str(line),
        }
    }
    out
}

/// The index of the colon, if this line is `token: value`.
fn header_name(line: &str) -> Option<usize> {
    let colon = line.find(':')?;
    let name = line[..colon].trim();
    (!name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then_some(colon)
}

/// Rule 4: `"name": "value"` where the name is a secret name.
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

/// Rule 5: `NAME=value` where the name is a secret name.
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
            _ => text[value_start..]
                .find(char::is_whitespace)
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

/// Does this name mark its value as a secret?
fn is_secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|fragment| name.contains(fragment))
}
