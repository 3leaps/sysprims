# Lint Configuration Decisions

This document records configuration decisions for lint tools in sysprims.

## LDR-001: yamllint Line Length (120 chars)

**Tool:** yamllint
**Rule:** line-length
**Default:** 80 characters
**Configured:** 120 characters
**Location:** `.yamllint`

### Context

yamllint's default 80-character line limit is too restrictive for:
- GitHub Actions workflows (long `run:` commands, URLs, matrix expressions)
- YAML config files with inline arrays

### Decision

Relax line-length to 120 characters. This aligns with:
- Common editor defaults (120 chars)
- GitHub's code review display width
- Practical YAML readability

### Affected Files

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.goneat/hooks.yaml`
- `.goneat/tools.yaml`

---

## LDR-002: yamllint Truthy Values

**Tool:** yamllint
**Rule:** truthy
**Default:** Only `true`, `false` allowed
**Configured:** Allow `true`, `false`, `on`, `off`
**Location:** `.yamllint`

### Context

GitHub Actions uses `on:` as the workflow trigger keyword. This is valid YAML but yamllint flags it as a "truthy value should be one of [false, true]".

### Decision

Configure truthy rule to allow `on` and `off` in addition to `true` and `false`. This is standard YAML and required for GitHub Actions compatibility.

### Affected Files

- `.github/workflows/ci.yml` (line 3: `on:`)
- `.github/workflows/release.yml` (line 3: `on:`)

---

## LDR-003: checkmake maxbodylength (70 lines)

**Tool:** checkmake
**Rule:** maxbodylength
**Default:** 20 lines
**Configured:** 70 lines
**Location:** `.goneat/assess.yaml`

### Context

checkmake's default maxbodylength of 20 lines is overly restrictive for real-world Makefiles. The sysprims Makefile has legitimate targets that exceed this:

| Target | Lines | Purpose |
|--------|-------|---------|
| help | 41 | Auto-generated help from target comments |
| bootstrap | 67 | Multi-platform toolchain setup |
| tools | 46 | Development tool installation |

### Decision

Increase maxbodylength to 70 lines. These targets are:
- Well-documented with comments
- Logically organized
- Necessary for comprehensive developer tooling

Splitting them would reduce readability without improving maintainability.

---

## Configuration Audit Log

| Date | LDR | Action | Author |
|------|-----|--------|--------|
| 2026-01-14 | LDR-001 | Created | Claude Opus 4.5 |
| 2026-01-14 | LDR-002 | Created | Claude Opus 4.5 |
| 2026-01-14 | LDR-003 | Created | Claude Opus 4.5 |
