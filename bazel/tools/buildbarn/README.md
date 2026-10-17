# Buildbarn

A remote cache for Bazel, from [buildbarn/bb-deployments][ref], reduced to one
storage node for a developer machine. The execution side is included, but it is
not usable for this build yet (see below).

[ref]: https://github.com/buildbarn/bb-deployments

## Run the cache

```bash
docker compose -f bazel/tools/buildbarn/compose.yaml up -d --wait
bazel build --config=remote-cache //crates/sandbox-types:sandbox_types
```

`--config=remote-cache` points Bazel at `grpc://localhost:8980` (see
`bazel/config/remote-cache.bazelrc`). To use another cache, pass
`--remote_cache=grpc://host:port` on the command line.

Services in the default profile:

| Service | Role |
|---------|------|
| `init` | One-shot. Creates the state directories that bb-storage expects. |
| `storage` | Stores blobs, action results, and file-system access cache entries. |
| `scheduler` | Answers capability queries. Runs no actions without a worker. |
| `frontend` | The gRPC endpoint Bazel uses, on `${BUILDBARN_CACHE_PORT:-8980}`. |

Stop with `docker compose -f bazel/tools/buildbarn/compose.yaml down`. Add `-v`
to delete the cache volumes too.

## Execution (profile `exec`)

`worker`, `runner-installer`, and `runner` run Linux actions. Start them with
`--profile exec`.

They do not work for this repository yet. Bazel's actions for this build run on
x86_64-apple-darwin, and the workers advertise Linux, so no action matches. The
build would also need a Linux platform and a Linux V8 archive. The darwin V8
archive is in `third_party/rusty_v8`. Until those exist, use the cache only.

## Configuration

`config/*.jsonnet` is read by the images. Each file says where it came from.

- `common.libsonnet` holds the storage address and the shared settings.
- `storage.jsonnet` sizes the local block devices: 8 GiB for blobs, 256 MiB for
  the action cache. Raise them for a shared cache.
- `newBlocks` is 3 for the CAS, as in the reference, and 1 for the action cache
  and the file-system access cache. bb-storage refuses to start with any other
  value for the action cache (its error text says so).

## Constraints

- **No authentication.** Every access policy is `allow: {}`. The frontend binds
  to `127.0.0.1` by default. Set `BUILDBARN_BIND` only on a private network, and
  never on a public one.
- **Start order.** The frontend caches its connection to the scheduler. If the
  frontend starts first and the scheduler comes up later, restart the frontend
  (`docker compose ... restart frontend`). Otherwise Bazel fails with
  `lookup scheduler: no such host`.
- **pprof is off.** The diagnostics endpoint exposes Prometheus metrics only.
