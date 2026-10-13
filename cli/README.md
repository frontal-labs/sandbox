# sandbox CLI

Command-line interface for Frontal Sandbox. The crate is `cli` (`cli/`), and it builds a binary named `sandbox`.

**Status: early scaffolding.** Every subcommand parses its arguments, but only prints a message. None of them execute code, create sessions, or call the server yet. See `cli/src/commands.rs`.

## Installation

Build and install from the repository root:

```bash
cargo install --path cli
# or build without installing
cargo build -p cli --bin sandbox
```

## Commands

| Command | Status | Notes |
|---------|--------|-------|
| `sandbox run <command>...` | Stub | Prints the command, cwd, and limits. Options: `-c/--cwd` (default `/work`), `--memory` (MB, default 512), `--cpu` (ms per second, default 1000). Execution has a `TODO`. |
| `sandbox shell` | Stub | Prints the cwd (`-c/--cwd`, default `/work`). Interactive shell has a `TODO`. |
| `sandbox snapshot <session_id>` | Stub | Prints a message only. |
| `sandbox fork <snapshot_id>` | Stub | Prints a message only. `-c/--count` (default 1). |
| `sandbox replay <trace_file>` | Stub | Prints the path only. |
| `sandbox serve` | Stub | Prints the address only. `--addr` (default `0.0.0.0:8080`). It does not start `sandbox-server`. |
| `sandbox bench` | Stub | Prints the name only. `-n/--name` (optional). |

Run `sandbox --help` or `sandbox <command> --help` for the generated help text.

## Examples

These show the current argument syntax. Today they only print what they would do.

```bash
# Run a command (prints only; execution is not implemented)
sandbox run --memory 512 --cpu 1000 "echo hello"

# Interactive shell (stub)
sandbox shell --cwd /work

# Snapshot and fork (stubs)
sandbox snapshot my-session
sandbox fork snap_abc123 --count 3

# Replay a trace (stub)
sandbox replay trace.jsonl

# Start the server address (stub; does not start sandbox-server)
sandbox serve --addr 127.0.0.1:8080
```

## Configuration

Not implemented. The CLI does not read `SANDBOX_API_URL`, `SANDBOX_API_KEY`, or any config file, and it has no `--url` or `--key` flags. The planned server configuration is tracked in `PLAN.md`.

## Not present

These are not implemented in `cli/src/commands.rs`, so do not use them:

- `sandbox admin` (drain host, force GC, inspect trace)
- `sandbox completions`
- `--config` on `serve`
- `--suite` on `bench` (the flag is `--name`)

## Related

- `sdks/rust/`: Rust SDK used by the CLI (its client methods are also stubs).
- `crates/sandbox-server/`: the server, whose handlers currently return placeholder strings.
- `PLAN.md`: target behavior and milestones.
