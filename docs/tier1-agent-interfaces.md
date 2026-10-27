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

The trace has its own cap (8 MiB per session). Streamed `exec` output is not capped.

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
twice. Unpinned blocks are not freed until checkpoint and blob GC lands, so the
figure is not the storage the server holds.

## Undo

`POST /v1/sessions/{id}/undo` with `{"steps": N}` restores the filesystem from N
checkpoints ago. The current state is pinned first, so undo can be reversed. Each
step drops the newest checkpoint. If fewer than N steps exist, the request fails
with `INVALID_ARGUMENT`.

Undo restores the filesystem only. A Tier 1 session keeps no processes between
commands, so none are lost. The undo itself is not recorded in the trace.

## Fork with a command

`POST /v1/sessions/{id}/fork` with `{"count": N, "argv": [...]}` pins the current
state, creates N independent children from it, and runs `argv` in each. The
response reports each child's session ID, exit code or error, and changed-file
count. Each child's command is recorded in that child's trace.

## Surfaces

| Operation | Server route | Rust SDK | CLI |
| --- | --- | --- | --- |
| Environment | `GET /v1/sessions/{id}/info` | `info` | `frontal-sandbox sandbox info <session>` |
| Checkpoint | `POST /v1/sessions/{id}/checkpoint` | `checkpoint` | `frontal-sandbox sandbox checkpoint <session>` |
| Undo | `POST /v1/sessions/{id}/undo` | `undo` | `frontal-sandbox sandbox undo <session> [N]` |
| Fork | `POST /v1/sessions/{id}/fork` | `fork_command` | `frontal-sandbox sandbox fork <session> -c N -- <cmd>` |

The top-level `frontal-sandbox fork <snapshot>` still forks a snapshot.

## Not yet implemented

- An in-shell `sandbox` builtin. The commands above run from outside the session.
- TypeScript and Python SDK methods for the checkpoint and fork routes.
- Storage reclaim for unpinned checkpoints. This waits for checkpoint and blob GC.
- A per-key quota on checkpoint bytes. Retention is capped per session only.
