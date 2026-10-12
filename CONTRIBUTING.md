# Contributing to Frontal Sandbox

Thank you for helping build Frontal Sandbox. The design and milestone
acceptance criteria live in [PLAN.md](PLAN.md); changes should align with that
document.

## Getting started

```bash
git clone https://github.com/frontal-labs/sandbox.git
cd sandbox
```

The repository is currently in the foundation stage. New implementation work
should follow the planned crate boundaries and preserve the core invariants:

- The VFS is the source of truth for every backend.
- The SDK does not depend on the active execution tier.
- Host access goes through capability checks.
- State changes emit trace events.
- Isolates are not a security boundary on their own.

## Changes

Use focused branches and Conventional Commits. Keep public APIs small, document
security-sensitive decisions, and add tests with each implementation milestone.
Avoid adding ambient host access or coupling the SDK to a specific backend.

Before opening a pull request, review the relevant acceptance criteria in
`PLAN.md` and describe what was verified. Do not claim that checks were run when
they were not.

## Reporting issues

Use [GitHub Issues](https://github.com/frontal-labs/sandbox/issues) for bugs and
feature requests. Report security vulnerabilities privately to
[security@frontal.dev](mailto:security@frontal.dev); see [SECURITY.md](SECURITY.md).
