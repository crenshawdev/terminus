#!/usr/bin/env bash
#
# The acceptance check for the npm package (AC9), start to finish, on this
# machine: build the release binary, stage it into this platform's package, pack
# both packages, install the tarballs into a throwaway prefix, and prove the
# installed `verbatim --version` is the version that was just built.
#
# It is an assertion, not a demo. Every step is checked, any failure exits
# non-zero with a line saying which one, and the prefix and the tarballs are gone
# by the time it returns.
#
# What it does NOT prove is in npm/README.md, and the gap is deliberate: a local
# tarball install exercises the shim's resolution but not npm's `os`/`cpu`
# selection of an optionalDependency, which only a registry install can.

set -euo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

fail() {
  printf 'pack-local.sh: %s\n' "$1" >&2
  exit 1
}

step() {
  printf '\n== %s\n' "$1"
}

# ---------------------------------------------------------------- the platform

# Which package this machine can actually stage a binary into. The other four
# manifests pack fine but have no binary until cross-compilation exists, so
# naming an unsupported host here is clearer than packing something empty.
case "$(uname -s)" in
  Linux) HOST_OS=linux ;;
  Darwin) HOST_OS=darwin ;;
  *) fail "no platform package for $(uname -s); this script proves AC9 on Linux and macOS only" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) HOST_ARCH=x64 ;;
  arm64 | aarch64) HOST_ARCH=arm64 ;;
  *) fail "no platform package for $(uname -m)" ;;
esac
PLATFORM="$HOST_OS-$HOST_ARCH"
PLATFORM_DIR="$REPO/npm/platforms/$PLATFORM"
[ -f "$PLATFORM_DIR/package.json" ] || fail "no manifest at npm/platforms/$PLATFORM"

# --------------------------------------------------------------- the build

step "building the release binary"
cargo build --release

# Ask cargo where it put things rather than assuming ./target: CARGO_TARGET_DIR
# and a `build.target-dir` in any cargo config both move it, and this machine
# sets the former. Guessing would copy a stale binary or none at all.
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps |
  node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.parse(s).target_directory))')"
BINARY="$TARGET_DIR/release/verbatim"
[ -x "$BINARY" ] || fail "no release binary at $BINARY"

EXPECTED_VERSION="$("$BINARY" --version)"
printf '   %s is %s\n' "$BINARY" "$EXPECTED_VERSION"

step "staging the binary into @verbatim/$PLATFORM"
mkdir -p "$PLATFORM_DIR/bin"
# -m 755 in the copy itself: npm preserves the mode it finds, and a binary packed
# without its executable bit installs as a file the shim cannot spawn.
install -m 755 "$BINARY" "$PLATFORM_DIR/bin/verbatim"

# --------------------------------------------------------------- pack

# Everything transient lives under one temp dir so cleanup is one removal that
# runs on success, on failure and on interrupt alike.
WORK="$(mktemp -d)"
cleanup() {
  local status=$?
  rm -rf "$WORK"
  if [ "$status" -ne 0 ]; then
    printf '\npack-local.sh: FAILED (exit %s)\n' "$status" >&2
  fi
}
trap cleanup EXIT

PREFIX="$WORK/prefix"
mkdir -p "$WORK/platform" "$WORK/thin"

step "packing both packages"
# One tarball per directory, so the filename comes from a glob and this script
# never has to predict how npm mangles a scoped name.
npm pack --pack-destination "$WORK/platform" "$PLATFORM_DIR"
npm pack --pack-destination "$WORK/thin" "$REPO/npm/verbatim"
PLATFORM_TGZ="$(echo "$WORK"/platform/*.tgz)"
THIN_TGZ="$(echo "$WORK"/thin/*.tgz)"
[ -f "$PLATFORM_TGZ" ] || fail "npm pack produced no tarball for $PLATFORM_DIR"
[ -f "$THIN_TGZ" ] || fail "npm pack produced no tarball for npm/verbatim"

step "checking the packed manifests"
# INST-01: no install-time script of any kind. Checked against the manifest npm
# actually put in the tarball, not the one in the tree, because that is the file
# a stranger's npm will read. Any `scripts` key at all fails - stricter than
# "no postinstall" on purpose, since there is no script this package needs.
for tgz in "$PLATFORM_TGZ" "$THIN_TGZ"; do
  tar -xzOf "$tgz" package/package.json |
    node -e '
      let s = "";
      process.stdin.on("data", (d) => (s += d)).on("end", () => {
        const m = JSON.parse(s);
        if (m.scripts) {
          console.error(`${m.name} declares scripts: ${Object.keys(m.scripts).join(", ")}`);
          process.exit(1);
        }
        console.log(`   ${m.name}@${m.version}: no scripts`);
      });
    ' || fail "$(basename "$tgz") carries an install-time script"
done

step "checking what the thin package ships"
THIN_LIST="$(tar -tzf "$THIN_TGZ")"
printf '%s\n' "$THIN_LIST" | sed 's/^/   /'
printf '%s\n' "$THIN_LIST" | grep -qx 'package/bin/verbatim.js' ||
  fail "the thin package is missing its shim"
printf '%s\n' "$THIN_LIST" | grep -qx 'package/README.md' ||
  fail "the thin package is missing its README"
# The thin package is thin: the 5 MB binary belongs to the platform package, and
# a `files` list that started matching it would double every install.
! printf '%s\n' "$THIN_LIST" | grep -qx 'package/bin/verbatim' ||
  fail "the thin package ships a binary; it must ship only the shim"

# --------------------------------------------------------------- install

step "installing both tarballs into a throwaway prefix"
# Order matters, and this is the one thing a local proof cannot borrow from a
# registry install. npm resolves optionalDependencies from the registry, where
# these packages do not exist yet, so the platform tarball goes in first and the
# thin one follows with its optional dependencies omitted - leaving the shim to
# resolve a platform package already sitting in node_modules.
npm install --global --prefix "$PREFIX" --no-audit --no-fund "$PLATFORM_TGZ"
npm install --global --prefix "$PREFIX" --no-audit --no-fund --omit=optional "$THIN_TGZ"

INSTALLED="$PREFIX/bin/verbatim"
[ -x "$INSTALLED" ] || fail "npm installed no executable at $INSTALLED"

step "running the installed verbatim"
ACTUAL_VERSION="$("$INSTALLED" --version)"
printf '   %s --version is %s\n' "$INSTALLED" "$ACTUAL_VERSION"
[ "$ACTUAL_VERSION" = "$EXPECTED_VERSION" ] ||
  fail "installed version $ACTUAL_VERSION is not the built version $EXPECTED_VERSION"

printf '\npack-local.sh: OK - installed verbatim %s from a packed tarball, no install script ran\n' "$ACTUAL_VERSION"
