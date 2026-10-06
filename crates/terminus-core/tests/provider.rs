//! The one door to the network, and the one `chat/completions` request that
//! goes through it (OBS-05, PRIV-03, D-05, D-08, D-09, D-20, D-21).
//!
//! The attempt log is process-global, so every test that makes a request holds
//! `NET`: two tests resetting and counting in parallel would each see the
//! other's attempts and both would be measuring nothing.
//!
//! The endpoint is a real `std::net::TcpListener` from `testkit`, not a mock
//! (D-20). Everything asserted below is read off bytes that crossed a socket.

use std::path::{Path, PathBuf};

/// Held by every test that resets or reads the attempt log.
#[cfg(feature = "testkit")]
static NET: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ---------------------------------------------------------------------------
// One client, named in one file

/// The workspace has exactly one HTTP client and exactly one file may name it.
///
/// A source-level assertion rather than a design note, for the reason D-21
/// gives: the strongest privacy claim in the product would otherwise be proven
/// only by a seam that a second client, added anywhere, would walk straight
/// past. This is the `inject_brief.rs` precedent - comment lines stripped the
/// same way, so a doc comment explaining the rule is not itself a violation of
/// it.
#[test]
fn the_http_client_is_named_in_exactly_one_source_file() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let binary = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crates directory")
        .join("terminus")
        .join("src");
    let allowed = core.join("observe").join("net.rs");

    let mut checked = 0;
    let mut naming = Vec::new();
    for root in [&core, &binary] {
        for path in rust_files(root) {
            let source = std::fs::read_to_string(&path).unwrap();
            let code: String = source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            if code.contains("ureq") && path != allowed {
                naming.push(path.display().to_string());
            }
            checked += 1;
        }
    }

    assert!(
        naming.is_empty(),
        "the HTTP client is named outside {}: {naming:?}",
        allowed.display()
    );
    // The falsifying half: a walk that found nothing would pass the assertion
    // above for the wrong reason, and so would one that never reached the
    // binary crate.
    assert!(checked > 30, "only {checked} source files were walked");
    let allowed_source = std::fs::read_to_string(&allowed).unwrap();
    let allowed_code: String = allowed_source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        allowed_code.contains("ureq"),
        "{} does not name the client, so this test is asserting over nothing",
        allowed.display()
    );
}

/// Every `.rs` file at or beneath `root`, sorted.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// The attempt log

/// One request against a real socket, and exactly one entry in the log.
#[test]
#[cfg(feature = "testkit")]
fn one_request_records_exactly_one_attempt() {
    use terminus_core::observe::net;
    use terminus_core::testkit;

    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let stub = testkit::HttpStub::serving(&[testkit::http_response(200, "OK", "{}")]);
    let url = format!("{}chat/completions", stub.base_url());

    net::attempts::reset();
    let response =
        net::post(&url, &[("content-type", "application/json")], b"{}").expect("the stub answered");

    assert_eq!(response.status, 200);
    assert_eq!(response.body, "{}");
    assert_eq!(
        net::attempts::destinations(),
        vec![url],
        "the attempt log did not record exactly this one destination"
    );
    assert_eq!(net::attempts::count(), 1);

    let requests = stub.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /v1/chat/completions "),
        "the stub received: {}",
        requests[0]
    );
}

/// A destination that refuses the connection is still an attempt: PRIV-03 is
/// about what this binary reaches for, not about what it reached.
#[test]
#[cfg(feature = "testkit")]
fn a_connection_that_fails_is_still_a_recorded_attempt() {
    use terminus_core::observe::net;

    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    // Bound, its port read, then dropped: nothing is listening there, and the
    // port was free a moment ago so nothing else is either.
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let url = format!("http://{dead}/chat/completions");

    net::attempts::reset();
    let error = net::post(&url, &[], b"{}").expect_err("nothing is listening there");

    assert_eq!(error.url(), url);
    assert_eq!(net::attempts::destinations(), vec![url]);
}

// ---------------------------------------------------------------------------
// The request (OBS-05, D-05, D-08, D-09)

