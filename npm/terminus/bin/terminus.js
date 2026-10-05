#!/usr/bin/env node
"use strict";

// The npm entry point for terminus, and the only JavaScript this project ships.
//
// npm's `bin` field can only name a path inside the declaring package, so it
// cannot point at a binary that lives in an optionalDependency (D-10). This shim
// is what closes that gap: it finds the per-platform package npm selected by
// `os` and `cpu`, execs the real binary, and gets out of the way.
//
// Node is install-time only. `terminus install` copies the RUNNING executable to
// the stable path (D-09), so exec'ing the platform binary here is exactly what
// makes the running process the copy source. After that the hooks and the MCP
// registration name the stable path directly (D-08), and nothing on the runtime
// path ever loads Node again - which is the point, because cold start is the
// product.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

// Every platform we publish a binary for, keyed the way process reports itself.
// A key missing here means a platform we do not build at all, which is a
// different failure from "built but not installed" and earns a different line.
const PACKAGES = {
  "linux-x64": "@terminus/linux-x64",
  "linux-arm64": "@terminus/linux-arm64",
  "darwin-arm64": "@terminus/darwin-arm64",
  "darwin-x64": "@terminus/darwin-x64",
  "win32-x64": "@terminus/win32-x64",
};

// One line on stderr and a non-zero exit - never a throw. An uncaught exception
// here prints a stack trace, and a stack trace is what a user reports as a crash
// in the tool rather than as a platform that has no build yet.
function fail(message) {
  process.stderr.write(`terminus: ${message}\n`);
  process.exit(1);
}

const target = `${process.platform}-${process.arch}`;
const pkg = PACKAGES[target];
if (!pkg) {
  fail(
    `no prebuilt binary for ${target} - terminus publishes ${Object.keys(PACKAGES).join(", ")}`,
  );
}

// Resolve through the platform package's own manifest instead of guessing at a
// node_modules layout: npm may hoist it, nest it under this package, or place it
// at a workspace root, and require.resolve answers wherever it actually landed.
// This works because the platform packages deliberately declare no `exports`
// field - adding one would hide `./package.json` and break this line.
let binary;
try {
  binary = path.join(
    path.dirname(require.resolve(`${pkg}/package.json`)),
    "bin",
    process.platform === "win32" ? "terminus.exe" : "terminus",
  );
} catch {
  fail(
    `${pkg} is not installed, and it is the package that carries the ${target} binary - reinstall terminus, or add it directly with: npm install ${pkg}`,
  );
}

// Present but empty is its own failure: an interrupted download or a `files`
// list that stopped matching leaves the manifest resolvable and the payload
// gone, and spawn's bare ENOENT would name neither the package nor the platform.
if (!fs.existsSync(binary)) {
  fail(`${pkg} is installed for ${target} but carries no binary at ${binary}`);
}

// stdio inherited, so terminus writes to the terminal the user is looking at and
// reads the stdin the hook payload arrives on. The child stays in this process
// group, which is what delivers Ctrl-C to it without a handler here.
const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  fail(`could not run ${binary} - ${result.error.message}`);
}

// A signal death leaves status null. Exiting 0 there would report success for a
// process that was killed, so say so and use the shell's own 128+n convention.
if (result.signal) {
  const number = os.constants.signals[result.signal];
  process.stderr.write(`terminus: terminated by ${result.signal}\n`);
  process.exit(number ? 128 + number : 1);
}

// The binary's exit code is the shim's exit code, untouched: terminus's CLI
// contract is 0/1/2 and a wrapper that flattens it would erase the difference
// between "no results" and "bad usage".
process.exit(result.status === null ? 1 : result.status);
