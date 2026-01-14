---
title: "{{MODULE}} Equivalence Test Protocol"
module: "{{MODULE}}"
version: "0.1"
status: "Draft"
last_updated: "{{DATE}}"
---

<!-- TEMPLATE: Replace all {{PLACEHOLDER}} values and remove these comments -->

# {{MODULE}} Equivalence Test Protocol

## 1) Purpose

This document defines the test protocol for verifying {{MODULE}} behaves correctly and, where applicable, equivalently to reference implementations.

**Equivalence means:**
- CLI produces compatible output for common use cases
- Library provides documented invariants
- Errors are reported consistently

**Equivalence does NOT mean:**
- Byte-for-byte identical output
- Supporting every flag/option of reference tools
- Matching undocumented or quirky behavior

## 2) Reference Implementations

<!-- List tools we compare against, if any -->

| Tool | Version | License | Platform | Notes |
|------|---------|---------|----------|-------|
| <!-- e.g., ps --> | <!-- Version --> | <!-- License --> | Linux/macOS | <!-- What we compare --> |

**Invocation method:** Subprocess only. We never read reference tool source code during testing.

## 3) Test Matrix

### Platforms

| OS | Arch | CI Runner | Notes |
|----|------|-----------|-------|
| Linux | x64 | ubuntu-latest | Primary |
| macOS | arm64 | macos-latest | |
| Windows | x64 | windows-latest | |

### Privilege Levels

| Level | Tests | Runner |
|-------|-------|--------|
| Non-root | All standard tests | Default CI |
| Root | <!-- Privileged tests, if any --> | Dedicated workflow |

## 4) Test Categories

### 4.1 Functional Tests

<!-- Core functionality tests -->

| Test | Description | Expected |
|------|-------------|----------|
| `test_{{feature}}_basic` | <!-- What it tests --> | <!-- Expected outcome --> |

### 4.2 Edge Cases

<!-- Boundary conditions and unusual inputs -->

| Test | Input | Expected |
|------|-------|----------|
| `test_{{edge_case}}` | <!-- Edge case input --> | <!-- Expected behavior --> |

### 4.3 Error Handling

<!-- Error condition tests -->

| Test | Condition | Expected Error |
|------|-----------|----------------|
| `test_error_{{condition}}` | <!-- Trigger --> | `SysprimsError::{{Type}}` |

### 4.4 Platform-Specific

<!-- Platform-specific behavior tests -->

| Test | Platform | Behavior |
|------|----------|----------|
| `test_{{feature}}_{{platform}}` | <!-- Platform --> | <!-- Expected --> |

### 4.5 Schema Validation

| Test | Description |
|------|-------------|
| `test_json_has_schema_id` | JSON output includes valid schema_id |
| `test_json_schema_valid` | Output validates against schema |

## 5) Equivalence Tests

<!-- Tests that compare against reference tools -->

### 5.1 Output Comparison

```rust
#[test]
fn {{module}}_matches_reference_{{aspect}}() {
    // Our implementation
    let ours = sysprims_{{module}}::{{function}}(...);

    // Reference (subprocess, not code)
    let reference = Command::new("{{reference_tool}}")
        .args(&[...])
        .output();

    // Compare behavior, not exact bytes
    assert_eq!(ours.{{field}}, reference.{{field}});
}
```

### 5.2 Behavioral Equivalence

| Behavior | Our Implementation | Reference | Match |
|----------|-------------------|-----------|-------|
| <!-- Behavior --> | <!-- Ours --> | <!-- Theirs --> | Yes/No/N/A |

## 6) Determinism and Flake Policy

### Timing Sensitivity

<!-- How we handle timing-sensitive tests -->

- <!-- Strategy for timing variations -->

### Acceptable Variations

<!-- Documented acceptable differences -->

| Field | Variation Allowed | Reason |
|-------|-------------------|--------|
| <!-- Field --> | <!-- Tolerance --> | <!-- Why --> |

### Fixture Processes

<!-- Test fixtures with known behavior -->

| Fixture | Purpose | Behavior |
|---------|---------|----------|
| <!-- Fixture --> | <!-- Why used --> | <!-- What it does --> |

## 7) Test Locations

| Type | Location |
|------|----------|
| Unit tests | `crates/{{crate}}/src/**/*.rs` |
| Integration tests | `crates/{{crate}}/tests/` |
| Equivalence tests | `tests/equivalence/{{module}}/` |
| Golden tests | `tests/golden/{{module}}/` |

## 8) Artifacts

### CI Outputs

| Artifact | Path | Purpose |
|----------|------|---------|
| Test results | `target/test-results/` | Pass/fail evidence |
| Coverage | `target/coverage/` | Coverage metrics |
| Golden outputs | `tests/golden/{{module}}/` | Reference outputs |

### Compliance Evidence

These artifacts are referenced in the compliance report:

- [ ] Test run logs (CI job link)
- [ ] Coverage report
- [ ] Equivalence comparison results

---

*Protocol version: 0.1*
*Last updated: {{DATE}}*
