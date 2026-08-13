---
phase: 4
plan: 5
requirements:
  - INST-01
files:
  - npm/README.md
  - npm/pack-local.sh
  - npm/verbatim/package.json
  - npm/verbatim/README.md
  - npm/verbatim/bin/verbatim.js
  - npm/platforms/linux-x64/package.json
  - npm/platforms/linux-arm64/package.json
  - npm/platforms/darwin-arm64/package.json
  - npm/platforms/darwin-x64/package.json
  - npm/platforms/win32-x64/package.json
  - .gitignore
---

# Phase 4: Hooks And Install - Plan 5 of 5 (the npm package)

**PARALLEL: this plan shares no file with PLAN-1 through PLAN-4.** It writes only
`npm/**` and `.gitignore`; it touches no Rust source, no `Cargo.toml` and no
test the other plans write. It may run at any point in the phase.

## Goal

The way a Claude Code user gets verbatim is `npx verbatim`, from a package that
runs no install-time script and ships one prebuilt binary per platform, proven
here by packing it and installing the tarball on this machine.

## Must be true when done

- `npm pack` on the in-repo workspace produces tarballs whose manifests carry no
  `postinstall` script of any kind (AC9, INST-01, PROJECT.md's distribution
  constraint).
- Installing the packed tarball puts a working `verbatim` on the path whose
  `--version` matches the Rust build's (AC9).
- The thin package's `bin` is a JS shim, and the per-platform binaries are
  `optionalDependencies` selected by `os` and `cpu` (D-10).
- The shim passes every argument through and exits with the binary's own exit
  code, so `npx verbatim install` and `npx verbatim search foo` behave as the
  binary does.
- A platform with no matching package fails with a message naming the package it
  looked for, not a stack trace.

## Context

- D-21 fixes the scope: the package is a workspace directory in this repo, proven
  by `npm pack` plus a local tarball install. Cross-compilation, release CI and
  any registry publish are a later shipping step and must not appear here. `node`
  v26.7.0 and `npm` 12.0.2 are on this machine; the release binary is 4,969,688
  bytes, 2.15 MB gzipped.
- D-10 fixes the shape: npm's `bin` field only references paths inside the
  declaring package, so the thin package's `bin` cannot point into an
  optionalDependency and must be a shim that resolves the platform package and
  execs the binary. This is the esbuild pattern the design brief names.
- D-09 is why the shim matters beyond convenience: `verbatim install` copies the
  running executable to the stable path, so the shim exec'ing the platform binary
  is what makes the running process the copy source.
- Out of scope: publishing, version bumping, signing, and any GitHub-release,
  Homebrew or Scoop channel.

## Tasks

### Task 1: The thin package and the per-platform packages

- **Files:** npm/verbatim/package.json, npm/verbatim/README.md,
  npm/platforms/linux-x64/package.json, npm/platforms/linux-arm64/package.json,
  npm/platforms/darwin-arm64/package.json, npm/platforms/darwin-x64/package.json,
  npm/platforms/win32-x64/package.json, .gitignore
- **Action:** Write the thin package's manifest: version `0.1.0` matching the
  Cargo workspace version, license `Apache-2.0`, the repository field, a `bin`
  entry mapping `verbatim` to `bin/verbatim.js`, `files` listing only the shim and
  the README, and `optionalDependencies` naming the five per-platform packages at
  the exact same version. It must carry no `scripts.postinstall` and no
  `scripts.preinstall` and no `scripts.install` - not an empty one, not a no-op
  one (INST-01). That single omission is what sidesteps the blocked-postinstall
  failure class entirely, and it is the reason install is a command the user runs
  rather than something that happens to them.
  Write one manifest per platform package, each declaring `os` and `cpu` so npm
  skips the ones that do not apply, each with `files` naming its single binary,
  and each with the same version. Use the design brief's naming
  (`@verbatim/linux-x64` and its siblings) unless a better one is chosen at
  publish time - the name is not settled by this phase.
  Add `.gitignore` entries for the staged binaries inside the platform packages
  and for `*.tgz` under `npm/`, so a 5 MB build artifact is never committed.
