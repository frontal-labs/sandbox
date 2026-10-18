# 0001: Config schema and loading

## Status
Accepted

## Date
2026-10-17

## Context
The server, scheduler, and worker each need configuration (listen addresses,
data dirs, auth, telemetry, pool sizes, Tier 2 flags). Today these are hardcoded
or passed as CLI flags with no schema, no defaults file, and no validation.
Operators need a single source of truth, environment-variable overrides, and a
machine-checkable schema.

## Decision
- Format: TOML (`infrastructure/config/defaults.toml` + `config/examples/*.toml`).
- Schema: JSON Schema under `infrastructure/config/schema/{server,scheduler,worker}.schema.json`.
- Rust side: each service crate gets a `config` module with a `Config` struct deriving `Deserialize` + `JsonSchema` (schemars is a workspace dependency).
- Loading precedence (highest wins): `SANDBOX_<SECTION>_<KEY>` env vars > `--config` TOML > `defaults.toml` compiled in > hardcoded Rust defaults.
- CLI flag: `--config <path>` on `serve`, `scheduler`, `worker` subcommands; default `/etc/sandbox/config.toml`.

## Consequences
### Positive
- One config file per environment; examples are validated against schemas in CI.
- Env overrides support 12-factor deployment without editing files.
- JSON Schema gives editors autocompletion and CI validation.

### Negative
- Adds a `toml` dependency to each service crate (or a shared `sandbox-config` crate).
- Schema drift between Rust structs and JSON Schema files must be kept in sync by hand (or via a codegen step later).

## Alternatives Considered
- YAML: rejected, no comments-free parsing simplicity edge cases and indention bugs.
- HCL: overkill, not needed for this surface.
- Pure CLI flags: insufficient for complex nested config (pools, resources, tier2).

## References
- plans/PLAN.md section 4 (configuration design)
- `infrastructure/config/`
