---
status: testing
phase: 5
fields_version: 1
started: 2026-10-14
updated: 2026-10-14
---

## Items

### 1. No argument downloads from the GitHub releases
expected: Run with no argument, both as `sh install.sh` and piped into `sh`, install.sh requests exactly `https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<os>-<arch>.manifest` and `https://github.com/crenshawdev/terminus/releases/latest/download/terminus-<os>-<arch>`, and the install finishes.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: install.sh:122-123,179-182; no_source_downloads_from_the_github_releases::sh and a_piped_script_with_no_source_downloads_from_the_github_releases::sh passed. The fake curl's log equals the two literal default URLs in order, the run exits 0, and assert_linked holds for 1.0.0.

### 2. A piped https source installs; http and two arguments are refused
expected: The script piped into `sh -s -- https://<server>` installs from that server's flat names. An `http://` argument and a two-argument run each exit non-zero, and the stable path and the versions folder are unchanged.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: a_piped_script_installs_from_its_source::sh, an_http_source_is_refused::sh and two_sources_are_refused::sh passed; assert_refused checks the stable path and versions tree are unchanged. Spot-checks: an http:// argument prints 'the source must start with https://' and two https:// arguments print only the usage line, both exiting 1.

### 3. Both download failures are refused with the URL tried
expected: A failed manifest download exits non-zero with `manifest download failed: <url>`; a failed binary download exits non-zero with `binary download failed: <url>`. In both cases the stable path and the versions folder are unchanged.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: install.sh:179-182; a_failed_manifest_download_is_refused::sh and a_failed_binary_download_is_refused::sh passed. stderr holds 'manifest download failed: <default>/terminus-<platform>.manifest' and 'binary download failed: <default>/terminus-<platform>', with the snapshot unchanged.

### 4. A cut-off script changes nothing
expected: A copy of install.sh cut off at any line break before its last line, piped into `sh`, changes nothing under HOME.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: Every line-break cut n=1..251, piped into /bin/sh -x and dash -x with an absolute scratch HOME and source https://127.0.0.1:9, left HOME empty and traced no uname, mktemp or curl. A control run of the whole script traced 6 (sh) and 4 (dash) such lines.

### 5. A checksum mismatch is still refused
expected: A download whose SHA-256 does not match the manifest is refused before anything is placed under HOME.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: a_digest_that_does_not_match_is_refused::sh passed (assert_refused: nothing placed). `git diff -w fc20d98 HEAD -- install.sh` leaves verify_download, stage and the link untouched.

### 6. The install.sh tests pass under sh and dash, and the workspace tests pass
expected: The Phase 4 install.sh tests pass against the flat names under both `/bin/sh` and dash; `sh -n install.sh` and `dash -n install.sh` exit 0; `cargo test --workspace --all-features` passes. (human-verify: needs dash)
criterion: AC5
status: pass
first_pass: pass
reported: accept item 10's run, pass

### 7. CHANGELOG and DESIGN-BRIEF show the new install
expected: The CHANGELOG.md Unreleased entry and the DESIGN-BRIEF.md install section show the curl line, the optional `https://` source and the flat names, no longer mention `<source>/<os>-<arch>/manifest`, and still say no release is published yet.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: CHANGELOG.md:17,20,24,30 and DESIGN-BRIEF.md:381,383,395 carry the curl line, sh -s -- https://<source>, the flat names and 'No release is published'. The grep for '<os>-<arch>/' and 'sh install.sh https' in both files is empty.

### 8. A cut in the middle of the last line drops a passed source
expected: behavior wrong - SUMMARY.md's goal check says the cut-off loop covered every byte length, but only line breaks were tested, and two byte cuts run main with no arguments
origin: verifier
status: pass
first_pass: fail
source: model
evidence: After f21b40f: `cargo test -p terminus --test install_script a_cut_off` -> a_cut_off_script_runs_nothing::sh ok, ::dash ok. The test pipes install.sh cut at every byte before the closing brace into `sh -s -- https://<source>` with a logging fake curl and asserts no curl call and an empty HOME. Run against the old `main "$@"` form it failed under both shells at byte 9642 (`cut at byte 9642 ran curl`).
reported: behavior wrong - SUMMARY.md's goal check says the cut-off loop covered every byte length, but only line breaks were tested, and two byte cuts run main with no arguments
severity: minor
cause: install.sh:252 `main "$@"` is the only call into the body. Two of its prefixes, `main` and `main `, are complete commands that call main with no arguments, and no arguments selects the GitHub default (install.sh:122-123). No committed test cuts the script at all: AC4 was checked by one-off loops at line breaks only, so SUMMARY.md's 'every byte length' is wrong.
fix: f21b40f, retest

