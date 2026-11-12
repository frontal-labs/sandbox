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

## Logs and request IDs

`sandbox serve` (frontal-sandbox is a legacy alias) logs to stderr. `RUST_LOG` sets the filter, and the default is `info`.
Each request gets an ID. A client may send `x-request-id` (printable, at most 128 bytes), and
the server echoes it. Otherwise the server generates one. The response carries the same
header. Each request is logged once, with its method, path, status, and duration, under a span
that holds the ID. API keys are not logged.

### Trace context

Requests follow the W3C `traceparent` header. A valid incoming trace ID is kept, and the
response carries it with a new span ID for this server, so a caller can follow a request into
the server's logs by its `trace_id`. Without a valid header, a new trace starts. This is the
trace-context format OpenTelemetry uses. Spans are not exported to a collector yet: the logs
carry the IDs, and an exporter is still to be built.

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
- No span export to an OpenTelemetry collector. Trace IDs are in the logs and the headers.
- No billing export. Usage is in memory only.
- No tenant-level quota. Only the per-key limits above exist.
- No Tier 2 (VM) sessions. Requests that need a VM are refused.
