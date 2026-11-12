# Sandbox CLI

The `sandbox` binary (`frontal-sandbox` is a legacy alias that runs the same binary), built from the `cli` crate. It runs commands in the local isolate runtime, drives a running server, and records and replays traces.

## Commands

| Command | What it does |
|---------|--------------|
| `sandbox run [--trace FILE] <command>...` | Runs the arguments, joined with spaces, as one shell script in a new local sandbox. `--trace` writes the script and its output to FILE. |
| `sandbox shell [--trace FILE]` | Reads one script per line from stdin in one local sandbox. `exit` or end of input stops it. `--trace` writes every line to FILE when the shell exits. |
| `sandbox replay <TRACE_FILE>` | Runs the scripts of a trace, in order, in a fresh local sandbox and compares each output with the recording. Exits 0 when every run matches and 1 when any run diverges. |
| `sandbox bench [-n NAME]` | Times the cases in the table below and prints one JSON line per case. |
| `sandbox serve [--addr ADDR] [--data-dir DIR]` | Starts the server in the foreground (default `0.0.0.0:8080`). Prints one API key to stderr on the first run. With `--data-dir`, keys, snapshots, and sleeping sessions survive a restart (ADR 0008). Without it, the server is in memory only. |
| `sandbox snapshot <SESSION_ID>` | Snapshots a session on the server named by `SANDBOX_BASE_URL`, with `SANDBOX_API_KEY`. |
| `sandbox fork <SNAPSHOT_ID> [-c N]` | Forks a snapshot into N sessions on that server. |

Run `sandbox --help` or `sandbox <command> --help` for the generated help text.

## Traces

A trace is JSON Lines: a header line, then one line per script. See [ADR 0007](../docs/adr/0007-trace-format-and-replay.md) for the format and for what replay does and does not check.

```bash
sandbox run --trace one.jsonl 'echo hi | cat > /a && cat /a'
sandbox replay one.jsonl        # replayed 1 runs: 0 diverged
```

## Benchmarks

`sandbox bench` runs these cases on the local isolate runtime, with 10 warm-up and 200 measured iterations each:

| Case | Operation |
|------|-----------|
| `exec_echo` | one `echo` script |
| `shell_pipeline` | `echo a \| cat \| cat` |
| `vfs_write_read_4k` | write 4 KiB to a file and read it back through the VFS |
| `snapshot_restore` | checkpoint the filesystem and start a session from the checkpoint |

Each line reports `iterations`, `mean_ns`, `min_ns`, `p50_ns`, `p90_ns`, `p99_ns`, and `max_ns`. Percentiles are exact nearest-rank values over the iterations. Compare runs on the same machine, and build with `--release` for numbers you intend to keep.

## Environment

- `SANDBOX_BASE_URL` (default `http://localhost:8080`) and `SANDBOX_API_KEY`: used by `snapshot` and `fork`.