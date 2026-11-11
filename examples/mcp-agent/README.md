# MCP Coding Agent Example

Demonstrates an agent that drives `frontal-sandbox mcp` over stdio (JSON-RPC 2.0) and uses the `exec`, `read_file`, `write_file`, and `list_dir` tools to edit code and run tests in a sandbox. The agent writes a buggy shell implementation and a test, observes the failing test, fixes the code, and verifies the tests pass.

## Prerequisites

- Rust toolchain (to build `frontal-sandbox`)
- Node.js 20+

## Run

```bash
cargo build -p cli --bin frontal-sandbox
npm install
npm start
```

By default the agent uses `../../target/debug/frontal-sandbox`. Override with `FRONTAL_SANDBOX_BIN`:

```bash
FRONTAL_SANDBOX_BIN=/path/to/frontal-sandbox npm start
```

## Smoke test

```bash
cargo build -p cli --bin frontal-sandbox
./smoke.sh
```

## What the agent does

1. Starts `frontal-sandbox mcp` as a subprocess and sends `initialize`.
2. Uses `list_dir` and `write_file` to create `math.sh` (with a deliberate bug) and `math.test.sh`.
3. Uses `exec` to run the tests and observes the failure.
4. Uses `read_file` and `write_file` to inspect and fix the implementation.
5. Uses `exec` to run the tests again and confirms they pass.

The MCP session is in-process and in memory; files and state persist only for the lifetime of the `mcp` process.
