# ADR-0001: License Policy

> **Status**: Accepted  
> **Date**: 2025-12-31  
> **Authors**: Architecture Council

## Context

sysprims's primary value proposition is providing GPL-free process utilities that can be statically linked into commercial and open-source software without "license toxicity" concerns. This requires:

1. Choosing an appropriate license for sysprims itself
2. Defining strict policies for dependency licenses
3. Establishing enforcement mechanisms

Enterprise users need confidence that including sysprims in their products won't trigger viral licensing requirements.

## Decision

### Project License

sysprims is dual-licensed under **MIT OR Apache-2.0**, following Rust ecosystem conventions.

**Rationale**:

- MIT provides maximum permissiveness
- Apache-2.0 provides explicit patent grants (valuable for enterprise)
- Dual licensing gives users flexibility

### Dependency Allowlist

Only dependencies with the following licenses are permitted:

| License      | SPDX ID      | Notes                     |
| ------------ | ------------ | ------------------------- |
| MIT          | MIT          | Permissive                |
| Apache-2.0   | Apache-2.0   | Permissive + patent grant |
| BSD-2-Clause | BSD-2-Clause | Permissive                |
| BSD-3-Clause | BSD-3-Clause | Permissive                |
| ISC          | ISC          | MIT-equivalent            |
| Zlib         | Zlib         | Permissive                |
| 0BSD         | 0BSD         | Public domain equivalent  |
| Unlicense    | Unlicense    | Public domain             |
| CC0-1.0      | CC0-1.0      | Public domain             |

### Dependency Denylist

The following licenses are **strictly prohibited**:

| License  | SPDX ID       | Reason                                  |
| -------- | ------------- | --------------------------------------- |
| GPL-2.0  | GPL-2.0-only  | Copyleft                                |
| GPL-3.0  | GPL-3.0-only  | Copyleft                                |
| LGPL-2.0 | LGPL-2.0-only | Weak copyleft (static linking concerns) |
| LGPL-2.1 | LGPL-2.1-only | Weak copyleft                           |
| LGPL-3.0 | LGPL-3.0-only | Weak copyleft                           |
| AGPL-3.0 | AGPL-3.0-only | Network copyleft                        |

### Enforcement

1. **cargo-deny**: CI runs `cargo deny check licenses` on every PR
2. **goneat**: Full SBOM and license analysis per build
3. **SBOM Generation**: Every release includes SPDX SBOM
4. **THIRD_PARTY_NOTICES.md**: Generated and maintained per release

### OS API Position

Direct use of operating system APIs (Win32, POSIX syscalls, Mach APIs) does not create derivative works — these are interfaces, not copyrighted implementations. sysprims binaries are fully redistributable under our license.

**Note**: This is our technical position, not legal advice. Consult counsel for specific situations.

## Consequences

### Positive

- Clear guidance for contributors on acceptable dependencies
- Enterprise users can adopt with confidence
- Automated enforcement prevents accidental violations

### Negative

- Some useful crates may be excluded (e.g., GPL-licensed options)
- Requires ongoing vigilance on dependency updates

### Neutral

- THIRD_PARTY_NOTICES.md must be maintained
- License audits become part of release process

## Alternatives Considered

### Alternative 1: MIT-Only

Single MIT license without Apache-2.0 option.

**Rejected**: Apache-2.0's patent grant is valuable for enterprise users concerned about patent litigation.

### Alternative 2: Allow LGPL for Dynamic Linking

Permit LGPL dependencies if dynamically linked.

**Rejected**: Our primary artifacts are statically linked (musl builds). The complexity of maintaining separate dynamic-only builds for LGPL compliance isn't worth it.

### Alternative 3: No Automated Enforcement

Manual review of dependencies.

**Rejected**: Too error-prone; easy to accidentally introduce prohibited licenses in transitive dependencies.

## References

- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
- [SPDX License List](https://spdx.org/licenses/)
- [Rust API Guidelines on Licensing](https://rust-lang.github.io/api-guidelines/necessities.html#crate-and-its-dependencies-have-a-permissive-license-c-permissive)
