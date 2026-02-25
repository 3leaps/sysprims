---
title: "{{MODULE}} Compliance Report"
module: "{{MODULE}}"
version: "0.1"
status: "Draft"
last_updated: "{{DATE}}"
---

<!-- TEMPLATE: Replace all {{PLACEHOLDER}} values and remove these comments -->

# {{MODULE}} Compliance Report

## Summary

| Item                   | Status                  |
| ---------------------- | ----------------------- |
| Spec version           | 0.1                     |
| Implementation version | 0.1.0                   |
| Tests passing          | <!-- Yes/No/Partial --> |
| Schema validated       | <!-- Yes/No -->         |
| Provenance complete    | <!-- Yes/No -->         |

## Spec Compliance

### Requirements Traceability

| Requirement    | Spec Section | Implementation | Test     | Evidence    | Status                     |
| -------------- | ------------ | -------------- | -------- | ----------- | -------------------------- |
| <!-- Req 1 --> | §4.1         | `function()`   | `test_*` | [CI Run](#) | <!-- Pass/Fail/Partial --> |

### Deviations

<!-- Document any intentional deviations from spec -->

| Deviation                  | Rationale    | Tracking            |
| -------------------------- | ------------ | ------------------- |
| <!-- None, or describe --> | <!-- Why --> | <!-- Issue link --> |

## Test Results

### Latest CI Run

- **Run ID:** <!-- CI run link -->
- **Date:** <!-- Date -->
- **Platforms:** Linux, macOS, Windows

### Coverage

| Metric          | Value      | Target |
| --------------- | ---------- | ------ |
| Line coverage   | <!-- % --> | 80%    |
| Branch coverage | <!-- % --> | 70%    |

### Test Summary

| Category    | Total      | Passed     | Failed | Skipped |
| ----------- | ---------- | ---------- | ------ | ------- |
| Unit        | <!-- N --> | <!-- N --> | 0      | 0       |
| Integration | <!-- N --> | <!-- N --> | 0      | 0       |
| Equivalence | <!-- N --> | <!-- N --> | 0      | 0       |

## Schema Compliance

### Schema Validation

| Output               | Schema ID                                                                    | Valid           |
| -------------------- | ---------------------------------------------------------------------------- | --------------- |
| <!-- Output type --> | `https://schemas.3leaps.dev/sysprims/{{module}}/v1.0.0/{{type}}.schema.json` | <!-- Yes/No --> |

### Schema Version

- Schema version: 1.0.0
- Embedded in output: Yes

## Platform Compliance

### Feature Matrix

| Feature          | Linux           | macOS           | Windows         | Notes          |
| ---------------- | --------------- | --------------- | --------------- | -------------- |
| <!-- Feature --> | <!-- Status --> | <!-- Status --> | <!-- Status --> | <!-- Notes --> |

### Known Limitations

| Platform          | Limitation          | Documented |
| ----------------- | ------------------- | ---------- |
| <!-- Platform --> | <!-- Limitation --> | Yes        |

## Provenance

- Provenance document: [`{{module}}-provenance.md`](./{{module}}-provenance.md)
- All sources documented: <!-- Yes/No -->
- GPL sources avoided: <!-- Yes/N/A -->

## Evidence Artifacts

| Artifact          | Location     | Purpose            |
| ----------------- | ------------ | ------------------ |
| CI test run       | <!-- URL --> | Test pass evidence |
| Coverage report   | <!-- URL --> | Coverage evidence  |
| Schema validation | <!-- URL --> | Schema compliance  |

## Sign-off

| Role      | Name          | Date          | Status                    |
| --------- | ------------- | ------------- | ------------------------- |
| Developer | <!-- Name --> | <!-- Date --> | <!-- Approved/Pending --> |
| Reviewer  | <!-- Name --> | <!-- Date --> | <!-- Approved/Pending --> |

---

_Compliance report version: 0.1_
_Last updated: {{DATE}}_
