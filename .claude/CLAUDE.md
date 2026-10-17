# Claude Code Configuration for Frontal Sandbox

This file provides project-specific guidance for working on Frontal Sandbox.

## Working principles

1. Read `plans/PLAN.md` before changing architecture or public interfaces.
2. Preserve the VFS, capability, tier-independence, and trace invariants.
3. Keep isolate and microVM concerns behind the session boundary.
4. Make security assumptions explicit in code and documentation.
5. Update documentation when behavior or API design changes.
6. Do not claim validation was performed unless it was actually performed.

## Project direction

Frontal Sandbox is a Rust-first, isolate-first execution platform for AI agents.
The planned components are listed in `plans/PLAN.md`, from `sandbox-types` and
`sandbox-vfs` through the server, scheduler, SDK, and CLI.

## Commit convention

Use Conventional Commits with scopes such as `types`, `vfs`, `shell`, `runtime`,
`policy`, `session`, `snapshot`, `trace`, `server`, `scheduler`, `sdk`, `cli`,
and `docs`.
