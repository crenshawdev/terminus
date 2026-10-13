---
phase: 1
plan: 2
requirements: [PRIV-05]
files:
  - Cargo.toml
  - crates/terminus-core/Cargo.toml
  - Cargo.lock
  - crates/terminus-core/src/testkit.rs
  - crates/terminus-core/tests/no_network.rs
  - crates/terminus/src/cmd/doctor.rs
  - crates/terminus/tests/doctor.rs
  - crates/terminus-core/src/credentials.rs
  - crates/terminus-core/src/lib.rs
  - crates/terminus-core/tests/credentials.rs
  - crates/terminus-core/src/observe/egress.rs
  - crates/terminus-core/src/observe/mod.rs
  - crates/terminus-core/tests/egress.rs
  - crates/terminus-core/src/recall/excerpt.rs
  - crates/terminus-core/src/recall/context.rs
  - crates/terminus-core/src/recall/search.rs
  - crates/terminus-core/src/inject/brief.rs
  - crates/terminus-core/src/config.rs
  - crates/terminus-core/src/error.rs
  - crates/terminus-core/tests/config.rs
  - crates/terminus/tests/status.rs
---

# Phase 1: No Model Calls - Plan 2

## Goal

Terminus never calls a model, never handles an API key and opens no network connection, matching Baley's ADR 0039. The summary half of observations goes with everything that exists only for it, while the mechanical half and the shape rules `redact_recall` uses stay.

## Must be true when done

- `cargo tree --workspace -e all --target all` lists no `ureq`, `reqwest`, `hyper` or `rustls`. A test fails if any file under `crates/*/src` names `std::net`, `TcpStream`, `TcpListener` or `UdpSocket`.
- No source file looks up an `_API_KEY` variable or names `credentials.toml`, and nothing reads an `api_key` setting.
- Any top-level `provider` key in `terminus.toml` makes `terminus status` exit non-zero. That covers a full table, an empty one, and one with `enabled = false`. The message names the file and says model calls were removed, and a key value written in the table appears in neither stdout nor stderr.
- `terminus doctor --json` carries no `credentials`, `config_mode`, `mode_checks` or `provider_local` check.
- With `redact_recall` on, recall output is filtered by the same shape rules as before, and the full suite passes.

## Context

- Plan 2 of 4, after plan 1. Plan 1 removed the judgment code and every test that drove it through a `[provider]` table, which is what lets the refusal land without breaking the suite.
- Binding decisions: D-04 (the refusal), D-05 (doctor), D-06 (egress) and D-08 (no network, enforced by a test). `withhold_secret_excerpt` stays, which is CONTEXT's flagged assumption kept as the default.
- Out of scope: moving `egress.rs` out of `observe/`.

## Tasks

### Task 1: Take out the HTTP client and hold the no-socket rule with a test

- **Files:** Cargo.toml, crates/terminus-core/Cargo.toml, Cargo.lock, crates/terminus-core/src/testkit.rs (anchor: `HttpStub`), crates/terminus-core/tests/no_network.rs
- **Action:**
  - **Dependency.** Remove `ureq` and its comment block from the root `[workspace.dependencies]`, and remove it from `terminus-core`'s `[dependencies]`. Let cargo rewrite `Cargo.lock`. Removing a dependency needs no network.
  - **`toml` comment.** Correct the comment on the `toml` entry: drop the paragraph that says "no HTTP client" used to hold and that `ureq` replaced it, so "nothing that opens a socket" once again describes the whole workspace. This is D-09's `Cargo.toml` half. It lands here because the `ureq` comment cannot outlive its entry.
  - **`testkit.rs`.** Delete the whole loopback HTTP section: the D-20 comment, `UNPARSEABLE_CONTENT`, `http_response`, `chat_completion`, `HttpStub` with its `impl`s and `Drop`, `read_http_request`, and the `std::io`, `std::net`, atomics and `Arc` imports that only that section used.
  - **New test, `crates/terminus-core/tests/no_network.rs`.**
    - One test, with no `testkit` gate, so `cargo test -p terminus-core` always runs it. It follows `nothing_in_the_ingest_path_names_the_egress_filter`, which is ungated for the same reason.
    - It walks every `.rs` file under both `crates/terminus-core/src` and `crates/terminus/src`, reaching the second from the parent of `CARGO_MANIFEST_DIR`.
    - It fails, naming the file and the term, if any file names `std::net`, `TcpStream`, `TcpListener` or `UdpSocket`. Comment lines are scanned too, since AC1 says "names".
    - It asserts it read a file count that an empty or misdirected walk could not reach, the way the ingest scan asserts `checked >= 3`.
  - **Fixture text stays.** The redaction fixtures keep `OPENAI_API_KEY=` and `"api_key"` as input text (D-08).
- **Verify:**
  - `cargo tree --workspace -e all --target all | rg -i '\b(ureq|reqwest|hyper|rustls)\b'` prints nothing.
  - `cargo test -p terminus-core --test no_network` passes.
  - Add `use std::net::TcpStream;` to a file in `crates/terminus/src`, then to one in `crates/terminus-core/src`. Each time the test must fail and name that file. Revert both.
  - `cargo test --workspace --all-features` passes and clippy with `-D warnings` is clean.

