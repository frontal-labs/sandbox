# 0036: TOML configuration for `serve` (infrastructure/PLAN.md §4)

## Status
Accepted

## Date
2026-11-07

## Context
`infrastructure/PLAN.md` §4 defines a TOML config schema (`/etc/sandbox/config.toml`),
`--config`, and precedence `SANDBOX_*` env > config file > built-in defaults > hardcoded
defaults. The server and CLI previously accepted only explicit flags and a few ad hoc
environment variables, so the infra examples/schemas were not consumable by the runtime.

## Decision
- Add `toml` (workspace) and `sandbox-server` config loading in
  `crates/sandbox-server/src/config.rs`: `ServerConfigFile` (serde, `deny_unknown_fields`,
  tolerant nested tables for schema sections not yet mapped), `ResolvedServeConfig`, and
  `load(path)`.
- Precedence for mapped `serve` fields: explicit CLI flags > `SANDBOX_*` / OTLP env
  variables (`OTEL_EXPORTER_OTLP_ENDPOINT`, `SANDBOX_DATABASE_URL`, etc.) > TOML file >
  defaults. An explicit missing `--config` path is an error; a missing default
  `/etc/sandbox/config.toml` (when `--config` is omitted) is not.
- CLI `serve` gains `--config` and makes mapped flags optional overrides. VM/qemu/accel/
  memory/boot/warm/python/bucket/endpoint remain CLI-resolved where the current runtime
  requires concrete local paths or types not fully represented in infra examples.
- `ResolvedServeConfig` carries `python_artifacts` for CLI override plumbing; it is not
  read from the TOML schema.

## Consequences
### Positive
- Infra TOML examples (`infrastructure/config/examples/*.toml`) can be parsed and drive
  core serve settings (addr, data_dir, idle sleep, telemetry OTLP endpoint, Postgres DSN,
  CAS bucket mapping where applicable) with tested precedence.
- `deny_unknown_fields` catches schema drift in mapped sections.

### Negative
- Not all infra schema fields affect runtime yet (auth/TLS/metering/pools/worker
  isolation/resource details are accepted as tables or ignored by behavior); full
  parity requires follow-up wiring in server/scheduler/worker.
- `ResolvedServeConfig` is serve-scoped; scheduler/worker daemons still need their own
  loaders per infra plan §4.2.

### Risks
- Env-var mutation in tests requires synchronization; config tests serialize env access.

## Alternatives Considered
- JSON/YAML config: rejected for operator ergonomics; plan specifies TOML.
- Full `schemars`-generated structs for every field now: deferred; current structs match
  shipped examples and fail closed on unknown fields in mapped structures.

## References
- `infrastructure/PLAN.md` §4
- `infrastructure/config/examples/{dev,staging,production}.toml`
- `infrastructure/config/schema/*.schema.json`
