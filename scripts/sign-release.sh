#!/bin/sh
# Sign and publish a draft release of Terminus, on the owner's machine:
#   sh scripts/sign-release.sh v<X.Y.Z>
# It refuses a release that is not a draft, downloads the draft's four binaries and four manifests, checks each binary
# against its manifest and against the attestation release.yml made when it built it,
# signs each manifest with ~/.ssh/crenshawdev-release and verifies each signature the
# way install.sh does. Only when all four pass does it upload the .sig files and
# publish the release as latest.
# The public key is read from install.sh's release_key line as a value; this
# script keeps no copy of it. valid_version, digest and read_manifest are copies of
# install.sh's, so it refuses the versions and the manifests the installer refuses
# without running any code out of install.sh. A test fails when a copy differs from
# install.sh's.
set -eu

LC_ALL=C
export LC_ALL

repo=crenshawdev/terminus
platforms='linux-x86_64 linux-aarch64 macos-x86_64 macos-aarch64'
usage='usage: sh scripts/sign-release.sh v<X.Y.Z>'

fail() {
    printf 'terminus: %s\n' "$*" >&2
    exit 1
}

digest() {
    if command -v sha256sum >/dev/null 2>&1; then
        sum=$(sha256sum) || fail 'cannot compute SHA-256'
    else
        sum=$(shasum -a 256) || fail 'cannot compute SHA-256 with shasum -a 256'
    fi
    printf '%s\n' "${sum%% *}"
}

valid_version() (
    case $1 in '' | *[!0-9.]* | .* | *. | *..*) return 1 ;; esac
    set -f
    IFS=.
    set -- $1
    [ "$#" -eq 3 ] || return 1
    for part do
        case $part in 0?*) return 1 ;; esac
        [ "${#part}" -le 20 ] || return 1
        if [ "${#part}" -eq 20 ]; then
            # Compare halves without overflowing the shell's signed integer.
            high=${part%??????????}
            low=${part#??????????}
            [ "$high" -le 1844674407 ] || return 1
            if [ "$high" -eq 1844674407 ]; then
                [ "$low" -le 3709551615 ] || return 1
            fi
        fi
    done
)

read_manifest() {
    {
        IFS= read -r version_line || fail 'manifest line 1 is missing or has no final newline'
        IFS= read -r digest_line || fail 'manifest line 2 is missing or has no final newline'
        extra=
        if IFS= read -r extra || [ -n "$extra" ]; then
            fail 'manifest has content after line 2'
        fi
    } < "$1"
    case $version_line in
        'version '*) version=${version_line#version } ;;
        *) fail 'manifest line 1 must start with "version "' ;;
    esac
    valid_version "$version" || fail 'manifest line 1 needs major.minor.patch with no leading zeros and each part fitting u64'
    case $digest_line in
        'sha256 '*) sha256=${digest_line#sha256 } ;;
        *) fail 'manifest line 2 must start with "sha256 "' ;;
    esac
    case $sha256 in *[!0-9a-f]*) fail 'manifest line 2 needs 64 lowercase hex digits' ;; esac
    [ "${#sha256}" -eq 64 ] || fail 'manifest line 2 needs 64 lowercase hex digits'
    # Some shells discard NUL bytes during read, so compare the exact text too.
    manifest_digest=$(digest < "$1")
    text_digest=$(printf 'version %s\nsha256 %s\n' "$version" "$sha256" | digest)
    [ "$manifest_digest" = "$text_digest" ] || fail 'manifest is not exactly two UTF-8 lines'
}

