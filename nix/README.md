# Nix

Nix configuration for reproducible development environments and builds.

## Contents

| File | Purpose |
|------|---------|
| `nix/devshell.nix` | Development shell with all required tools (Rust, Bazel, protoc, etc.) |
| `nix/overlays.nix` | Package overlays for specific versions |
| `nix/packages/default.nix` | Project-specific package definitions |
| `nix/checks.nix` | CI checks (formatting, linting, tests) |

## Usage

Enter the development shell:

```bash
nix develop
```

Or use direnv with `.envrc`:

```bash
echo "use nix" > .envrc
direnv allow
```

## License

Apache-2.0