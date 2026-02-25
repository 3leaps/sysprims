# ADR-0006: Dependency Governance

> **Status**: Accepted
> **Date**: 2025-12-31
> **Authors**: Architecture Council

## Context

sysprims's value proposition includes minimal dependency footprint and GPL-free guarantees. We need systematic governance to:

1. Prevent accidental introduction of problematic dependencies
2. Maintain small binary sizes
3. Ensure supply chain security
4. Provide clear provenance for enterprise users

## Decision

### Dependency Tiers

Dependencies are classified into tiers based on when they're included:

| Tier           | Features                   | Target Size | Dependencies                 |
| -------------- | -------------------------- | ----------- | ---------------------------- |
| **Minimal**    | `default-features = false` | ~800KB      | libc, windows-sys, thiserror |
| **Standard**   | `default`                  | ~1.2MB      | + serde, serde_json, time    |
| **Observable** | `tracing`                  | ~1.4MB      | + tracing                    |
| **Extended**   | `proc_ext`                 | ~1.5MB      | + syscall wrappers           |
| **Full**       | `sysinfo_backend`          | ~2.5MB      | + sysinfo (~50 deps)         |

### Enforcement Tools

#### cargo-deny

Primary license and security enforcement:

```toml
# deny.toml
[licenses]
allow = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "Unlicense", "CC0-1.0"]
deny = ["GPL-2.0", "GPL-3.0", "LGPL-2.0", "LGPL-2.1", "LGPL-3.0", "AGPL-3.0"]
copyleft = "deny"

[advisories]
vulnerability = "deny"
unmaintained = "warn"

[bans]
# Prevent specific problematic crates
deny = []
```

#### cargo-audit

Security advisory checking:

```bash
cargo audit
```

#### goneat

Full SBOM and license analysis per build:

```bash
goneat analyze --format sbom --output sbom-${VERSION}.spdx.json
goneat check --policy deny.toml
```

#### cargo-sbom

SPDX SBOM generation for releases:

```bash
cargo sbom --output-format spdx > sbom-${VERSION}.spdx.json
```

### Dependency Addition Process

1. **Evaluate necessity**: Can we achieve this without a new dependency?
2. **License check**: Is it on the allowlist?
3. **Transitive review**: What does it pull in?
4. **Size impact**: How much does it add to binary?
5. **Maintenance status**: Is it actively maintained?
6. **Security history**: Any past vulnerabilities?

New dependencies require ADR if they:

- Add >100KB to binary size
- Add >10 transitive dependencies
- Are not in the "well-known safe" list

### Well-Known Safe Dependencies

These are pre-approved for use without additional review:

| Crate       | Tier       | Purpose               |
| ----------- | ---------- | --------------------- |
| libc        | Minimal    | POSIX syscalls        |
| windows-sys | Minimal    | Win32 API             |
| cfg-if      | Minimal    | Platform conditionals |
| thiserror   | Minimal    | Error derives         |
| serde       | Standard   | Serialization         |
| serde_json  | Standard   | JSON                  |
| time        | Standard   | Timestamps            |
| clap        | CLI        | Argument parsing      |
| tracing     | Observable | Telemetry             |

### CI Pipeline

```yaml
jobs:
  dependency-check:
    steps:
      - name: License check
        run: cargo deny check licenses

      - name: Security audit
        run: cargo audit

      - name: goneat analysis
        run: |
          goneat analyze --format sbom --output sbom.json
          goneat check --policy deny.toml

      - name: Size check
        run: |
          cargo build --release --target x86_64-unknown-linux-musl
          size=$(stat -c%s target/x86_64-unknown-linux-musl/release/sysprims-timeout)
          if [ $size -gt 2000000 ]; then
            echo "Binary size exceeds 2MB threshold"
            exit 1
          fi
```

### THIRD_PARTY_NOTICES.md

Generated per release with:

```bash
cargo about generate about.hbs > THIRD_PARTY_NOTICES.md
```

Contains:

- All dependency names and versions
- License text for each
- Copyright notices

## Consequences

### Positive

- Automated enforcement prevents accidents
- Clear process for adding dependencies
- SBOM provides enterprise audit trail
- Size targets keep binaries small

### Negative

- Overhead for dependency additions
- May reject useful crates due to license
- CI time for full analysis

### Neutral

- goneat becomes part of toolchain
- Release process includes SBOM generation
- Documentation includes dependency rationale

## Alternatives Considered

### Alternative 1: Manual Review Only

No automated tooling, rely on PR review.

**Rejected**: Too error-prone; transitive dependencies easy to miss.

### Alternative 2: Allow LGPL Dynamically

Permit LGPL for dynamic linking only.

**Rejected**: Primary artifacts are statically linked (musl). Complexity not worth it.

### Alternative 3: No Size Targets

Don't enforce binary size limits.

**Rejected**: "Minimal core" is a key value proposition. Must be enforced.

## References

- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
- [cargo-audit](https://github.com/RustSec/rustsec/tree/main/cargo-audit)
- [cargo-sbom](https://github.com/psastras/sbom-rs)
- [goneat](https://github.com/fulmenhq/goneat)
