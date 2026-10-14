# 0002: Patched Deno crates from frontal-labs repos

## Status
Accepted

## Date
2026-10-14

## Context
`sandbox-runtime` depends on several Deno crates. The upstream releases of those crates pin versions that have open advisories, and Cargo cannot move them without a patch:

- `deno_tls` 0.246.0 and `deno_fetch` 0.283.0 pin `rustls =0.23.40`. Advisory: TLS 1.3 handshake messages accepted across encryption-level boundaries.
- `deno_fetch` 0.283.0 and `deno_net` 0.251.0 use `hickory-resolver` / `hickory-proto` 0.25. Advisories: DNSSEC validation, CNAME handling, NSEC3 looping, and O(n²) name compression. hickory-proto 0.25 has no patched release.
- `deno_crypto`, `deno_fs`, `deno_io`, and `deno_subprocess_windows` pin `rand =0.8.5`. Advisory: unsound with a custom logger that calls `rand::rng()`.

Those pins sit in the monorepo workspace, so they cannot be changed with a version bump.

## Decision
Build the affected crates from standalone repositories in the `frontal-labs` org, and apply them through `[patch.crates-io]` in the workspace `Cargo.toml`:

| Repo | Upstream | Change |
| --- | --- | --- |
| `frontal-labs/deno_tls` | `ext/tls` 0.246.0 | rustls 0.23.45 |
| `frontal-labs/deno_net` | `ext/net` 0.251.0 | hickory 0.26; Apple system DNS config read from `/etc/resolv.conf` |
| `frontal-labs/deno_fetch` | `ext/fetch` 0.283.0 | rustls 0.23.45; hickory 0.26; same Apple DNS config |
| `frontal-labs/deno_crypto` | `ext/crypto` 0.273.0 | rand 0.8.6 |
| `frontal-labs/deno_fs` | `ext/fs` 0.169.0 | rand 0.8.6 |
| `frontal-labs/deno_io` | `ext/io` 0.169.0 | rand 0.8.6 (Windows only) |
| `frontal-labs/deno_subprocess_windows` | `runtime/subprocess_windows` 0.54.0 | rand 0.8.6 |

Each repo is a copy of the crate at tag `v2.9.7`. Workspace dependency versions are pinned explicitly, and each repo has a `NOTICE.md` listing its changes. The repos are private.

The upstream `deno` monorepo fork (`frontal-labs/deno`) was used only to find the release commit. It is not a dependency and has been removed.

## Consequences
### Positive
- The advisories above are cleared in `Cargo.lock`, without giving up the Deno crates.
- The hickory port keeps the public `deno_net` and `deno_fetch` APIs, and the existing unit tests pass.

### Negative
- Seven repos to maintain, and each upstream Deno release has to be re-merged by hand.
- On Apple platforms, `deno_net` and `deno_fetch` read `/etc/resolv.conf` instead of SystemConfiguration. Scoped IPv6 DNS servers such as `fe80::1%en0` keep working, but VPN-specific resolver settings are not picked up on macOS.
- `deno_fetch`'s proxy code carries upstream advisories of its own. `sandbox-runtime` does not register `deno_fetch` ops, so the proxy code is not reachable from sandbox code today.

### Alternatives considered
- **Remove the unused dependencies.** `sandbox-runtime` declares `deno_fetch` and `deno_crypto` but its code uses only `deno_core`. Removing them would drop the affected crates from the graph and need no patches. Recommended as the follow-up, to be decided separately.
- **Wait for upstream releases.** Upstream still pins these versions in the releases checked (v2.9.7).
