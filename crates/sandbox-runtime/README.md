# Sandbox Runtime

V8 host built on `deno_core`. Runs JavaScript in a fresh isolate per call.

## Status

Working, with these limits. Covered by unit tests in `src/runtime.rs`.

- `execute_script(code, ScriptLimits) -> ScriptOutcome` runs classic scripts.
- `console.log` is captured. Output is capped (`max_output_bytes`, default 1 MiB).
- Wall-clock timeout (`timeout`, default 5 s) through a watchdog that calls `terminate_execution`.
- Heap cap (`max_heap_bytes`, default 64 MiB) through V8 heap limits. A near-limit callback terminates the script, so the process is not aborted.
- Errors and timeouts are script outcomes (`exit_code` 1 with a message on stderr), not host errors.

Not supported: modules, TypeScript, filesystem or network access, and host environment access. The only op is `op_sandbox_print`. Ops are host capabilities, so adding one needs a policy check.

## Overview

- `runtime.rs`: isolate lifecycle, watchdog, heap guard. This module holds the crate's `unsafe`, each block with a `SAFETY:` comment.
- `ops.rs`: `op_sandbox_print` and the captured-output buffer.
