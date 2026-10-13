# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for significant design decisions.

## Format

Each ADR follows this template:

```markdown
# NNNN: Short Title

## Status
Proposed | Accepted | Superseded by NNNN

## Date
YYYY-MM-DD

## Context
What is the issue that we're seeing that is motivating this decision or change?

## Decision
What is the change that we're proposing and/or doing?

## Consequences
### Positive
- 

### Negative
- 

### Risks
- 

## Alternatives Considered
- 

## References
- Links to related issues, PRs, docs
```

## Index

| ID | Title | Status | Date |
|----|-------|--------|------|

No ADR files exist yet. Only this README is present in `docs/adr/`. Add rows
here as `NNNN-title.md` files are created.

## Process

1. Create new ADR file: `docs/adr/NNNN-short-title.md` (next free number, starting at 0001)
2. Fill in template
3. Submit PR for review
4. On acceptance, update status to "Accepted"
5. If superseded, link to new ADR and mark old as "Superseded"

## When to Write an ADR

- Public API changes
- Security model changes
- New crate or major refactor
- Technology choices (e.g., V8 vs WASM for Python)
- Protocol changes (guest agent, trace format)
- Performance-critical design decisions