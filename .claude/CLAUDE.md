# Claude Code Configuration for Vibe

This file provides Claude-specific configuration for working with the Vibe repository.

## Skill Configuration

Claude automatically loads skills from the `.claude/skills` directory, which is symlinked to the project's shared `skills` directory.

## Agent Guidelines

When operating as an autonomous software engineer, Claude should follow these principles:

1. **Inspect First**: Always examine the codebase structure and existing patterns before making changes
2. **Plan Before Coding**: Create a clear, step-by-step implementation plan
3. **Use Skills Proactively**: Automatically invoke relevant skills based on task requirements
4. **Reuse Existing Patterns**: Prefer extending established solutions over creating new approaches
5. **Iterative Development**: Make small, verifiable changes that can be independently validated
6. **Continuous Testing**: Run tests after every meaningful change to prevent regressions
7. **Documentation Updates**: Keep documentation synchronized with code modifications
8. **Parallel Execution**: Leverage concurrent processing for independent tasks when beneficial
9. **Completion Verification**: Consider work complete only when fully implemented, tested, and integrated

## Standard Workflow

### 1. Exploration Phase
- Use `@workspace` to understand project structure
- Search for similar implementations with `@grep` or `@web`
- Review relevant documentation

### 2. Planning Phase
- Create a numbered list of implementation steps
- Identify which skills are needed for the task
- Determine how to test the implementation

### 3. Implementation Phase
- Implement one piece of functionality at a time
- Use appropriate Cursor commands:
  - `/chat` for discussions and planning
  - `/edit` for targeted code changes
  - `/terminal` for running commands
- Apply skills as needed:
  - `@code-review` for self-review
  - `@test` to generate/update tests
  - `@docs` to update documentation

### 4. Validation Phase
- Run relevant tests: `bun run test` or scoped tests
- Check types: `bun run typecheck`
- Validate code quality: `bun run lint`
- Verify formatting: `bun run format:check`
- Manual verification when needed

### 5. Completion Phase
- Update documentation (READMEs, JSDoc, comments)
- Clean up debugging code and temporary files
- Ensure git history has meaningful commit messages
- Verify all acceptance criteria are met

## Project Commands

- Install: `bun install`
- Build: `bun run build`
- Develop: `bun run dev`
- Test: `bun run test`
- Type check: `bun run typecheck`
- Lint: `bun run lint`
- Fix lint: `bun run lint:fix`
- Format: `bun run format`
- Check format: `bun run format:check`

## Rust/Cargo Commands (for native modules)

- Build Rust: `bun run cargo:build`
- Test Rust: `bun run cargo:test`
- Check Rust: `bun run cargo:check`
- Clippy: `bun run cargo:clippy`
- Format Rust: `bun run cargo:fmt`

## Commit Message Convention

Use conventional commits:
- `feat:` for new features
- `fix:` for bug fixes
- `docs:` for documentation changes
- `style:` for formatting changes
- `refactor:` for code restructuring
- `perf:` for performance improvements
- `test:` for test-related changes
- `chore:` for maintenance tasks

Include scope when relevant: `feat(auth): add JWT validation`

Provide motivation in commit body for non-obvious changes:
- What problem does this solve?
- Why was this approach chosen?
- What alternatives were considered?