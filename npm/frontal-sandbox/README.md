# @frontal-labs/frontal-sandbox

Command-line client for Frontal Sandbox. Installs the prebuilt `frontal-sandbox` binary for this platform.

## Install

```bash
npm install --global @frontal-labs/frontal-sandbox
```

Or from the monorepo root:

```bash
npm install --workspace=@frontal-labs/frontal-sandbox
```

Requires Node.js >= 20.

## Usage

```bash
frontal-sandbox [command] [flags]
```

Run `frontal-sandbox --help` to see available commands.

## How it works

This package is a thin launcher. `bin/frontal-sandbox.js` detects `process.platform` and `process.arch`, resolves the matching `@frontal-labs/sandbox-cli-<target>` platform package installed as an optional dependency, and spawns its bundled `frontal-sandbox` (or `frontal-sandbox.exe` on Windows) binary, forwarding all arguments.

Supported targets:

| platform | arch | target triple |
|----------|------|---------------|
| darwin | arm64 | aarch64-apple-darwin |
| darwin | x64 | x86_64-apple-darwin |
| linux | x64 | x86_64-unknown-linux-gnu |
| linux | arm64 | aarch64-unknown-linux-gnu |
| win32 | x64 | x86_64-pc-windows-msvc |

Install npm without `--omit=optional` so the platform package for your machine is included. If you see `the binary package ... is not installed`, reinstall without `--omit=optional`.

## Publishing

Published to GitHub Packages (`https://npm.pkg.github.com`) as configured in `publishConfig`.

## License

Apache-2.0
