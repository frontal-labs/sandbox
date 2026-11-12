import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { TARGETS, findTarget, packageName, exeFor } from "../lib/platform.js";

const pkgRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = JSON.parse(readFileSync(join(pkgRoot, "package.json"), "utf8"));

test("optionalDependencies lists exactly the prebuilt platform packages at the wrapper version", () => {
  const expected = TARGETS.map(packageName).sort();
  assert.deepEqual(Object.keys(pkg.optionalDependencies).sort(), expected);
  for (const version of Object.values(pkg.optionalDependencies)) {
    assert.equal(version, pkg.version);
  }
});

test("findTarget resolves every supported platform and arch", () => {
  assert.equal(findTarget("darwin", "arm64").triple, "aarch64-apple-darwin");
  assert.equal(findTarget("darwin", "x64").triple, "x86_64-apple-darwin");
  assert.equal(findTarget("linux", "x64").triple, "x86_64-unknown-linux-gnu");
  assert.equal(findTarget("linux", "arm64").triple, "aarch64-unknown-linux-gnu");
  assert.equal(findTarget("win32", "x64").triple, "x86_64-pc-windows-msvc");
});

test("findTarget returns undefined for platforms without a prebuilt binary", () => {
  assert.equal(findTarget("win32", "arm64"), undefined);
  assert.equal(findTarget("linux", "ia32"), undefined);
  assert.equal(findTarget("freebsd", "x64"), undefined);
});

test("the canonical binary is sandbox and the alias is frontal-sandbox", () => {
  for (const target of TARGETS) {
    assert.equal(target.exe, target.os === "win32" ? "sandbox.exe" : "sandbox");
    assert.equal(
      target.alias,
      target.os === "win32" ? "frontal-sandbox.exe" : "frontal-sandbox",
    );
  }
});

test("exeFor returns the alias when invoked as the alias and the canonical name otherwise", () => {
  assert.equal(exeFor("frontal-sandbox"), "frontal-sandbox");
  assert.equal(exeFor("frontal-sandbox.exe"), "frontal-sandbox.exe");
  assert.equal(exeFor("sandbox"), "sandbox");
  assert.equal(exeFor("sandbox.exe"), "sandbox.exe");
  assert.equal(exeFor("/usr/local/bin/sandbox"), "sandbox");
  assert.equal(exeFor("/usr/local/bin/frontal-sandbox"), "frontal-sandbox");
  assert.equal(exeFor("node"), "sandbox");
});