#!/usr/bin/env node
// Runs the frontal-sandbox binary from the platform package that matches this machine.
// Each platform package is listed in optionalDependencies, and npm installs only
// the one whose os and cpu fields match.
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { findTarget, packageName } from "../lib/platform.js";

const require = createRequire(import.meta.url);

const key = `${process.platform}-${process.arch}`;
const target = findTarget(process.platform, process.arch);
if (!target) {
  console.error(`frontal-sandbox: no prebuilt binary for ${key}`);
  process.exit(1);
}

const pkg = packageName(target);
let binary;
try {
  binary = require.resolve(`${pkg}/${target.exe}`);
} catch {
  console.error(
    `frontal-sandbox: the binary package ${pkg} is not installed. Reinstall without --omit=optional.`,
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`frontal-sandbox: ${result.error.message}`);
  process.exit(1);
}
// If the binary was killed by a signal, re-raise it so the caller sees the same exit reason.
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
