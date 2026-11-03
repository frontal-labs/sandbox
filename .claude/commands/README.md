# Claude Code Commands for Frontal Sandbox

This directory currently contains no slash command definitions (no `*.md`
command files). `aliases.sh` holds shell shortcuts only; see its notes below.

Planned command families, not yet implemented:

- `/sandbox:plan`: review the project specification and identify the next milestone.
- `/sandbox:architecture`: inspect boundaries, invariants, and data flow.
- `/sandbox:security`: review capabilities, isolation, and promotion paths.
- `/sandbox:types`: work on shared IDs, errors, events, and configuration.
- `/sandbox:vfs`: work on the copy-on-write virtual filesystem.
- `/sandbox:shell`: work on parsing, interpretation, and builtins.
- `/sandbox:session`: work on the session and backend boundary.
- `/sandbox:trace`: work on event logging, diffs, and replay.
- `/sandbox:docs`: update design and API documentation.

Each command should inspect first, make a focused change, and report validation
honestly. Do not run project commands when the user explicitly requests review or
editing without execution.

Note: several `aliases.sh` entries call scripts that do not exist in the root
`package.json` (`dev`, `build`, `test`, `typecheck`, `lint:fix`, and the
`cargo:*` scripts).
Use the commands in the root [README.md](../../README.md#development) instead.
