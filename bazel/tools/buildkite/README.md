# Buildkite

`ci.sh` runs one Bazel phase for a Buildkite step. The pipeline definition is in
`bazel/config/buildkite/`.

```bash
bazel/tools/buildkite/ci.sh build   # //:build_all //:build_rust //:build_sdks
bazel/tools/buildkite/ci.sh test    # //:test_rust //:test_ts
```

## Pipeline setup

1. Create a Buildkite pipeline for this repository.
2. Set its **Steps** to the content of `bazel/config/buildkite/bootstrap.yml`,
   or commit that content as `.buildkite/pipeline.yml`. The bootstrap step
   uploads `bazel/config/buildkite/pipeline.yml`, so the real pipeline stays
   under `bazel/`.
3. Run an agent on a **macOS** host with the queue `macos`. The build targets
   `x86_64-apple-darwin` (see `bazel/platforms`). A Linux agent cannot run it.
4. Put `bazelisk` on the agent's `PATH` as `bazel`, so the root `.bazelversion`
   selects the Bazel version.
5. Set `BUILDBARN_CACHE_URL` in the agent's environment, for example
   `grpc://cache.internal:8980`, to use the remote cache. Without it the phases
   still run, with no cache, and the script says so.

## Checking the pipeline

The agent can parse both files without a token that talks to Buildkite:

```bash
buildkite-agent pipeline upload --dry-run --agent-access-token placeholder \
  --format yaml bazel/config/buildkite/pipeline.yml
```

`--dry-run` still needs a token value. Any placeholder works, because the dry
run never uploads.

## Constraints

- The script runs under macOS `/bin/bash` 3.2. Do not use bash-4 features.
- The remote cache is unauthenticated (see `tools/buildbarn/README.md`). Run
  agents on a network that can reach it, and nothing else.
