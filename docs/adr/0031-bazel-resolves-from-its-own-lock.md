# 0031: Bazel resolves crates from its own lock

## Status
Accepted

## Date
2026-11-03

## Context
The Bazel build takes its external crates from `crate.from_cargo` in `MODULE.bazel`. Under that rule, `rules_rust` read and wrote the workspace `Cargo.lock`. Two problems followed:

- Bazel adds a helper package, `direct-cargo-bazel-deps`, for the optional features that its `crate.spec` entries request. Each build wrote it into `Cargo.lock`. `cargo metadata --locked` then refused the lock, so the file could not be committed as cargo's canonical lock.
- A `crate.spec` for `sqlx` pinned 0.8.6, while the workspace uses 0.9.0 with the same features. Bazel built a second sqlx stack, which brought in `rsa`. RUSTSEC-2023-0071 covers `rsa`, and the workspace had removed it.

Before this change the Bazel build did not run at all (PLAN §14 recorded that the targets were not updated for the new dependencies).

## Decision
- `from_cargo` reads `Cargo.Bazel.lock`, a copy of `Cargo.lock` that Bazel may change. `Cargo.lock` is maintained by cargo alone, and `cargo metadata --locked` passes on it.
- The `sqlx` spec matches the workspace version, 0.9.0.
- `sqlx-postgres` is built with its `offline` feature. The universe already builds `sqlx-core` with that feature, and the Executor trait requires `describe` only under it. Cargo does not enable it, and the Cargo build is unchanged.
- The Bazel targets declare every crate their sources import.

## Consequences
### Positive
- Bazel and cargo keep separate locks, and the rsa entry does not come back through Bazel.
- `bazel build //crates/... //cli/... //sdks/rust/...` succeeds, and `bazel test //:test_rust` passes 37 of 37 tests.

### Negative
- `Cargo.Bazel.lock` has to be refreshed when workspace dependencies change. Bazel rewrites it on the next build, so the change appears in the diff.
- The `offline` annotation is a Bazel-only feature choice. If the universe changes, it has to be revisited.

### Risks
- Two lock files can drift. The check is to build with Bazel after a dependency change and commit the refreshed `Cargo.Bazel.lock`.

## Alternatives Considered
- `skip_cargo_lockfile_overwrite`. Rejected: the splice step then needs the synthetic package in the input lock, so the build fails under `--locked`.
- `strip_internal_dependencies_from_cargo_lockfile`. Rejected: it removed the workspace's own packages from the lock.
- Drop the optional `crate.spec` entries. Rejected for now: the clustered scheduler and vsock targets depend on them.

## References
- ADR 0026 (package layers and lockfile).
- PLAN §14, the Bazel item.
