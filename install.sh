#!/bin/sh
# Install Terminus. No release is published yet.
# With no argument it downloads from the GitHub releases on crenshawdev/terminus;
# one https:// argument names another source.
# Before it reads the manifest, it checks terminus-<os>-<arch>.manifest.sig with
# ssh-keygen against the crenshawdev release key pinned below, under the namespace
# terminus-release. Nothing overrides the key, and a release it does not verify is refused.
# The script ends by running terminus install through the stable path, which writes Claude Code's wiring.
# The body is one brace group, which the shell reads whole before running any of it,
# so a download cut off before the closing brace runs nothing. Run flat, a copy cut after
# the link moves would leave the new version linked, never run terminus install, and exit 0.
set -eu

LC_ALL=C
export LC_ALL

# The same text as MARKER in crates/terminus/src/cmd/install/binary.rs; a test ties the two.
marker='terminus-stable-binary:8f3d1a6c-72b4-4e59-9c0d-5a1e7b34f082'

# The public half of the crenshawdev release key, the only key a release may be signed with.
release_key='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q'

fail() {
    printf 'terminus: %s\n' "$*" >&2
    exit 1
}

# The bytes come from od as numbers. dash takes its locale once at startup, so
# under a UTF-8 locale the LC_ALL=C above never reaches its own patterns.
valid_utf8() (
    bytes=$(printf '%s' "$1" | od -An -v -tu1) || return 1
    need=0 low=128 high=191
    for byte in $bytes; do
        if [ "$need" -gt 0 ]; then
            [ "$byte" -ge "$low" ] && [ "$byte" -le "$high" ] || return 1
            need=$((need - 1)) low=128 high=191
        elif [ "$byte" -lt 128 ]; then
            :
        elif [ "$byte" -lt 194 ]; then
            return 1
        elif [ "$byte" -lt 224 ]; then
            need=1
        elif [ "$byte" -lt 240 ]; then
            need=2
            [ "$byte" -ne 224 ] || low=160
            [ "$byte" -ne 237 ] || high=159
        elif [ "$byte" -lt 245 ]; then
            need=3
            [ "$byte" -ne 240 ] || low=144
            [ "$byte" -ne 244 ] || high=143
        else
            return 1
        fi
    done
    [ "$need" -eq 0 ]
)

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

digest() {
    if command -v sha256sum >/dev/null 2>&1; then
        sum=$(sha256sum) || fail 'cannot compute SHA-256'
    else
        sum=$(shasum -a 256) || fail 'cannot compute SHA-256 with shasum -a 256'
    fi
    printf '%s\n' "${sum%% *}"
}

verify_download() {
    actual=$(digest < "$scratch/terminus")
    [ "$actual" = "$sha256" ] ||
        fail "download SHA-256 is $actual, but the manifest names $sha256"
}

