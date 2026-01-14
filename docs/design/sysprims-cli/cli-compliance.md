---
title: "sysprims-cli Compliance Report"
module: "sysprims-cli"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-cli Compliance Report

## Summary

| Item | Status |
|------|--------|
| Spec version | 1.0 |
| Implementation version | 0.1.0 |
| Tests passing | Yes |
| Provenance complete | Yes |

## Spec Compliance

### Requirements Traceability

| Requirement | Spec Section | Implementation | Test | Status |
|-------------|--------------|----------------|------|--------|
| kill subcommand | §4.1 | `KillArgs`, `run_kill()` | integration | Pass |
| timeout subcommand | §4.2 | `TimeoutArgs`, `run_timeout()` | integration | Pass |
| pstat subcommand | §4.3 | `PstatArgs`, `run_pstat()` | integration | Pass |
| Duration parsing | §5 | `parse_duration()` | unit | Pass |
| Exit code 124 | §6 | main.rs exit handling | integration | Pass |
| JSON schema_id | §7 | Via library crates | integration | Pass |

### Deviations

None. Implementation matches spec.

## Test Results

### Test Summary

CLI tests are primarily integration tests that invoke the binary and verify behavior.

| Category | Tests | Status |
|----------|-------|--------|
| Argument parsing | Integration | Pass |
| Exit codes | Integration | Pass |
| Output formats | Integration | Pass |

## Platform Compliance

### Feature Matrix

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| kill | Full | Full | Partial (TERM/KILL only) |
| timeout | Full | Full | Full |
| pstat | Full | Full | Full |

### Known Limitations

| Platform | Limitation | Documented |
|----------|------------|------------|
| Windows | kill only supports TERM/KILL | Yes (README) |

## Provenance

- Provenance document: [`cli-provenance.md`](./cli-provenance.md)
- All sources documented: Yes
- Implementation is original (thin wrapper)

## Evidence Artifacts

| Artifact | Location | Purpose |
|----------|----------|---------|
| Binary | `target/release/sysprims` | Built artifact |
| Implementation | `crates/sysprims-cli/src/main.rs` | Source |

## Sign-off

| Role | Name | Date | Status |
|------|------|------|--------|
| Developer | sysprims team | 2026-01-09 | Complete |
| Reviewer | - | - | Pending |

---

*Compliance report version: 1.0*
*Last updated: 2026-01-09*
