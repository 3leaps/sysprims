# Module Design Templates

This directory contains templates for documenting sysprims modules per the [Module Design SOP](../../standards/module-design-sop.md).

## Usage

1. Copy the templates to `docs/design/<module>/`
2. Rename each file: `<template>-template.md` → `<module>-<template>.md`
3. Fill in the placeholders (marked with `{{PLACEHOLDER}}`)
4. Remove template instructions (marked with `<!-- TEMPLATE: ... -->`)

## Templates

| Template                        | Purpose                               |
| ------------------------------- | ------------------------------------- |
| `spec-template.md`              | Module specification and API contract |
| `equivalence-tests-template.md` | Test protocol and acceptance criteria |
| `compliance-template.md`        | Evidence that requirements are met    |
| `provenance-template.md`        | Sources consulted and avoided         |

## Example

For `sysprims-proc`:

```bash
mkdir -p docs/design/sysprims-proc
cp docs/templates/module-design/spec-template.md docs/design/sysprims-proc/proc-spec.md
cp docs/templates/module-design/equivalence-tests-template.md docs/design/sysprims-proc/proc-equivalence-tests.md
cp docs/templates/module-design/compliance-template.md docs/design/sysprims-proc/proc-compliance.md
cp docs/templates/module-design/provenance-template.md docs/design/sysprims-proc/proc-provenance.md
```

Then edit each file to fill in module-specific content.

## Checklist

Before considering a module's design docs complete:

- [ ] All four documents created
- [ ] All `{{PLACEHOLDER}}` values replaced
- [ ] All `<!-- TEMPLATE: ... -->` comments removed
- [ ] Spec version matches implementation
- [ ] Traceability matrix complete
- [ ] Provenance sources verified
