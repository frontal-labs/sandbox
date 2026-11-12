# Development Guide

This guide covers building and working on the repository as it exists today.
Most crates are scaffolds (see [MILESTONES.md](MILESTONES.md) and the
"Current State" table in [ARCHITECTURE.md](ARCHITECTURE.md)), so a successful
build does not mean the feature works.

## Prerequisites

- **Rust 1.95**, pinned in `rust-toolchain.toml`. The file also requests the
  `rustfmt` and `clippy` components and the targets `aarch64-apple-darwin`,
  `x86_64-apple-darwin`, and `x86_64-unknown-linux-gnu`. `rustup` installs
  these automatically when you run cargo in the repo.
- **Node.js 20+** (`engines` in the root `package.json`).
- **Bun 1.3.3** (`packageManager` in the root `package.json`). Used to install
  JavaScript dependencies.
- **Bazel** (via `MODULE.bazel`, `.bazelrc`, and `BUILD.bazel`). Bazelisk reads
  the pinned `.bazelversion`, so install Bazelisk, not Bazel. Bazel builds the
  whole workspace and its tests. See [BUILD_SYSTEM.md](BUILD_SYSTEM.md).
- **Python 3.11+** for the Python SDK (`sdks/python/pyproject.toml`).
- **protoc** (Protocol Buffers compiler), on `PATH` or set as `PROTOC`. Needed
  to build `crates/sandbox-proto`. Bazel builds protoc itself from the
  `protobuf` module. Optional: **buf**, for `buf lint proto`.

## Quick Start (Rust)

```bash
git clone https://github.com/frontal-labs/sandbox.git
cd sandbox

# rustup installs the pinned toolchain from rust-toolchain.toml
cargo build --workspace
cargo test --workspace      # runs the tests that exist; see TESTING.md
cargo run --bin frontal-sandbox -- --help
```

The `frontal-sandbox` binary comes from the `cli` crate (`cli/Cargo.toml`). The crate
name is `cli`, not `sandbox-cli`.

## Quick Start (TypeScript and Python)

```bash
# JavaScript dependencies (workspace root includes sdks/typescript)
bun install

# Root scripts, which run Bazel targets for the TypeScript SDK
npm run ts:build     # bazel build //sdks/typescript:all
npm run ts:test      # bazel test //sdks/typescript:test
npm run ts:lint      # bazel run //sdks/typescript:lint

# Lint and format (Biome 2.5.3, configured by biome.json)
npm run lint
npm run format:check
npm run format

# SDK-local scripts (sdks/typescript/package.json)
cd sdks/typescript && bun run build && bun run test

# Python SDK
cd sdks/python && python -m venv .venv && . .venv/bin/activate
pip install -e ".[dev]" && pytest
```

Notes:

- `npm run ts:*` scripts call Bazel. The `ts:lint` target name is
  `//sdks/typescript:lint`, but `sdks/typescript/BUILD.bazel` defines only
  `tsconfig`, `sandbox_sdk`, `sandbox_sdk_test`, `test`, and `all`. Verify the
  lint target before relying on it.
- `.bazelrc` picks the target platform from the host OS: x86_64 macOS on a Mac,
  x86_64 Linux on Linux. Pass `--config=linux_aarch64` for aarch64 Linux. See
  [BUILD_SYSTEM.md](BUILD_SYSTEM.md).

## Project Structure

Top-level layout, as committed:

```
sandbox/
├── Cargo.toml              # Cargo workspace root (18 members)
├── rust-toolchain.toml     # Rust 1.95
├── crates/                 # Rust crates sandbox-*
├── cli/                    # Crate `cli`, binary `frontal-sandbox`
├── sdks/
│   ├── rust/               # Crate `sandbox-sdk`
│   ├── typescript/         # npm `@frontal-labs/sandbox-sdk` (Bun, Bazel, vitest)
│   └── python/             # Package `sandbox_sdk` (pyproject.toml)
├── benchmarks/             # Benchmark sources and load tests (not a workspace member)
├── docs/                   # This directory
├── package.json            # Root scripts (Biome, Bazel wrappers)
├── MODULE.bazel, BUILD.bazel, .bazelrc
└── .github/workflows/      # CI workflows
```

`packages/` exists but is empty. The TypeScript SDK lives at `sdks/typescript`.

## Cargo Workspace Conventions

- **Members** are listed in the root `Cargo.toml`. A new crate must be added
  there.
