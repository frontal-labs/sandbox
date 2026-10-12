# Claude Code Commands for Vige Development

## Agent Commands

### /vibe:analyze
Analyze the codebase structure and provide insights about architecture patterns
Usage: /vibe:analyze [path]

### /vibe:generate:agent
Generate a new Vibe agent with standard structure
Usage: /vibe:generate:agent [name] [description]

### /vibe:generate:tool
Generate a new Vibe tool with proper type safety
Usage: /vibe:generate:tool [name] [description]

### /vibe:generate:workflow
Generate a new Vibe workflow definition
Usage: /vibe:generate:workflow [name] [description]

### /vibe:test:agent
Run tests for a specific agent
Usage: /vibe:test:agent [agent-name]

### /vibe:test:tool
Run tests for a specific tool
Usage: /vibe:test:tool [tool-name]

## Development Commands

### /vibe:dev:start
Start the development server
Usage: /vibe:dev:start

### /vibe:dev:build
Build the project
Usage: /vibe:dev:build

### /vibe:dev:test
Run the test suite
Usage: /vibe:dev:test

### /vibe:dev:lint
Run the linter
Usage: /vibe:dev:lint

### /vibe:dev:fmt
Format the code
Usage: /vibe:dev:fmt

## Code Quality Commands

### /vibe:quality:check
Run all code quality checks (lint, typecheck, format)
Usage: /vibe:quality:check

### /vibe:quality:fix
Auto-fix code quality issues where possible
Usage: /vibe:quality:fix

### /vibe:security:audit
Run security audit on dependencies
Usage: /vibe:security:audit

## Documentation Commands

### /vibe:docs:generate
Generate documentation from code comments
Usage: /vibe:docs:generate

### /vibe:docs:validate
Validate existing documentation
Usage: /vibe:docs:validate

## Utility Commands

### /vibe:cache:clear
Clear development caches
Usage: /vibe:cache:clear

### /vibe:deps:update
Update project dependencies
Usage: /vibe:deps:update

### /vibe:repl:start
Start a REPL for interactive development
Usage: /vibe:repl:start