# The npm workspace

Six packages live here, none of them published yet.

| Directory | Package | What it holds |
|---|---|---|
| `verbatim/` | `verbatim` | The thin package: a JS shim and a README, ~3 kB packed |
| `platforms/linux-x64/` | `@verbatim/linux-x64` | One prebuilt binary |
| `platforms/linux-arm64/` | `@verbatim/linux-arm64` | One prebuilt binary |
| `platforms/darwin-arm64/` | `@verbatim/darwin-arm64` | One prebuilt binary |
| `platforms/darwin-x64/` | `@verbatim/darwin-x64` | One prebuilt binary |
| `platforms/win32-x64/` | `@verbatim/win32-x64` | One prebuilt binary |

The thin package names the five platform packages as `optionalDependencies`.
Each of those declares `os` and `cpu`, so npm downloads the one that matches the
machine and skips the other four. `verbatim` itself carries no binary, which is
why `npx verbatim` costs one 3 kB download plus one 2.4 MB binary rather than
five.

**No package here has a `scripts` key.** Not a `postinstall`, not a `preinstall`,
not an `install`, not an empty one. Downloading Verbatim executes nothing;
`verbatim install` is a command the user runs. `pack-local.sh` asserts this
against the manifest inside each tarball, which is the file a stranger's npm
actually reads.

The shim is the only JavaScript this project ships. npm's `bin` field can only
name a path inside the declaring package, so it cannot point at a binary in an
optional dependency; the shim resolves the platform package and execs the binary.
Node is on the install path only. Once `verbatim install` has copied the binary
to its stable path, Claude Code's hooks call that path directly.

Two things the manifests depend on and a future edit could quietly break:

- The platform packages declare **no `exports` field**. The shim resolves them
  with `require.resolve("@verbatim/<platform>/package.json")`, and an `exports`
  field would hide that subpath.
- The platform packages declare **no `bin` field**. The thin package already
  claims the `verbatim` bin name, and a second claimant on the same name is a
  collision that buys nothing.

## Proving it locally

```
bash npm/pack-local.sh
```

That is AC9 end to end: build `--release`, stage the binary into this platform's
package, pack both packages, install the tarballs into a throwaway prefix, and
assert the installed `verbatim --version` is the version just built. It exits
non-zero on any failure and removes its prefix and its tarballs on the way out.
The staged binary under `npm/platforms/<platform>/bin/` is left behind and is
gitignored.

Only the host platform can be proven this way. The other four manifests pack
fine, but their binaries arrive with cross-compilation, which is a later shipping
step.

## What the local proof does not cover

The script installs the platform tarball into the prefix first and the thin
tarball second with `--omit=optional`. It has to: npm resolves
`optionalDependencies` from the registry, and these packages are not on the
registry yet, so an ordinary install of the thin tarball alone would find nothing
to satisfy them with.

The consequence is a real gap. The local proof exercises **the shim's
resolution** - that `require.resolve` finds the platform package wherever npm put
it, that the binary runs, that arguments and exit codes pass through. It does not
exercise **npm's `os`/`cpu` selection**, because nothing here ever asks npm to
choose among five candidates; the one candidate is placed by hand. That selection
is only exercised by a registry install, and it belongs to the publish step.

Two other things this workspace has not settled, both of them publish-step
questions:

- The npm name `verbatim` and the `@verbatim` scope have not been checked for
  availability. `crates.io`'s `verbatim` is already taken and the fallback there
  is `verbatim-cli`; npm may need the same treatment.
- Publishing itself: version bumping, signing, release CI and cross-compiled
  binaries for the four platforms that have none. None of that lives here.
