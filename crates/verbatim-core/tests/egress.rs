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

// ---------------------------------------------------------------------------
// What is actually sent (PRIV-01, PRIV-03, D-01, D-14, D-15, D-16)

/// The bytes `provider::complete` hands `net::post`, read off a loopback stub.
///
/// Everything above this line is the filter answering about a string a test
/// wrote. Everything below it is the filter answering about the request the
/// product really builds: a secret planted in a transcript turn, ingested
/// through the real `ingest::run`, projected by the real `text::project`, and
/// carried into a real `judgment::judge` call whose socket a stub is on the
/// other end of. That distinction is the whole of this phase - the whole-body
/// call these tests replaced passed on a sample and did nothing on the wire.
///
/// Gated per item and NOT with a file-level `#![cfg(feature = "testkit")]`
/// (D-16): a file-level gate makes `cargo test --test egress` compile an empty
/// binary and report green, which would retire
/// `nothing_in_the_ingest_path_names_the_egress_filter` silently.
#[cfg(feature = "testkit")]
mod wire {
    use std::path::PathBuf;

    use rusqlite::Connection;
    use verbatim_core::config::{Config, CONFIG_FILE_NAME, REDACTED};
    use verbatim_core::observe::judgment::{self, Verdict};
    use verbatim_core::observe::{cost, egress};
    use verbatim_core::store::DB_FILE_NAME;
    use verbatim_core::testkit::{self, HttpStub};
    use verbatim_core::{ingest, observe};

    /// The transcript every test here sends: one turn per credential shape.
    const FIXTURE: &str = "session-secrets.jsonl";

    /// What the model is told it is, so the stub's one answer is enough.
    const MODEL: &str = "wire-stub";

    /// The nine planted sentinels, each with the marker of the rule that is
    /// the only thing able to catch it, and a name for the failure message.
    ///
    /// One table rather than nine tests: the claim is about the SET - every
    /// shape gone, each one accounted for by a named rule - and a table makes a
    /// shape that quietly stopped being planted impossible to miss.
    ///
    /// The last two are the DOUBLE-QUOTED spellings of the first and fourth.
    /// They are separate rows rather than a rewrite of those rows because they
    /// were caught by nothing until 2026-08-30: rule 4 handed a quoted name to
    /// rule 5, which only fires once the quoted run closes before the colon.
    const PLANTED: &[(&str, &str, &str)] = &[
        (
            "an Authorization: Bearer header",
            "sk-VBEGRESS-authz-9f2",
            REDACTED,
        ),
        ("a JSON \"password\" pair", "pw-VBEGRESS-mash-4d1", REDACTED),
        (
            "a space-separated --token flag",
            "tk-VBEGRESS-flagv-7c3",
            egress::REDACTED_FLAG_VALUE,
        ),
        ("a Cookie header", "sid-VBEGRESS-crumb-2b8", REDACTED),
        (
            "a bare JWT",
            "eyJhbGciOiJIUzI1NiJ9.VBEGRESSjwt3e7",
            egress::REDACTED_JWT,
        ),
        (
            "a GitHub token",
            "ghp_VBEGRESSgh0zq",
            egress::REDACTED_GITHUB_TOKEN,
        ),
        (
            "a connection URL's userinfo",
            "dsn-VBEGRESS-pg-8a4",
            egress::REDACTED_URL_USERINFO,
        ),
        (
            "a double-quoted Authorization header, the curl spelling",
            "sk-VBEGRESS-curlz-5e6",
            REDACTED,
        ),
        (
            "a double-quoted Cookie header, the curl spelling",
            "sid-VBEGRESS-qcrumb-1f9",
            REDACTED,
        ),
    ];

    /// The `Cookie` turn's prose, which follows the value on the SAME line.
    const AFTER_THE_COOKIE: &str = "the retry after that answered 204 with an empty body";

    /// One archived, observed session, ready to be judged.
    struct Wire {
        _dir: tempfile::TempDir,
        data_dir: PathBuf,
        config_dir: PathBuf,
        session_key: String,
    }

