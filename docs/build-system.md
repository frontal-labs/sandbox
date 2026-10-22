# Build system

Frontal Sandbox has two layers, and they do not depend on each other:

- **Nix** owns the developer environment. A flake provides a pinned dev shell
  (Bazelisk, the Rust toolchain, formatters, linters) and the repository checks
  that `nix flake check` runs. Nix is optional.
- **Bazel** (with Bzlmod) owns the build and the tests. It fetches its own Rust,
  Python, Node and protoc toolchains, so a plain machine with Bazelisk can build
  and test without Nix.

Cargo still works for day-to-day Rust development. `Cargo.toml` and `Cargo.lock`
remain the source of truth for crate versions. Bazel reads `Cargo.lock` through
crate_universe, so the two stay in step, but nothing enforces that beyond the
module lock check below.

## Layout

```text
flake.nix              Nix entry point: inputs and outputs only
flake.lock             Pins every Nix input (nixpkgs, flake-utils, rust-overlay)
nix/
├── overlays.nix       nixpkgs overlays (rust-overlay)
├── packages/          Custom tools (`bazel`, a wrapper around bazelisk)
├── devshell.nix       The dev shell
└── checks.nix         What `nix flake check` runs
.envrc.example         Optional direnv hook; copy to .envrc

.bazelversion          Bazel version for bazelisk (9.3.0)
MODULE.bazel           Bazel modules, toolchains and crate universe
MODULE.bazel.lock      Lockfile for MODULE.bazel; commit it
.bazelrc               Shared flags; imports bazel/config/*.bazelrc
bazel/config/          ci.bazelrc, remote-cache.bazelrc, user.bazelrc.example
bazel/platforms/       Target platforms
BUILD.bazel            Per-package build files; one per Cargo package
```

## Nix

Enter the dev shell with `nix develop`, or install direnv and copy the example:

```bash
cp .envrc.example .envrc && direnv allow   # optional
nix develop                                 # without direnv
```

The shell provides `bazel` (a wrapper around bazelisk that reads
`.bazelversion`), the Rust toolchain from `rust-toolchain.toml`, protoc, Node 22,
Python 3.12, ruff, biome, taplo, shfmt, shellcheck, nixfmt, cargo-deny and
cargo-audit.

Run the checks with:

```bash
nix flake check
```

The checks read the source tree and do not build the workspace:

| Check | Runs |
|-------|------|
| `rust-fmt` | `cargo fmt --all --check` |
| `nix-fmt` | `nixfmt --check` on `flake.nix` and `nix/` |
| `shell-lint` | `shellcheck` and `shfmt -d` on `bazel/tools/buildkite/ci.sh` |
| `toml-fmt` | `taplo fmt --check` on the workspace TOML files |

Every input version is pinned by `flake.lock`. To move a pin, run
`nix flake update <input>` and commit the new lock.

### Nix store

Nix manages `/nix` alone. Do not edit, move or delete anything under `/nix/store`
or `/nix/var` by hand. Reference store paths only through flakes, dev shells and
derivations, and never hardcode them. To reclaim space, run `nix store gc`.

Bazel does not read the Nix store. Its toolchains are fetched by Bazel's own
repository rules. A remote Bazel worker therefore needs no Nix store paths, unless
a future change makes the build depend on a Nix-provided toolchain. In that case
every worker must have the same `/nix/store` paths.

## Bazel

Bazelisk reads `.bazelversion` and fetches that Bazel release. Run:

```bash
bazel build //...
bazel test //...
```

The default target follows the host OS: `x86_64-apple-darwin` on macOS and
`x86_64-unknown-linux-gnu` on Linux. Pass `--config=linux_aarch64` to build for
aarch64 Linux.

Toolchains come from Bazel modules, not from the host:

