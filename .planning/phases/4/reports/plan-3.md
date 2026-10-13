PLAN COMPLETE
Plan: .planning/phases/4/PLAN-3.md
Tasks: 3 of 3
| Task | Commit | Note |
|---|---|---|
| 1 - DESIGN-BRIEF's binary layout describes the link and the versions folder | 6c13f2f | §12 table, §13 installer line, Binary layout and Upgrade rewritten from install.sh, binary.rs and uninstall.rs as they stand. Verify greps as predicted: stale phrases absent, versions path at lines 365 (§12) and 389 (§13), same spelling in install.sh:135 and binary.rs. clippy and check clean |
| 2 - The network statement names install.sh's download as the one network use | 9ee9524 | PROJECT.md Privacy line now: no telemetry, binary opens no network connection, install.sh's curl download is the one network use; stale model-provider, egress and 0600 clauses dropped with nothing added. Distribution names install.sh and says no release is published. DESIGN-BRIEF §10 quote and closing line scoped to the binary. Verify greps as predicted. clippy and check clean |
| 3 - The changelog records install.sh and the link | d3c95f9 | [Unreleased] gains an Added section before Changed (install.sh, its manifest and systems, verify_download, unsigned artifacts only, TERMINUS_BIN_DIR and foreign-file refusals, the one network use) and six Changed bullets (link, local-build command, same-version rebuild, kept versions, plain-file install becoming the link, uninstall). Verify range shows all four required names, as predicted. clippy and check clean |
Suite: workflow.test_command is null; ran cargo test --workspace --all-features (the command CONTEXT AC6 names). Every test binary reported ok, 0 failed.
Deviations: none
Open items: PROJECT.md still carries stale text outside this plan's Privacy and Distribution lines: the Key Decisions rows "Observations opt-in; parser does mechanical, LLM only judgment", "Redact at egress, never at ingest; ... local providers send nothing over the wire" and "Shared credentials at ~/.config/jcrenshaw/credentials.toml" describe model calls and credentials that phase 1 removed, and the "Canonical stable binary path" row predates the link. Left for whoever owns PROJECT.md's decision table.