    /// Archive [`FIXTURE`] through the real ingest path, close it, and give it
    /// the mechanical row the judgment half fills in.
    ///
    /// The idle rule is short-circuited the way `tests/judgment.rs` does it, so
    /// the fixture's own old timestamps do not have to be rewritten.
    fn wire() -> Wire {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let work = dir.path().join("work");
        let config_dir = dir.path().join("config");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();

        let path = testkit::copy_rooted_fixture_into(FIXTURE, &work, &root);
        match ingest::run(&data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{FIXTURE}: {other:?}"),
        }
        let session_key = path.canonicalize().unwrap().to_string_lossy().into_owned();

        let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
        conn.execute("UPDATE session_meta SET is_final = 1", [])
            .unwrap();
        let observed = observe::observe_new(&conn, &Config::default());
        assert_eq!(observed.written, 1, "{observed:?}");

        Wire {
            _dir: dir,
            data_dir,
            config_dir,
            session_key,
        }
    }

    impl Wire {
        fn conn(&self) -> Connection {
            Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
        }

        /// Every `turns.id` of the session, which is what a `turn_id=` anchor
        /// in the prompt has to be one of.
        fn turn_ids(&self) -> Vec<i64> {
            self.conn()
                .prepare("SELECT id FROM turns WHERE session_key = ?1 ORDER BY turn_seq")
                .unwrap()
                .query_map([self.session_key.as_str()], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        }

        /// A config pointing judgment at `stub`, differing ONLY in the `local`
        /// key.
        ///
        /// `None` writes no `local` line at all, which is the arm a forgotten
        /// declaration reaches and the one AC1 is about. It is deliberately not
        /// spelled `false` here: what has to be true is that the absence itself
        /// filters.
        fn config(&self, stub: &HttpStub, local: Option<bool>) -> Config {
            let declaration = match local {
                Some(value) => format!("local = {value}\n"),
                None => String::new(),
            };
            std::fs::write(
                self.config_dir.join(CONFIG_FILE_NAME),
                format!(
                    "[provider]\nenabled = true\nbase_url = \"{}\"\n\
                     model = \"{MODEL}\"\n{declaration}",
                    stub.base_url()
                ),
            )
            .unwrap();
            Config::load_from(&self.config_dir).unwrap()
        }

        /// Drive one real judgment call and answer with the body the stub
        /// recorded.
        ///
        /// No body-building helper is exported for this (D-14): a body the
        /// product does not build is the same failure this phase exists to fix.
        fn recorded_body(&self, local: Option<bool>, again: bool) -> String {
            let stub = HttpStub::serving(&[testkit::chat_completion(&answer(), 1, 1)]);
            let config = self.config(&stub, local);
            let conn = self.conn();
            let verdict = if again {
                judgment::judge_again(&conn, &config, None, &self.session_key)
            } else {
                judgment::judge(&conn, &config, None, &self.session_key)
            };
            assert!(
                matches!(verdict, Verdict::Stored { .. }),
                "the call the body is read off did not happen: {verdict:?}"
            );
            let sent = stub.requests().remove(0);
            sent.split("\r\n\r\n")
                .nth(1)
                .expect("a request body")
                .to_owned()
        }

        /// Both bodies from ONE store, so the two differ in the `local` key and
        /// in nothing else - not in the turn ids the prompt is written around.
        fn both_bodies(&self) -> (String, String) {
            let filtered = self.recorded_body(None, false);
            let whole = self.recorded_body(Some(true), true);
            (filtered, whole)
        }
    }

    /// A valid judgment with no claims in it: three empty arrays anchor to
    /// nothing, so one request is enough and the stub is never asked for a
    /// retry it has no response for.
    fn answer() -> String {
        serde_json::json!({
            "topic": "a sync run that pasted its credentials into the transcript",
            "outcome": judgment::OUTCOMES[0],
            "decisions": [],
            "learned": [],
            "unresolved": [],
        })
        .to_string()
    }

