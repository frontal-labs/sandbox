# Frontal Sandbox

![Frontal Banner](./banner.jpg)

**Frontal Sandbox is a workspace for experimentation and prototyping.**

Built by [Frontal](https://frontal.dev) as a flexible place to explore ideas, prototypes, and future product directions.

## Features

- **Agent-first architecture** — Define, compose, and orchestrate AI agents with a type-safe API
- **Model-agnostic** — Plug in any LLM provider (OpenAI, Anthropic, Google, etc.) via a unified interface
- **Tool & function calling** — First-class support for tool use with structured input/output validation
- **Memory & state** — Persistent agent memory across sessions and conversations
- **Skills & workflows** — Composable building blocks for complex multi-step agent behaviors
- **Governance & safety** — Built-in policy engine, guardrails, and human-in-the-loop controls
- **Observability** — Tracing, logging, and metrics out of the box
- **Plugin system** — Extend Vibe with custom plugins and third-party integrations
- **Dependency injection** — Flexible DI container for wiring agent dependencies
- **Evaluation framework** — Benchmark and test agent performance systematically
- **Native performance** — optional Rust accelerators (via Node.js bindings) for the bundler,
  accurate context-window token counting, and OpenAI stream folding; every native path has a
  pure-TypeScript fallback, so the framework runs unchanged without them

## Getting Started

```bash
# Install dependencies
bun install

# Build all packages
bun run build

# Run tests
bun run test

# Start development mode
bun run dev
```

## Packages

| Package | Description |
|---------|-------------|
| `@frontal-labs/vibe` | Barrel package |
| `@frontal-labs/vibe-cli` | CLI tool for scaffolding and managing projects |
| `vibe/core` | Central orchestrator that wires all modules together |
| `vibe/agent` | Agent definition, lifecycle, and execution |
| `vibe/model` | LLM provider abstraction layer |
| `vibe/tools` | Tool and function-calling system |
| `vibe/memory` | Agent memory and state management |
| `vibe/runtime` | Execution runtime for agents |
| `vibe/skills` | Composable skill definitions |
| `vibe/workflows` | Multi-step workflow orchestration |
| `vibe/ontology` | Semantic layer — entities, relations, and retrieval grounding |
| `vibe/governance` | Policy engine and safety controls |
| `vibe/security` | Security primitives and sandboxing |
| `vibe/evals` | Evaluation and benchmarking utilities |
| `vibe/observability` | Logging, tracing, and metrics |
| `vibe/plugin` | Plugin system for extensibility |
| `vibe/di` | Dependency injection container |
| `vibe/config` | Configuration management |
| `vibe/build` | Build tooling and bundling |
| `vibe/deploy` | Deployment utilities |
| `vibe/devtools` | Developer experience tools |
| `vibe/tracing` | Distributed tracing |
| `vibe/errors` | Structured error handling |
| `vibe/logger` | Structured logging |
| `vibe/shared` | Shared types and utilities |
| `vibe/adapters` | Third-party service adapters |
| `vibe/lifecycle` | Startup/shutdown lifecycle management |
| `vibe/mcp-server` | MCP server that lets AI agents operate and extend Vibe |

## Project Structure

```
vibe/
├── apps/            # Documentation site
├── benchmarks/      # Performance benchmarks
├── cookbooks/       # Example recipes and guides
├── crates/          # Rust native modules (bundler, NAPI bindings)
├── docs/            # Project documentation
├── editors/         # Editor integrations
├── examples/        # Example projects
├── packages/        # Core TypeScript packages
├── scripts/         # Build and CI scripts
├── skills/          # Built-in skill definitions
├── tests/           # Integration tests
├── tools/           # Dev tooling and generators
├── vibe/            # Barrel package
├── vibe-cli/        # CLI tool for scaffolding and managing projects 
└──
```

## Tech Stack

- **Runtime:** [Bun](https://bun.sh) v1.3+
- **Language:** TypeScript 5.6+ / Rust
- **Build:** Turborepo + Vite
- **Linting:** Biome
- **Testing:** Vitest
- **Versioning:** Changesets
- **Git hooks:** pre-commit (Biome, commitlint, Rust fmt/clippy)

## License

Apache-2.0
