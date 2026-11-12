# Build system

Frontal Sandbox uses two build systems that do not depend on each other:

- **Bazel** (with Bzlmod) owns the build and the tests. It fetches its own Rust,
  Python, Node and protoc toolchains, so a plain machine with Bazelisk can build
  and test without any extra dependencies.
- **Cargo** remains the source of truth for day-to-day Rust development.
  `Cargo.toml` and `Cargo.lock` pin crate versions. Bazel reads `Cargo.lock`
  through crate_universe, so the two stay in step, but nothing enforces that
  beyond the module lock check below.

## Layout

```text
.bazelversion          Bazel version for bazelisk (9.3.0)
MODULE.bazel           Bazel modules, toolchains and crate universe
MODULE.bazel.lock      Lockfile for MODULE.bazel; commit it
.bazelrc               Shared flags; imports bazel/config/*.bazelrc
bazel/config/          ci.bazelrc, remote-cache.bazelrc, user.bazelrc.example
bazel/platforms/       Target platforms
BUILD.bazel            Per-package build files; one per Cargo package
```

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

`.github/workflows/bazel.yml` runs one job:

- **Bazel**: `bazel build //...` and `bazel test //...` with Bazelisk installed
  by `bazel-contrib/setup-bazel`.

The Rust, supply-chain, and SDK jobs in `ci.yml` are unchanged. They still run
Cargo.

## Known gaps

- **Sandboxing is off for Bazel actions on macOS and Linux.** `--spawn_strategy=local`
  keeps the execroot path fixed, so two clean builds produce identical outputs.
  Under the darwin and linux sandboxes, each action runs in a directory named
  for its PID, and C and Rust debug info record that path. The trade-off: an
  action that reads an undeclared input is not caught.
- Worker isolation tests need cgroups. Without them they skip, as they do under
  Cargo.
- Remote execution is not configured. The toolchains target the host
  architecture, and the remote cache is cache-only.
- Doctests are not run under Bazel.
