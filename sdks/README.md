# Frontal Sandbox SDKs

Client SDKs for the Frontal Sandbox REST API (`/v1`). Each SDK provides idiomatic bindings for its language.

## Available SDKs

| Language | Package | Path | Status |
|----------|---------|------|--------|
| Rust | `sandbox-sdk` | `sdks/rust/` | Early - source distribution via GHCR |
| TypeScript | `@frontal-labs/sandbox-sdk` | `sdks/typescript/` | Early - OCI artifact via GHCR |
| Python | `sandbox-sdk` | `sdks/python/` | Early - OCI artifact via GHCR |

## Common Features

- **Session lifecycle**: create, get, destroy
- **Code execution**: exec, streaming exec
- **Filesystem**: read, write, delete
- **Snapshots**: create, list, get, fork
- **Error handling**: returned as values, not thrown

## Distribution

All SDKs are distributed as OCI artifacts on GHCR, not via language-native package registries:

```bash
# Pull any SDK
oras pull ghcr.io/frontal-labs/sandbox-sdk-<lang>:<version>
```

This avoids publishing internal dependencies (`sandbox-types`) to public registries.

## Configuration

All SDKs support the same environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `SANDBOX_BASE_URL` | Sandbox server URL | `http://localhost:8080` |
| `SANDBOX_API_KEY` | Bearer token for auth | (required) |

## Wire Format

All SDKs use the same snake_case JSON wire format defined by `sandbox-server` and the OpenAPI spec in `openapi/v1/`. TypeScript and Python SDKs convert between idiomatic naming (camelCase/snake_case) and the wire format automatically.

## Development

Build and test all SDKs from the repository root:

```bash
# Rust
cargo test -p sandbox-sdk

# TypeScript
cd sdks/typescript && npm run test

# Python
pytest sdks/python/tests
```

## License

Apache-2.0