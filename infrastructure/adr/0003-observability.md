# 0003: Observability

## Status
Accepted

## Date
2026-10-17

## Context
plans/PLAN.md section 10 lists metrics, logs, and traces as planned but not defined
in code. Infrastructure needs a concrete, minimal observability surface that
the service crates will implement against.

## Decision
- Metrics: Prometheus scrape at `/metrics` on each service (9090 for server).
  Metric names: `sandbox_session_starts_total`, `sandbox_session_start_latency_seconds`,
  `sandbox_exec_duration_seconds`, `sandbox_promotion_total`, `sandbox_policy_denials_total`,
  `sandbox_oom_kills_total`, `sandbox_cas_hit_rate`, `sandbox_pool_depth`,
  `sandbox_idle_sessions`, `sandbox_cpu_ms_total`, `sandbox_memory_gb_seconds`.
- Alerts and recording rules: `monitoring/alerting-rules.yaml`, `recording-rules.yaml`.
- Dashboard: `monitoring/grafana/dashboard.json`.
- Logs: structured JSON with request ID and session ID (server middleware stub exists).
- Traces: OpenTelemetry via `otlp_endpoint` in config (planned, not wired).

## Consequences
### Positive
- Operators get dashboards and alerts on day one of M5.
- Metric names are fixed, so service crates implement to a contract.

### Negative
- Metric names are not yet in code; service crates must add them (tracking issue).
- Alert thresholds are starting guesses; tune after production data.

## Alternatives Considered
- Datadog / CloudWatch: rejected; Prometheus is self-hosted and matches the
  on-prem / bring-your-own-host model.

## References
- plans/PLAN.md sections 6, 10
- docs/PERFORMANCE.md
