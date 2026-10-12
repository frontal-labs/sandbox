# Contributing to Vibe

Thank you for your interest in contributing to Vibe! We welcome contributions
from everyone. By participating in this project, you agree to abide by our
[Code of Conduct](CODE_OF_CONDUCT.md).

## Getting Started

### Fork and Clone

1. Fork the repository on GitHub.
2. Clone your fork locally:
   ```bash
   git clone https://github.com/YOUR-USERNAME/vibe.git
   cd vibe
   ```
3. Add the upstream repository as a remote:
   ```bash
   git remote add upstream https://github.com/vibeapp/vibe.git
   ```

### Set Up the Development Environment

#### Option A: Nix (recommended — single command, reproducible)

1. Install [Nix](https://nixos.org/download.html) with flakes enabled:
   ```bash
   # Enable flakes in ~/.config/nix/nix.conf or /etc/nix/nix.conf
   experimental-features = nix-command flakes
   ```

2. Enter the development environment:
   ```bash
   nix develop
   # Or with direnv (auto-loads on cd):
   echo "use flake" > .envrc && direnv allow
   ```

   This provides: Rust 1.95.0, Bun 1.3.3, Node 22, Biome, pre-commit, git, jq — all pinned.

3. Install dependencies and build:
   ```bash
   bun install --frozen-lockfile
   bun run build
   ```

#### Option B: Manual tool installation

1. Install [Bun](https://bun.sh):
   ```bash
   curl -fsSL https://bun.sh/install | bash
   ```
2. Install project dependencies:
   ```bash
   bun install
   ```
3. Install the git hooks (replaces husky — language-agnostic, runs Biome,
   commitlint, and the Rust toolchain so local checks match CI):
   ```bash
   bun run precommit:install
   # or: pipx install pre-commit && pre-commit install --hook-type pre-commit --hook-type commit-msg
   ```
4. Build all packages:
   ```bash
   bun run build
   ```

#### Rust Toolchain (the bundler addon)

Only needed when working in `crates/*`. **With Nix: already included in `nix develop`.**

Without Nix:
1. Install [rustup](https://rustup.rs/). The repo pins its Rust version via
   `rust-toolchain.toml` (channel plus the `rustfmt` and `clippy` components), so
   rustup selects the right toolchain automatically inside the repo.
2. Build and test the Cargo workspace:
   ```bash
   cargo build
   cargo test
   ```
3. Format and lint before pushing (both gate CI):
   ```bash
   cargo fmt
   cargo clippy --all-targets -D warnings
   ```

A PR that touches `crates/` must have `cargo fmt`, `cargo clippy` (no warnings), and
`cargo test` green — the Rust analog of the `bun run ci:check` gate.

## Development Workflow

### Branch Naming

Use descriptive branch names with a conventional prefix:

- `feat/` — new features (e.g., `feat/add-system-monitoring`)
- `fix/` — bug fixes (e.g., `fix/resolve-memory-leak`)
- `chore/` — maintenance (e.g., `chore/update-dependencies`)
- `docs/` — documentation (e.g., `docs/add-api-guide`)
- `refactor/` — code refactoring (e.g., `refactor/extract-parser`)

### Conventional Commits

Commit messages are validated by a `commitlint` pre-commit hook (configured in
`.pre-commit-config.yaml`). All commits must follow the [Conventional Commits](https://www.conventionalcommits.org/)
specification:

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

Types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `perf`, `ci`, `style`.

Examples:
- `feat(core): add lifecycle hook system`
- `fix(logger): handle edge case in transport`
- `docs(api): document plugin interface`

### Pull Request Process

1. Create a feature branch from `main`.
2. Make your changes following the code style guidelines.
3. Write or update tests as needed.
4. Run the full test suite locally:
    ```bash
    bun test
    ```
5. Push your branch and open a pull request.
6. Ensure all CI checks pass.
7. Request review from maintainers.

## Code Style

We use [Biome](https://biomejs.dev/) for formatting and linting. Configuration
is in `biome.json` and `packages/biome-config/biome.json`.

```bash
# Format code
bun run lint --fix
```

Rules:
- Strict TypeScript mode (`strict: true`)
- No unused variables or parameters
- Path aliases defined in `tsconfig.json`
- Prefer named exports over default exports
- Use `type` imports for type-only imports

### General Guidelines

- Keep changes focused — one feature or fix per PR.
- Avoid adding unnecessary dependencies.
- Write clear, self-documenting code rather than adding comments.
- Use meaningful variable and function names.
- Keep functions small and focused on a single responsibility.

## Testing

All packages use [Vitest](https://vitest.dev/).

```bash
# Run all tests
bun test

# Run tests for a specific package
bun test --filter vibe/core

# Run tests in watch mode
cd packages/core && bun vitest --watch

# Run with coverage
bun test -- --coverage
```

### Test Structure

- Place tests in a `tests/` directory next to the source.
- Use descriptive test names that explain the expected behavior.
- Follow the Arrange-Act-Assert pattern.

## Pull Request Requirements

### PR Title Format

PR titles must follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>
```

### Labels

Every PR must have at least one of these labels:
- `semver:patch` — bug fixes and minor changes
- `semver:minor` — new features (backward compatible)
- `semver:major` — breaking changes

### Review Process

1. At least one maintainer review is required.
2. All CI checks must pass (lint, test, build, typecheck).
3. Changes may be requested by reviewers.
4. Once approved, a maintainer will merge the PR.

## Issue Reporting

### Bug Reports

Open a [bug report](https://github.com/vibeapp/vibe/issues/new?template=bug_report.yml)
with:
- A clear, descriptive title
- Steps to reproduce the behavior
- Expected vs. actual behavior
- Environment details (OS, Vibe version, Node version)
- Minimal reproduction case, if possible

### Feature Requests

Open a [feature request](https://github.com/vibeapp/vibe/issues/new?template=feature_request.yml)
with:
- A clear, descriptive title
- Use case and motivation
- Proposed solution or API design
- Alternatives you've considered

### Security Issues

Do **not** open a public issue for security vulnerabilities. Please report
security issues privately to **security@vibe.dev**. See our
[Security Policy](SECURITY.md) for details.

## Project Structure

```
vibe/
├── packages/         # TypeScript framework (bun + Turborepo)
│   ├── core/         # Main system entry point
│   ├── agent/        # Agent definition and loop
│   ├── model/        # Model provider layer (Anthropic, OpenAI-compatible, fake)
│   ├── tools/        # Tool / function-calling system
│   ├── di/           # Dependency injection container
│   ├── runtime/      # Runtime execution engine (retry, cancellation)
│   ├── build/        # Bundler / build tooling (uses the Rust addon)
│   └── …             # config, memory, workflows, skills, ontology, governance, …
├── crates/           # Rust bundler addon (Cargo workspace)
│   ├── vibe_bundler/ # oxc-based static analysis of agent/tool TS modules
│   └── vibe_napi/    # napi binding exposing the analysis to vibe/build
├── docs/             # Documentation
├── tools/            # Build and development tools
├── turbo.json        # Turborepo configuration
├── Cargo.toml        # Cargo workspace root
└── package.json      # bun workspace root
```

## Recognition

All contributors will be recognized in our
[CONTRIBUTORS.md](CONTRIBUTORS.md) file. We value every contribution, whether
it's code, documentation, bug reports, or community support.

---

Thank you for helping make Vibe better!