# Whatever carries the marker is a terminus build and may be replaced, as
# terminus install decides it. A link to nothing is replaced only when it points
# into the versions folder, which is what is left after that folder is deleted.
check_stable_path() {
    if [ -d "$stable" ]; then
        fail "$stable: a folder is there, or a symbolic link to one"
    elif [ -L "$stable" ] && [ ! -e "$stable" ]; then
        target=$(readlink "$stable") || fail "$stable: cannot read symbolic link"
        rest=${target#"$versions/"}
        folder=${rest%%/*}
        file=${rest#*/}
        if [ "$rest" = "$target" ] || [ "$file" != terminus ] || ! valid_version "$folder"; then
            fail "$stable: symbolic link to nothing, outside $versions/ ($target)"
        fi
    elif [ -f "$stable" ]; then
        grep -q -F -e "$marker" "$stable" ||
            fail "$stable: a file that is not a terminus build is there"
    elif [ -e "$stable" ] || [ -L "$stable" ]; then
        fail "$stable: an entry other than a file or a symbolic link is there"
    fi
}

stage() {
    mkdir -p "$versions/$version"
    staging_temporary=$(mktemp "$versions/$version/.terminus.XXXXXXXXXX")
    chmod 755 "$scratch/terminus"
    mv "$scratch/terminus" "$staging_temporary"
    mv -f "$staging_temporary" "$staged"
    staging_temporary=
}

{
    case $# in
        0) source=https://github.com/crenshawdev/terminus/releases/latest/download ;;
        1) case $1 in https://*) source=$1 ;; *) fail 'the source must start with https://' ;; esac ;;
        *) fail 'usage: sh install.sh [https://<source>]' ;;
    esac
    while [ "${source%/}" != "$source" ]; do source=${source%/}; done

    [ -z "${TERMINUS_BIN_DIR-}" ] ||
        fail "TERMINUS_BIN_DIR is set: terminus install would point the hooks and the MCP entry into $TERMINUS_BIN_DIR while this script links ~/.local/bin/terminus; unset it and run again"

    case ${HOME-} in
        '') fail 'HOME is unset or empty; set it to your absolute home folder' ;;
        /*) ;;
        *) fail 'HOME is relative; set it to your absolute home folder' ;;
    esac
    valid_utf8 "$HOME" || fail 'HOME is not valid UTF-8; set it to a path that is'
    # dash 0.5.13 started without a UTF-8 locale adds a byte before some UTF-8
    # bytes each time it copies a value, and every path below is a copy of HOME.
    home_copy=$HOME
    [ "$home_copy" = "$HOME" ] && [ "$(printf '%s' "$HOME")" = "$HOME" ] ||
        fail 'this shell changes some bytes of HOME when it copies them (dash without a UTF-8 locale); run the script with LANG=C.UTF-8 sh, or with bash'

    # Spell HOME without empty or . parts and without following folder links, so a
    # link this script made reads back as $versions/<version>/terminus.
    install_home=
    remaining=$HOME/
    while [ -n "$remaining" ]; do
        component=${remaining%%/*}
        remaining=${remaining#*/}
        case $component in
            '' | .) ;;
            ..) fail 'HOME contains ..; set it to an absolute path without ..' ;;
            *) install_home=$install_home/$component ;;
        esac
    done
    versions=$install_home/.local/lib/crenshawdev/terminus/versions
    stable_folder=$install_home/.local/bin
    stable=$stable_folder/terminus
    check_stable_path

    case $(uname -s) in
        Linux) os=linux ;;
        Darwin) os=macos ;;
        *) fail 'unsupported operating system; Linux or Darwin is required' ;;
    esac
    case $(uname -m) in
        x86_64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) fail 'unsupported architecture; x86_64 or aarch64 is required' ;;
    esac

    scratch=$(mktemp -d)
    staging_temporary=
    link_temporary=
    cleanup() {
        [ -z "$staging_temporary" ] || rm -f "$staging_temporary"
        [ -z "$link_temporary" ] || rm -f "$link_temporary"
        rm -rf "$scratch"
    }
    trap cleanup 0
    trap 'exit 1' HUP INT TERM

    curl -q --proto '=https' --tlsv1.2 -fsSL "$source/terminus-$os-$arch.manifest" -o "$scratch/manifest" ||
        fail "manifest download failed: $source/terminus-$os-$arch.manifest"
    curl -q --proto '=https' --tlsv1.2 -fsSL "$source/terminus-$os-$arch.manifest.sig" -o "$scratch/manifest.sig" ||
        fail "signature download failed: $source/terminus-$os-$arch.manifest.sig"

    # ssh-keygen reads the signed text only from stdin, and its own output is dropped.
    printf 'crenshawdev-release %s\n' "$release_key" > "$scratch/allowed_signers"
    ssh-keygen -Y verify -f "$scratch/allowed_signers" -I crenshawdev-release -n terminus-release \
        -s "$scratch/manifest.sig" < "$scratch/manifest" > /dev/null 2>&1 ||
        fail 'the manifest does not verify against the pinned crenshawdev release key'

    curl -q --proto '=https' --tlsv1.2 -fsSL "$source/terminus-$os-$arch" -o "$scratch/terminus" ||
        fail "binary download failed: $source/terminus-$os-$arch"

    {
        IFS= read -r version_line || fail 'manifest line 1 is missing or has no final newline'
        IFS= read -r digest_line || fail 'manifest line 2 is missing or has no final newline'
        extra=
        if IFS= read -r extra || [ -n "$extra" ]; then
            fail 'manifest has content after line 2'
        fi
    } < "$scratch/manifest"
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
    manifest_digest=$(digest < "$scratch/manifest")
    text_digest=$(printf 'version %s\nsha256 %s\n' "$version" "$sha256" | digest)
    [ "$manifest_digest" = "$text_digest" ] || fail 'manifest is not exactly two UTF-8 lines'

    verify_download

    # A rebuild at the same version is renamed over the old file, and a process
    # running that file keeps its inode. Old version folders are never deleted.
    umask 022
    staged=$versions/$version/terminus
    if [ -L "$staged" ]; then
        fail "$staged: staging conflict, a symbolic link is there"
    elif [ -f "$staged" ]; then
        actual=$(digest < "$staged")
        if [ "$actual" = "$sha256" ]; then
            [ -x "$staged" ] || fail "$staged: staging conflict, the file is not executable"
        else
            stage
        fi
    elif [ -e "$staged" ]; then
        fail "$staged: staging conflict, an entry other than a regular file is there"
    else
        stage
    fi

    mkdir -p "$stable_folder"
    temporary=$stable_folder/.${scratch##*/}
    ln -s "$staged" "$temporary"
    [ -L "$temporary" ] && [ "$(readlink "$temporary")" = "$staged" ] ||
        fail "$temporary: temporary symbolic link does not point to $staged"
    link_temporary=$temporary
    check_stable_path
    mv -f "$link_temporary" "$stable"
    link_temporary=

    printf 'terminus: placed version %s at %s\n' "$version" "$stable"
    printf 'terminus: versions present in %s:\n' "$versions"
    for folder in "$versions"/*; do
        [ -d "$folder" ] && [ ! -L "$folder" ] || continue
        [ -f "$folder/terminus" ] && [ ! -L "$folder/terminus" ] || continue
        present=${folder##*/}
        valid_version "$present" || continue
        printf '  %s\n' "$present"
    done

    "$stable" install
}
