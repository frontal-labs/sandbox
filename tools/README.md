# Tools

This directory contains standalone tools and utilities used by the sandbox project.

## Structure

```
tools/
├── wasm/              # WASI tools built as WebAssembly modules
│   ├── jq/            # jq_tool - JSON processor (jaq-based)
│   ├── rg/            # rg_tool - ripgrep-compatible search
│   ├── Cargo.toml     # Workspace for WASI tools
│   ├── build.sh       # Build script producing WASM modules + hashes
│   └── README.md      # Detailed documentation for WASI tools
└── README.md          # This file
```

## WASI Tools (`tools/wasm/`)

The sandbox runs certain tools as **WASI modules** (WebAssembly System Interface) targeting `wasm32-wasip1`. These are pure Rust implementations with no C dependencies, enabling them to run in any WASI-compatible runtime.

### Available Tools

| Tool | Crate | Implementation | Status |
|------|-------|---------------|--------|
| `jq` | `jq_tool` | [jaq](https://github.com/01mf02/jaq) (pure Rust) | ✅ Complete |
| `rg` | `rg_tool` | [grep](https://github.com/BurntSushi/ripgrep) crates (pure Rust) | ✅ Complete |

### Planned Tools (Not Yet Built)

- `git` - via [gix](https://github.com/Byron/gix) or WASI build
- `sqlite3` - requires C compilation with wasi-sdk
- `python` - requires C compilation with wasi-sdk

### Building

```bash
# From repository root
tools/wasm/build.sh

# Verify with conformance tests
cargo test -p sandbox-wasm --test jq_conformance --test rg_conformance --test registry_manifest
```

The build script:
1. Compiles both tools for `wasm32-wasip1` with size optimizations
2. Copies the `.wasm` modules to `crates/sandbox-wasm/fixtures/`
3. Prints BLAKE3 hashes for the tool registry manifest

### Tool Registry

After rebuilding, copy the printed hashes into:
```
crates/sandbox-wasm/fixtures/tools/tools.toml
```

The `registry_manifest` test validates that the deployed modules match the registered hashes.

### Requirements

- Rust with `wasm32-wasip1` target: `rustup target add wasm32-wasip1`
- `b3sum` for hash generation: `cargo install b3sum`

### Architecture

Each tool is a separate crate in the workspace:
- `jq_tool` - Uses `jaq-core`, `jaq-std`, `jaq-json`
- `rg_tool` - Uses `grep-matcher`, `grep-regex`, `grep-searcher`, `walkdir`

Both expose a library API for embedding and a CLI for testing.

## License

Apache-2.0