# Tier 1 agent interfaces

This page describes what an isolate-tier (Tier 1) session gives an agent: its
environment file, capped output, checkpoints, undo, and fork. The design is in
`plans/IMAGE.md` §4.5. This page records what is implemented and where it stops.

## Environment: `/etc/agent/env.json`

Every Tier 1 session has `/etc/agent/env.json`, schema 1. The server writes it on
create and on fork. A fork copies the snapshot's copy and changes only `session`,
unless that copy is over 64 KiB or does not parse, in which case the child gets a
fresh file. A wake does not rewrite it, because the file is part of the filesystem.

Fields that Tier 1 cannot fill yet are honest about it:

- `image` is `{"base": null, "layers": []}`, because image layers arrive in I3.
- `packages` is `[]`, for the same reason.
- `commands` is `["sandbox"]`. `cua` is desktop-only.
- `cpu_ms_budget` is `null`. The limits model has no matching budget.
- `user` is `"agent"`. Tier 1 has no real uid.

`GET /v1/sessions/{id}/info`, `frontal-sandbox sandbox info`, and the Rust SDK's
`info` return the file.

### Protection

`env.json` is host-owned. The agent cannot write, truncate, delete, or rename it,
and it cannot remove the directory that holds it. Those attempts fail with
`PERMISSION_DENIED`. The host writes it through `write_system_file`.

The file is exempt from quotas, so a session's limits apply only to what the agent
writes. Only the first 64 KiB of an exempt file is exempt. Any excess counts like
agent data, which keeps an oversized legacy file from bypassing the limits.

## Output capping

A buffered `exec` keeps the first and last 8 KiB of each stream by default. The
result replaces the middle with one marker line:

```
…[N lines omitted · full: /trace/exec/<seq>.out]
```

`ExecRequest.output_limit` sets the total bytes kept per stream. The full stream
is in the session's trace, so it is served at `GET /v1/sessions/{id}/fs/trace/exec/<seq>.out`
for stdout and `.err` for stderr. If the run was not recorded, the marker says
`full output not recorded` and names no path.

A single run may produce up to 8 MiB of output. The trace holds 8 MiB per session.
A run that does not fit there keeps its output as blobs instead, so its full stream
still reads at `trace/exec/<seq>.out`. The blobs are capped at 64 MiB per session,
and beyond that the marker says `full output not recorded`. Such a run is not in the
trace, so the trace reports itself incomplete and cannot be replayed faithfully.
Blobs are collected like any other blob: a run's output is kept while the session
still pins a checkpoint that needs it, and otherwise the next GC removes it.
Streamed `exec` output is not capped.

## Checkpoints

A checkpoint pins a committed filesystem root. Two policies:

- `per_exec` (the default): each `exec` pins a checkpoint after it runs. The
  session's creation and each fork also pin one, so undo always has a target.
- `manual`: a checkpoint is pinned only by `POST /v1/sessions/{id}/checkpoint`.

`checkpoint_retain` (1 to 256, default 50) limits how many roots stay pinned. The
oldest are unpinned. Pinned roots survive a durable restart, and GC keeps the blobs
of every pinned root.

`GET /v1/sessions/{id}` reports `checkpoints: {policy, count, retain, bytes}`.
`bytes` is the logical size of the pinned roots. A block shared by two roots counts
twice, so the figure is not the storage the server holds.

Each key's pinned checkpoints are bounded at 1 GiB of logical bytes across all its
sessions. `FRONTAL_SANDBOX_CHECKPOINT_BYTES_PER_KEY` sets the bound, in bytes, and the
server reads it once at startup. Over that, the key's oldest checkpoints are dropped first. A per-exec pin
that still does not fit is skipped, and an explicit checkpoint, undo, or fork fails
with `RESOURCE_EXHAUSTED`.

Unpinning is real storage reclaim. A collection (`POST /v1/gc`, or the scheduled
sweep under `frontal-sandbox serve`) removes the blobs of unpinned roots that no
other root uses.

## Undo

`POST /v1/sessions/{id}/undo` with `{"steps": N}` restores the filesystem from N
checkpoints ago. The current state is pinned first, so undo can be reversed. Each
step drops the newest checkpoint. If fewer than N steps exist, the request fails
with `INVALID_ARGUMENT`.

Undo restores the filesystem only. A Tier 1 session keeps no processes between
commands, so none are lost. The undo itself is not recorded in the trace.

Inside the session, `sandbox checkpoint`, `sandbox undo [N]`, and
`sandbox fork N -- <cmd>` run as ordinary execs. The server recognizes them by argv,
so they work through the exec route. The reply is the same text the host commands print. They
exit 1 when the operation fails, and 2 when the usage is wrong. A multi-command script
run through the shell does not reach them: `sandbox` in a script runs only `info`.

## Shell additions

`sh -c <script>` runs a script in a subshell, as `( ... )` does: its variables do not
leak out, and `exit` ends only the `sh`. `$RANDOM` is a value from 0 to 32767, seeded per
shell, and an explicit assignment overrides it.

Each recorded run stores the generator state it started from. A replay restores that
state first, so the replayed run draws the same `$RANDOM` values. The state is optional
in the trace format, so traces written before it existed still replay.

### Limits, and why they are set where they are

- **Spilled output is capped at 64 MiB per session.** Blobs hold the output of runs
  that did not fit in the trace. The cap bounds storage a single session can hold.
  Past it, a run's output is not kept, and its marker says so.
- **The checkpoint budget counts logical bytes.** Each root's size is the sum of its
  file contents, so a block shared by two roots counts twice. Counting physical
  storage would need a per-root measurement of new blocks, which the store does not
  yet report.
- **Traces written before `$RANDOM` state was recorded cannot replay exactly.** Those
  runs carry no generator state, so their `$RANDOM` values cannot be reproduced. The
  state cannot be recovered after the fact.

## Fork with a command

`POST /v1/sessions/{id}/fork` with `{"count": N, "argv": [...]}` pins the current
state, creates N independent children from it, and runs `argv` in each. The
response reports each child's session ID, exit code or error, and changed-file
count. Each child's command is recorded in that child's trace.

The IMAGE.md example `sandbox fork 3 -- sh -c 'echo $RANDOM > /r'` runs as written.
Each child writes its own `/r`.

## Surfaces

| Operation | Server route | Rust SDK | CLI |
| --- | --- | --- | --- |
| Environment | `GET /v1/sessions/{id}/info` | `info` | `frontal-sandbox sandbox info <session>` |
| Checkpoint | `POST /v1/sessions/{id}/checkpoint` | `checkpoint` | `frontal-sandbox sandbox checkpoint <session>` |
| Undo | `POST /v1/sessions/{id}/undo` | `undo` | `frontal-sandbox sandbox undo <session> [N]` |
| Fork | `POST /v1/sessions/{id}/fork` | `fork_command` | `frontal-sandbox sandbox fork <session> -c N -- <cmd>` |

The top-level `frontal-sandbox fork <snapshot>` still forks a snapshot.

Inside the shell, `sandbox info` prints the same file. The shell's own `sandbox`
builtin runs only `info`. The other subcommands are handled by the exec route, as
described under Undo.

## SDKs

The Rust SDK's `Client` has the four methods. The TypeScript and Python SDKs have a
`Tier1Client` in their own modules (`sdks/typescript/src/client/tier1.ts` and
`sdks/python/sandbox_sdk/tier1.py`). Neither is re-exported from the package index
yet, because those index files had uncommitted changes when this was written.

## Not yet implemented

- A running server does not expose its GC schedule through the API. Operators set
  the interval with `frontal-sandbox serve`.
