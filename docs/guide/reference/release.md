# How a release is built, signed and checked

A Terminus release is built in CI, signed on the owner's machine with a key CI never holds, and checked by `install.sh` against that key before anything is installed. This page covers each step, the key itself, what the signature does and does not protect, and how to check a release by hand.

## What the binary never does

- It opens no network connection. No source file in either crate names a socket type, and [`no_network.rs`](https://github.com/jcrenshaw-dev/terminus/blob/main/crates/terminus-core/tests/no_network.rs) fails the test run if one ever does, comment lines included.
- It calls no model and reads no API key. A `terminus.toml` that still has a `[provider]` table fails to load with `<path> still has a [provider] table. Model calls were removed from Terminus, so nothing reads that table any more. Delete it from the file to continue.` ([`config.rs`](https://github.com/jcrenshaw-dev/terminus/blob/main/crates/terminus-core/src/config.rs)).
- The only text it hands to a model goes through Claude Code. Hooks and the MCP server write archived text into Claude Code's session, and Claude Code sends it to its model. With `[privacy] redact_recall` on, that text is filtered on the way out ([`egress.rs`](https://github.com/jcrenshaw-dev/terminus/blob/main/crates/terminus-core/src/observe/egress.rs)).

The one network use in installing or updating Terminus is `install.sh`'s `curl` downloads.

## Version numbers

From 1.0.0 on, each release bumps the middle number of the version, a release with only fixes bumps the last, and the first moves only for a large change. The number does not promise compatibility. The [changelog](https://github.com/jcrenshaw-dev/terminus/blob/main/CHANGELOG.md) says what each release changes.

## How a release is built

From [`.github/workflows/release.yml`](https://github.com/jcrenshaw-dev/terminus/blob/main/.github/workflows/release.yml):

1. Pushing a tag of the form `v<X.Y.Z>` starts the workflow. Nothing else does.
2. Four build jobs, each on its own runner, build `terminus-linux-x86_64`, `terminus-linux-aarch64`, `terminus-macos-x86_64` and `terminus-macos-aarch64` with `cargo build --release --locked` and no features. Each job checks that the binary's `--version` matches the tag, that a Linux binary is statically linked, that a macOS binary links nothing outside `/usr/lib/`, and that the binary carries the marker `install.sh` and `terminus install` look for.
3. Each build job writes the binary's manifest, `terminus-<os>-<arch>.manifest`, which is exactly two lines:

   ```
   version X.Y.Z
   sha256 <64 lowercase hex digits>
   ```

4. Each build job attests its binary with [`actions/attest`](https://github.com/actions/attest), a signed statement that `release.yml` built it from the tag on a GitHub-hosted runner.
5. The draft job runs only when all four builds pass. It checks that it holds exactly the four binaries and four manifests, and creates the release as a draft, never as a prerelease and never published.

The workflow's permissions are `contents: read`. The build jobs add `id-token: write` and `attestations: write`, which they use only to attest their binaries. The draft job alone has `contents: write`, and it uses only the workflow's own token. No workflow names a signing key, so CI can build and attest a draft but cannot sign one.

## How a release is signed

The owner runs [`scripts/sign-release.sh`](https://github.com/jcrenshaw-dev/terminus/blob/main/scripts/sign-release.sh) on his own machine as `sh scripts/sign-release.sh v<X.Y.Z>`. In order, it:

1. Refuses a tag that does not start with `v`.
2. Reads the public key from `install.sh`'s one `release_key=` line, and keeps no copy of its own.
3. Checks the tag with its own copy of `install.sh`'s `valid_version`. A test CI runs holds the signer's copies of `valid_version`, `digest` and the manifest check to `install.sh`'s text, so it refuses exactly what the installer refuses and runs none of `install.sh`.
4. Refuses a release that is not a draft, because signing a published release would replace signatures users already check against.
5. Downloads the draft and refuses it unless all four binaries and all four manifests are there.
6. Checks every binary's SHA-256 against its manifest, and that every manifest is exactly the two lines above with the tag's version on the first, before it signs anything. A manifest whose version differs from the tag is refused.
7. Checks every binary with `gh attestation verify`, before it signs anything, requiring an attestation from `.github/workflows/release.yml` in this repository, for the tag being signed, made on a GitHub-hosted runner. A binary CI did not build has none and is refused, even when its manifest matches it.
8. Signs and verifies one manifest at a time. It signs the manifest with the `crenshawdev-release` key under the namespace `terminus-release`, writing `terminus-<os>-<arch>.manifest.sig`, and verifies that signature with the same `ssh-keygen -Y verify` command `install.sh` runs before it signs the next.
9. Only when all four pass, uploads the four `.sig` files and publishes the release as the latest. A failed upload or publish leaves the release a draft.

## What install.sh checks

[`install.sh`](https://github.com/jcrenshaw-dev/terminus/blob/main/install.sh) makes these checks in this order. Each refusal prints `terminus: <reason>` to stderr and exits 1, and nothing is installed.

1. **The source.** With no argument it downloads from `https://github.com/jcrenshaw-dev/terminus/releases/latest/download`. One argument names another source, and it must start with `https://`. More than one argument is refused.
2. **`TERMINUS_BIN_DIR`.** Refused when it is set, because `terminus install` would point the hooks and the MCP entry into that folder while the script links `~/.local/bin/terminus`.
3. **`HOME`.** Refused when it is unset or empty, relative, not valid UTF-8, has a path part that is exactly `..`, or when the shell changes its bytes on copying it, which dash does when started without a UTF-8 locale.
4. **What sits at `~/.local/bin/terminus`.** Refused when it is a folder or a link to one, a file that is not a Terminus build, a link to nothing that does not point into `~/.local/lib/crenshawdev/terminus/versions/`, or anything other than a file or a link. Nothing there, a Terminus build, or a link to `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus` is fine.
5. **The OS and architecture.** Linux or macOS, on x86_64 or aarch64 (`arm64` counts as aarch64). Anything else is refused.
6. **`ssh-keygen`.** Refused when it is not on `PATH`. The message names the package: `openssh-client` on Debian and Ubuntu, `openssh-clients` on Fedora, `openssh` on Arch.
7. **The manifest and signature downloads.** `terminus-<os>-<arch>.manifest`, then `terminus-<os>-<arch>.manifest.sig`, each over HTTPS only with TLS 1.2 or later. A failed download is refused.
8. **The signature.** The manifest has to verify against the pinned key below, as `crenshawdev-release` under the namespace `terminus-release`. The script writes its own allowed-signers file from `release_key`, so no allowed-signers file of yours is read.
9. **The binary download.** `terminus-<os>-<arch>`, fetched only after the signature has verified.
10. **The manifest's two lines.** Refused unless line 1 is `version ` followed by `major.minor.patch` with no leading zeros and each part fitting a u64, line 2 is `sha256 ` followed by 64 lowercase hex digits, each line ends in a newline, and nothing follows line 2.
11. **The binary's SHA-256.** Refused unless it equals the manifest's.

Only then does it store the binary as `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, check `~/.local/bin/terminus` again, move that link to the new binary and run `terminus install`. Storing is refused when a link, something other than a regular file, or a copy of the same binary without execute permission is already at that path.

The script's body is one brace group, which the shell reads in full before running any of it. The lines before it only set options and variables and define functions, so a download cut off partway runs nothing.

## The pinned key

```
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMsYk5LI8lVbJqw1+hALUBas7hJP32dwNJOjPwQQ+prd
```

Identity `crenshawdev-release`, namespace `terminus-release`, fingerprint `SHA256:LjJFym0RMrQjeUVyDBsZk0P0vxahenqGV3x/MQmnG0U`. This is `install.sh`'s `release_key` line, and the only key a release may be signed with. The script never fetches it, and no argument, setting or environment variable changes it.

## What the signature protects

- Each manifest, signed by `crenshawdev-release` under `terminus-release`, and through the manifest's SHA-256 the binary it names.
- CI holds no signing key. The draft job uses only the workflow's own token, so nothing CI produces carries a signature.
- Only a draft is signed, so the signatures on a published release are never replaced.
- The signer checks each binary against its manifest before signing, so a draft whose binary and manifest disagree is never signed.
- The signer checks each binary's attestation before signing, so a binary and manifest swapped into the draft after CI built it are never signed, even by someone holding a token that can write to the repository.

## What the signature does not protect

- `install.sh` itself. It is fetched unsigned over HTTPS from `main`.
- An older, validly signed release served from a source you pass. It installs, because there is no version floor.
- Key loss or rotation. There is no revocation, and a new key means a new `install.sh`.
- Proof at install time that the binary was built from the tagged source. The signer checks the attestation before signing, and `install.sh` checks only the signature.
- A change to `release.yml` or the source, pushed and tagged by someone with write access to the repository. CI builds and attests that code like any other, so the attestation says where a binary was built, not who wrote what it was built from.

## Checking a release by hand

This repeats `install.sh`'s own check. `<os>` is `linux` or `macos`, and `<arch>` is `x86_64` or `aarch64`.

1. Download `terminus-<os>-<arch>`, `terminus-<os>-<arch>.manifest` and `terminus-<os>-<arch>.manifest.sig` from the release, for example:

   ```sh
   curl -fsSLO https://github.com/jcrenshaw-dev/terminus/releases/latest/download/terminus-<os>-<arch>
   curl -fsSLO https://github.com/jcrenshaw-dev/terminus/releases/latest/download/terminus-<os>-<arch>.manifest
   curl -fsSLO https://github.com/jcrenshaw-dev/terminus/releases/latest/download/terminus-<os>-<arch>.manifest.sig
   ```

2. Write an allowed-signers file holding this one line, with the full key:

   ```
   crenshawdev-release ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMsYk5LI8lVbJqw1+hALUBas7hJP32dwNJOjPwQQ+prd
   ```

3. Verify the manifest, with `<file>` the allowed-signers file:

   ```sh
   ssh-keygen -Y verify -f <file> -I crenshawdev-release -n terminus-release -s terminus-<os>-<arch>.manifest.sig < terminus-<os>-<arch>.manifest
   ```

   A pass prints `Good "terminus-release" signature for crenshawdev-release with ED25519 key SHA256:LjJFym0RMrQjeUVyDBsZk0P0vxahenqGV3x/MQmnG0U` and exits 0.

4. Compare the binary's SHA-256 with the hex on the manifest's second line:

   ```sh
   sha256sum terminus-<os>-<arch>          # Linux
   shasum -a 256 terminus-<os>-<arch>      # macOS
   sed -n 2p terminus-<os>-<arch>.manifest
   ```

Any other result, a verify that prints anything else or exits non-zero, or a hash that differs, means the file is not the one the key signed.
