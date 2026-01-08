# ADR-0000: ADR Process

> **Status**: Accepted  
> **Date**: 2025-12-31  
> **Authors**: Architecture Council

## Context

sysprims is a foundational library that will be consumed by multiple ecosystems (Fulmen, external users) across multiple languages (Rust, Go, Python, TypeScript). Architectural decisions have long-term implications for:

- API stability and backward compatibility
- Cross-platform behavior consistency
- License compliance
- Performance characteristics

We need a lightweight but formal process for documenting significant technical decisions.

## Decision

We adopt the Architecture Decision Record (ADR) format as described by Michael Nygard, with the following conventions:

### When to Write an ADR

An ADR is required for decisions that:

1. Affect public API surface (library or FFI)
2. Change dependency policy or add new dependencies
3. Alter platform support or behavior parity
4. Modify schema contracts
5. Impact security boundaries
6. Change build or release processes

An ADR is **not** required for:

- Bug fixes
- Documentation improvements
- Internal refactoring that doesn't affect public contracts
- Dependency version bumps (unless changing major versions)

### ADR Lifecycle

```
Proposed ──► Accepted ──► [Deprecated | Superseded]
    │
    └──► Rejected (delete or keep as reference)
```

### Approval Process

1. Author creates ADR in a feature branch
2. PR opened with `adr` label
3. Minimum one approval from Platform Architecture team
4. 48-hour comment period for significant changes
5. Merge and update index

### Numbering

ADRs are numbered sequentially (0001, 0002, ...). Numbers are never reused.

## Consequences

### Positive

- Decisions are documented and searchable
- New team members can understand historical context
- Reduces repeated discussions of settled matters

### Negative

- Overhead for documenting decisions
- Risk of ADRs becoming stale if not maintained

### Neutral

- ADRs become part of repository documentation
- CI may validate ADR format in future

## Alternatives Considered

### Alternative 1: No Formal Process

Rely on PR descriptions and comments for decision history.

**Rejected**: Difficult to find historical context; decisions scattered across many PRs.

### Alternative 2: Wiki-Based Documentation

Use GitHub Wiki for architecture documentation.

**Rejected**: Wikis are disconnected from code; harder to version with releases.

### Alternative 3: RFCs

Full RFC process with formal review periods.

**Rejected**: Too heavyweight for current project size; can adopt later if needed.

## References

- [Michael Nygard's ADR article](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
- [adr.github.io](https://adr.github.io/)
