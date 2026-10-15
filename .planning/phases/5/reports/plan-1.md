PLAN COMPLETE
Plan: .planning/phases/5/PLAN-1.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Download the flat release asset names | b33847d | install.sh asks for `$source/terminus-$os-$arch.manifest` and `$source/terminus-$os-$arch`; fixture serves the flat names through a new `Fixture::binary()`; 14/14 install_script tests pass, `sh -n` 0 |
| 2 - Make the source optional, defaulting to the GitHub releases | 050c66e | `case $#` picks the default, one https source, or the usage `sh install.sh [https://<source>]`; harness takes `&[String]`; fake `curl` via `Fixture::curl_tools`; new `no_source_downloads_from_the_github_releases`, `two_sources_are_refused`; 16/16 pass; one-character typo in the default URL fails the test showing both logged URLs, reverted; two-argument run exits 1 with only the usage line |
| 3 - Test both download refusals on the default source | 03d20a0 | Test file only. `curl_tools(failing)` exits 22 for the named file; `assert_refused` returns the output; new `a_failed_manifest_download_is_refused`, `a_failed_binary_download_is_refused`; module doc rewritten; 18/18 pass; manifest message pointed at the binary URL fails the manifest test (17/18), reverted |
| 4 - Wrap the script body in one function, and test the piped forms | da6bf05 | Body from `case $#` through `"$stable" install` is `main`, called as `main "$@"` on the last line; `git diff -w` shows only the wrapper and header comment; harness `Fixture::piped` writes the script into sh's stdin pipe and closes it; new `a_piped_script_with_no_source_downloads_from_the_github_releases`, `a_piped_script_installs_from_its_source`; 20/20 pass; `install_sh_holds_the_same_marker` passes; cut-off check over n=1..243: HOME folder empty, 0 trace lines naming uname/mktemp/curl (control on the flat HEAD~ script: 492 such lines); clippy --workspace --all-features clean |
Deviations: none
Open items: none
