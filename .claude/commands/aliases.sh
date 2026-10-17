# Claude Code Command Aliases for Frontal Sandbox
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

# Analysis shortcuts
alias analyze:plan="rg -n 'Milestone|Acceptance|Goal|Non-goal' plans/PLAN.md"

# Utility shortcuts
alias cache:clear="rm -rf target node_modules/.cache"
alias reset:hard="git reset --hard HEAD && git clean -fd"
alias prune:branches="git branch --merged | grep -v '\\*\\|main\\|master' | xargs -n 1 git branch -d"

# Sandbox-specific shortcuts
alias sandbox:plan="sed -n '1,240p' plans/PLAN.md"
alias sandbox:architecture="rg -n 'invariant|backend|capability|promotion' plans/PLAN.md"
alias sandbox:security="rg -n 'security|capability|network|isolate|microVM' plans/PLAN.md"
