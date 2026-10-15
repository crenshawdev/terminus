---
status: testing
phase: 4
fields_version: 1
started: 2026-10-13
updated: 2026-10-13
---

## Items

### 1. install.sh refuses a bad checksum, an unsupported system or architecture, and a non-https source
expected: Each of the four runs exits non-zero, and afterwards the stable path and the versions folder are byte-identical to before (a bad manifest SHA-256, uname -s other than Linux or Darwin, uname -m other than x86_64, aarch64 or arm64, and an http:// source).
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: install.sh:109, :140-149, :71-75 called at :191 before staging at :195-210. Tests a_digest_that_does_not_match_is_refused, an_unsupported_system_is_refused, an_unsupported_architecture_is_refused and an_http_source_is_refused (crates/terminus/tests/install_script.rs:600-637) each assert a non-zero exit and an unchanged snapshot of the stable path, every entry and inode under the versions folder, settings.json and .claude.json, with no claude call. All ok in cargo test --workspace --all-features (784 passed, 0 failed).

### 2. Installing version A and then version B keeps A
expected: After two install.sh runs, ~/.local/bin/terminus links to versions/B/terminus, versions/A/terminus has the same bytes and inode as before, and a terminus mcp started through the link before the second run still runs from A's file per /proc/<pid>/exe.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: Test successive_installs_keep_every_version (install_script.rs:702-727) asserts the link targets versions/2.0.0/terminus, versions/1.0.0/terminus keeps its bytes and inode, and /proc/<pid>/exe of a terminus mcp started through the link before the second install equals A's canonical path. The /proc block is Linux-gated and ran here. ok in the full-suite run.

### 3. install.sh finishes the install with claude present, and fails without it
expected: With a fake claude that succeeds, one run exits 0, merges the four hook entries naming the stable path, and logs claude mcp add-json --scope user terminus. With no claude on PATH, the run exits non-zero after the receipt's "install incomplete" line.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: install.sh:232 ends with "$stable" install under set -eu. Test one_run_installs_and_wires_claude_code (install_script.rs:521-553) asserts exit 0, one hook entry per event naming the stable path, and a logged mcp add-json --scope user terminus with command the stable path and args ["mcp"]. Test no_claude_exits_non_zero_after_the_link_is_placed (:557-571) asserts a non-zero exit with "install incomplete: the mcp server is not registered" on stdout. Both ok.

### 4. A foreign file at the stable path is refused, and a marked plain file becomes the link
expected: With a file lacking the marker at the stable path, both install.sh and terminus install exit non-zero and leave the file unchanged. A plain file carrying the marker is replaced by the link into the versions folder.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: install.sh:91-93 and crates/terminus/src/cmd/install/mod.rs:119-121 refuse an unmarked file before any write. Tests a_file_without_the_marker_is_refused (install_script.rs:652-659), a_foreign_binary_at_the_stable_path_stops_everything (crates/terminus/tests/install.rs:522-560, file bytes unchanged and no versions folder), a_marked_plain_file_is_replaced_by_the_link (install_script.rs:752-760) and a_marked_plain_file_at_the_stable_path_becomes_the_link (install.rs:454-470) all ok. install_sh_holds_the_same_marker (binary.rs:401-411) ties the script's marker to MARKER; ok.

### 5. terminus install from a build stores it as a version, and from the stable path changes nothing
expected: target/release/terminus install leaves the stable path a link to versions/<its version>/terminus holding the build's bytes. A rebuild at the same version with different bytes replaces that file. ~/.local/bin/terminus install leaves the link and the version file with the same inodes and the link with the same target.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: binary.rs:197-266 (place, linked_to by device and inode, store) called from mod.rs:137. Tests install_places_this_build_at_the_stable_path (install.rs:393-450), a_rebuild_at_the_same_version_replaces_its_file_and_a_rerun_does_not (:474-518), install_from_the_stable_path_touches_neither_the_link_nor_the_file (:1399-1423) and install_through_a_link_to_another_version_folder_leaves_it_alone (:1428-1450) all ok. They run cargo's debug build rather than target/release/terminus; binary.rs has no profile-dependent code.

### 6. Uninstall removes the link and keeps the versions; the script parses and the suite passes
expected: terminus uninstall leaves nothing at the stable path, every file in the versions folder still there, and prints the versions folder's path. sh -n install.sh exits 0. cargo test --workspace --all-features passes.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: uninstall.rs:89 and :366-392 remove an Ours link with remove_file; :91-99 print the versions folder. Test install_then_uninstall_leaves_both_files_exactly_as_they_were (tests/uninstall.rs, assertions at 331-351) asserts symlink_metadata(stable) is NotFound, the version file keeps the build's bytes and the output names the versions folder; ok. sh -n install.sh exited 0. cargo test --workspace --all-features: 784 passed, 0 failed, exit 0.

### 7. The docs describe the link, the versions folder and install.sh
expected: DESIGN-BRIEF.md describes the link and the versions folder and says nothing about a copy at the stable path; PROJECT.md no longer contains "except the configured model provider"; CHANGELOG.md [Unreleased] records install.sh.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: No match in DESIGN-BRIEF.md for re-copies, That copy is canonical, copies the running binary, Binary (canonical) or arrives in phase 4; DESIGN-BRIEF.md:364-365, :389-400 and :421 describe the link, the versions folder, both install paths and uninstall. No match in .planning/PROJECT.md for configured model provider, redaction at egress, 0600 or arrives in phase 4; :104 states the network line. CHANGELOG.md:10-46 records install.sh under Added and the link, versions folder and local build under Changed.

### 8. install.sh refuses TERMINUS_BIN_DIR and a dangling link outside the versions folder
expected: With TERMINUS_BIN_DIR set, or with a dangling link at the stable path pointing outside the versions folder, install.sh exits non-zero and changes nothing; a dangling link into the versions folder is replaced.
status: pass
first_pass: pass
source: verifier
evidence: install.sh:112-113 and :83-90. Tests terminus_bin_dir_set_is_refused (install_script.rs:641-649), a_dangling_link_outside_the_versions_folder_is_refused (:663-674) and a_dangling_link_into_the_versions_folder_is_replaced (:764-775) all ok in the full-suite run.

### 9. terminus uninstall leaves a dangling link into the versions folder at the stable path and says nothing is there
expected: behavior wrong - unplace treats a dangling link as Vacant, so the link this phase's own layout leaves behind once the versions folder is gone is never removed, and the report line is false
origin: verifier
status: pass
first_pass: fail
source: model
evidence: Retest on 6208560 in a scratch HOME (CLAUDE_CONFIG_DIR and TERMINUS_DATA_DIR in the scratch folder): target/debug/terminus install left ~/.local/bin/terminus a link to versions/0.1.1/terminus; after rm -rf of the versions folder, terminus uninstall exited 0 and printed "removed: a link to <HOME>/.local/lib/crenshawdev/terminus/versions/0.1.1/terminus, which is not there", and nothing was left at the stable path. cargo test -p terminus --test uninstall: 19 passed, including a_link_left_dangling_by_a_deleted_versions_folder_is_removed (fails at tests/uninstall.rs:658 with the old Vacant arm restored) and a_dangling_link_outside_the_versions_folder_is_reported_and_kept. cargo test --workspace --all-features: 786 passed, 0 failed; clippy -D warnings clean.
reported: behavior wrong - unplace treats a dangling link as Vacant, so the link this phase's own layout leaves behind once the versions folder is gone is never removed, and the report line is false
severity: minor
cause: unplace (crates/terminus/src/cmd/uninstall.rs:366-372) asks binary::occupant, which opens the stable path with File::open (binary.rs:145-148). That follows the link, so a link to nothing comes back NotFound and is read as Vacant: uninstall prints "not there: nothing to remove" and leaves the link. Vacant is right for install, which writes a new link over it, and wrong for uninstall, where a link is an entry to remove.
fix: 6208560, retest

## Summary

total: 9
passed: 9
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 1
