# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub: on this repository, open the **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and what an attacker could do with it. You will get an answer, and credit in the fix if you want it.

## Scope

Reports about Terminus on the `main` branch and about any published release are in scope. Of most interest:

- `install.sh` installing a binary when the manifest does not verify against the pinned release key or the binary's SHA-256 does not match the manifest, or downloading the binary before the manifest's signature verifies;
- the Terminus binary opening a network connection;
- Terminus reading a project listed under `exclude` in `terminus.toml`, or returning its sessions;
- with `[privacy] redact_recall` on, a secret of a shape the redaction filter is built to replace reaching Claude Code through a hook or an MCP tool;
- an archived session whose stored blocks no longer match their recorded hashes, which `terminus verify` still passes.
