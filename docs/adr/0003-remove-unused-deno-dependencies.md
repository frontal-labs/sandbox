# 0003: Remove unused Deno dependencies instead of patching them

## Status
Accepted. Supersedes 0002.

## Date
2026-10-14

## Context
ADR 0002 patched seven Deno crates to clear advisories in their upstream pins. That work assumed `sandbox-runtime` needed `deno_fetch` and `deno_crypto`. Its code uses only `deno_core` and registers no Deno web extensions, so both crates were declared but unused.

## Decision
- Remove `deno_fetch` and `deno_crypto` from the workspace and from `sandbox-runtime`.
- Remove all `[patch.crates-io]` entries. Nothing else in the graph depended on the patched crates.
- The `frontal-labs` Deno repos are no longer used by the build. They can be deleted.

## Consequences
### Positive
- The advisories no longer apply to the build, because the affected crates are not in `Cargo.lock`.
- The build needs no network access to private or public Git sources, and the Bazel `.bazelrc` git settings are gone.

### Negative
- `plans/PLAN.md` section 4.5 lists `deno_fetch` and `deno_crypto` as planned runtime extensions (web fetch and WebCrypto). Adding them back means pulling upstream crates again. Upstream still pins the vulnerable versions, so the patch approach in 0002 would return with them, or the releases must be newer by then. Revisit before enabling those extensions.
- Runtime `fetch()` and `crypto.subtle` are unavailable until then. The sandbox has never exposed them.

## Alternatives considered
- Keep the patches. Rejected: they guard code that is not built.
