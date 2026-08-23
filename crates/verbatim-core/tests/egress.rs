//! The destination-keyed egress filter and the error scrubber (PRIV-03, D-13,
//! D-16).
//!
//! The two halves under test are separate claims. The FILTER is conditional on
//! a declared destination and must be a byte-for-byte no-op when the provider
//! is local, because a local call is not egress and filtering it would throw
//! away detail for nothing. The SCRUBBER is unconditional, because its output
//! goes to `runs.error` and to stderr - both local, both durable - whatever the
//! provider was.

use std::path::Path;

use verbatim_core::config::{Config, Secret, REDACTED};
use verbatim_core::observe::egress;

/// A key value nothing here can produce by accident.
const KEY: &str = "sk-VERBATIMEGRESS-71c4a8-do-not-log";

/// A body carrying a secret in each of the two shapes the plan names.
fn body() -> String {
    format!(
        "POST /v1/chat/completions HTTP/1.1\r\n\
         Authorization: Bearer {KEY}\r\n\
         Content-Type: application/json\r\n\
         \r\n\
         export OPENAI_API_KEY={KEY}\n"
    )
}

fn assert_withholds(rendered: &str, what: &str) {
    for fragment in [KEY, "VERBATIMEGRESS", "71c4a8"] {
        assert!(
            !rendered.contains(fragment),
            "{what} carries {fragment:?}: {rendered}"
        );
    }
}

// ---------------------------------------------------------------------------
// The declared destination (D-13)

/// `local = true` is the one thing that sends the bytes as they were built.
#[test]
fn a_local_destination_sends_the_body_byte_identical() {
    let body = body();
    let sent = egress::for_destination(true, Some(&Secret::new(KEY)), &body);

    assert_eq!(
        sent, body,
        "a local provider is not egress and is not filtered"
    );
    assert!(
        matches!(sent, std::borrow::Cow::Borrowed(_)),
        "the local path copied the body, so it is not provably the same bytes"
    );
}

/// `local = false` and `local` absent are the same thing, which is the whole of
/// what makes a forgotten key fail safe.
#[test]
fn a_remote_or_undeclared_destination_sends_neither_secret() {
    let body = body();
    // Both arms, and the second one read off a config with no `local` key
    // rather than spelled `false` here: that is the claim - a forgotten
    // declaration reaches this function as the filtered arm (D-13).
    for local in [false, Config::default().provider_local()] {
        let sent = egress::for_destination(local, Some(&Secret::new(KEY)), &body);
        assert_withholds(&sent, "the filtered body");
        assert!(
            sent.contains(REDACTED) || sent.contains(egress::REDACTED_CREDENTIAL),
            "the body was filtered without saying so: {sent}"
        );
        assert!(
            sent.contains("chat/completions"),
            "the filter took the request line with it: {sent}"
        );
        assert!(
            sent.contains("Content-Type: application/json"),
            "the filter took an ordinary header with it: {sent}"
        );
    }
}

/// Both shapes go even when the loader resolved nothing, so the filter does not
/// depend on already knowing the value.
#[test]
fn the_shape_rules_hold_without_a_resolved_credential() {
    let body = body();
    let sent = egress::for_destination(false, None, &body);

    assert_withholds(&sent, "the filtered body");
    assert!(
        sent.contains("Authorization: [redacted]"),
        "the authorization header survived: {sent}"
    );
    assert!(
        sent.contains("OPENAI_API_KEY=[redacted]"),
        "the assignment survived: {sent}"
    );
}

// ---------------------------------------------------------------------------
// The error boundary (D-16)

/// A 401 whose body echoes the key back is exactly the case `runs.error` would
/// otherwise store forever.
#[test]
fn the_scrubber_takes_the_credential_out_of_a_401_body() {
    let credential = Secret::new(KEY);
    let four_oh_one = format!(
        "{{\"error\":{{\"message\":\"Incorrect API key provided: {KEY}. \
         You can find your API key at https://example.invalid/keys\",\
         \"code\":\"invalid_api_key\"}}}}"
    );

    let scrubbed = egress::scrub(Some(&credential), &four_oh_one);

    assert_withholds(&scrubbed, "the scrubbed 401");
    assert!(
        scrubbed.contains(egress::REDACTED_CREDENTIAL),
        "the scrubber removed the key without saying it had: {scrubbed}"
    );
    assert!(
        scrubbed.contains("invalid_api_key"),
        "the scrubber removed the reason along with the key: {scrubbed}"
    );
}

