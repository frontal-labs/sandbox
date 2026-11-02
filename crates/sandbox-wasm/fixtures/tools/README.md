# Building Real WASM Tools

This directory contains build scripts and configurations for compiling real tools to WASI (wasm32-wasip1).

## Tools to Build

### Rust-based tools (easier - native wasm32-wasip1 support)
- **jq** - JSON processor (use `jaq` or `jq-rs` as pure Rust alternatives)
- **ripgrep** - Fast search tool (pure Rust)
- **fd** - Fast find alternative (pure Rust)

### C-based tools (need wasi-sdk)
- **sqlite3** - SQL database engine
- **git** - Version control
- **python** - Python interpreter

## Building Rust Tools

```bash
# For pure Rust tools, add wasm32-wasip1 target and build
cargo build --target wasm32-wasip1 --release
```

## Building C Tools with wasi-sdk

```bash
# Install wasi-sdk (on macOS)
brew install wasi-sdk

# Set environment variables
export CC=/opt/homebrew/opt/wasi-sdk/bin/clang
export AR=/opt/homebrew/opt/wasi-sdk/bin/llvm-ar
export RANLIB=/opt/homebrew/opt/wasi-sdk/bin/llvm-ranlib

# Configure and build with wasi-sdk
./configure --host=wasm32-wasi --prefix=/path/to/install
make && make install
```

## Current Demo Tools

The `tools/` directory contains demo tools built for wasm32-wasip1:
- `echo_tool` - Echoes arguments
- `cat_tool` - Reads stdin, writes to stdout
- `env_tool` - Prints environment as JSON

These are in `crates/sandbox-wasm/fixtures/` and registered in the WASM registry.

## Future Work

To add real tools:
1. Add tool source as git submodule or vendor
2. Create build script in `scripts/build-wasm-tools.sh`
3. Add to registry with `tools.toml` manifest
4. Update `sandbox-wasm` to load from registry