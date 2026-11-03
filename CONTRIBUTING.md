# Contributing to Frontal Sandbox

Thank you for helping build Frontal Sandbox. Changes should align with the project's design and milestone acceptance criteria.

## Getting started

```bash
git clone https://github.com/frontal-labs/sandbox.git
cd sandbox
```

Build, lint, and test commands are listed in [README.md](README.md#development).

The repository is currently in the foundation stage. New implementation work
should follow the planned crate boundaries and preserve the core invariants:

- The VFS is the source of truth for every backend.
- The SDK does not depend on the active execution tier.
- Host access goes through capability checks.
- State changes emit trace events.
- Isolates are not a security boundary on their own.

## Changes

Use focused branches and Conventional Commits, for example
`feat(vfs): add rename`. Suggested scopes include `types`, `vfs`, `shell`,
`runtime`, `policy`, `session`, `snapshot`, `trace`, `server`, `scheduler`,
`sdk`, `cli`, and `docs`. PR titles are checked against the Conventional Commits
pattern (types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `perf`, `ci`,
`build`, `style`, `revert`). Keep public APIs small, document
security-sensitive decisions, and add tests with each implementation milestone.
Avoid adding ambient host access or coupling the SDK to a specific backend.

Before opening a pull request, review the relevant acceptance criteria and describe what was verified. Do not claim that checks were run when
they were not.

## Reporting issues

Use [GitHub Issues](https://github.com/frontal-labs/sandbox/issues) for bugs and
feature requests. Report security vulnerabilities privately to
[security@frontal.dev](mailto:security@frontal.dev); see [SECURITY.md](SECURITY.md).
