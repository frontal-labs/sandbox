# Documentation

This directory holds the project's documentation. Most files describe the
target design from [plans/PLAN.md](../plans/PLAN.md). Where a feature is not
implemented, the "Current State" sections say so.

## Guides

| File | What it covers |
|------|----------------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System overview, invariants, crate structure, data flows |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Building, testing, releasing, and day-to-day development |
| [TESTING.md](TESTING.md) | What tests exist, planned strategy, and how to run them |
| [DEPLOYMENT.md](DEPLOYMENT.md) | What runs today and what is planned for production |
| [SECURITY_MODEL.md](SECURITY_MODEL.md) | The security model under design |
| [THREAT_MODEL.md](THREAT_MODEL.md) | Threat model |
| [LIMITS.md](LIMITS.md) | Session and resource limits |
| [MILESTONES.md](MILESTONES.md) | Milestone plan |
| [PERFORMANCE.md](PERFORMANCE.md) | Performance targets and measurements |
| [API_REFERENCE.md](API_REFERENCE.md) | API reference |
| [BUILD_SYSTEM.md](BUILD_SYSTEM.md) | Build system (Nix + Bazel) |

## Architecture Decision Records

[docs/adr/README.md](adr/README.md) — 27 accepted ADRs and 2 proposed.
Each records a significant design decision with context, consequences, and
alternatives.

## Benchmark Decisions

[docs/decisions/README.md](decisions/README.md) — Records decisions driven by
benchmark results. Empty until the first one is written.