---
phase: 3
plan: 4
requirements:
  - PRIV-02
files:
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/credentials.rs
  - crates/verbatim/src/cmd/doctor.rs
  - crates/verbatim-core/tests/config.rs
  - crates/verbatim-core/tests/credentials.rs
  - crates/verbatim/tests/doctor.rs
  - crates/verbatim/tests/config_mode.rs
---

# Phase 3: Owner-Only On Disk - Plan 4

## Goal

`verbatim.toml` carrying `provider.api_key` gets the mode check the shared
credentials file already has, and `verbatim doctor` prints the `chmod` that
fixes each credential refusal, states the Windows ACL deferral in words on
every platform, and notes - without refusing - a `provider.local = true`
whose `base_url` is not a loopback address.

## Must be true when done

- With judgment on and no key in the environment, a `verbatim.toml` at 0644
  carrying `provider.api_key` is refused with a message naming the path and
  the octal mode, the message never calls it a credentials file, and the key's
  value appears in no byte of stdout, stderr or `runs.error`. The same file at
  0600 is accepted.
- With the key absent from the file, or supplied through the environment, a
  0644 `verbatim.toml` still loads and `verbatim status` exits 0; with judgment
  off it loads too.
- `verbatim doctor` against a 0644 `verbatim.toml` holding a key and a 0644
  shared credentials file prints a runnable `chmod` line for each; running the
  printed lines makes the next `doctor` report both ok.
- On this Linux machine `verbatim doctor` states, once, in words, that its
  mode checks cover both files and that no Windows ACL is examined by this
  build.
- With `provider.local = true` and a non-loopback `base_url`, `doctor` prints a
  note and exits 0; with a loopback `base_url` no such note appears; the
  provider test `the_body_on_the_wire_is_filtered_for_remote_and_whole_for_local`
  is unchanged and passes.

## Context

Locked: D-07 (the refusal fires in `credentials::resolve` where tier 2 is
consumed, never in `Config::load_from`), D-13 (`Config` gains the path it was
loaded from; `from_parts` and `Default` carry none), D-14 (same `mode & 0o077`
test, same path-and-mode-only payload, a DISTINCT error case whose wording
does not say "credentials file"), D-15 (new doctor findings are new checks
with stable `--json` names; the loopback mismatch is `State::Note`), D-16
(hand-written host extraction over the `base_url` string, no URL crate), D-17
(doctor's "each refusal" is the two credential refusals; the Windows deferral
stated once, covering both, on every platform). Out of scope: any change to
the shared file's own read-and-refuse behaviour, and Windows ACL enforcement.
`credentials::permissions` already returns `Unchecked` on the non-Unix arm,
so reusing it gives the `verbatim.toml` check D-15's acceptance for free.

## Tasks

### Task 1: Config remembers the file it was loaded from

- **Files:** crates/verbatim-core/src/config.rs (`Config`, `load_from`,
  `from_parts`, `impl Default for Config`),
  crates/verbatim-core/tests/config.rs
