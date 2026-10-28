# 0024: TypeScript by type stripping

## Status
Accepted.

## Date
2026-10-20

## Context
PLAN.md §9 M1 requires that a `.ts` script runs against the VFS. The runtime runs classic JavaScript in V8. TypeScript needs to be turned into JavaScript first. The workspace already declared `swc_core` for the runtime, but no code used it.

## Decision
- **`node FILE.ts` parses the file as TypeScript and strips it.** `sandbox_runtime::strip_types` uses `swc_core`: `parse_file_as_module` with TypeScript syntax, then `resolver`, then the TypeScript `strip` transform, then `Emitter` to JavaScript. The result runs in the same classic-script runner as any `.js` file.
- **It is a transform, not a checker.** Types are removed, not verified. A file with a type error runs. That is what an agent's script expects from `node`, and checking would need the full TypeScript compiler, which is not in this build.
- **Constructs with runtime code are lowered by swc.** Enums and parameter properties run, and tests check them. Decorators are not covered by a test, so their behavior is not claimed.
- **Modules are refused as they are for `.js`.** `import` and `export` need the module loader, which the runner does not have.
- **Parse errors fail the run with exit 1 and nothing runs.**

## Consequences
### Positive
- A `.ts` file runs without an external compiler, inside the sandbox, with the same limits as `.js`.
- The dependency was already declared, so the change adds no new crate to the workspace. It enables several `swc_core` features.

### Negative
- The compile time of `sandbox-runtime` grows with the swc features it enables.
- Type errors are not reported. A script that relies on the compiler to catch them will run instead.

## Alternatives Considered
- **Shell out to an external `tsc` or `esbuild`.** Rejected. It needs a binary in the guest, and the sandbox has no process spawning.
- **A hand-written type eraser.** Rejected. Erasing types correctly needs a real parser, and a regex-based eraser would mangle valid code.
- **Full type checking.** Deferred. It needs the TypeScript compiler, which does not run here.

## References
- [PLAN.md §9 M1](../../PLAN.md).
- `crates/sandbox-runtime/src/typescript.rs`, `crates/sandbox-shell/src/builtins.rs` (`node`), `crates/sandbox-shell/tests/typescript.rs`.
