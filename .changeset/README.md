# Changesets

This folder is managed by [changesets](https://github.com/changesets/changesets).
Do not edit files here manually — use `npm run changeset` instead.

## How it works

1. On a PR that changes `sdks/` or `npm/sandbox-cli/`, run `npm run changeset`
   to create a changeset file (`.changeset/<name>.md`). The PR check in
   `pr-checks.yml` requires a changeset for any change under `sdks/`.
2. When the changeset lands on `main`, the `version` job in `release.yml`
   opens (or updates) a "Version SDKs" PR via `changesets/action`.
3. Merging that PR runs `npm run version-packages`, which:
   - runs `changeset version` (bumps the TypeScript package version and writes
     a CHANGELOG)
   - runs `scripts/sync-sdk-versions.mjs` (copies the version into
     `sdks/python/pyproject.toml`, `sdks/rust/Cargo.toml`, and refreshes
     `Cargo.lock`)
4. The `publish` job packs each SDK and pushes it to GHCR. The `publish-npm`
   job publishes the TypeScript SDK to public npm. The `release` job creates
   the `sdk-v<version>` tag and GitHub release.

## Available packages

Changesets tracks these packages:

- `@frontal-labs/sandbox-sdk` (`sdks/typescript/package.json`)

The Python (`sdks/python`) and Rust (`sdks/rust`) SDKs are versioned in lockstep
via `scripts/sync-sdk-versions.mjs`; they do not have their own changeset entries.

The CLI (`npm/sandbox-cli`) is versioned by `release-binaries.yml` from `v*`
tags and is not managed by changesets.

## Commands

```bash
npm run changeset     # add a changeset (interactive)
npm run version-packages   # bump versions (run by the release workflow)
```
