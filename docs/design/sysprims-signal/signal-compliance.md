---
title: "sysprims-signal Compliance Report"
module: "sysprims-signal"
version: "1.0"
status: "Active"
last_updated: "2026-01-09"
---

# sysprims-signal Compliance Report

## Summary

| Item | Status |
|------|--------|
| Spec version | 1.0 |
| Implementation version | 0.1.0 |
| Tests passing | Yes |
| ADR-0011 compliance | Yes |
| Provenance complete | Yes |

## Spec Compliance

### Requirements Traceability

| Requirement | Spec Section | Implementation | Test | Status |
|-------------|--------------|----------------|------|--------|
| Parse TERM/SIGTERM/term | §4.2 | `kill_by_name`, `resolve_signal_number` | `resolve_signal_number_*` | Pass |
| PID 0 rejected | §4.5 | `validate_pid` | `kill_rejects_pid_zero` | Pass |
| PID > MAX_SAFE_PID rejected | §4.5 | `validate_pid` | `kill_rejects_pid_exceeding_*` | Pass |
| MAX_SAFE_PID boundary | §4.1 | `MAX_SAFE_PID = i32::MAX` | `max_safe_pid_is_i32_max` | Pass |
| NotFound for missing PID | §4.4 | Unix/Windows impl | integration | Pass |
| PermissionDenied | §4.4 | Unix/Windows impl | integration | Pass |
| Windows killpg NotSupported | §4.2 | `killpg` returns NotSupported | `killpg_is_not_supported_*` | Pass |
| rsfulmen constants | §4.3 | `pub use rsfulmen::*` | `rsfulmen_constants_available` | Pass |

### ADR-0011 Compliance (Safety-Critical)

| Check | Status | Test |
|-------|--------|------|
| PID 0 rejected before syscall | Pass | `kill_rejects_pid_zero` |
| u32::MAX rejected (would be -1) | Pass | `kill_rejects_pid_exceeding_max_safe` |
| i32::MAX + 1 rejected | Pass | `kill_rejects_pid_at_boundary` |
| i32::MAX accepted (last safe value) | Pass | `kill_accepts_pid_at_max_safe` |
| Error message explains danger | Pass | Message includes overflow warning |

### Deviations

None. Implementation matches spec.

## Test Results

### Test Summary

| Category | Tests | Status |
|----------|-------|--------|
| PID validation (ADR-0011) | 7 | Pass |
| Signal resolution | 3 | Pass |
| rsfulmen integration | 1 | Pass |
| Platform-specific | 1 | Pass |

**Key safety tests:**
- `kill_rejects_pid_zero` - Prevents signaling caller's process group
- `kill_rejects_pid_exceeding_max_safe` - Prevents kill(-1, sig) catastrophe
- `kill_rejects_pid_at_boundary` - Boundary condition check
- `kill_accepts_pid_at_max_safe` - Confirms valid PIDs work

## Platform Compliance

### Feature Matrix

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| kill(pid, sig) | Native | Native | TerminateProcess |
| killpg(pgid, sig) | Native | Native | NotSupported |
| SIGTERM | 15 | 15 | TerminateProcess |
| SIGKILL | 9 | 9 | TerminateProcess |
| SIGINT | 2 | 2 | Best-effort |
| SIGHUP | 1 | 1 | NotSupported |

### Known Limitations

| Platform | Limitation | Documented |
|----------|------------|------------|
| Windows | killpg not supported | Yes (spec §4.2) |
| Windows | SIGINT best-effort | Yes (spec §6) |
| Windows | HUP/USR1/USR2 not supported | Yes (spec §6) |

## Provenance

- Provenance document: [`signal-provenance.md`](./signal-provenance.md)
- All sources documented: Yes
- Implementation derived from POSIX specification and platform APIs

## Evidence Artifacts

| Artifact | Location | Purpose |
|----------|----------|---------|
| Test run | `cargo test -p sysprims-signal` | Test pass evidence |
| Implementation | `crates/sysprims-signal/src/lib.rs` | Source verification |
| Safety doc | `docs/safety/signal-dispatch.md` | ADR-0011 rationale |

## Sign-off

| Role | Name | Date | Status |
|------|------|------|--------|
| Developer | sysprims team | 2026-01-09 | Complete |
| Reviewer | - | - | Pending |

---

*Compliance report version: 1.0*
*Last updated: 2026-01-09*
