# Claude Code Commands for Frontal Sandbox

Project commands should be added as the planned Rust crates and service tools
are implemented. Use the following command families for the current roadmap:

- `/sandbox:plan` — review `PLAN.md` and identify the next milestone.
- `/sandbox:architecture` — inspect boundaries, invariants, and data flow.
- `/sandbox:security` — review capabilities, isolation, and promotion paths.
- `/sandbox:types` — work on shared IDs, errors, events, and configuration.
- `/sandbox:vfs` — work on the copy-on-write virtual filesystem.
- `/sandbox:shell` — work on parsing, interpretation, and builtins.
- `/sandbox:session` — work on the session and backend boundary.
- `/sandbox:trace` — work on event logging, diffs, and replay.
- `/sandbox:docs` — update design and API documentation.

Each command should inspect first, make a focused change, and report validation
honestly. Do not run project commands when the user explicitly requests review or
editing without execution.
