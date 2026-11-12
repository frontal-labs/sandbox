#!/usr/bin/env node
// Runs the sandbox binary from the platform package that matches this machine.
// Each platform package is listed in optionalDependencies, and npm installs only
// the one whose os and cpu fields match.
//
// Both the canonical `sandbox` and the legacy `frontal-sandbox` binaries are shipped
// in every platform package. This wrapper spawns the one that matches the name it was
// invoked as, so `sandbox` and `frontal-sandbox` both work.
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { findTarget, packageName } from "../lib/platform.js";

const require = createRequire(import.meta.url);

// The name this wrapper was invoked as, without any path.
const invoked = (process.argv[1] ?? "").split(/[\\/]/).pop() ?? "";
const exe =
  invoked === "frontal-sandbox" ||
  invoked === "frontal-sandbox.exe" ||
  invoked === "sandbox" ||
  invoked === "sandbox.exe"
    ? invoked
    : "sandbox";

const key = `${process.platform}-${process.arch}`;
const target = findTarget(process.platform, process.arch);
if (!target) {
  console.error(`${exe}: no prebuilt binary for ${key}`);
  process.exit(1);
}

const pkg = packageName(target);
let binary;
try {
  binary = require.resolve(`${pkg}/${exe}`);
} catch {
  console.error(
    `${exe}: the binary package ${pkg} is not installed. Reinstall without --omit=optional.`,
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`${exe}: ${result.error.message}`);
  process.exit(1);
}
// If the binary was killed by a signal, re-raise it so the caller sees the same exit reason.
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);