| Tool | Source |
|------|--------|
| Rust 1.95 | `rules_rust`, with crates from `Cargo.lock` through crate_universe |
| protoc | `protobuf` module, built from source |
| Python 3.12 | `rules_python`, with `sdks/python/requirements_lock.txt` |
| Node and TypeScript | `aspect_rules_js` and `aspect_rules_ts`, from the pnpm lock |
| rusty_v8 | Release archives pinned by SHA-256 in `MODULE.bazel` |

`rules_nixpkgs` is not used. No Bazel-native option was missing, and the build
does not use remote execution.

### Keeping the module lock current

`MODULE.bazel.lock` must match `MODULE.bazel` and `Cargo.lock`. Check it with:

```bash
bazel mod deps --lockfile_mode=error
```

Refresh it after changing a module or a crate spec with
`bazel mod deps --lockfile_mode=update`. Bazel then also rewrites the root
`Cargo.lock` to add a synthetic `direct-cargo-bazel-deps` package. Discard that
change before committing, because Cargo does not use it.

### Build outputs and disk

Bazel's output root holds the external repositories and every build output. On a
machine whose home volume is nearly full, move it with a command-line flag:

```bash
bazel --output_user_root=/path/with/space/bazel-root build //...
```

Set `TMPDIR` to a volume with space as well. Clang writes temporary files there,
and a full temp volume shows up as a segfault or `failed to create temporary
directory`. `bazel/config/user.bazelrc.example` describes this option.

### Test coverage

`//:test_rust` is a test suite over every Rust test target. It is a suite, not a
wildcard, so a new test target has to be added to it. `bazel test //...` runs all
of them, plus the TypeScript and Python tests.

Rust unit tests in `src/` use a `rust_test` with `crate = ":<lib>"`. Each file in
`tests/` gets its own `rust_test`. The Cargo dev-dependencies are listed in the
test's `deps`.

Measured on the committed HEAD with the Bazel changes applied (macOS, arm64
host, x86_64 target):

| Suite | Result |
|-------|--------|
| `cargo test --workspace` (old suite) | 383 passed, 0 failed, 1 ignored |
| `bazel test //...` Rust test cases | 383 passed, 0 failed |
| `bazel test //...` Python (pytest) | 11 passed |

The one ignored item in Cargo is a doctest. `bazel test` does not run doctests,
and no doctest passes under Cargo, so the 383 match.

Three Bazel-only settings replace what Cargo provides:

- `sandbox_server` sets `CARGO_PKG_AUTHORS` and `CARGO_PKG_LICENSE` at compile
  time. utoipa reads them to fill the OpenAPI `contact` and `license`, and the
  committed spec was generated with them. The values repeat `Cargo.toml`, and
  `tests/openapi.rs` fails if they drift.
- `cli_worker_process_test` sets `CARGO_BIN_EXE_frontal-sandbox` at run time.
  Bazel sets that variable only at compile time for the `env!` tests.
- `sdks/python` runs pytest from the workspace root, so its path is
  workspace-relative.

## CI

`.github/workflows/bazel.yml` runs two jobs:

- **Nix**: `nix flake check`, then `bazel build //...` and `bazel test //...`
  inside `nix develop`.
- **Bazel without Nix**: `bazel build //...` and `bazel test //...` with
  Bazelisk installed by `bazel-contrib/setup-bazel`.

The Rust, supply-chain, and SDK jobs in `ci.yml` are unchanged. They still run
Cargo.

## Known gaps

- **Nix is not verified.** No `flake.lock` is committed yet, because generating
  it needs Nix, and Nix is not installed on the machine used for these checks.
  `nix flake check` and `nix develop` have not run. Run `nix flake lock` once,
  then commit the lock.
- **Linux is not verified** on a Linux machine. The Linux targets and the CI job
  without Nix have not run.
- **The `ci.yml` workflows are unchanged** and still use Cargo. `bazel.yml` has
  not run on GitHub.
- Worker isolation tests need cgroups. Without them they skip, as they do under
  Cargo.
- Remote execution is not configured. The toolchains target the host
  architecture, and the remote cache is cache-only.
- Doctests are not run under Bazel.
