---
title: "{{MODULE}} Provenance"
module: "{{MODULE}}"
version: "0.1"
status: "Draft"
last_updated: "{{DATE}}"
---

<!-- TEMPLATE: Replace all {{PLACEHOLDER}} values and remove these comments -->

# Provenance: {{MODULE}}

This document records the sources consulted for implementing `{{MODULE}}`, ensuring clear provenance for all functionality.

## Policy

- POSIX and platform specifications are the primary reference
- BSD/MIT/Apache licensed implementations may be consulted for understanding
- Behavioral comparison against tools (via subprocess) is permitted for testing
- Source code of restrictively-licensed tools (GPL/LGPL/AGPL) is NOT consulted

## {{Function/Feature 1}}

### Specification Sources (Primary)

<!-- List authoritative specifications -->

1. **<!-- Spec name -->**
   - URL: <!-- URL -->
   - License: <!-- License or "Specification (no license restriction)" -->
   - Used for: <!-- What we derived from this -->

### Implementation References (Consulted)

<!-- List permissively-licensed code we looked at for understanding -->

1. **<!-- Project/file name -->**
   - URL: <!-- URL -->
   - License: <!-- Must be BSD/MIT/Apache/ISC or similar -->
   - Consulted for: <!-- What aspect -->

### NOT Consulted

<!-- Explicitly list what we avoided -->

- <!-- Tool name --> (<!-- License, e.g., GPL-3.0 -->) - NOT consulted

### Implementation Notes

<!-- Brief description of the approach derived from specs -->

The implementation:
1. <!-- Step 1 derived from spec -->
2. <!-- Step 2 -->
3. <!-- Step 3 -->

<!-- Repeat for additional functions/features -->

## Behavioral Testing

Tests may compare output against system tools via subprocess invocation. This validates behavioral equivalence without any code reading:

```rust
#[test]
fn matches_system_behavior() {
    // Our implementation
    let ours = {{module}}::{{function}}(...);

    // System tool (subprocess - no license concern)
    let system = Command::new("{{tool}}")
        .args(&[...])
        .output();

    // Compare behavior, not code
    assert_eq!(ours.{{field}}, expected);
}
```

## Certification

This module's implementation is derived from:

- [ ] Public specifications (POSIX, platform docs)
- [ ] Permissively-licensed references (BSD/MIT/Apache/ISC)
- [ ] Original implementation where no reference was needed

No GPL/LGPL/AGPL source code was consulted during development.

---

*Provenance version: 0.1*
*Last updated: {{DATE}}*
*Maintainer: sysprims team*
