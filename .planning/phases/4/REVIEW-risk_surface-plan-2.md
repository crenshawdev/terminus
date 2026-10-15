{
  "findings": [
    {
      "file": "install.sh",
      "line": 103,
      "severity": "low",
      "claim": "The staging directory is trusted without rejecting symbolic links in the versions path, so installation can overwrite a file outside the versions folder.",
      "failure_scenario": "If `~/.local/lib/crenshawdev/terminus/versions/1.2.3` already exists as a symlink to an arbitrary directory, `mkdir -p` accepts it. `mktemp`, `mv -f \"$staging_temporary\" \"$staged\"`, and the later stable link then operate on `<arbitrary directory>/terminus`; an existing unowned file there can be replaced with the downloaded binary."
    },
    {
      "file": "install.sh",
      "line": 220,
      "severity": "low",
      "claim": "Stable-path ownership checking and replacement are separated by a race, allowing an unowned entry to be overwritten.",
      "failure_scenario": "After `check_stable_path` succeeds, another process can create or replace `~/.local/bin/terminus` with an unowned regular file before line 221 executes. `mv -f` then replaces that unchecked file. If the raced-in entry is a symlink to a directory, platform `mv` behavior can instead move the temporary link into that directory, potentially writing outside the named stable folder and leaving the stable path unchanged."
    }
  ]
}
