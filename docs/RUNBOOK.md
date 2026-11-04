# Runbook

What the server exposes, and what to check when something looks wrong. It describes the
code as it is. Items that are not built are listed under "Not yet available".

## Metrics

`GET /metrics` returns Prometheus text (version 0.0.4). It needs an API key, as every route
except `/v1/openapi.json` does. Scrape it with `Authorization: Bearer <key>`.

| Metric | Type | Meaning |
| --- | --- | --- |
| `sandbox_sessions_created_total` | counter | sessions created since the server started |
| `sandbox_sessions_slept_total` | counter | sessions put to sleep |
| `sandbox_sessions_woken_total` | counter | sessions woken |
| `sandbox_execs_total` | counter | commands run |
| `sandbox_exec_wall_ms_total` | counter | wall time of those commands, in milliseconds |
| `sandbox_bytes_written_total` | counter | bytes written through the filesystem API |
| `sandbox_sessions_active` | gauge | sessions the server holds now |
| `sandbox_snapshots_active` | gauge | snapshots the server holds now |

The counters are in memory and reset on restart, so a rate can drop to zero after one.
They sum every key, so they carry no tenant label. Per-key usage is at `GET /v1/usage`.

## Limits that refuse new work

These return `RESOURCE_EXHAUSTED`. They are deliberate.

| Limit | Value | What to do |
| --- | --- | --- |
| Sessions per key | 64 | destroy idle sessions, or use another key |
| Sessions per server | 4,096 | raise the constant, after measuring memory (`examples/capacity.rs`) |
| Snapshots per key | 32 | delete snapshots, which frees the key's share |

Sleeping a session takes a snapshot, so a key that sleeps many sessions can reach the
snapshot limit first.

## Garbage collection

`POST /v1/gc` runs a collection, at most once a minute. The server also runs one every
600 seconds by default (`--gc-interval-secs`). A collection holds every session's gate, so
a long command delays it.

## Not yet available

- No health endpoint beyond the OpenAPI document.
- No tracing export, and no OpenTelemetry spans.
- No billing export. Usage is in memory only.
- No tenant-level quota. Only the per-key limits above exist.
- No Tier 2 (VM) sessions. Requests that need a VM are refused.
