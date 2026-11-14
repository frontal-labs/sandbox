# Docker

Container setup for the Frontal Sandbox server. The image runs `sandbox serve` (frontal-sandbox is a legacy alias),
the CLI subcommand that starts the HTTP API in `crates/sandbox-server`.

> **Status: local and development use.** The API has no TLS, no rate limiting,
> and no key provisioning beyond the first-start key. Do not publish the port
> beyond loopback without a TLS and auth gateway in front. See
> [docs/DEPLOYMENT.md](../docs/DEPLOYMENT.md) and
> [docs/SECURITY_MODEL.md](../docs/SECURITY_MODEL.md).

## Files

| File | Purpose |
|------|---------|
| `Dockerfile` | Two-stage build. Builder: `rust:1.95-bookworm`, builds `cli` with `--locked`. Runtime: `debian:bookworm-slim`, non-root, tini as PID 1. |
| `Dockerfile.dockerignore` | Build-context exclusions. BuildKit reads it automatically because it sits next to the Dockerfile. |
| `compose.yaml` | The local stack: build, state volume, loopback port, health check, hardening. |
| `.env.example` | Optional overrides for `compose.yaml`. Copy to `docker/.env`. |
| `smoke-test.sh` | End-to-end check on an isolated stack (own project, volume, and port). |

## `node` needs a delegated cgroup

On Linux the server runs each `node` script in a child process with a cgroup memory limit. The container needs a delegated cgroup v2 directory in `SANDBOX_CGROUP_ROOT`, with the `cpu`, `memory`, and `pids` controllers enabled, and the same privileges that worker isolation needs. Without that, `node` is refused, and nothing runs in the server process. See `docker/run-linux.sh` for the setup used in development.

## Quick start

Requirements: Docker with Compose v2 (`docker compose version`).

```bash
# From the repository root
docker compose -f docker/compose.yaml up -d --build --wait

# The first start prints the API key once. Save it.
docker compose -f docker/compose.yaml logs sandbox | grep 'API key'

# Check the server. This route needs no key.
curl -fsS http://127.0.0.1:8080/v1/openapi.json >/dev/null && echo up

# Every other /v1 route needs the key.
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/v1/sessions   # 401
```

Run the smoke test to check the whole path, including restart persistence:

```bash
docker/smoke-test.sh
```

It creates a throwaway project, volume, and port (18080 by default, override
with `SMOKE_PORT`) and removes them on exit. Your real state volume is not
touched.

## API key

- The key is generated on the first start of a durable server and printed once,
  to stderr, which is the container log.
- Only a digest is stored in the state volume, so the key cannot be shown again.
  If the key is lost, delete the state volume (`down -v`) and start again. That
  also deletes all sessions and snapshots.
- A key is sent as `Authorization: Bearer <key>`.

## Configuration

Set these in `docker/.env`, or in the shell before running `docker compose`.
The defaults are in `compose.yaml`, and `.env.example` lists them.

| Variable | Default | Effect |
|----------|---------|--------|
| `SANDBOX_HOST_BIND` | `127.0.0.1` | Host interface to publish on. |
| `SANDBOX_PORT` | `8080` | Host port. The container always listens on 8080. |
| `SANDBOX_STATE_VOLUME` | `frontal-sandbox-state` | Named volume for keys, snapshots, and sleeping sessions. |
| `SANDBOX_IMAGE` | `frontal-sandbox:local` | Image tag. |
| `SANDBOX_MEM_LIMIT` | `2g` | Container memory limit. |
| `SANDBOX_PIDS_LIMIT` | `512` | Container process limit. |
| `RUST_VERSION` | `1.95` | Builder image tag. Keep it equal to `rust-toolchain.toml`. |
| `DEBIAN_SUITE` | `bookworm` | Base distribution for both stages. |
| `RUST_BACKTRACE` | `1` | Panic backtraces. |

The server's flags (`--addr`, `--data-dir`) are fixed in the image's `CMD`. To
run without persistence, override the command:

```bash
docker compose -f docker/compose.yaml run --rm sandbox serve --addr 0.0.0.0:8080
```

That mode keeps everything in memory, and a new key is printed on each start.

## Security model

- **Isolation boundary.** Session code runs inside the server process. The
  container is the outer boundary. Tier 2 (QEMU microVMs) is not
  implemented, and no `/dev/kvm` device is passed through.
- **Non-root.** The process runs as uid/gid `10001`, with no login shell.
- **Read-only root filesystem.** Writes go to the state volume and to a 256 MB
  `/tmp` tmpfs.
- **Capabilities.** `cap_drop: ALL`, and `no-new-privileges`. The server
  listens on port 8080, so it needs no capabilities.
- **Loopback by default.** The published port is bound to `127.0.0.1`.
- **Secrets.** Compose and the image contain no secrets. `docker/.env` is
  git-ignored, and the build context excludes `.env*` files.

Not provided yet: TLS, rate limiting, a metrics endpoint, a health endpoint
beyond the OpenAPI document, and request logs (the CLI installs no tracing
subscriber). Plan around these before any shared deployment.

## Operations

```bash
# Logs
docker compose -f docker/compose.yaml logs -f sandbox

# Stop, keep state
docker compose -f docker/compose.yaml down

# Stop and delete state (sessions, snapshots, keys)
docker compose -f docker/compose.yaml down -v

# Rebuild after a code change
docker compose -f docker/compose.yaml up -d --build --wait
```

## CI

`.github/workflows/docker.yml` builds the image on pull requests (when the
inputs change) and on pushes to `master`. It then runs the image with the same
hardened flags as compose, and checks that the health check passes, that
`/v1/openapi.json` returns 200, and that a protected route returns 401.

A `v*` tag also publishes the image to `ghcr.io/<owner>/sandbox`, with
semver tags and the commit SHA. Publishing needs the `packages: write` scope,
which only the publish job requests.

CI does not run `smoke-test.sh` yet. The restart check fails until the server
persists sessions on create (see the note in `smoke-test.sh`).

## Build notes

- **Context.** The build context is the repository root, because `cargo` needs
  every workspace member, and `proto/` is needed by `sandbox-proto/build.rs`.
- **Network at build time.** `rusty_v8`, a dependency of `cli` through
  `deno_core`, downloads a prebuilt V8 archive when it compiles. Builds fail
  offline. If you have a local copy of the archive, `RUSTY_V8_ARCHIVE` can point
  to it, which is not wired into the Dockerfile yet.
- **Toolchain.** `rust-toolchain.toml` pins 1.95 and lists Apple targets. Rustup
  in the builder may fetch those target components, which adds build time.
- **Cache.** BuildKit cache mounts keep the cargo registry and `target/`
  between builds, so only changed crates recompile.
- **Dev container.** `.devcontainer/Dockerfile` is a development image. It is
  unrelated to this deployment image.
