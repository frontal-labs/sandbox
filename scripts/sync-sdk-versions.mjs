#!/usr/bin/env node
// One version covers the TypeScript, Python, and Rust SDKs. Changesets bumps the
// TypeScript package, and this copies that version into the other two manifests
// and refreshes Cargo.lock to match.
import { execFileSync } from "node:child_process"
import { readFileSync, writeFileSync } from "node:fs"

const version = JSON.parse(readFileSync("sdks/typescript/package.json", "utf8")).version
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error(`not a plain semver version: ${version}`)
}

// The first `version = ` line in each file is the package's own version.
// Dependency versions appear later in the file, so they are not touched.
function setFirstVersion(path, pattern) {
  const src = readFileSync(path, "utf8")
  const next = src.replace(pattern, (line) => line.replace(/"[^"]*"/, `"${version}"`))
  if (next === src && !src.includes(`version = "${version}"`)) {
    throw new Error(`no version line found in ${path}`)
  }
  writeFileSync(path, next)
}

setFirstVersion("sdks/python/pyproject.toml", /^version = ".*"$/m)
setFirstVersion("sdks/rust/Cargo.toml", /^version = ".*"$/m)

// Cargo rewrites Cargo.lock for the changed package version.
execFileSync("cargo", ["metadata", "--format-version", "1"], { stdio: ["ignore", "ignore", "inherit"] })
console.log(`SDK versions set to ${version}`)
