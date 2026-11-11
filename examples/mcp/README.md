# MCP server

`frontal-sandbox mcp` serves one sandbox session to an MCP client over stdio. The tools are
`exec`, `read_file`, `write_file`, and `list_dir`. The session lives as long as the process,
so working directory, environment, and files persist between calls.

## Register it with Claude Code

```sh
cargo install --path cli        # or put frontal-sandbox on PATH
claude mcp add frontal-sandbox -- frontal-sandbox mcp
```

## Check it without a client

```sh
cargo build -p cli --bin frontal-sandbox
examples/mcp/smoke.sh
```

## Limits

- The session is in-process and in memory. Nothing survives the process.
- `exec` runs in the isolate tier. Commands that need a real binary (for example `ffmpeg`)
  are refused until the VM tier (PLAN §4.11) is built.
- Only the `2024-11-05` protocol revision is implemented. Resources and prompts are not.
