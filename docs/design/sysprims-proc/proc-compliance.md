---
title: "sysprims-proc Compliance Report"
module: "sysprims-proc"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-proc Compliance Report

## Summary

| Item | Status |
|------|--------|
| Spec version | 1.0 |
| Implementation version | 0.1.0 |
| Tests passing | Yes (platform-specific counts; see CI logs) |
| Schema validated | Yes |
| Provenance complete | Yes |

## Spec Compliance

### Requirements Traceability

| Requirement | Spec Section | Implementation | Test | Status |
|-------------|--------------|----------------|------|--------|
| Required fields pid+name | §4.4 | `ProcessInfo` | `test_get_self` | Pass |
| Strict filter schema | §4.4 | `ProcessFilter` with `deny_unknown_fields` | `test_filter_unknown_field_rejected` | Pass |
| Unknown filter keys rejected | §4.4 | serde `deny_unknown_fields` | `test_filter_unknown_field_rejected` | Pass |
| cpu% normalized 0-100 | §4.4 | `cpu_percent: f64` | `test_cpu_normalized` | Pass |
| cpu_above range validation | §4.4 | `ProcessFilter::validate()` | `test_filter_validation_cpu_range` | Pass |
| schema_id embedded | §4.4 | `PROCESS_INFO_V1` constant | `test_snapshot_has_schema_id` | Pass |
| No fake data | §4.4 | Optional fields use `Option<T>` | `test_get_self_has_valid_fields` | Pass |
| PID 0 rejected | §4.4 | `get_process()` validation | `test_invalid_pid_zero` | Pass |

### Deviations

None. Implementation matches spec.

## Test Results

### Coverage

| Metric | Value | Target |
|--------|-------|--------|
| Line coverage | ~85% | 80% |
| Tests passing | See CI logs | 100% |

### Test Summary

| Category | Total | Passed | Failed | Skipped |
|----------|-------|--------|--------|---------|
| Unit + Platform | See CI logs | - | - | - |
| Integration | - | - | - | - |
| Equivalence | - | - | - | - |

**Key tests:**
- `test_snapshot_not_empty` - Verifies process enumeration works
- `test_snapshot_has_schema_id` - Verifies ADR-0005 compliance
- `test_filter_unknown_field_rejected` - Verifies strict validation
- `test_cpu_normalized` - Verifies CPU normalization invariant
- `test_invalid_pid_zero` - Verifies PID validation

## Schema Compliance

| Output | Schema ID | Valid |
|--------|-----------|-------|
| ProcessSnapshot | `https://schemas.3leaps.dev/sysprims/process/v1.0.0/process-info.schema.json` | Yes |

Schema ID is embedded in all JSON output via the `schema_id` field.

## Platform Compliance

### Feature Matrix

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| Process enumeration | /proc | libproc | Toolhelp32 |
| Process info | /proc/[pid]/* | proc_pidinfo | OpenProcess |
| CPU usage | /proc/[pid]/stat | proc_pidinfo | GetProcessTimes |
| Memory usage | /proc/[pid]/statm | proc_pidinfo | GetProcessMemoryInfo |
| User lookup | /proc/[pid]/status | proc_pidinfo | Token queries |

### Known Limitations

| Platform | Limitation | Documented |
|----------|------------|------------|
| macOS | SIP limits visibility to user processes | Yes (spec §7) |
| Windows | User field may be unavailable | Yes (spec §4.4) |

## Provenance

- Provenance document: [`proc-provenance.md`](./proc-provenance.md)
- All sources documented: Yes
- Implementation derived from platform APIs and POSIX specification

## Evidence Artifacts

| Artifact | Location | Purpose |
|----------|----------|---------|
| Test run | `cargo test -p sysprims-proc` | Test pass evidence |
| Implementation | `crates/sysprims-proc/src/lib.rs` | Source verification |

## Sign-off

| Role | Name | Date | Status |
|------|------|------|--------|
| Developer | sysprims team | 2026-01-09 | Complete |
| Reviewer | - | - | Pending |

---

*Compliance report version: 1.0*
*Last updated: 2026-01-09*
