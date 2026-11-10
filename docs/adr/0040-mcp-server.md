# 0040: MCP server over stdio

## Status
Accepted

## Date
2026-11-10

## Context
PLAN §6 (M6) asks for an MCP server exposing `exec` and `fs` as tools, so an agent can use a
sandbox without a bespoke SDK. The sandbox already has an in-process session (`IsolateBackend`,
used by the CLI's `run` and `shell`), so the server needs no network listener or API key.

## Decision
- `frontal-sandbox mcp` serves the MCP stdio transport: one JSON object per line, on stdin and
  stdout. The protocol revision is `2024-11-05`.
- Four tools, each returning MCP text content with `isError` on failure: `exec` (a shell script,
  with its stdout, stderr and exit status), `read_file`, `write_file`, and `list_dir`.
- A tool failure is a tool result with `isError: true`. A malformed message or an unknown method
  is a JSON-RPC error. Notifications (messages without an `id`) get no reply.
- The session lives as long as the process. Its files and environment persist between calls, and
  nothing is saved after the process exits.
- Protocol logic lives in `handle`, which takes one message and returns its reply, so it is
  tested without stdio.

## Consequences
### Positive
- Any MCP client can drive a sandbox with `claude mcp add frontal-sandbox -- frontal-sandbox mcp`.
- No new dependency. The server reuses the CLI's existing session type.

### Negative
- Resources, prompts, and sampling are not implemented.
- The session is in memory only. A tool call cannot reach a durable session on a server (a
  future change could add a `--server` option that talks to `sandbox-server`).
- Commands that need a real binary are refused until the VM tier exists (PLAN §4.11).

## Alternatives Considered
- Serve MCP over HTTP from `sandbox-server`. Not chosen for this step: it needs an auth story for
  MCP clients, and stdio already covers local agents.

## References
- PLAN §6 (M6), §4.15. ADR 0030 (WebSocket exec), ADR 0039 (coreutils).
