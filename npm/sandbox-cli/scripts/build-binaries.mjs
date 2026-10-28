#!/usr/bin/env node
// Builds the frontal-sandbox binary for each target and stages one platform package per target
// under platforms/<triple>/. The release workflow (.github/workflows/release-binaries.yml) does
// the same for every tagged release. Use this script to check the packaging locally.
//
// Usage: node scripts/build-binaries.mjs [triple ...]
//   No arguments builds the host target. Other targets need their rustup target and linker installed.
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { TARGETS, findTarget, packageName } from "../lib/platform.js";

const pkgRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(pkgRoot, "../..");
const pkg = JSON.parse(readFileSync(join(pkgRoot, "package.json"), "utf8"));

function fail(message) {
  console.error(`build-binaries: ${message}`);
  process.exit(1);
}

const requested = process.argv.slice(2);
let selected;
if (requested.length === 0) {
  const host = findTarget(process.platform, process.arch);
  if (!host) {
    fail(`no prebuilt target for ${process.platform}-${process.arch}`);
  }
  selected = [host];
} else {
  selected = requested.map((triple) => {
    const target = TARGETS.find((candidate) => candidate.triple === triple);
    if (!target) {
      fail(`unknown target ${triple}. Expected one of: ${TARGETS.map((t) => t.triple).join(", ")}`);
    }
    return target;
  });
}

for (const target of selected) {
  // The version reaches the binary at compile time (see cli/src/commands.rs), as in the release workflow.
  // cargo is taken from CARGO when a parent cargo sets it, and otherwise from PATH.
  execFileSync(process.env.CARGO ?? "cargo", ["build", "--release", "-p", "cli", "--target", target.triple], {
    cwd: repoRoot,
    env: { ...process.env, SANDBOX_VERSION: pkg.version },
    stdio: "inherit",
  });

  const built = join(repoRoot, "target", target.triple, "release", target.exe);
  const dir = join(pkgRoot, "platforms", target.triple);
  rmSync(dir, { force: true, recursive: true });
  mkdirSync(dir, { recursive: true });
  const staged = join(dir, target.exe);
  copyFileSync(built, staged);
  if (target.os !== "win32") {
    chmodSync(staged, 0o755);
  }

  // Same shape as the package the release workflow publishes for this target.
  const platformPackage = {
    name: packageName(target),
    version: pkg.version,
    description: `frontal-sandbox CLI binary for ${target.triple}`,
    os: [target.os],
    cpu: [target.cpu],
    files: [target.exe],
    license: "Apache-2.0",
    repository: "https://github.com/frontal-labs/sandbox",
    publishConfig: { registry: "https://npm.pkg.github.com" },
  };
  writeFileSync(join(dir, "package.json"), `${JSON.stringify(platformPackage, null, 2)}\n`);
  console.log(`staged ${packageName(target)} in platforms/${target.triple}`);
}