- **Action:** Add to `Config` an optional path field that `load_from` fills
  with `config_dir.join(CONFIG_FILE_NAME)` - the one place the path exists
  today - whether or not the file was there (a refusal must name the file the
  values came from, and under `VERBATIM_CONFIG_DIR` that is not
  `config_dir()` re-resolved later, D-13), and that `from_parts` and
  `Default` leave as none. Expose it through an accessor beside the other
  `provider_*` accessors. The derives on `Config` (`Clone, PartialEq, Eq,
  Debug`) stay and the field must not break them. `resolve` and every existing
  accessor are unchanged; no new dependency.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --test config` passes, including a new test showing a config
  loaded through `load_from` from a directory holding a `verbatim.toml`
  reports that exact path, one loaded from a directory holding no file reports
  the path it would have read, and `Config::from_parts` and
  `Config::default()` report none. `cargo test --manifest-path
  /code/verbatim/Cargo.toml --workspace` still compiles every crate.

### Task 2: Refuse a wide verbatim.toml where its key is consumed

- **Files:** crates/verbatim-core/src/credentials.rs (`Error`, `resolve`),
  crates/verbatim-core/tests/credentials.rs,
  crates/verbatim/tests/config_mode.rs
- **Action:** Add a third `credentials::Error` variant beside `TooOpen` and
  `Unreadable`, carrying a path and a mode and nothing else (D-14), whose
  `Display` names the path, renders the mode as three octal digits, and says
  this is verbatim's own config file readable beyond its owner - never the
  words "credentials file", because a user with a 644 `verbatim.toml` must not
  go looking for `~/.config/jcrenshaw/credentials.toml`. In `resolve`, at the
  `provider_api_key()` arm and only there - after the environment tier has
  come up empty, after the `provider_enabled()` gate that already returns
  early - read the config's source path from Task 1's accessor and run
  `credentials::permissions` on it: `TooOpen` raises the new variant,
  `Unreadable` raises `Error::Unreadable` with that path, `Owner`,
  `Unchecked` (D-15) and `Absent` (the file was there a moment ago; a race
  with the user editing it is not this program's failure) return the key. A
  config with a key and no source path cannot be built through any public
  constructor; if one arrives, return the key. `Config::load_from` is not
  touched (D-07): a wide file with judgment off, or with the key in the
  environment, must keep loading. The new `Display` text passes through
  `egress::scrub` on the observe path like every other note, and carries
  nothing scrub would need to catch.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --test credentials` passes, including new `#[cfg(unix)]` tests
  on the existing bench: `config(Some(key))` with the written `verbatim.toml`
  set to 0644 makes `resolve` return the new variant with that path and mode
  0o644, the rendered error contains the path and `644` and none of
  `fragments(key)`, and the same at 0600 returns the key; with judgment off,
  or with `PROVIDER_ENV` set, the 0644 file resolves without error. And
  `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim --test
  config_mode` passes a new `#[cfg(unix)]` binary test, benched like
  `tests/observations.rs` (config dir, data dir, Claude dir pinned;
  `OPENROUTER_API_KEY` removed from the environment; `JCRENSHAW_CONFIG_DIR`
  pinned to an empty directory), that writes a `[provider]` block with
  `enabled = true`, `name = "openrouter"`, a distinctive `api_key`, a
  `base_url` of `http://127.0.0.1:1/` and a model, sets the file to 0644, runs
  `verbatim ingest`, and asserts stderr names the config path and `644`, that
  no fragment of the key is in stdout, stderr or `SELECT error FROM runs`, and
  that the run exited 0 (the judgment step returns notes, `ingest.rs`); then
  at 0600 the same ingest prints no refusal; then with the key line removed
  and the file back at 0644 `verbatim status` exits 0; then with the key line
  present at 0644 and `OPENROUTER_API_KEY` set in the spawn's environment the
  ingest prints no refusal.

### Task 3: Doctor prints the chmod for verbatim.toml and states the deferral

- **Files:** crates/verbatim/src/cmd/doctor.rs (`run`, `credentials`),
  crates/verbatim/tests/doctor.rs
- **Action:** Two new checks pushed right after the existing `credentials`
  check, each with a stable `--json` name chosen here: `config_mode` and
  `mode_checks`. `config_mode` reports `verbatim.toml`'s mode the way
  `credentials` reports the shared file's, reading the path from the loaded
  config's Task 1 accessor when `Config::load()` succeeded and from
  `config_dir()` joined with `CONFIG_FILE_NAME` when it did not, through
  `credentials::permissions` so the answer cannot disagree with the refusal:
  absent is `State::Note` naming where it looked; owner-only is `State::Ok`
  with the mode; too open WITH `provider_api_key()` present is
  `State::Problem` with the finding naming path and mode and a fix of `chmod
  600 '<path>'` (the shell-quoted shape the `credentials` check already
  prints); too open with no key is `State::Ok` stating the mode and that the
  file holds no key; too open when the config did not parse (so whether a key
  is there is unknowable) is `State::Unknown` with the same chmod as its fix;
  `Unchecked` and `Unreadable` mirror the `credentials` arms. Never open or
  render the file. `mode_checks` is `State::Note` on every platform, no fix,
  and its finding says in words that the `credentials` and `config_mode`
  checks read Unix mode bits and that this build examines no Windows ACL, so
  a file on Windows is accepted unverified (D-15, D-17) - stated once, here,
  so it is visible on Linux and not only inside the `Unchecked` arm. The
  existing `credentials` check and its `Unchecked` wording are unchanged.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --test doctor` passes, including new `#[cfg(unix)]` tests on the existing
  `Fixture`: a `config/verbatim.toml` holding `[provider]` with an `api_key`
  at 0644 makes `config_mode` a problem, exit 1, the finding contains `644`
  and the path, `assert_no_credential_reaches_a_stream`'s pattern finds no
  fragment of that key in either output, and running the printed fix through
  `Fixture::shell` makes the next `doctor` report `config_mode` ok and exit 0;
  the same file at 0644 with no `api_key` is ok and exits 0; an absent file is
  a note; with BOTH a 0644 shared file and a 0644 keyed `verbatim.toml`, the
  report carries two fix lines, one per file, and running both makes the next
  run exit 0 (AC6); `mode_checks` is present as a note in the human report
  and under that key in `--json` with a finding containing `Windows` and
  `ACL`; `the_json_document_carries_every_check_by_name` passes unmodified.

