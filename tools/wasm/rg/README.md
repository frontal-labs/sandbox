# rg_tool - WebAssembly Ripgrep

A WebAssembly port of [ripgrep](https://github.com/BurntSushi/ripgrep) (rg) - a line-oriented search tool that recursively searches directory trees for a regex pattern.

## Features

- **Pure Rust**: Built with grep-matcher, grep-regex, grep-searcher, and walkdir
- **WebAssembly**: Compiles to WASM for browser and edge runtime compatibility
- **Fast Searching**: Uses Rust's regex engine with SIMD optimizations
- **Cross-Platform**: Works in browsers, Node.js, and WASM runtimes

## Building

```bash
# Install wasm32 target
rustup target add wasm32-unknown-unknown

# Build for WebAssembly
cargo build --target wasm32-unknown-unknown --release

# Or use wasm-pack for npm-compatible output
wasm-pack build --target web
```

## Usage

### As a Rust Library

```rust
use rg_tool::{search, SearchOptions};

let options = SearchOptions {
    pattern: "TODO|FIXME",
    path: "./src",
    case_sensitive: false,
    ..Default::default()
};

let results = search(options)?;
for result in results {
    println!("{}:{}: {}", result.path, result.line_number, result.line);
}
```

### In WebAssembly (JavaScript/TypeScript)

```javascript
import init, { search } from './pkg/rg_tool.js';

await init();

const results = search({
    pattern: "TODO|FIXME",
    path: "./src",
    caseSensitive: false,
    maxResults: 100
});

for (const result of results) {
    console.log(`${result.path}:${result.lineNumber}: ${result.line}`);
}
```

## Supported Features

- **Regex Patterns**: Full Rust regex syntax support
- **File Types**: Automatic detection and filtering by extension
- **Case Sensitivity**: Toggle with `-i`/`-s` flags
- **Line Numbers**: Optional line number output
- **Context Lines**: Show lines before/after matches (`-A`, `-B`, `-C`)
- **File Filtering**: Include/exclude patterns (`-g`, `-t`, `-T`)
- **Hidden Files**: Search hidden files and directories (`--hidden`)
- **Follow Symlinks**: Follow symbolic links (`--follow`)
- **Max Depth**: Limit directory traversal depth
- **Max Results**: Limit number of matches returned

## Project Structure

```
rg_tool/
├── Cargo.toml
├── src/
│   └── lib.rs          # Main entry point with search function
└── README.md
```

## Dependencies

- [grep-matcher](https://crates.io/crates/grep-matcher) - Pattern matching abstraction
- [grep-regex](https://crates.io/crates/grep-regex) - Regex implementation for grep-matcher
- [grep-searcher](https://crates.io/crates/grep-searcher) - File searching infrastructure
- [walkdir](https://crates.io/crates/walkdir) - Directory traversal

## Differences from CLI ripgrep

This WASM version is designed for embedding in applications rather than as a standalone CLI tool. Key differences:

- No direct filesystem access in browser (requires virtual filesystem or pre-loaded content)
- Streaming results via callbacks/iterators rather than stdout
- Configuration via structured options object rather than CLI flags
- Optimized for programmatic use with typed results

## License

Apache-2.0