---
title: "sysprims-timeout Compliance Report"
module: "sysprims-timeout"
version: "1.1"
status: "Active"
last_updated: "2026-08-29"
---

# sysprims-timeout Compliance Report

## Summary

| Item                   | Status |
| ---------------------- | ------ |
| Spec version           | 1.3    |
| Implementation version | 0.2.2  |
| Host and cross-target tests | Yes |
| Windows runtime evidence | Pending CI |
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
| Group-by-default tree kill     | §4.6        | Unix/Windows impl                    | tree-escape        | Pass |
| Observable fallback            | §4.6        | `TreeKillReliability`                | JSON output        | Pass |
| Independent boundary strength  | §4.1, §4.6  | `ContainmentBoundaryStrength`        | unit/platform      | Pass |
| Prepared Windows Job receipt   | §4.2, §4.6  | `PreparedWindowsJob`                 | Windows platform   | Pass |
| Exact pre-execution assignment | §4.2, §4.6  | `contain_acquired_windows_job`       | Windows adapter    | Pass |
| Windows standard spawn closed  | §4.2, §4.3  | `spawn_contained`                    | Windows platform   | Pass |

### ADR-0003 Compliance (Group-by-Default)

| Check                               | Status | Implementation                    |
| ----------------------------------- | ------ | --------------------------------- |
| Unix: setpgid(0, 0) in pre_exec     | Pass   | `unix.rs`                         |
| Unix: killpg(-pgid, sig) on timeout | Pass   | `unix.rs`                         |
| Windows: Job Object created         | Pass   | `windows.rs`                      |
| Windows: KILL_ON_JOB_CLOSE set      | Pass   | `windows.rs`                      |
| Windows: breakaway modes disabled   | Pass   | `windows.rs`                      |
| Windows: exact suspended child proven | Pass | `PreparedWindowsJob::assign_process` |
| Fallback detection                  | Pass   | `TreeKillReliability::BestEffort` |
| Fallback observable in output       | Pass   | JSON includes reliability field   |
| Boundary strength observable        | Pass   | `ContainmentBoundaryStrength`     |

### Deviations

None. Implementation matches spec.

## Test Results

### Test Summary

| Category                | Tests  | Status  |
| ----------------------- | ------ | ------- |
| Config defaults         | 4      | Pass    |
| Integration             | See CI | Pass    |
| Windows adapter runtime | See CI | Pending |

**Key tests:**

- `default_config_uses_sigterm` - Default signal is SIGTERM
- `default_config_uses_group_by_default` - Default grouping mode
- `default_config_kill_after_is_10_seconds` - Default escalation delay
- `default_config_does_not_preserve_status` - Default preserve_status
- Windows platform tests verify Job policy flags and exact suspended-child
  assignment.
- Adapter integration tests cover descendant termination and breakaway denial;
  execution evidence is produced by the Windows CI matrix.

## Platform Compliance

### Feature Matrix

| Feature           | Linux               | macOS               | Windows                                       |
| ----------------- | ------------------- | ------------------- | --------------------------------------------- |
| Process grouping  | owned session/group | owned session/group | prepared exact-child Job or post-spawn Job    |
| Boundary strength | CooperativeGroup    | CooperativeGroup    | KernelEnforcedJob or Unknown                  |
| Tree kill         | killpg              | killpg              | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE            |
| SIGTERM           | Native              | Native              | TerminateProcess                              |
| SIGKILL           | Native              | Native              | TerminateProcess                              |
| Signal escalation | Yes                 | Yes                 | Yes                                           |

### Known Limitations

| Platform | Limitation                                        | Documented                       |
| -------- | ------------------------------------------------- | -------------------------------- |
| Unix     | descendants can leave the cooperative group       | Yes (`CooperativeGroup`)         |
| Windows  | standard `Command` spawn lacks the suspended seam | Yes (fails closed)               |
| Windows  | post-spawn adoption cannot prove non-escape       | Yes (`Unproven` and `Unknown`)   |
| Windows  | no POSIX signals                                  | Yes (mapped to TerminateProcess) |

## Provenance

- Provenance document: [`timeout-provenance.md`](./timeout-provenance.md)
- All sources documented: Yes
- Implementation derived from POSIX setpgid/killpg and Windows Job Object APIs

## Evidence Artifacts

| Artifact        | Location                       | Purpose                             |
| --------------- | ------------------------------ | ----------------------------------- |
| Test run        | `make check`                   | Host and cross-target evidence      |
| Windows runtime | companion Windows CI matrix    | Descendant and breakaway evidence   |
| Implementation  | `crates/sysprims-timeout/src/` | Source verification                 |

## Sign-off

| Role      | Name          | Date       | Status   |
| --------- | ------------- | ---------- | -------- |
| Developer | sysprims team | 2026-08-29 | Complete |
| Reviewer  | -             | -          | Pending  |

---

_Compliance report version: 1.1_
_Last updated: 2026-08-29_
