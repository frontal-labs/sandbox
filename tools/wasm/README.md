# WASI tools for the sandbox

The tools the sandbox runs as WASI modules (PLAN §4.6), built for `wasm32-wasip1`.

| Tool | Crate | Built on | Status |
| --- | --- | --- | --- |
| `jq` | `jq` | jaq (pure Rust) | Real. Matches reference jq 1.7.1 on the corpus in `crates/sandbox-wasm/tests/data/jq_cases.json`. |
| `rg` | `rg` | the grep crates (pure Rust) | Real. Matches ripgrep 15.2 on stdin in `crates/sandbox-wasm/tests/data/rg_cases.json`. Recursive search follows its defaults, except that `.gitignore` is not read. |

Not yet built as real tools (PLAN §4.6, M2): `git` (gix or a WASI build), `sqlite3` and
`python` (C, which needs a WASI sysroot such as wasi-sdk; Apple clang cannot target
wasm32-wasip1). The fixtures in `crates/sandbox-wasm/fixtures/tools/` are small stand-ins
for those, and the manifest there says so.

## Building

```sh
tools/wasm/build.sh
cargo test -p sandbox-wasm --test jq_conformance --test rg_conformance --test registry_manifest
```

After a rebuild, the printed hashes must be copied into `crates/sandbox-wasm/fixtures/tools/tools.toml`.
