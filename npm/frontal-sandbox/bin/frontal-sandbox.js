#!/usr/bin/env node
// Runs the sandbox binary from the platform package that matches this machine.
// Each platform package is listed in optionalDependencies, and npm installs only
// the one whose os and cpu fields match.
"use strict";

const { spawnSync } = require("node:child_process");

const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};

const key = `${process.platform}-${process.arch}`;
const target = TARGETS[key];
if (!target) {
  console.error(`sandbox: no prebuilt binary for ${key}`);
  process.exit(1);
}

const exe = process.platform === "win32" ? "sandbox.exe" : "sandbox";
let binary;
try {
  binary = require.resolve(`@frontal-labs/sandbox-cli-${target}/${exe}`);
} catch {
  console.error(
    `sandbox: the binary package @frontal-labs/sandbox-cli-${target} is not installed. Reinstall without --omit=optional.`,
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`sandbox: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