### Task 2: Doctor loses the key checks, and the credentials module goes with them

- **Files:** crates/terminus/src/cmd/doctor.rs (anchor: `run` / `archive`), crates/terminus/tests/doctor.rs, crates/terminus-core/src/credentials.rs, crates/terminus-core/src/lib.rs, crates/terminus-core/tests/credentials.rs
- **Action:**
  - **Doctor's checks (D-05).**
    - `doctor::run` stops calling `credentials`, `config_mode` and `mode_checks`.
    - `archive` stops calling `provider_local`, and stops pushing its `unknown("provider_local", ...)` arm when the config will not load. The `config_roots` problem it pushes stays, because that is where a refused `[provider]` table shows up in doctor (D-04).
    - Delete those four methods, `host_of` and `is_loopback`, plus the imports only they used: `credentials::{self, Permissions}` and whatever else the compiler then flags. Keep `removal` and every other helper.
  - **Doctor's tests.**
    - In `tests/doctor.rs`, delete these sections and every test in them: credentials, `terminus.toml` mode, the Windows ACL deferral and `provider_local`. That includes the helpers `credentials_file`, `assert_no_credential_reaches_a_stream`, `keyed_config` and `local_provider`.
    - Delete the fixture members only they used, if nothing else calls them: `shared_dir`, `credentials`, `write_credentials`, the `JCRENSHAW_CONFIG_DIR` pin, `set_mode` and the config writer.
    - In their place, add one test that runs `doctor --json` and asserts that none of `credentials`, `config_mode`, `mode_checks` or `provider_local` is among the checks (AC7).
  - **Credentials module.** Delete `crates/terminus-core/src/credentials.rs`, `tests/credentials.rs` and the `pub mod credentials;` line in `lib.rs`. With doctor's two checks gone, nothing reads the shared file or resolves a key. `owner_only` stays, because the store and install use it.
- **Verify:** `rg -n 'credentials::|config_mode|mode_checks|provider_local|host_of|is_loopback' crates --glob '*.rs'` prints only the new absence test's lines. `cargo test -p terminus --all-features --test doctor` passes, `cargo test --workspace --all-features` passes, and clippy with `-D warnings` is clean.

### Task 3: Egress keeps its shape rules and loses the credential