- **No cycles**: dependencies flow from `sandbox-types` and `sandbox-cas` up to
  `sandbox-server` and `cli` (project specification section 11).
- **Lints**: `clippy.toml` and `rustfmt.toml` are committed. CI runs clippy with
  `-D warnings`.
- **Shared dependencies** use `workspace = true` (see `[workspace.dependencies]`
  in the root `Cargo.toml`).
- **Dependency policy**: `deny.toml` configures `cargo deny`, and
  `.cargo/audit.toml` configures `cargo audit`. CI runs both in the
  `supply-chain` job. Run `cargo deny check` and `cargo audit` before adding a
  dependency. An advisory may be ignored only with a reason recorded in both
  files.

## Adding a New Crate

1. Create `crates/sandbox-<name>/Cargo.toml` and `src/lib.rs`.
2. Add the path to `[workspace] members` in the root `Cargo.toml`.
3. Define public traits and types in `src/lib.rs`.
4. Write tests alongside the implementation (see [TESTING.md](TESTING.md)).
5. Add a `README.md` with purpose, invariants, and an example (project specification section 11).
6. Update [ARCHITECTURE.md](ARCHITECTURE.md) if the architecture changes.

## Coding Standards

- No `unwrap()` outside tests and `main` (project specification section 11).
- Use `?` with error types from `sandbox-types` (`SandboxError`, `ErrorCode`).
- Every `unsafe` block needs a `// SAFETY:` comment and a test.
- Public API changes need an ADR in `docs/adr/` (see
  [docs/adr/README.md](adr/README.md)).
- Benchmark-driven choices go in `docs/decisions/`
  (see [docs/decisions/README.md](decisions/README.md)).

### Conventional Commits

Scopes listed in `CLAUDE.md`: `types`, `vfs`, `shell`, `runtime`, `policy`,
`session`, `snapshot`, `trace`, `server`, `scheduler`, `sdk`, `cli`, `docs`.

```
feat(vfs): add overlay commit
fix(cas): handle partial disk write
docs(adr): record decision on V8 snapshot format
```

`commitlint.config.js` is committed at the repo root.

## Testing

See [TESTING.md](TESTING.md) for what exists and what is planned. Commands
that run today:

```bash
cargo test --workspace          # all Rust tests (only sandbox-cas has any)
cargo test -p sandbox-cas       # the crate with tests
cargo test -p cli               # no tests yet; compiles only
npm run ts:test                 # TypeScript SDK via Bazel
cd sdks/typescript && bun run test   # vitest, same tests
cd sdks/python && pytest
```

These commands have not been run as part of this documentation update, so
their results are not recorded here.

Conformance: `crates/sandbox-shell/conformance/cases.py` holds the scripts.
`python3 record.py` runs each one under the local GNU bash and rewrites
`cases.json`, which records the bash version. `cargo test -p sandbox-shell
--test conformance` replays the file. Re-record only when the corpus changes,
and review the diff, since the expected output comes from bash.

Linux tests: worker isolation needs Linux, cgroup v2, and namespaces. `docker/run-linux.sh <command>` runs a command in a privileged Linux container with a delegated cgroup (`SANDBOX_CGROUP_ROOT`). Example: `docker/run-linux.sh cargo test -p cli --test worker_isolation`. It also runs the seccomp and runtime tests. Clustered state needs a Postgres database used only for tests: `SANDBOX_TEST_DATABASE_URL=postgres://… cargo test -p sandbox-server --features clustered --test clustered`.

Fuzzing: the targets in `fuzz/`
nightly and `cargo-fuzz`. `cargo fuzz` runs the `cargo` found on `PATH`, and
the Homebrew `cargo` on some machines ignores rustup. Put the nightly `bin`
directory first on `PATH`:

```bash
NB=$(dirname "$(rustup which --toolchain nightly cargo)")
cd fuzz && PATH="$NB:$PATH" cargo fuzz run vfs_paths -- -max_total_time=90 -timeout=10
```

Benchmarks: `cargo bench --workspace --no-run` is what CI runs as a compile
check. `benchmarks/` is not a workspace member, so it is not included in that
command.

## Documentation

- **Every crate**: a README with purpose, invariants, and example (project specification section 11).
- **Public API changes**: ADR in `docs/adr/NNNN-title.md`.
- **Benchmark decisions**: record in `docs/decisions/`.
- **Architecture changes**: update `docs/ARCHITECTURE.md`.

### ADR Template

See [docs/adr/README.md](adr/README.md) for the template and process.

