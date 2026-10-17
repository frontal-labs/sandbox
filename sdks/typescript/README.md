# Sandbox SDK (TypeScript)

TypeScript SDK for [Frontal Sandbox](https://github.com/frontal-labs/sandbox), the execution platform for AI agents. The npm package is `@frontal-labs/sandbox-sdk`. Source is in `sdks/typescript/`.

**Status: early.** The client covers sessions, exec, snapshots, and forks over the server's `/v1` API. Field names and byte encodings match the server's wire format, and the tests check that against server-shaped responses and against a live server. Streaming exec is refused with `UNSUPPORTED`. "Embedded" mode is a wrapper over a remote server, not a local backend.

## Installation

Releases are pushed to GHCR as an OCI artifact, `ghcr.io/frontal-labs/sandbox-sdk-ts`. npm cannot install from GHCR, so pull the packed tarball with [`oras`](https://oras.land) and install that file:

```bash
oras pull ghcr.io/frontal-labs/sandbox-sdk-ts:0.1.0
npm install ./frontal-labs-sandbox-sdk-0.1.0.tgz
```

If the package is private, run `oras login ghcr.io` first. To build from source instead:

```bash
cd sdks/typescript
npm install
npm run build   # tsc --build, emits dist/
```

Requires Node.js 20+ (per `engines` in `package.json`).

## Quick Start

This shows the usage against a running server.

```typescript
import { Client, isSandboxError } from "@frontal-labs/sandbox-sdk"

// Explicit configuration
const client = new Client({
  baseUrl: "http://localhost:8080",
  apiKey: "sk_...", // optional
})

// Or read SANDBOX_BASE_URL and SANDBOX_API_KEY from the environment
// const client = Client.fromEnv()

const session = await client.createSession({
  tenant_id: "my-tenant",
  tier_preference: "auto",
})
if (isSandboxError(session)) throw session

const result = await client.exec(session, {
  argv: ["npm", "test"],
  cwd: "/work",
})

if (isSandboxError(result)) {
  console.error("Execution failed:", result.code, result.message)
} else {
  console.log("Exit code:", result.exit_code)
  console.log("Stdout:", new TextDecoder().decode(result.stdout))
}
```

## Embedded Mode (not implemented as described)

`EmbeddedSandbox` does not run a local backend. Today it is a thin wrapper over `Client`:

- `EmbeddedSandbox.new()` checks `http://localhost:8080/health` when `SANDBOX_BASE_URL` is unset, then creates a session on that server. It returns `UNAVAILABLE` if the check fails. The server does not route `/health` yet.
- `serverPath` in `EmbeddedSandboxConfig` is accepted and ignored.
- `exec(command)` splits the command on whitespace into argv and runs it through the remote `Client`. Quoting is not interpreted, and the isolate shell has no `sh`.

Local in-process execution is planned. See `PLAN.md`.

```typescript
import { EmbeddedSandbox, isSandboxError } from "@frontal-labs/sandbox-sdk"

const sandbox = await EmbeddedSandbox.new({ tenant_id: "local-dev" })
if (isSandboxError(sandbox)) throw sandbox

const result = await sandbox.exec("echo hi")
await sandbox.destroy()
```

## API Reference

Status key: **Implemented** means the code calls the server; it does not mean the server responds today.

### Client

#### `new Client(config: ClientConfig)`

```typescript
interface ClientConfig {
  baseUrl: string      // e.g. "http://localhost:8080"
  apiKey?: string      // sent as "Authorization: Bearer <key>"
}
```

#### `Client.fromEnv(): Client`

Reads `SANDBOX_BASE_URL` (default `http://localhost:8080`) and `SANDBOX_API_KEY` (optional).

#### `client.createSession(config: SessionConfig): Promise<Result<string>>`

Validates the config with Zod, then POSTs it. Returns the session ID.

```typescript
interface SessionConfig {
  tenant_id: string                             // required, min length 1
  tierPreference?: "auto" | "isolate" | "vm"    // default "auto"
  limits?: ResourceLimits
  network?: NetworkPolicy                       // default default_deny: true
  env?: Record<string, string>
  cwd?: string                                  // default "/work"
  initialSnapshot?: string
  secrets?: SecretsConfig
  labels?: Record<string, string>
}
```

#### `client.getSession(sessionId: string): Promise<Result<SessionState>>`

Gets session status and metadata.

#### `client.destroySession(sessionId: string): Promise<Result<void>>`

Destroys a session.

#### `client.exec(sessionId: string, request: ExecRequest): Promise<Result<ExecResult>>`

Runs a command and waits for the result (non-streaming).

```typescript
interface ExecRequest {
  argv: string[]          // at least one non-empty element
  env?: Record<string, string>
  cwd?: string
  stdin?: Uint8Array
  timeoutMs?: number
  stream?: boolean        // default false
}

interface ExecResult {
  exit_code: number
  stdout: Uint8Array
  stderr: Uint8Array
  cpuMs: number
  wallMs: number
}
```

#### `client.execStream(sessionId: string, request: ExecRequest): AsyncGenerator<Result<ExecResult>>`

POSTs to the exec endpoint with `?stream=true` and parses newline-delimited JSON. Each line must match the `ExecResult` schema, so the server must send full `ExecResult` objects per line. The streaming chunk format is not defined by the server yet.

```typescript
for await (const chunk of client.execStream(sessionId, { argv: ["npm", "test"] })) {
  if (isSandboxError(chunk)) {
    console.error("Error:", chunk.message)
    break
  }
  process.stdout.write(new TextDecoder().decode(chunk.stdout))
}
```

#### `client.snapshot(sessionId: string): Promise<Result<Snapshot>>`

Creates a snapshot of the session.

#### `client.listSnapshots(sessionId: string): Promise<Result<Snapshot[]>>`

Lists snapshots for a session.

#### `client.getSnapshot(sessionId: string, snapshotId: string): Promise<Result<Snapshot>>`

Gets one snapshot.

### EmbeddedSandbox

See "Embedded Mode" above.

- `EmbeddedSandbox.new(config: EmbeddedSandboxConfig): Promise<Result<EmbeddedSandbox>>`
- `sandbox.exec(command: string): Promise<Result<ExecResult>>`
- `sandbox.snapshot(): Promise<Result<Snapshot>>`
- `sandbox.destroy(): Promise<Result<void>>`
- `sandbox.getSessionId(): string | undefined`

### Types

Zod schemas and inferred types are exported from `@frontal-labs/sandbox-sdk/types` (and re-exported from the root). Examples: `SessionConfigSchema`, `ExecRequestSchema`, `ExecResultSchema`, `SessionStateSchema`, `SnapshotSchema`, `TierSchema`, `TierPreferenceSchema`, and the `ResourceLimits`, `NetworkPolicy`, `NetworkPolicyRule`, `SecretsConfig`, and `SecretEntry` types. See `sdks/typescript/src/types/index.ts`.

### Errors

Errors are returned as values, not thrown. `Result<T>` is `T | SandboxError`. `SandboxError` is an interface extending `Error` with a `code` field.

```typescript
import { isSandboxError, SandboxErrorCodes, type Result, type SandboxError } from "@frontal-labs/sandbox-sdk/errors"
```

`ErrorCode` values: `INTERNAL`, `INVALID_ARGUMENT`, `NOT_FOUND`, `ALREADY_EXISTS`, `PERMISSION_DENIED`, `RESOURCE_EXHAUSTED`, `UNAVAILABLE`, `TIMEOUT`, `CANCELLED`, `INVALID_STATE`, `UNSUPPORTED`, `POLICY_DENIED`, `PROMOTION_FAILED`, `SNAPSHOT_FAILED`, `VM_BOOT_FAILED`, `NETWORK_ERROR`, `CAS_ERROR`, `VFS_ERROR`, `SHELL_ERROR`, `RUNTIME_ERROR`, `WASM_ERROR`.

Note: the client currently maps every non-OK HTTP response to `INTERNAL`. It does not map HTTP status codes to `NOT_FOUND`, `POLICY_DENIED`, or `TIMEOUT`, so branch on those codes only once the server maps them.

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SANDBOX_BASE_URL` | Sandbox server URL | `http://localhost:8080` |
| `SANDBOX_API_KEY` | API key sent as a bearer token | none |

## Known issues

- **Not routed on the server:** the `GET` session list and the `/health` endpoint that `EmbeddedSandbox.new` checks. Any HTTP answer from that path counts as reachable.
- **Tests.** `tests/types.test.ts` covers the types. `tests/wire.test.ts` checks the wire format with a stubbed `fetch`, and runs one live test when `SANDBOX_BASE_URL` and `SANDBOX_API_KEY` are set.
- **Streaming exec, sleep, VFS watch, replay, and bench** are not implemented. Streaming exec is refused with `UNSUPPORTED`.

## Development

Build and tests run through Bazel from the repo root (see the `ts:*` scripts in the root `package.json`). Package scripts also work from this directory:

```bash
bun run build      # tsc --build
bun run lint       # biome check src
bun run test       # vitest run
```

These commands have not been run as part of this doc refresh.

## Project Structure

```
src/
├── index.ts           # Re-exports the modules below
├── client/
│   └── index.ts       # HTTP client for sandbox-server
├── embedded/
│   └── index.ts       # EmbeddedSandbox wrapper over Client (needs a server)
├── errors/
│   └── index.ts       # SandboxError type and constructors
└── types/
    └── index.ts       # Zod schemas and inferred types
tests/
└── types.test.ts      # Type-level tests
```

## License

Apache-2.0