/// The scrubber does not care about the destination: the same text goes the
/// same way whether the provider was local or not, because its output is stored
/// locally either way.
#[test]
fn the_scrubber_is_not_keyed_on_the_destination() {
    let credential = Secret::new(KEY);
    let text = format!("connect failed while sending {KEY}");

    assert_eq!(
        egress::scrub(Some(&credential), &text),
        egress::scrub(Some(&credential), &text)
    );
    assert_withholds(
        &egress::scrub(Some(&credential), &text),
        "the scrubbed text",
    );
}

// ---------------------------------------------------------------------------
// The rule set

/// A JSON pair keeps its structure: the name and the punctuation stay so the
/// reader can still see the shape of what was sent.
#[test]
fn a_json_pair_loses_its_value_and_keeps_its_name() {
    let scrubbed = egress::scrub(None, "{\"model\":\"qwen3:8b\",\"api_key\":\"sk-abc123\"}");

    assert_eq!(
        scrubbed,
        format!("{{\"model\":\"qwen3:8b\",\"api_key\":\"{REDACTED}\"}}")
    );
}

/// A pretty-printed pair is the case the header rule must not eat: it would
/// take the trailing comma and leave a document that is not JSON at all.
#[test]
fn a_pretty_printed_json_pair_keeps_its_punctuation() {
    let scrubbed = egress::scrub(None, "{\n  \"api_key\": \"sk-abc123\",\n  \"n\": 1\n}");

    assert_eq!(
        scrubbed,
        format!("{{\n  \"api_key\": \"{REDACTED}\",\n  \"n\": 1\n}}")
    );
}

/// A PEM private key goes whole; a certificate beside it does not, because a
/// certificate is public and a reader may need it.
#[test]
fn a_pem_private_key_goes_and_a_certificate_stays() {
    let text = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n\
                -----BEGIN RSA PRIVATE KEY-----\nSECRETLINES\n-----END RSA PRIVATE KEY-----\n\
                after\n";

    let scrubbed = egress::scrub(None, text);

    assert!(
        !scrubbed.contains("SECRETLINES"),
        "the private key survived: {scrubbed}"
    );
    assert!(
        scrubbed.contains(egress::REDACTED_PRIVATE_KEY),
        "the private key went silently: {scrubbed}"
    );
    assert!(
        scrubbed.contains("-----BEGIN CERTIFICATE-----"),
        "the certificate went too: {scrubbed}"
    );
    assert!(
        scrubbed.contains("after"),
        "everything after the block went with it: {scrubbed}"
    );
}

/// Nothing here changes text that carries no secret. A filter that mangled an
/// ordinary payload would make the remote path lie about what was sent.
#[test]
fn text_with_nothing_secret_in_it_comes_back_unchanged() {
    let plain = "{\"model\":\"qwen3:8b\",\"messages\":[{\"role\":\"user\",\
                 \"content\":\"summarise session 3 = the one about sprockets\"}]}\n\
                 a line with a colon: and an = sign\n";

    assert_eq!(egress::scrub(None, plain), plain);
}

/// A multi-byte payload is cut at character boundaries or not at all.
#[test]
fn a_payload_holding_multibyte_text_survives_the_scan() {
    let text = "{\"note\":\"café — naïve ☕\",\"api_key\":\"sk-abc\"}";

    let scrubbed = egress::scrub(None, text);

    assert!(scrubbed.contains("café — naïve ☕"));
    assert!(!scrubbed.contains("sk-abc"));
}

// ---------------------------------------------------------------------------
// Egress only, never ingest

/// `.planning/PROJECT.md` bars ingest-time redaction outright: it makes the
/// store lossy and destroys what it strips. The ingest write path must
/// therefore not be able to reach this module at all.
#[test]
fn nothing_in_the_ingest_path_names_the_egress_filter() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ingest");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("read src/ingest") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for banned in ["egress", "scrub", "redact"] {
            assert!(!code.contains(banned), "{} names {banned}", path.display());
        }
        checked += 1;
    }
    assert!(checked >= 3, "only {checked} files in {}", dir.display());
}