## CI Pipeline

`.github/workflows/ci.yml` (triggered on push and PR to `main`) has one job,
`rust`. It runs the following with the toolchain pinned in `rust-toolchain.toml`:

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo bench --workspace --no-run`

A `supply-chain` job runs `cargo deny check` and `cargo audit --deny warnings`,
with the tool versions pinned in the workflow.

Two more jobs run the SDK suites: `sdk-typescript` (build, test, and an
`npm pack --dry-run`) and `sdk-python` (editable install, `ruff check`, and
`pytest`). The new jobs have not run on GitHub yet, so a passing run is not
confirmed.

`pr-checks.yml` requires a changeset (`.changeset/*.md`) on any PR that changes
files under `sdks/`. The older semver-label check is still there.

Other workflows in `.github/workflows/`: `benchmarks.yml`, `codeql-analysis.yml`,
and `release-binaries.yml`. `bazel.yml` runs the Bazel build and tests with
Bazelisk installed by `bazel-contrib/setup-bazel`.

## Releasing the SDKs

The three SDKs share one version. `sdks/typescript/package.json` is the source
of truth. Changesets manages it.

1. Add a changeset with `npm run changeset` on the PR that changes an SDK.
   Use `"@frontal-labs/sandbox-sdk": minor` or `patch` (or `major`).
2. When changesets land on `main`, `sdk-release.yml` opens a "version SDKs"
   PR through `npm run version-packages`. That runs `changeset version`, then
   `scripts/sync-sdk-versions.mjs`, which copies the version into
   `sdks/python/pyproject.toml`, `sdks/rust/Cargo.toml`, and `Cargo.lock`.
3. Merging that PR runs `scripts/release/publish-sdk.sh` for `ts`, `py`, and
   `rs`. Each script packs the SDK and pushes it to GHCR as an OCI artifact:
   - `ghcr.io/frontal-labs/sandbox-sdk-ts:<version>` holds the npm tarball.
   - `ghcr.io/frontal-labs/sandbox-sdk-py:<version>` holds the wheel and sdist.
   - `ghcr.io/frontal-labs/sandbox-sdk-rs:<version>` holds a source snapshot.
4. The release job creates tag `sdk-v<version>` and a GitHub release.

GHCR is not a package index. npm, pip, and cargo cannot install from it, so
consumers pull the artifact with `oras`. See each SDK README. The Rust SDK has
no crates.io package because `sandbox-types` is not published there.

Run the publish step without pushing: `scripts/release/publish-sdk.sh <ts|py|rs>
--dry-run`. The workflow's manual trigger also defaults to a dry run.

Other workflows in `.github/workflows/`: `pr-checks.yml`, `benchmarks.yml`,
`codeql-analysis.yml`, and `release-binaries.yml`. `release-binaries.yml` creates
the GitHub release for each `v*` tag, with `SHA256SUMS` attached.

## Local Development Loop

```bash
# Run the CLI (run and shell work; the other subcommands are not implemented)
cargo run --bin frontal-sandbox -- --help
cargo run --bin frontal-sandbox -- run "echo hello"
```

There is no server binary yet. `crates/sandbox-server` is a library
(`src/lib.rs`), and `frontal-sandbox serve` only prints a message. See
[API_REFERENCE.md](API_REFERENCE.md).

## Debugging

### Logging

The `cli` crate depends on `tracing-subscriber`. Use `RUST_LOG` with the
standard `tracing` filter syntax if the binary initializes a subscriber:

```bash
RUST_LOG=debug cargo run --bin frontal-sandbox -- run "ls"
RUST_LOG=sandbox_vfs=trace cargo test -p sandbox-vfs
```

Whether the CLI or tests actually install a subscriber has not been verified.

## Known gaps

- `docs/` and `.github/` may still reference tools not in this repo (for
  example `sandbox-admin`, `tools/`, `images/`, `deploy/`, `bench/`,
  `tests/`). They do not exist yet.
- The release process (`release.yml`, `.changeset/`) was not reviewed in this
  update.

## Contracts

- **Protobuf** (`proto/`): edit the `.proto` files, then `cargo build -p
  sandbox-proto` regenerates the Rust bindings. Check with `buf lint proto` and
  `buf build proto`.
- **OpenAPI** (`openapi/v1/`): after changing a route or a REST type, run
  `cargo run -p sandbox-server --bin gen_openapi`. Then run `cargo test -p
  sandbox-server --test openapi`, which fails if the committed spec is stale.
