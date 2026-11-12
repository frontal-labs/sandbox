# Browser Agent Example

Demonstrates an agent that drives `sandbox mcp` over stdio (JSON-RPC 2.0) and uses the `exec`, `read_file`, `write_file`, and `list_dir` tools to perform a browser-style task inside the sandbox. The agent writes an HTML page into the sandbox, runs a shell extraction command against it, and validates the extracted page data (title, status, and list items).

This mirrors the MCP agent example pattern but targets a browser-task workflow: load page content and extract structured information from it.

`frontal-sandbox` is a legacy alias that works identically.

## Prerequisites

- Rust toolchain (to build `sandbox`)
- Node.js 20+

## Run

```bash
cargo build -p cli --bin sandbox
npm install --ignore-scripts
npm start
```

By default the agent uses `../../target/debug/sandbox`. Override with `SANDBOX_BIN` (or the legacy `FRONTAL_SANDBOX_BIN`):

```bash
SANDBOX_BIN=/path/to/sandbox npm start
```

## Smoke test

```bash
cargo build -p cli --bin sandbox
./smoke.sh
```

## What the agent does

1. Starts `sandbox mcp` as a subprocess and sends `initialize`.
2. Uses `write_file` to create `/page.html` (a sample page).
3. Uses `exec` to run a shell extraction over the page and print extracted JSON.
4. Validates the extracted title, status, and list items and reports success.

The MCP session is in-process and in memory; files and state persist only for the lifetime of the `mcp` process.