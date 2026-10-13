# @frontal/sandbox-sdk

TypeScript SDK for [Frontal Sandbox](https://github.com/frontal-labs/sandbox) — instant, low-cost execution environments for AI agents.

## Installation

```bash
npm install @frontal/sandbox-sdk
# or
bun add @frontal/sandbox-sdk
# or
pnpm add @frontal/sandbox-sdk
```

Requires Node.js 20+ or Bun 1.0+.

## Quick Start

```typescript
import { Client, EmbeddedSandbox } from "@frontal/sandbox-sdk"

// Connect to a running sandbox server
const client = new Client({
  baseUrl: "http://localhost:8080",
  apiKey: "sk_..." // optional, from env SANDBOX_API_KEY
})

// Or auto-detect from environment
const client = Client.fromEnv()

// Create a session
const session = await client.createSession({
  tenantId: "my-tenant",
  tierPreference: "auto", // auto-promote to VM when needed
})

// Execute a command
const result = await client.exec(session, {
  argv: ["npm", "test"],
  cwd: "/work",
})

if (result instanceof Error) {
  console.error("Execution failed:", result.message)
} else {
  console.log("Exit code:", result.exitCode)
  console.log("Stdout:", new TextDecoder().decode(result.stdout))
}
```

## Embedded Mode (No Server Required)

Run sandbox locally without a separate server process:

```typescript
import { EmbeddedSandbox } from "@frontal/sandbox-sdk"

const sandbox = await EmbeddedSandbox.new({
  tenantId: "local-dev",
  tierPreference: "isolate",
})

const result = await sandbox.exec("git clone https://github.com/x/y && cd y && npm test")

await sandbox.destroy()
```

## API Reference

### Client

#### `new Client(config: ClientConfig)`

```typescript
interface ClientConfig {
  baseUrl: string      // e.g. "http://localhost:8080"
  apiKey?: string      // optional, can use SANDBOX_API_KEY env var
}
```

#### `Client.fromEnv(): Client`

Creates a client from environment variables:
- `SANDBOX_BASE_URL` (default: `http://localhost:8080`)
- `SANDBOX_API_KEY` (optional)

#### `client.createSession(config: SessionConfig): Promise<Result<string>>`

Creates a new session. Returns session ID on success.

```typescript
interface SessionConfig {
  tenantId: string
  tierPreference?: "auto" | "isolate" | "vm"
  limits?: ResourceLimits
  network?: NetworkPolicy
  env?: Record<string, string>
  cwd?: string
  initialSnapshot?: string
  secrets?: SecretsConfig
  labels?: Record<string, string>
}
```

#### `client.getSession(sessionId: string): Promise<Result<SessionState>>`

Gets session status and metadata.

#### `client.destroySession(sessionId: string): Promise<Result<void>>`

Destroys a session and frees resources.

#### `client.exec(sessionId: string, request: ExecRequest): Promise<Result<ExecResult>>`

Executes a command (non-streaming).

```typescript
interface ExecRequest {
  argv: string[]
  env?: Record<string, string>
  cwd?: string
  stdin?: Uint8Array
  timeoutMs?: number
  stream?: false
}

interface ExecResult {
  exitCode: number
  stdout: Uint8Array
  stderr: Uint8Array
  cpuMs: number
  wallMs: number
}
```

#### `client.execStream(sessionId: string, request: ExecRequest): AsyncGenerator<Result<ExecResult>>`

Executes a command with streaming output (NDJSON).

```typescript
for await (const chunk of client.execStream(sessionId, { argv: ["npm", "test"], stream: true })) {
  if (chunk instanceof Error) {
    console.error("Error:", chunk.message)
    break
  }
  if (chunk.stdout.length) process.stdout.write(new TextDecoder().decode(chunk.stdout))
  if (chunk.stderr.length) process.stderr.write(new TextDecoder().decode(chunk.stderr))
  if (chunk.exitCode !== undefined) console.log("Exit:", chunk.exitCode)
}
```

#### `client.snapshot(sessionId: string): Promise<Result<Snapshot>>`

Creates a snapshot of the session.

#### `client.listSnapshots(sessionId: string): Promise<Result<Snapshot[]>>`

Lists all snapshots for a session.

#### `client.getSnapshot(sessionId: string, snapshotId: string): Promise<Result<Snapshot>>`

Gets a specific snapshot.

### EmbeddedSandbox

#### `EmbeddedSandbox.new(config: EmbeddedSandboxConfig): Promise<Result<EmbeddedSandbox>>`

Creates an embedded sandbox (starts local server if needed).

```typescript
interface EmbeddedSandboxConfig extends SessionConfig {
  serverPath?: string // path to sandbox-server binary
}
```

#### `sandbox.exec(command: string): Promise<Result<ExecResult>>`

Executes a shell command string.

#### `sandbox.snapshot(): Promise<Result<Snapshot>>`

Creates a snapshot.

#### `sandbox.destroy(): Promise<Result<void>>`

Destroys the sandbox.

#### `sandbox.getSessionId(): string | undefined`

Returns the current session ID.

### Types

All types are exported and validated with Zod schemas:

```typescript
import {
  SessionConfig,
  SessionConfigSchema,
  ExecRequest,
  ExecRequestSchema,
  ExecResult,
  ExecResultSchema,
  SessionState,
  SessionStateSchema,
  Snapshot,
  SnapshotSchema,
  Tier,
  TierPreference,
  ResourceLimits,
  NetworkPolicy,
  NetworkPolicyRule,
  SecretsConfig,
  SecretEntry,
  TierSchema,
  TierPreferenceSchema,
  // ... schemas for validation
} from "@frontal/sandbox-sdk/types"
```

### Errors

```typescript
import {
  SandboxError,
  SandboxErrorCodes,
  isSandboxError,
  Result,
} from "@frontal/sandbox-sdk/errors"

// Error codes
type ErrorCode =
  | "INTERNAL"
  | "INVALID_ARGUMENT"
  | "NOT_FOUND"
  | "ALREADY_EXISTS"
  | "PERMISSION_DENIED"
  | "RESOURCE_EXHAUSTED"
  | "UNAVAILABLE"
  | "TIMEOUT"
  | "CANCELLED"
  | "INVALID_STATE"
  | "UNSUPPORTED"
  | "POLICY_DENIED"
  | "PROMOTION_FAILED"
  | "SNAPSHOT_FAILED"
  | "VM_BOOT_FAILED"
  | "NETWORK_ERROR"
  | "CAS_ERROR"
  | "VFS_ERROR"
  | "SHELL_ERROR"
  | "RUNTIME_ERROR"
  | "WASM_ERROR"

// Result type: T | SandboxError
type Result<T> = T | SandboxError

// Check if error
if (isSandboxError(result)) {
  console.log(result.code, result.message)
}
```

## Error Handling

All methods return `Result<T>` which is a union of `T | SandboxError`. Check with `isSandboxError()`:

```typescript
const result = await client.exec(sessionId, { argv: ["ls"] })

if (isSandboxError(result)) {
  switch (result.code) {
    case "NOT_FOUND":
      console.log("Session not found")
      break
    case "POLICY_DENIED":
      console.log("Command blocked by policy")
      break
    case "TIMEOUT":
      console.log("Execution timed out")
      break
    default:
      console.error("Error:", result.message)
  }
} else {
  // Success: result is ExecResult
  console.log(new TextDecoder().decode(result.stdout))
}
```

## Streaming Example

```typescript
import { Client } from "@frontal/sandbox-sdk"

const client = Client.fromEnv()
const sessionId = await client.createSession({ tenantId: "demo" })

for await (const chunk of client.execStream(sessionId, {
  argv: ["python3", "-c", "import time; [print(i) or time.sleep(0.5) for i in range(5)]"],
  stream: true,
})) {
  if (isSandboxError(chunk)) {
    console.error("Stream error:", chunk.message)
    break
  }
  if (chunk.stdout.length) process.stdout.write(new TextDecoder().decode(chunk.stdout))
}
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SANDBOX_BASE_URL` | Sandbox server URL | `http://localhost:8080` |
| `SANDBOX_API_KEY` | API key for authentication | - |

## Development

```bash
# Install dependencies
bun install

# Build
bun run build

# Watch mode
bun run dev

# Lint
bun run lint

# Format
bun run format

# Test
bun run test

# Test watch
bun run test:watch
```

## Project Structure

```
src/
├── index.ts           # Main exports
├── client/
│   └── index.ts       # HTTP client for sandbox server
├── embedded/
│   └── index.ts       # Embedded sandbox (local server)
├── errors/
│   └── index.ts       # Error types and helpers
└── types/
    └── index.ts       # Zod schemas and TypeScript types
```

## License

Apache-2.0