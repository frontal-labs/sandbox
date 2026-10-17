# bazel

Everything Bazel-specific that is not a BUILD file in a package. The root
`MODULE.bazel`, `BUILD.bazel`, `.bazelrc`, and `.bazelversion` stay at the
repository root, because Bazel and bazelisk read them there.

## Layout

```
bazel/
├── config/                 Flags and pipeline definitions (read by Bazel and CI)
│   ├── ci.bazelrc          --config=ci: plain logs, verbose failures
│   ├── remote-cache.bazelrc  --config=remote-cache: Buildbarn cache
│   ├── user.bazelrc.example  per-machine overrides (copy to user.bazelrc; git-ignored)
│   └── buildkite/
│       ├── bootstrap.yml   the step to paste into the Buildkite pipeline
│       └── pipeline.yml    build and test steps
├── defs/                   Shared Starlark macros (none yet; see defs/README.md)
├── extensions/             Bzlmod module extensions (none yet; see extensions/README.md)
├── platforms/
│   └── BUILD.bazel         //bazel/platforms:x86_64_apple_darwin
└── tools/
    ├── buildbarn/          Remote cache stack (docker compose + configs)
    └── buildkite/          ci.sh: one script per pipeline phase
```

Root files that reference this tree:

| File | What it does here |
|------|-------------------|
| `.bazelversion` | Pins Bazel 9.3.0 for bazelisk. |
| `.bazelrc` | Sets the platform and imports `bazel/config/*.bazelrc`. |
| `MODULE.bazel` | Registers `//bazel/platforms:x86_64_apple_darwin` as the execution platform. |
| `.gitignore` | Ignores `bazel/config/user.bazelrc`. |

## Common tasks

```bash
# Build and test locally
bazel build //:build_all
bazel test //:test_rust

# Use the remote cache (start the stack first; see tools/buildbarn/README.md)
docker compose -f bazel/tools/buildbarn/compose.yaml up -d --wait
bazel build --config=remote-cache //cli:sandbox

# Run the same phases CI runs
bazel/tools/buildkite/ci.sh build
bazel/tools/buildkite/ci.sh test
```

CI runs on Buildkite. See `tools/buildkite/README.md` for the agent and pipeline
setup.

## Platform

The build targets `x86_64-apple-darwin` only. Two things enforce it:

- `bazel/platforms` defines the platform, and `.bazelrc` selects it.
- The rusty_v8 archive (`third_party/rusty_v8`) and the crate_universe
  `supported_platform_triples` in `MODULE.bazel` both list darwin only.

Because of that, the build cannot run on Linux. That also rules out remote
execution on Buildbarn's Linux workers. Remote caching is unaffected, because the
cache stores outputs, whatever platform produced them. Remote execution needs a
Linux platform, a Linux V8 archive, and crate annotations for Linux. None of those
exist yet.

## Verified

- The remote cache stores and serves outputs. A build of
  `//crates/sandbox-types:sandbox_types` populated the cache. After `bazel clean`,
  the rebuild took 5.9 seconds with 241 remote cache hits.
- The restructure kept cache identity. The hits came after the platform moved to
  `bazel/platforms`.
- Both Buildkite files parse through `buildkite-agent pipeline upload --dry-run`.
- The CI aliases (`//:build_all`, `//:build_rust`, `//:build_sdks`,
  `//:test_rust`, `//:test_ts`) resolve.

Not verified: a full `ci.sh build` or `ci.sh test` run, and the Buildkite pipeline
on a real agent. The tests in `test_rust` and `test_ts` were not run in this
session.
