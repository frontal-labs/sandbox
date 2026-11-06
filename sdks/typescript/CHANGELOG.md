# @frontal-labs/sandbox-sdk

## 0.2.0

### Minor Changes

- 68939d9: Breaking (0.x minor): the TypeScript SDK now uses camelCase names for types, fields, and parameters, and converts them to the server's snake_case wire format. Migrate `tenant_id` to `tenantId`, `tier_preference` to `tierPreference`, `default_deny` to `defaultDeny`, `exit_code` to `exitCode`, and so on. The wire format is unchanged.
  
  Also new: `Tier1Client` for checkpoints, undo, `info`, and fork with a command, exported from the package root. Filesystem paths containing `.` or `..` segments are refused with `INVALID_ARGUMENT` before any request is sent, and session IDs are percent-encoded in URLs.

### Patch Changes

- 7a93906: Build Bazel workspace and pass Rust tests on macOS.
- f77f5c0: Docs: the README links to PLAN.md have been removed.
- 8a40971: Rebrand references to Frontal Sandbox: README and install docs now use the scoped package name, and the packaged tarball name matches the npm name.
- 240ad9a: Update vitest to 4.1.12 to resolve security advisories GHSA-qxg6-89mg-rqv8 and GHSA-xp26-3454-mjgp
- d4cec06: Update zod to 4.6.7 to resolve security advisory GHSA-mrhx-c7xc-m3w5 (CVE-2023-54404)

## 0.1.0

### Minor Changes

- First release of the TypeScript, Python, and Rust SDKs. Each one is a client for `sandbox-server`: sessions, exec, files, snapshots, and forks. Streaming exec is refused with `UNSUPPORTED`.
