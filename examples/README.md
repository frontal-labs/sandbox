# Frontal Sandbox SDK Examples

Examples demonstrating different use cases and features of the Frontal Sandbox TypeScript SDK. Each example is self-contained and runnable independently.

| Example | Description |
|---------|-------------|
| `sandbox-basics` | Session creation, exec, file ops, snapshot, fork, destroy |
| `filesystem-snapshots` | Create, list, and fork from filesystem snapshots |
| `code-runner` | Run code in a sandboxed Node.js environment |
| `install-packages` | Install and use system packages |
| `sleep-wake` | Put a session to sleep and wake it |
| `fork-parallel` | Fork a snapshot into parallel workers |
| `env-vars` | Pass environment variables to a session |

## Getting Started

Each example can be run independently:

```bash
cd examples/<example-name>
npm install
npm start
```

All examples require a running Frontal Sandbox server (default `http://localhost:8080`).

## Environment

- `SANDBOX_BASE_URL` (default `http://localhost:8080`)
- `SANDBOX_API_KEY` (optional)
