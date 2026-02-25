---
title: "sysprims-timeout Compliance Report"
module: "sysprims-timeout"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-timeout Compliance Report

## Summary

| Item                   | Status |
| ---------------------- | ------ |
| Spec version           | 1.0    |
| Implementation version | 0.1.0  |
| Tests passing          | Yes    |
| ADR-0003 compliance    | Yes    |
| Provenance complete    | Yes    |

## Spec Compliance

### Requirements Traceability

| Requirement                   | Spec Section | Implementation             | Test                                      | Status |
| ----------------------------- | ------------ | -------------------------- | ----------------------------------------- | ------ |
| Default SIGTERM               | §4.1         | `TimeoutConfig::default()` | `default_config_uses_sigterm`             | Pass   |
| Default GroupByDefault        | §4.1         | `TimeoutConfig::default()` | `default_config_uses_group_by_default`    | Pass   |
| Default 10s kill_after        | §4.1         | `TimeoutConfig::default()` | `default_config_kill_after_is_10_seconds` | Pass   |
| Default preserve_status false | §4.1         | `TimeoutConfig::default()` | `default_config_does_not_preserve_status` | Pass   |
| Exit 124 on timeout           | §5           | CLI implementation         | integration                               | Pass   |
| Exit 127 on not-found         | §5           | `NotFound` error           | integration                               | Pass   |
| Exit 126 on not-executable    | §5           | `PermissionDenied` error   | integration                               | Pass   |
| Group-by-default tree kill    | §4.4         | Unix/Windows impl          | tree-escape                               | Pass   |
| Observable fallback           | §4.4         | `TreeKillReliability`      | JSON output                               | Pass   |

### ADR-0003 Compliance (Group-by-Default)

| Check                               | Status | Implementation                    |
| ----------------------------------- | ------ | --------------------------------- |
| Unix: setpgid(0, 0) in pre_exec     | Pass   | `unix.rs`                         |
| Unix: killpg(-pgid, sig) on timeout | Pass   | `unix.rs`                         |
| Windows: Job Object created         | Pass   | `windows.rs`                      |
| Windows: KILL_ON_JOB_CLOSE set      | Pass   | `windows.rs`                      |
| Fallback detection                  | Pass   | `TreeKillReliability::BestEffort` |
| Fallback observable in output       | Pass   | JSON includes reliability field   |

### Deviations

None. Implementation matches spec.

## Test Results

### Test Summary

| Category        | Tests  | Status |
| --------------- | ------ | ------ |
| Config defaults | 4      | Pass   |
| Integration     | See CI | Pass   |

**Key tests:**

- `default_config_uses_sigterm` - Default signal is SIGTERM
- `default_config_uses_group_by_default` - Default grouping mode
- `default_config_kill_after_is_10_seconds` - Default escalation delay
- `default_config_does_not_preserve_status` - Default preserve_status

## Platform Compliance

### Feature Matrix

| Feature           | Linux   | macOS   | Windows                            |
| ----------------- | ------- | ------- | ---------------------------------- |
| Process grouping  | setpgid | setpgid | Job Object                         |
| Tree kill         | killpg  | killpg  | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE |
| SIGTERM           | Native  | Native  | TerminateProcess                   |
| SIGKILL           | Native  | Native  | TerminateProcess                   |
| Signal escalation | Yes     | Yes     | Yes                                |

### Known Limitations

| Platform | Limitation              | Documented                       |
| -------- | ----------------------- | -------------------------------- |
| All      | setsid can escape group | Yes (returns BestEffort)         |
| Windows  | No POSIX signals        | Yes (mapped to TerminateProcess) |

## Provenance

- Provenance document: [`timeout-provenance.md`](./timeout-provenance.md)
- All sources documented: Yes
- Implementation derived from POSIX setpgid/killpg and Windows Job Object APIs

## Evidence Artifacts

| Artifact       | Location                         | Purpose             |
| -------------- | -------------------------------- | ------------------- |
| Test run       | `cargo test -p sysprims-timeout` | Test pass evidence  |
| Implementation | `crates/sysprims-timeout/src/`   | Source verification |

## Sign-off

| Role      | Name          | Date       | Status   |
| --------- | ------------- | ---------- | -------- |
| Developer | sysprims team | 2026-01-09 | Complete |
| Reviewer  | -             | -          | Pending  |

---

_Compliance report version: 1.0_
_Last updated: 2026-01-09_
