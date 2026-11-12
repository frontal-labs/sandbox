# @frontal-labs/sandbox-cli

Command-line client for Frontal Sandbox. Installs the prebuilt `frontal-sandbox` binary for this platform.

## Install

```bash
npm install --global @frontal-labs/sandbox-cli
```

Or from the monorepo root:

```bash
npm install --workspace=@frontal-labs/sandbox-cli
```

Requires Node.js >= 20.19.

## Usage

```bash
frontal-sandbox [command] [flags]
```

Run `frontal-sandbox --help` to see available commands.

## How it works

This package is a thin launcher. `bin/sandbox.js` detects `process.platform` and `process.arch`, resolves the matching `@frontal-labs/sandbox-cli-<target>` platform package installed as an optional dependency, and spawns its bundled `frontal-sandbox` (or `frontal-sandbox.exe` on Windows) binary, forwarding all arguments. The target table lives in `lib/platform.js`.

Supported targets:

| platform | arch | target triple |
|----------|------|---------------|
| darwin | arm64 | aarch64-apple-darwin |
| darwin | x64 | x86_64-apple-darwin |
| linux | x64 | x86_64-unknown-linux-gnu |
| linux | arm64 | aarch64-unknown-linux-gnu |
| win32 | x64 | x86_64-pc-windows-msvc |

Install npm without `--omit=optional` so the platform package for your machine is included. If you see `the binary package ... is not installed`, reinstall without `--omit=optional`.

## Development

```bash
npm test                                   # checks the target table against package.json
npm run build                              # builds the host target with cargo and stages platforms/<triple>/
node scripts/build-binaries.mjs x86_64-apple-darwin   # build a specific target
```

`npm run build` runs `cargo build --release -p cli` from the repository root. The binary is built from the `cli` crate. Its version comes from `SANDBOX_VERSION`, which the build script sets from `package.json`. Staged packages go in `platforms/` (git-ignored).

## Publishing

Releases are published by `.github/workflows/release-binaries.yml` on `v*` tags. The workflow builds each target, publishes the platform packages, and then publishes the wrapper with its `optionalDependencies` set to the released versions. All packages are published to [public npm](https://www.npmjs.com/) under the `@frontal-labs` scope.

## License

Apache-2.0
