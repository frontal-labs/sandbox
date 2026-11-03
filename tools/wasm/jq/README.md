# jq_tool - WebAssembly JSON Processor

A WebAssembly port of [jaq](https://github.com/01mf02/jaq), a pure Rust implementation of jq - the lightweight and flexible command-line JSON processor.

## Features

- **Pure Rust**: Built with jaq-core, jaq-std, and jaq-json - no C dependencies
- **WebAssembly**: Compiles to WASM for browser and edge runtime compatibility
- **jq Compatible**: Supports jq filter syntax and semantics
- **Zero Dependencies**: No external system libraries required

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
use jq_tool::{run_jq, JqOptions};

let input = r#"{"name": "Alice", "age": 30}"#;
let filter = ".name";
let output = run_jq(input, filter)?;
```

### In WebAssembly (JavaScript/TypeScript)

```javascript
import init, { run_jq } from './pkg/jq_tool.js';

await init();

const input = JSON.stringify({ name: "Alice", age: 30 });
const filter = ".name";
const result = run_jq(input, filter);
console.log(result); // "Alice"
```

## Supported jq Features

- Object/array indexing: `.foo`, `.[0]`
- Iteration: `.[]`, `.[]?`
- Pipe: `|`
- Filter composition: `,`
- Functions: `map()`, `select()`, `length`, `keys`, etc.
- String interpolation: `"\(.name)"`
- Conditionals: `if/then/else/end`
- Variables: `$var`
- And much more...

## Project Structure

```
jq_tool/
├── Cargo.toml
├── src/
│   └── lib.rs          # Main entry point with run_jq function
└── README.md
```

## Dependencies

- [jaq-core](https://crates.io/crates/jaq-core) - Core jq interpreter
- [jaq-std](https://crates.io/crates/jaq-std) - Standard library functions
- [jaq-json](https://crates.io/crates/jaq-json) - JSON parsing/serialization

## License

Apache-2.0