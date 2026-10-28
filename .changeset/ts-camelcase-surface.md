---
"@frontal-labs/sandbox-sdk": minor
---

Breaking (0.x minor): the TypeScript SDK now uses camelCase names for types, fields, and parameters, and converts them to the server's snake_case wire format. Migrate `tenant_id` to `tenantId`, `tier_preference` to `tierPreference`, `default_deny` to `defaultDeny`, `exit_code` to `exitCode`, and so on. The wire format is unchanged.

Also new: `Tier1Client` for checkpoints, undo, `info`, and fork with a command, exported from the package root. Filesystem paths containing `.` or `..` segments are refused with `INVALID_ARGUMENT` before any request is sent, and session IDs are percent-encoded in URLs.
