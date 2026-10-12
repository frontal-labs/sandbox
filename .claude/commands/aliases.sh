# Claude Code Command Aliases for Vibe
# These are shorthand references to common operations

# Development shortcuts
alias dev:start="bun run dev"
alias dev:build="bun run build"
alias dev:test="bun run test"
alias dev:lint="bun run lint"
alias dev:fmt="bun run format"
alias dev:typecheck="bun run typecheck"

# Testing shortcuts
alias test:unit="bun run test"
alias test:watch="bun run test --watch"
alias test:coverage="bun run test --coverage"
alias test:ui="bun run test --ui"

# Linting shortcuts
alias lint:check="bun run lint"
alias lint:fix="bun run lint:fix"
alias format:check="bun run format:check"
alias format:write="bun run format"

# Package management
alias pkg:install="bun install"
alias pkg:update="bun update"
alias pkg:outdated="bun outdated"

# Rust specific (for native modules)
alias rust:build="bun run cargo:build"
alias rust:test="bun run cargo:test"
alias rust:check="bun run cargo:check"
alias rust:clippy="bun run cargo:clippy"
alias rust:fmt="bun run cargo:fmt"

# Changeset workflow
alias changeset:new="bun run changeset"
alias version:patch="bun run version"
alias release:publish="bun run release"

# Analysis shortcuts
alias analyze:deps="bun run depcheck"
alias analyze:complexity="npm run complexity-report"
alias analyze:bundlesize="npm run bundle-analysis"

# Utility shortcuts
alias cache:clear="rm -rf node_modules/.cache && rm -rf .turbo && bun install"
alias reset:hard="git reset --hard HEAD && git clean -fd"
alias prune:branches="git branch --merged | grep -v '\\*\\|main\\|master' | xargs -n 1 git branch -d"

# Vibe specific shortcuts
alias vibe:agent:create="python scripts/generate_agent.py"
alias vibe:tool:create="python scripts/generate_tool.py"
alias vibe:wf:create="python scripts/generate_workflow.py"