[ "$#" -eq 1 ] || fail "$usage"
tag=$1
case $tag in v*) ;; *) fail "$usage" ;; esac
tag_version=${tag#v}

case $0 in */*) here=${0%/*} ;; *) here=. ;; esac
install_sh=$here/../install.sh
[ -f "$install_sh" ] || fail "$install_sh: not found; the release key is read from it"
count=$(grep -c -e '^release_key=' "$install_sh") || :
[ "$count" = 1 ] || fail "$install_sh: needs exactly one release_key= line, found ${count:-none}"
line=$(grep -e '^release_key=' "$install_sh")
key=${line#release_key=\'}
key=${key%\'}
case $key in '' | *"'"*) fail "$install_sh: the release_key= line is not release_key='<key>'" ;; esac
[ "$line" = "release_key='$key'" ] || fail "$install_sh: the release_key= line is not release_key='<key>'"

# The tag is checked with this script's own valid_version, a copy of install.sh's,
# so it refuses exactly what the installer refuses.
valid_version "$tag_version" || fail "$usage"

# Only a draft is signed. On a published release the upload would replace the
# signatures users already verify against, and the edit would mark it latest.
is_draft=$(gh release view "$tag" --repo "$repo" --json isDraft --jq .isDraft) ||
    fail "cannot read the release $tag from $repo"
[ "$is_draft" = true ] ||
    fail "the release $tag is not a draft; only a draft is signed and published"

work=$(mktemp -d)
trap 'rm -rf "$work"' 0
trap 'exit 1' HUP INT TERM

gh release download "$tag" --repo "$repo" --dir "$work" ||
    fail "cannot download the draft $tag from $repo"

for platform in $platforms; do
    for name in "terminus-$platform" "terminus-$platform.manifest"; do
        [ -f "$work/$name" ] || fail "the draft $tag has no $name"
    done
done

# Every binary and manifest is checked before anything is signed, so a bad draft
# never asks for the passphrase. A manifest install.sh would refuse is refused here
# with install.sh's own message.
for platform in $platforms; do
    binary=terminus-$platform
    actual=$(digest < "$work/$binary")
    read_manifest "$work/$binary.manifest"
    [ "$sha256" = "$actual" ] ||
        fail "$binary: SHA-256 mismatch, the binary is $actual and its manifest names another value"
    [ "$version" = "$tag_version" ] ||
        fail "$binary.manifest: not exactly the lines version $tag_version and sha256 $actual"
done

# A binary and manifest swapped into the draft agree with each other, so each
# binary must also carry the attestation release.yml made for this tag on a
# GitHub-hosted runner. Only that workflow can make one.
for platform in $platforms; do
    binary=terminus-$platform
    gh attestation verify "$work/$binary" --repo "$repo" \
        --signer-workflow "$repo/.github/workflows/release.yml" \
        --source-ref "refs/tags/$tag" --deny-self-hosted-runners > /dev/null ||
        fail "$binary: no attestation from release.yml at $tag, so it is not the binary CI built"
done

printf 'crenshawdev-release %s\n' "$key" > "$work/allowed_signers"
for platform in $platforms; do
    manifest=$work/terminus-$platform.manifest
    # The manifest goes in on stdin, because ssh-keygen given a file name stops at
    # "Overwrite (y/n)?" once a .sig exists. With stdin not a terminal, ssh-keygen
    # tries ssh-askpass on a graphical desktop; never keeps the prompt on the terminal.
    SSH_ASKPASS_REQUIRE=never ssh-keygen -Y sign -f "$HOME/.ssh/crenshawdev-release" -n terminus-release \
        < "$manifest" > "$manifest.sig" ||
        fail "cannot sign terminus-$platform.manifest"
    # The same command install.sh runs.
    ssh-keygen -Y verify -f "$work/allowed_signers" -I crenshawdev-release -n terminus-release \
        -s "$manifest.sig" < "$manifest" > /dev/null 2>&1 ||
        fail "the signature on terminus-$platform.manifest does not verify against the key pinned in install.sh"
done

set --
for platform in $platforms; do
    set -- "$@" "$work/terminus-$platform.manifest.sig"
done
# --clobber lets a rerun after a failed publish replace its own earlier signatures.
gh release upload "$tag" "$@" --repo "$repo" --clobber ||
    fail "uploading the signatures to $tag failed; the release is still a draft"
gh release edit "$tag" --repo "$repo" --draft=false --prerelease=false --latest ||
    fail "publishing $tag failed; the signatures are uploaded and the release is still a draft"

printf 'terminus: published %s as the latest release of %s\n' "$tag" "$repo"