- **Verify:** `node -e "const p=require('./npm/verbatim/package.json');
  if (p.scripts) { for (const k of Object.keys(p.scripts)) if (/install/.test(k))
  throw new Error(k) }"` exits 0; `npm pkg get version --prefix npm/verbatim`
  prints the same version as `cargo pkgid -p verbatim`; every platform manifest
  parses and declares `os` and `cpu`; `git status --porcelain` after staging a
  binary into a platform package shows nothing untracked.

### Task 2: The shim that finds and execs the binary

- **Files:** npm/verbatim/bin/verbatim.js
- **Action:** A Node script that maps `process.platform` and `process.arch` to a
  platform package name, resolves that package through `require.resolve` of its
  manifest so it works wherever npm placed it rather than assuming a hoisted
  `node_modules` layout, joins the binary's filename (`verbatim.exe` on Windows,
  `verbatim` elsewhere), and runs it with every argument after the script passed
  through, stdio inherited, exiting with the child's own exit code and reporting a
  signal death rather than swallowing it. Node is install-time only; nothing at
  runtime touches it, because the hooks and the MCP registration name the stable
  path directly (D-08).
  A platform with no matching package, or a package present but missing its
  binary, exits non-zero with one line naming the package it looked for and the
  platform-arch pair it derived - never a stack trace, which is what a user
  reports as a crash. Make the file executable and give it a shebang: npm creates
  the bin link from the manifest, but a shim without one fails when invoked
  directly.
- **Verify:** With a platform package staged and linked, `node
  npm/verbatim/bin/verbatim.js --version` prints the same string as
  `target/release/verbatim --version`; `node npm/verbatim/bin/verbatim.js
  nonsense-command` exits 2 the way the binary does; running the shim with the
  platform package removed exits non-zero and prints one line containing the
  expected package name and no `at Object.<anonymous>` frame.

### Task 3: Pack it, install the tarball, prove the version

- **Files:** npm/pack-local.sh, npm/README.md
- **Action:** A script that performs AC9 end to end on this machine: build
  `--release`, copy `target/release/verbatim` into this platform's package
  directory with its executable bit, `npm pack` both that package and the thin
  one, then install into a throwaway prefix and run the installed `verbatim
  --version`.
  The install order matters and is the one thing a local proof cannot borrow from
  a registry install: npm resolves `optionalDependencies` from the registry, where
  these packages do not exist yet, so install the platform tarball into the prefix
  first and then the thin tarball with optional dependencies omitted, leaving the
  shim to resolve the platform package already sitting in `node_modules`. Say in
  `npm/README.md` that this is what the local proof does and that it therefore
  exercises the shim's resolution but not npm's optional-dependency selection,
  which only a registry install exercises - that gap is a known one and belongs to
  the publish step, not to this phase (D-21).
  The script must clean up its prefix and its tarballs, and must fail loudly with
  a non-zero exit if any step fails, so it is usable as the acceptance check
  rather than as a demo.
- **Verify:** `bash npm/pack-local.sh` exits 0 and its output shows the installed
  `verbatim --version` string equal to `target/release/verbatim --version`;
  `tar -tzf` on the thin package's tarball lists the shim and the README and no
  binary; `npm pack --dry-run --json --prefix npm/verbatim | jq -e
  '.[0].entryCount > 0 and (.[0].name | test("verbatim"))'` succeeds; grepping the
  packed manifests for `postinstall` finds nothing.

## Notes

- The npm name `verbatim` and the `@verbatim` scope have not been checked for
  availability. `crates.io`'s `verbatim` is already taken (`.planning/PROJECT.md`
  records `verbatim-cli` as the fallback there), so the npm name may need the same
  treatment. It is a publish-step question and D-21 puts publishing outside this
  phase - flagged for the human rather than decided here.
- Only `linux-x64` can be staged with a real binary on this machine. The other
  four manifests exist and are packable, but their binaries arrive with
  cross-compilation, which D-21 puts in a later shipping step. AC9 is satisfied on
  the host platform.
