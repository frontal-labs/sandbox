# Pull Request Template

## Description

Please describe your changes in detail.

## Related Issue

Please link to the issue this PR addresses (if applicable).

Closes #

## Checklist

Please check if your PR fulfills the following requirements:

- [ ] The PR title and commit messages follow Conventional Commits (see CONTRIBUTING.md)
- [ ] A semver label is set (`semver-patch`, `semver-minor`, or `semver-major`)
- [ ] Tests for the changes have been added (for bug fixes / features)
- [ ] Rust formatting passes (`cargo fmt --all --check`)
- [ ] Clippy passes (`cargo clippy --workspace --all-targets -- -D warnings`)
- [ ] Rust tests pass (`cargo test --workspace`), if Rust code changed
- [ ] TypeScript SDK lint and tests pass (`bun run lint`, `bun run ts:test`), if TypeScript changed
- [ ] Documentation has been updated if necessary (including PLAN.md if behavior or public API changed)
- [ ] I have only claimed checks in this description that I actually ran

## Additional Information

Add any other context about the problem here.