    /// The top-level object's keys, in the order the body TEXT states them.
    ///
    /// Read off the text and not off a parsed map on purpose: `serde_json`'s
    /// map here is a `BTreeMap`, so parsing both sides would sort them into
    /// agreement and the assertion would hold whatever the bodies said.
    fn top_level_keys(body: &str) -> Vec<String> {
        let bytes = body.as_bytes();
        let mut keys = Vec::new();
        let mut depth = 0usize;
        let mut at = 0usize;
        while at < bytes.len() {
            match bytes[at] {
                b'{' | b'[' => {
                    depth += 1;
                    at += 1;
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    at += 1;
                }
                b'"' => {
                    let start = at + 1;
                    let mut end = start;
                    while end < bytes.len() && bytes[end] != b'"' {
                        end += if bytes[end] == b'\\' { 2 } else { 1 };
                    }
                    if depth == 1 && bytes.get(end + 1) == Some(&b':') {
                        keys.push(body[start..end].to_owned());
                    }
                    at = end + 1;
                }
                _ => at += 1,
            }
        }
        keys
    }

    /// The `messages` array's `role` values, in order.
    fn roles(body: &serde_json::Value) -> Vec<String> {
        body["messages"]
            .as_array()
            .expect("a messages array")
            .iter()
            .map(|m| m["role"].as_str().expect("a role").to_owned())
            .collect()
    }

    // -----------------------------------------------------------------------
    // AC1: the secret does not leave

    /// The claim the phase exists for: a secret typed into a session does not
    /// reach a remote endpoint, and the undeclared destination is remote.
    ///
    /// The shape is chosen so the *placement* is what is under test. A JSON
    /// `"password"` pair is invisible to the assignment rule and invisible to
    /// the whole-body scan this replaced - in a serialized body its quotes are
    /// `\"` and the pair rule walks straight past them - so the only reason it
    /// can be gone here is that the filter ran on the content string.
    #[test]
    fn a_secret_in_an_ingested_turn_does_not_reach_an_undeclared_destination() {
        let wire = wire();
        let (filtered, whole) = wire.both_bodies();

        assert!(
            !filtered.contains("pw-VBEGRESS-mash-4d1"),
            "the planted password went out to an undeclared destination: {filtered}"
        );
        assert!(
            !filtered.contains("sk-VBEGRESS-authz-9f2"),
            "the planted Authorization value went out: {filtered}"
        );
        // The falsifying half: the same call with the destination declared
        // local sends both, so the assertion above is about the filter and not
        // about a fixture that failed to plant anything.
        assert!(
            whole.contains("pw-VBEGRESS-mash-4d1") && whole.contains("sk-VBEGRESS-authz-9f2"),
            "the fixture planted nothing, so nothing was proven: {whole}"
        );
    }

    // -----------------------------------------------------------------------
    // AC3: still the same document

    /// Filtering changes values and nothing else about the request.
    ///
    /// A filter that came back with a document the endpoint cannot read, or one
    /// message short, would fail the remote path in a way no local run could
    /// ever show.
    #[test]
    fn the_filtered_body_is_the_same_document_with_different_values() {
        let wire = wire();
        let (filtered, whole) = wire.both_bodies();

        let parsed: serde_json::Value = serde_json::from_str(&filtered)
            .unwrap_or_else(|e| panic!("the filtered body is not JSON ({e}): {filtered}"));
        let unfiltered: serde_json::Value = serde_json::from_str(&whole).unwrap();

        assert_eq!(
            parsed["messages"].as_array().map(Vec::len),
            unfiltered["messages"].as_array().map(Vec::len),
            "the filter changed how many messages were sent"
        );
        assert_eq!(
            roles(&parsed),
            roles(&unfiltered),
            "the filter changed the roles or their order"
        );

        let keys = top_level_keys(&filtered);
        assert!(
            keys.contains(&"messages".to_owned()) && keys.contains(&"model".to_owned()),
            "the key reader found no request shape at all: {keys:?}"
        );
        assert_eq!(
            keys,
            top_level_keys(&whole),
            "the filter changed the top-level key sequence"
        );
    }

