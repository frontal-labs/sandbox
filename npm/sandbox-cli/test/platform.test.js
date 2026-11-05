import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { TARGETS, findTarget, packageName } from "../lib/platform.js";

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

test("the Windows binary is frontal-sandbox.exe and the others have no extension", () => {
  for (const target of TARGETS) {
    if (target.os === "win32") {
      assert.equal(target.exe, "frontal-sandbox.exe");
    } else {
      assert.equal(target.exe, "frontal-sandbox");
    }
  }
});