#[cfg(feature = "testkit")]
mod call {
    use super::NET;

    use terminus_core::config::{Config, Secret, CONFIG_FILE_NAME};
    use terminus_core::observe::provider::{self, Kind, Message, Usage};
    use terminus_core::observe::{egress, net};
    use terminus_core::testkit::{self, HttpStub};

    /// A key value nothing here can produce by accident.
    const KEY: &str = "sk-TERMINUSPROVIDER-3d90fe-do-not-log";
    const MODEL: &str = "qwen3:8b";

    /// A config pointing at `stub`, with the destination declared as `local`.
    fn config(stub: &HttpStub, local: bool) -> (tempfile::TempDir, Config) {
        config_with_format(stub, local, "json_schema")
    }

    fn config_with_format(
        stub: &HttpStub,
        local: bool,
        response_format: &str,
    ) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            format!(
                "[provider]\nenabled = true\nbase_url = \"{}\"\n\
                 model = \"{MODEL}\"\nlocal = {local}\n\
                 response_format = \"{response_format}\"\n",
                stub.base_url()
            ),
        )
        .unwrap();
        let config = Config::load_from(dir.path()).unwrap();
        (dir, config)
    }

    fn assert_withholds(rendered: &str, what: &str) {
        for fragment in [KEY, "TERMINUSPROVIDER", "3d90fe"] {
            assert!(
                !rendered.contains(fragment),
                "{what} carries {fragment:?}: {rendered}"
            );
        }
    }

    /// The body of a request the stub received.
    fn body_of(request: &str) -> &str {
        request.split("\r\n\r\n").nth(1).expect("a request body")
    }

    /// The whole request, in one pass: one connection, the right path, the
    /// model, D-09's `response_format`, and the credential in the header.
    #[test]
    fn one_call_sends_one_request_carrying_the_model_the_format_and_the_key() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion("{\"ok\":true}", 325, 69)]);
        let (_dir, config) = config(&stub, true);

        let held_to = serde_json::json!({ "name": "session_observation", "schema": {} });

        net::attempts::reset();
        let completion = provider::complete(
            &config,
            Some(&Secret::new(KEY)),
            &[Message::user("summarise this session")],
            Some(&held_to),
        )
        .expect("the stub answered");

        assert_eq!(completion.content, "{\"ok\":true}");
        assert_eq!(
            net::attempts::count(),
            1,
            "the call did not go through the counted constructor exactly once: {:?}",
            net::attempts::destinations()
        );

        let requests = stub.requests();
        assert_eq!(requests.len(), 1, "more than one request reached the stub");
        let request = &requests[0];

        let start = request.lines().next().unwrap();
        let path = start.split(' ').nth(1).unwrap_or_default();
        assert!(
            start.starts_with("POST ") && path.ends_with("chat/completions"),
            "the request line is {start:?}"
        );
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case(&format!("authorization: Bearer {KEY}"))),
            "no authorization header carried the resolved credential: {request}"
        );

        let body: serde_json::Value = serde_json::from_str(body_of(request)).unwrap();
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            "session_observation"
        );
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "summarise this session");
    }

    /// D-08: `message` carrying a non-standard `reasoning` key beside `content`
    /// is what the 2026-08-21 ollama probe actually returned, and a parser that
    /// stringified the message object would feed a kilobyte of chain-of-thought
    /// into the caller's JSON parse.
    #[test]
    fn a_message_carrying_reasoning_beside_content_parses_the_content_only() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let canned = serde_json::json!({
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "reasoning": "Okay, the user wants a summary. Let me think about \
                                  what happened in this session step by step...",
                    "content": "{\"decisions\":[]}",
                    "tool_calls": null,
                },
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3 },
        })
        .to_string();
        let stub = HttpStub::serving(&[testkit::http_response(200, "OK", &canned)]);
        let (_dir, config) = config(&stub, true);

        let completion = provider::complete(&config, None, &[Message::user("x")], None)
            .expect("the stub answered");

        assert_eq!(completion.content, "{\"decisions\":[]}");
        assert!(
            !completion.content.contains("Okay, the user wants"),
            "the reasoning key reached the content: {}",
            completion.content
        );
    }

    /// PLAN-3's budget accumulates what comes back here, so what comes back has
    /// to be what the provider said.
    #[test]
    fn the_reported_token_counts_are_the_ones_the_endpoint_returned() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion("{}", 325, 69)]);
        let (_dir, config) = config(&stub, true);

        let completion = provider::complete(&config, None, &[Message::user("x")], None).unwrap();

        assert_eq!(
            completion.usage,
            Some(Usage {
                prompt_tokens: 325,
                completion_tokens: 69,
                total_tokens: 394,
            })
        );
    }

    /// AC4's regression guard, and the one a permissive endpoint cannot be.
    ///
    /// `json_schema` mode is refused by a strict remote endpoint - DeepSeek
    /// answers 400 `missing field json_schema` - when `type` names the mode and
    /// no sibling `json_schema` object carries the schema. ollama and this stub
    /// both accept the truncated form, so nothing that only asserts "the stub
    /// answered" can see the defect. This asserts the BYTES instead.
    #[test]
    fn the_schema_rides_the_request_in_the_field_a_strict_endpoint_reads() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion("{}", 1, 1)]);
        let (_dir, config) = config(&stub, true);
        let held_to = serde_json::json!({
            "name": "session_observation",
            "schema": { "type": "object", "additionalProperties": false },
        });

        provider::complete(&config, None, &[Message::user("x")], Some(&held_to))
            .expect("the stub answered");

        let sent = stub.requests().remove(0);
        let body: serde_json::Value =
            serde_json::from_str(body_of(&sent)).expect("the request body is JSON");
        let format = &body["response_format"];

        assert_eq!(format["type"], "json_schema");
        assert_eq!(format["json_schema"]["name"], "session_observation");
        assert_eq!(format["json_schema"]["schema"], held_to["schema"]);
        // `strict` belongs INSIDE `json_schema`, not beside `type`, which is
        // where it sat while the schema was travelling in the prompt alone.
        assert_eq!(format["json_schema"]["strict"], true);
        assert!(format.get("strict").is_none());
    }

    /// The DeepSeek case: `response_format = "json_object"` asks for the mode
    /// that endpoint implements, over the same code path and the same body.
    /// The schema is not sent - `json_object` has nowhere to put one - and the
    /// instruction turn stays the thing that names the shape.
    #[test]
    fn the_json_object_mode_sends_the_shape_deepseek_accepts() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion("{}", 1, 1)]);
        let (_dir, config) = config_with_format(&stub, true, "json_object");
        let held_to = serde_json::json!({ "name": "session_observation", "schema": {} });

        provider::complete(&config, None, &[Message::user("x")], Some(&held_to))
            .expect("the stub answered");

        let sent = stub.requests().remove(0);
        let body: serde_json::Value =
            serde_json::from_str(body_of(&sent)).expect("the request body is JSON");

        assert_eq!(body["response_format"]["type"], "json_object");
        assert!(
            body["response_format"].get("json_schema").is_none(),
            "json_object mode carries no schema: {}",
            body["response_format"]
        );
    }

    /// `response_format = "none"` sends the key at all, for an endpoint that
    /// rejects it outright. A schema in hand does not override the setting.
    #[test]
    fn the_none_mode_sends_no_response_format_even_with_a_schema() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion("{}", 1, 1)]);
        let (_dir, config) = config_with_format(&stub, true, "none");
        let held_to = serde_json::json!({ "name": "session_observation", "schema": {} });

        provider::complete(&config, None, &[Message::user("x")], Some(&held_to))
            .expect("the stub answered");

        let sent = stub.requests().remove(0);
        let body: serde_json::Value =
            serde_json::from_str(body_of(&sent)).expect("the request body is JSON");

        assert!(body.get("response_format").is_none());
    }

    /// The other half: no schema means no `response_format` at all, rather than
    /// the half of one that got AC4 refused. An endpoint that enforces the field
    /// has nothing to reject when the mode is never named.
    #[test]
    fn a_request_with_no_schema_names_no_response_format() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion("{}", 1, 1)]);
        let (_dir, config) = config(&stub, true);

        provider::complete(&config, None, &[Message::user("x")], None).expect("the stub answered");

        let sent = stub.requests().remove(0);
        let body: serde_json::Value =
            serde_json::from_str(body_of(&sent)).expect("the request body is JSON");

        assert!(body.get("response_format").is_none());
    }

    /// A `usage` object the provider left out entirely is `None` and not zeros:
    /// a budget accumulating zeros would never be reached (D-11, D-12).
    #[test]
    fn a_response_with_no_usage_object_reports_none_rather_than_zeros() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let canned = serde_json::json!({
            "choices": [{ "message": { "content": "{}" } }],
        })
        .to_string();
        let stub = HttpStub::serving(&[testkit::http_response(200, "OK", &canned)]);
        let (_dir, config) = config(&stub, true);

        let completion = provider::complete(&config, None, &[Message::user("x")], None).unwrap();

        assert_eq!(completion.usage, None);
    }

    /// D-16's case: a 401 whose body echoes the key back. `runs.error` is free
    /// text and `terminus status` prints it, so this one is durable.
    #[test]
    fn a_401_echoing_the_key_comes_back_as_an_error_with_no_byte_of_it() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let canned = serde_json::json!({
            "error": {
                "message": format!("Incorrect API key provided: {KEY}"),
                "code": "invalid_api_key",
            },
        })
        .to_string();
        // The falsifying half: the body really does carry the key, so the
        // assertions below are about the scrubber and not about an empty body.
        assert!(canned.contains(KEY));
        let stub = HttpStub::serving(&[testkit::http_response(401, "Unauthorized", &canned)]);
        let (_dir, config) = config(&stub, false);

        let error = provider::complete(
            &config,
            Some(&Secret::new(KEY)),
            &[Message::user("x")],
            None,
        )
        .expect_err("a 401 is not a completion");

        assert_eq!(error.kind(), Kind::Status(401));
        assert_eq!(error.status(), Some(401));
        assert_withholds(&error.to_string(), "the error");
        assert_withholds(error.detail(), "the error detail");
        assert_withholds(&format!("{error:?}"), "the debug-formatted error");
        assert!(
            error.to_string().contains("invalid_api_key"),
            "the scrubber took the reason with the key: {error}"
        );
    }

    /// A 2xx that is not a chat completion is this module's own error, not a
    /// panic and not a silently empty answer.
    #[test]
    fn a_200_that_is_not_a_chat_completion_is_a_malformed_error() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[
            testkit::http_response(200, "OK", "not json at all"),
            testkit::http_response(200, "OK", "{\"choices\":[]}"),
        ]);
        let (_dir, config) = config(&stub, true);

        for _ in 0..2 {
            let error = provider::complete(&config, None, &[Message::user("x")], None)
                .expect_err("neither body is a chat completion");
            assert_eq!(error.kind(), Kind::Malformed);
        }
    }

    /// A dead endpoint is a transport failure, and the attempt is still logged.
    #[test]
    fn an_endpoint_that_is_not_there_is_a_transport_error() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let dead = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            format!(
                "[provider]\nenabled = true\nbase_url = \"http://{dead}/v1/\"\n\
                 model = \"{MODEL}\"\n"
            ),
        )
        .unwrap();
        let config = Config::load_from(dir.path()).unwrap();

        net::attempts::reset();
        let error = provider::complete(&config, None, &[Message::user("x")], None)
            .expect_err("nothing is listening there");

        assert_eq!(error.kind(), Kind::Transport);
        assert_eq!(net::attempts::count(), 1);
    }

    /// A config that asks for judgment and names no endpoint builds no request:
    /// the attempt log stays empty, which is the only way "nothing was called"
    /// is provable.
    ///
    /// Enabled deliberately, so this is `NotConfigured` and not `Disabled`: the
    /// user said yes and left something out, which is the case worth a note.
    #[test]
    fn a_config_naming_no_endpoint_makes_no_request() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[provider]\nenabled = true\n",
        )
        .unwrap();
        let config = Config::load_from(dir.path()).unwrap();

        net::attempts::reset();
        let error = provider::complete(&config, None, &[Message::user("x")], None)
            .expect_err("there is nothing to call");

        assert_eq!(error.kind(), Kind::NotConfigured);
        assert_eq!(net::attempts::count(), 0);

        // And the default config, which asks for nothing at all, is the other
        // kind: silence rather than a note.
        assert_eq!(
            provider::complete(&Config::default(), None, &[Message::user("x")], None)
                .expect_err("there is nothing to call")
                .kind(),
            Kind::Disabled
        );
        assert_eq!(net::attempts::count(), 0);
    }

    /// OBS-02: with judgment off, this plan's code builds no request. Proven
    /// off the attempt log and off a live stub that received nothing, rather
    /// than off the absence of a caller.
    #[test]
    fn a_provider_that_is_not_enabled_builds_no_request() {
        let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
        let stub = HttpStub::serving(&[testkit::chat_completion("{}", 1, 1)]);
        // Everything a call needs except the one key that asks for it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            format!(
                "[provider]\nbase_url = \"{}\"\nmodel = \"{MODEL}\"\n",
                stub.base_url()
            ),
        )
        .unwrap();
        let config = Config::load_from(dir.path()).unwrap();

        net::attempts::reset();
        let error = provider::complete(
            &config,
            Some(&Secret::new(KEY)),
            &[Message::user("x")],
            None,
        )
        .expect_err("judgment is off");

        assert_eq!(error.kind(), Kind::Disabled);
        assert_eq!(
            net::attempts::count(),
            0,
            "a disabled provider reached the network: {:?}",
            net::attempts::destinations()
        );
        assert!(
            stub.requests().is_empty(),
            "a disabled provider sent something to the endpoint"
        );
    }

    // -----------------------------------------------------------------------
    // The declared destination, on the wire (D-13)

    /// A secret in the session text reaches a remote endpoint filtered and a
    /// local one whole, and the declared key is the only difference.
    #[test]
    fn the_body_on_the_wire_is_filtered_for_remote_and_whole_for_local() {
        // Two secrets in one line, on purpose. The first is the resolved
        // credential, caught by name. The second is a shape rule's only
        // catch AND it ends the string, which is where a filter that ran to
        // the next space would swallow the JSON closing quote with it.
        let leaky =
            format!("a tool result held OPENAI_API_KEY={KEY} and GITHUB_TOKEN=ghp_abc123XYZ");

        for (local, expect_present) in [(true, true), (false, false)] {
            let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
            let stub = HttpStub::serving(&[testkit::chat_completion("{}", 1, 1)]);
            let (_dir, config) = config(&stub, local);

            provider::complete(
                &config,
                Some(&Secret::new(KEY)),
                &[Message::user(leaky.clone())],
                None,
            )
            .expect("the stub answered");
            let sent = stub.requests().remove(0);
            let body = body_of(&sent).to_owned();

            assert_eq!(
                body.contains(KEY),
                expect_present,
                "local = {local} sent the wrong thing: {body}"
            );
            assert_eq!(
                body.contains("ghp_abc123XYZ"),
                expect_present,
                "local = {local} sent the wrong thing: {body}"
            );
            if !expect_present {
                assert!(
                    body.contains(egress::REDACTED_CREDENTIAL) || body.contains("[redacted]"),
                    "the body was filtered without saying so: {body}"
                );
                assert!(
                    !body.contains("ghp_abc123XYZ"),
                    "the second secret survived: {body}"
                );
                // Still a document the endpoint can read: the filter must not
                // take a closing quote or a comma with the value.
                serde_json::from_str::<serde_json::Value>(&body)
                    .unwrap_or_else(|e| panic!("the filtered body is not JSON ({e}): {body}"));
            }
        }
    }
}