    /// D-08: the instruction turn goes through the filter too, and today's
    /// instructions come out of it byte for byte.
    ///
    /// It is redacted rather than skipped because a prompt is text like any
    /// other and an exemption would be a hole. This is the test that says a
    /// later prompt edit introducing a matching word - a `--key` example, a
    /// `name: value` line - changed what the model was asked, and says it at
    /// the moment the edit lands rather than after a model starts answering the
    /// wrong schema.
    #[test]
    fn the_instruction_turn_survives_the_filter_byte_for_byte() {
        let wire = wire();
        let (filtered, whole) = wire.both_bodies();

        let parsed: serde_json::Value = serde_json::from_str(&filtered).unwrap();
        let unfiltered: serde_json::Value = serde_json::from_str(&whole).unwrap();

        let system = parsed["messages"][0]["content"]
            .as_str()
            .expect("a system turn");
        assert_eq!(
            parsed["messages"][0]["role"], "system",
            "the first message is not the instruction turn"
        );
        assert_eq!(
            system,
            unfiltered["messages"][0]["content"]
                .as_str()
                .expect("a system turn"),
            "the filter changed the instructions"
        );
        assert!(
            system.contains("turn_id"),
            "the instructions are not the ones under test: {system}"
        );
    }

    // -----------------------------------------------------------------------
    // AC6's other half: the fixture is really what is being judged

    /// The premise every assertion above rests on: the session reached the
    /// provider at all, rather than being skipped for being too short.
    #[test]
    fn the_fixture_carries_enough_turns_to_be_judged() {
        let wire = wire();
        let ids = wire.turn_ids();

        assert!(
            ids.len() >= cost::MIN_TURNS,
            "{FIXTURE} has {} turns, fewer than the {} a judgment is bought for",
            ids.len(),
            cost::MIN_TURNS
        );
    }

    // -----------------------------------------------------------------------
    // AC2 and AC5: every shape, and what is left standing

    /// Each of the seven planted shapes is gone, and the rule that took it said
    /// so by name.
    ///
    /// Four of the seven sit in the fixture with no name-keyed text beside
    /// them, which is what makes this an assertion about a rule rather than
    /// about the assignment rule catching a `NAME=` that happened to be
    /// nearby.
    #[test]
    fn every_planted_shape_is_gone_and_its_rule_named_itself() {
        let wire = wire();
        let (filtered, whole) = wire.both_bodies();

        for (what, sentinel, marker) in PLANTED {
            assert!(
                !filtered.contains(sentinel),
                "{what} went out on the wire ({sentinel}): {filtered}"
            );
            assert!(
                filtered.contains(marker),
                "{what} went without {marker}, so a reader cannot tell what was taken: {filtered}"
            );
            // Loudly, rather than as a pass: a shape the fixture stopped
            // planting would satisfy the absence above for the wrong reason.
            assert!(
                whole.contains(sentinel),
                "{what} is not planted in {FIXTURE} ({sentinel}), so its absence proves nothing"
            );
        }
    }

    /// AC5: a bounded match. The `Cookie` value goes and the rest of that turn
    /// does not.
    ///
    /// The anchor is the reason this matters. A rule running to end of line
    /// would erase the rest of the turn while leaving its `turn_id=` standing,
    /// so the model would be shown a fragment and could still hang a claim on
    /// it - a claim the store would accept, because the id is real.
    #[test]
    fn a_mid_line_cookie_takes_its_value_and_leaves_the_turn_standing() {
        let wire = wire();
        let filtered = wire.recorded_body(None, false);
        let shown: serde_json::Value = serde_json::from_str(&filtered).unwrap();
        let shown = shown["messages"][1]["content"]
            .as_str()
            .expect("a user turn");

        assert!(
            !shown.contains("sid-VBEGRESS-crumb-2b8"),
            "the cookie went out: {shown}"
        );
        assert!(
            shown.contains(AFTER_THE_COOKIE),
            "the match ran past the value and took the rest of the turn: {shown}"
        );
        // Read off the store rather than hardcoded: an anchor a test invented
        // is not the anchor the model was given.
        let anchors: Vec<String> = wire
            .turn_ids()
            .iter()
            .map(|id| format!("turn_id={id}"))
            .collect();
        assert!(
            anchors.len() >= 6,
            "too few anchors to assert on: {anchors:?}"
        );
        for anchor in &anchors {
            assert!(
                shown.contains(anchor.as_str()),
                "{anchor} was not shown to the model: {shown}"
            );
        }
    }
}
