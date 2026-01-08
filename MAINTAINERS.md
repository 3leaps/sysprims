# Maintainers

This document lists the maintainers for 3leaps/sysprims.

## Human Maintainers

| Name          | GitHub      | Email                    | Role            |
| ------------- | ----------- | ------------------------ | --------------- |
| Dave Thompson | @3leapsdave | dave.thompson@3leaps.net | Lead maintainer |

## Autonomous Agents

_None configured. This repository uses supervised mode only._

When autonomous agents are adopted, they will be listed here with their GitHub accounts and escalation contacts. See [agent-identity standard](https://crucible.3leaps.dev/repository/agent-identity) for the identity scheme.

## AI-Assisted Development

This repository uses AI assistants in **supervised mode**. Configuration is in [AGENTS.md](AGENTS.md).

Key points:

- AI assistants operate under human supervision
- All commits require human review and approval
- Human maintainer is accountable (Committer-of-Record)
- No persistent agent identity across sessions
- Roles (not named identities) define agent scope

### Security-Sensitive Classification

This repository is classified as **security-sensitive** due to:

- Process control utilities affecting system stability
- FFI boundary requiring careful memory safety
- Signal handling with security implications
- Cross-platform behavior predictability requirements

All security-sensitive changes require explicit maintainer review.

## Governance

This repository is maintained by 3 Leaps, LLC.

For governance policies, see:

- [3leaps/oss-policies](https://github.com/3leaps/oss-policies)
- [LICENSE-MIT](LICENSE-MIT)
- [LICENSE-APACHE](LICENSE-APACHE)

## Contact

- **General**: hello@3leaps.net
- **Legal**: legal@3leaps.net
- **Security**: security@3leaps.net

## Review Requirements

| Change Type | Reviewer Required |
| ----------- | ----------------- |
| Code changes | Lead maintainer |
| FFI boundary changes | Lead maintainer + secrev role |
| Dependency additions | Lead maintainer (license check required) |
| ADR changes | Lead maintainer |
| Documentation | Lead maintainer |
| CI/CD changes | Lead maintainer |

## Release Authority

Only human maintainers may:

- Tag releases
- Push to protected branches
- Approve PRs for merge
- Publish to crates.io, PyPI, npm, or Go module proxy
