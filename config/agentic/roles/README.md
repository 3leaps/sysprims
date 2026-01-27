# Role Catalog (sysprims)

Agentic role prompts for AI agent sessions in this repository.

Roles extend [crucible baseline roles](https://crucible.3leaps.dev/catalog/roles/) with
sysprims-specific scope, responsibilities, and validation requirements.

## Available Roles

| Role | Slug | Category | Purpose |
|------|------|----------|---------|
| [Development Lead](devlead.yaml) | `devlead` | agentic | Core implementation, architecture |
| [Security Review](secrev.yaml) | `secrev` | review | Security analysis, FFI safety, PID validation |
| [Quality Assurance](qa.yaml) | `qa` | review | Testing, cross-platform coverage |
| [Release Engineering](releng.yaml) | `releng` | automation | Release coordination with CI/CD platform validation |
| [CI/CD Automation](cicd.yaml) | `cicd` | automation | Pipelines, runners, platform matrix |
| [Information Architect](infoarch.yaml) | `infoarch` | agentic | Documentation, schemas, standards |

## Key Customizations for sysprims

All roles include sysprims-specific extensions:

### Safety Protocols

Every role that touches code references:
- [REPOSITORY_SAFETY_PROTOCOLS.md](../../../REPOSITORY_SAFETY_PROTOCOLS.md) - mandatory reading
- [ADR-0011: PID Validation Safety](../../../docs/decisions/ADR-0011-pid-validation-safety.md) - prevents kill(-1) disasters

### Platform Matrix

Roles that involve builds or releases reference:
- [Platform Support Matrix](../../../docs/standards/platform-support.md) - canonical 6-platform reference

### Pre-Push Validation

The `releng` and `cicd` roles include explicit pre-push checklists:
- actionlint validation
- shellcheck validation
- Runner availability verification
- Platform matrix consistency checks

## Usage

Reference roles in session prompts or AGENTS.md:

```yaml
roles:
  - slug: releng
    source: config/agentic/roles/releng.yaml
```

Or load directly in a session:

```
Role: releng (config/agentic/roles/releng.yaml)
```

## Role Selection Guide

| Task | Primary Role | May Escalate To |
|------|--------------|-----------------|
| Feature implementation | devlead | secrev (security), qa (testing) |
| Bug fixes | devlead | qa (regression tests) |
| Security review | secrev | human maintainers (critical) |
| Test design | qa | devlead (implementation questions) |
| CI/CD changes | cicd | releng (release workflows), secrev (secrets) |
| Release preparation | releng | cicd (workflow issues), human maintainers (approval) |
| Documentation | infoarch | devlead (technical accuracy) |

## Schema

Role files conform to the [role-prompt schema](https://schemas.3leaps.dev/agentic/v0/role-prompt.schema.json).