- **Files:** crates/terminus-core/src/observe/egress.rs (anchor: `redact` / `Redaction`), crates/terminus-core/src/observe/mod.rs (module doc's `egress` paragraph), crates/terminus-core/tests/egress.rs, crates/terminus-core/src/recall/excerpt.rs, crates/terminus-core/src/recall/context.rs, crates/terminus-core/src/recall/search.rs, crates/terminus-core/src/inject/brief.rs
- **Action:**
  - **Removals (D-06).** Remove rule 1 (`replace_credential`, `REDACTED_CREDENTIAL`, `MIN_CREDENTIAL_CHARS`), `for_destination` and `scrub`. `redact` and `for_model_context` take the text alone.
  - **`Redaction`.** It loses its `credential` field, and `Redaction::of` reads only `Config::redact_recall`. Since `Redaction` then borrows nothing, its lifetime parameter goes too.
  - **Callers.** The `&Redaction<'_>` annotations in `recall/excerpt.rs`, `recall/context.rs`, `recall/search.rs` and `inject/brief.rs` drop the `<'_>`. These are one-token edits. Keeping a `PhantomData` only to spare them would leave a lifetime that means nothing.
  - **What stays unchanged.**
    - The shape rules, their markers, `SECRET_NAMES`, `GITHUB_TOKEN_PREFIXES` and the length floors. "The same rules" in criterion 5 means these (D-06).
    - The module stays at `observe/egress.rs`.
  - **Module doc.** Rewrite it for one entry point and the shape rules, renumbered without rule 1. Drop its claim that `feedback`'s notes are scrubbed on their way to `runs.error`, because no feedback file ever called `scrub` (CONTEXT flagged assumption). Some comments explain a rule by the provider request it once filtered, or by how `judgment::transcript` wrote a turn. Reword those to say what the rule matches in a projected turn.
  - **Unit tests in `egress.rs`.** They call `for_model_context` where they called `scrub(None, ...)`, with the same inputs and the same expectations.
  - **`tests/egress.rs`.**
    - Delete `a_local_destination_sends_the_body_byte_identical`, `a_remote_or_undeclared_destination_sends_neither_secret`, `the_scrubber_takes_the_credential_out_of_a_401_body` and `the_scrubber_is_not_keyed_on_the_destination`.
    - Route `the_shape_rules_hold_without_a_resolved_credential` and the rule-set tests through `for_model_context`.
    - Keep `nothing_in_the_ingest_path_names_the_egress_filter` as it is.
    - Rewrite the file doc to say what is under test now.
  - **`observe/mod.rs`.** Rewrite the module doc's `egress` paragraph: `egress` holds the shape rules recall applies when `redact_recall` is on, and nothing redacts at ingest.
- **Verify:** `rg -n 'for_destination|scrub\(|REDACTED_CREDENTIAL|MIN_CREDENTIAL_CHARS|config::Secret|Redaction<' crates --glob '*.rs'` prints nothing. `cargo test --workspace --all-features` passes, including the redaction tests in `redacted_recall`, `recall`, `egress`, and the `mcp`, `brief` and `recall_cli` knob tests, unchanged. Clippy with `-D warnings` is clean.

### Task 4: A `[provider]` table refuses to load

- **Files:** crates/terminus-core/src/config.rs (anchor: `FileConfig` / `Config::load_from`), crates/terminus-core/src/error.rs (anchor: `Error::ConfigParse`), crates/terminus-core/tests/config.rs, crates/terminus/tests/status.rs
- **Action:**
  - **Removals (D-04).** Remove `FileProvider`, `Provider`, `Config::provider` and every `provider_*` accessor, along with `ResponseFormat` and `Secret`. Each exists only to carry a `[provider]` value, and after Tasks 2 and 3 nothing reads one. `REDACTED` stays because the egress shape rules use it. Its doc stops describing `Secret`.
  - **The captured field.** `FileConfig` keeps a `provider` field whose type accepts any TOML value and keeps none of it: serde's `de::IgnoredAny` behind an `Option`, defaulted. Nothing from the table is ever held in memory.
  - **The refusal.**
    - `Config::load_from` checks that field after a successful parse and before `Config::resolve`.
    - If the field is present, it returns a new `Error` variant of its own. The variant carries the path, and its message names the file, says model calls were removed from Terminus, and says to delete the table.
    - It must not return `Error::ConfigParse`, whose text says the file is not valid TOML.
    - The message is in-tool text, so it should be plain and complete, per `~/.claude/john-voice.md` "In-tool text".
  - **What counts as a provider key.** Any top-level `provider` key is refused: a full table, an empty one, `enabled = false`, or a bare scalar. A malformed line inside a leftover table still fails the parse first and goes through `withhold_secret_excerpt` unchanged (flagged assumption, kept).
  - **Docs in `config.rs`.**
    - Rewrite `FileConfig`'s "unknown keys are ignored" doc to name this as the one deliberate exception, and why: a table that once enabled a paid model call and held a key must not load silently as if it were still honored.
    - Correct the docs that describe the provider: `FilePrivacy`'s "Its own table, and not a key on `[provider]`" paragraph (there is no judgment write filter any more), `FileRetention`'s mention of `[provider]`, the `Default for Config` comment, the `base_url`/`api_key` example on `non_empty`, and `source_path`'s claim that `credentials::resolve` and doctor call it.
  - **Core tests.**
    - In `tests/config.rs`, replace the `[provider]` section with refusal tests over the four shapes above. Each one fails `Config::load_from` with the new variant. Its message names `terminus.toml` and the removal. A sentinel written into the table appears in neither its `Display` nor its `Debug`.
    - Keep `a_malformed_api_key_line_is_not_echoed_by_the_parse_error` exactly as it is.
    - The `Secret` and `ResponseFormat` imports go.
  - **AC3 at the process boundary.** In `crates/terminus/tests/status.rs`, run `terminus status` twice: once against a `terminus.toml` holding an empty `[provider]`, and once against one holding `api_key = "<sentinel>"`. Each run exits non-zero, stderr names the file and says model calls were removed, and the sentinel is in neither stdout nor stderr.
  - **Hooks need no change.** `hook` already returns nothing when the config will not load, and the hook-spawned pass already exits before writing a `runs` row. That is D-04's stated consequence, and `status` and doctor's `config_roots` are where it shows.
- **Verify:**
  - `cargo test -p terminus-core --all-features --test config` and `cargo test -p terminus --all-features --test status` pass.
  - By hand, from the repo root:
    - Run `d=$(mktemp -d); mkdir -p "$d/cfg" "$d/claude/projects"; printf '[provider]\napi_key = "sk-PLANREFUSAL-5150"\n' > "$d/cfg/terminus.toml"; TERMINUS_DATA_DIR="$d/data" TERMINUS_CONFIG_DIR="$d/cfg" CLAUDE_CONFIG_DIR="$d/claude" cargo run -q -p terminus -- status; echo "exit $?"`. It must print the refusal naming `$d/cfg/terminus.toml` and exit 1.
    - The same environment with `doctor --json` must report `config_roots` as a problem whose finding names the file.
    - Neither run's output contains `PLANREFUSAL`.
  - AC2: `rg -n '_API_KEY|credentials\.toml' crates/*/src` prints nothing. `rg -n 'api_key' crates/*/src` prints only `withhold_secret_excerpt` and the egress unit-test inputs.
  - The full suite passes and clippy with `-D warnings` is clean.

## Notes

- `Secret` and `ResponseFormat` are removed as part of "the `[provider]` table and its accessors" (CONTEXT scope). Both are types used only by provider accessors.
- `Config::source_path` and the `source` field lose their last non-test caller, doctor's `config_mode`. They are kept because CONTEXT does not name them, and the human decides whether they go.
- Plan 1's Notes record why this plan follows plan 1 rather than leading the phase as the CONTEXT Plan shape suggested.
