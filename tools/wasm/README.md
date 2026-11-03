# WASI tools for the sandbox

The tools the sandbox runs as WASI modules (PLAN §4.6), built for `wasm32-wasip1`.

| Tool | Crate | Built on | Status |
| --- | --- | --- | --- |
| `jq` | `jq` | jaq (pure Rust) | Real. Matches reference jq 1.7.1 on the corpus in `crates/sandbox-wasm/tests/data/jq_cases.json`. |
| `rg` | `rg` | the grep crates (pure Rust) | Real. Matches ripgrep 15.2 on stdin in `crates/sandbox-wasm/tests/data/rg_cases.json`. Recursive search follows its defaults, except that `.gitignore` is not read. |
| `sqlite3` | `sqlite3/` (C) | SQLite 3.51.3, the amalgamation bundled by `libsqlite3-sys` | Real. Matches the reference CLI on the cases in `crates/sandbox-wasm/tests/data/sqlite_cases.json`, and a file database persists between runs. Built in the Linux dev container, because Apple clang cannot target wasm32-wasi. |

Not yet built as real tools (PLAN §4.6, M2): `git` (gix or a WASI build) and `python`
(CPython needs a WASI build and an initialised stdlib). The fixtures in
`crates/sandbox-wasm/fixtures/tools/` are small stand-ins for those, and the manifest
there says so.

## Building

```sh
tools/wasm/build.sh                  # jq and rg, on the host
docker/run-linux.sh tools/wasm/sqlite/build.sh   # sqlite3, in the Linux dev container
cp tools/wasm/sqlite/build/sqlite3_tool.wasm crates/sandbox-wasm/fixtures/
cargo test -p sandbox-wasm --test jq_conformance --test rg_conformance --test sqlite_conformance --test registry_manifest
```

After a rebuild, the new hashes (`b3sum --no-names`) must be copied into
`crates/sandbox-wasm/fixtures/tools/tools.toml`.

## License

Apache-2.0