### Task 4: Doctor notes a local provider that is not on loopback

- **Files:** crates/verbatim/src/cmd/doctor.rs (`archive`),
  crates/verbatim/tests/doctor.rs
- **Action:** A third new check, stable name `provider_local`, pushed from
  `archive` once the config has loaded (and pushed through the existing
  `unknown` helper when it did not). It reads `provider_local()` and
  `provider_base_url()`. When `local` is false, or `local` is true and the
  host is loopback, it is `State::Ok` with a one-line finding saying what was
  declared; when `local` is true and the host is NOT loopback it is
  `State::Note` - never `State::Problem`, so `doctor` still exits 0 (D-15) -
  and the finding says the provider is declared local, names the host, and
  says bytes go there unfiltered because the declaration decides and not the
  address (D-13 of v0.1.0 stands; this changes no behaviour). `local` true
  with no `base_url` is `State::Unknown`. Host extraction is a hand-written
  scan over the string (D-16, the phase 2 precedent): skip an optional
  `scheme://`, take the authority up to the first `/`, `?` or `#`, drop
  everything up to and including the LAST `@` so
  `http://127.0.0.1:11434@evil.example/` classifies as `evil.example`, then
  if the host starts with `[` take what is inside the brackets, otherwise cut
  at the last `:` only when what follows is all digits. Loopback is exactly:
  `localhost` (ASCII case-insensitive), any dotted-quad whose first octet is
  `127`, and `::1`. No `url` crate, no regex crate, no new dependency.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim
  --test doctor` passes new tests that write a `[provider]` block with
  `local = true` and each of `http://example.invalid:11434`,
  `http://127.0.0.1:11434@evil.example/` and `https://10.0.0.5/v1` and show
  `provider_local` is a note with exit 0, and with each of
  `http://localhost:11434`, `http://127.0.0.1:11434/`, `http://[::1]:11434`
  and `HTTP://LOCALHOST/` show it is ok; with `local` absent and a remote
  `base_url` it is ok. `cargo test --manifest-path /code/verbatim/Cargo.toml
  -p verbatim-core --test provider` passes with `tests/provider.rs`
  unmodified (`git -C /code/verbatim diff --quiet
  crates/verbatim-core/tests/provider.rs` exits 0), and `git -C /code/verbatim
  diff --quiet Cargo.lock` exits 0.

## Notes

- This plan shares no file with plans 1-3 and has no ordering dependency on
  them; it runs in numeric order only because the phase as a whole is
  sequential (plans 1-3 overlap on `owner_only.rs`).
- Planner choices recorded: the three `--json` names (`config_mode`,
  `mode_checks`, `provider_local`); the deferral is its own `Note` check
  because a check line is the only vehicle doctor's report and `--json`
  document share, and putting it inside both mode checks would state it twice
  against D-17's "once".
- Recalled v0.1.0 phase 7 UAT ("no assertion over stdout, stderr, runs.error
  or the observations table finds the key's value") is why Task 2's binary
  test reads `runs.error` as well as both streams.