### 9. dash in a C locale corrupts a HOME holding byte 0x82
expected: behavior wrong - dash 0.5.13.4, started with no LANG, inserts 0x81 bytes before 0x82 in parameter-expansion results, so install.sh builds its paths outside the real HOME
origin: verifier
status: pass
first_pass: fail
source: model
evidence: After f484431: `cargo test -p terminus --test install_script` -> 44 passed, 0 failed; a_home_dash_would_change_is_refused::sh ok (HOME .../jürgen installs under /bin/sh) and ::dash ok (refused under dash in the C locale with 'this shell changes some bytes of HOME', nothing changed, then installs under dash with LANG=C.UTF-8). With the check removed, ::dash failed: `mv: cannot move ... to '.../j'$'\303\201\201\201\201\274''rgen/.local/lib/...'`.
reported: behavior wrong - dash 0.5.13.4, started with no LANG, inserts 0x81 bytes before 0x82 in parameter-expansion results, so install.sh builds its paths outside the real HOME
severity: minor
cause: dash 0.5.13.4 started without a UTF-8 locale inserts a 0x81 byte before any of eleven continuation bytes (0x82 0x8b 0x8e 0x90 0x9b 0x9e 0xa0 0xbc 0xbd 0xbe 0xbf) whenever it assigns or substitutes a value: `x=$HOME` and `$(printf %s "$HOME")` both come back changed, while "$HOME" passed as an argument does not. So ü (c3 bc), à (c3 a0) and ł (c5 82) are hit, not only 0x82. install.sh copies HOME at :142 and reads paths back through $(mktemp) and $(readlink), so no rewrite of the script avoids it; valid_utf8 accepts the HOME and the run continues on altered paths. LANG=C.UTF-8 or LC_ALL=C.UTF-8 at dash startup removes it, and /bin/sh (bash) is unaffected. Measured 2026-10-14 on dash 0.5.13.4-1.1; Debian and Ubuntu's 0.5.12 not measured.
fix: f484431, retest

### 10. From /code/terminus, run `sh -n install.sh && dash -n install.sh`, then `cargo test -p terminus --test install_script`, then `cargo test --workspace --all-features`
expected: Both -n checks exit 0. The install_script run reports 40 passed and 0 failed (20 ::sh, 20 ::dash). The workspace suite passes with 0 failed (plan 2's report recorded 812 passed).
origin: verifier
why_human: Not a limit of the model. The dispatching agent tagged this item human-verify (needs dash) and told the verifier not to run it as a machine check, so it was not run. Nothing here is irreversible or out of reach: dash 0.5.13.4 is now installed at /usr/bin/dash, so the walk can run these commands itself.
status: pass
first_pass: pass
source: model
evidence: `sh -n install.sh` exit 0; `dash -n install.sh` exit 0 (dash 0.5.13.4 at /usr/bin/dash). `cargo test -p terminus --test install_script` -> `40 passed; 0 failed`. `cargo test --workspace --all-features --no-fail-fast` -> 812 passed, 0 failed across 64 suites, exit 0. An earlier full run the same session failed two timing tests in crates/terminus/tests/hook.rs (every_event_exits_zero_with_an_empty_stdout_inside_the_budget at :405, every_event_stays_inside_the_budget_with_the_redaction_knob_on at :816; both assert hook p99 under 10 ms); `cargo test -p terminus --test hook` alone then passed 8/8, and the full rerun passed. Those tests do not touch install.sh.

## Summary

total: 10
passed: 10
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 2
