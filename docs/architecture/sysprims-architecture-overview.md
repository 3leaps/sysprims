# sysprims Architecture Overview

> **Document Status**: Bootstrap  
> **Last Updated**: 2025-12-31  
> **ADR References**: [0002](./adr/0002-crate-structure.md), [0003](./adr/0003-group-by-default.md)

## Executive Summary

sysprims is a cross-platform process utilities library implemented in Rust with first-class bindings for Go, Python, and TypeScript. The architecture prioritizes:

1. **Embeddability** — Library-first design for direct integration
2. **License Cleanliness** — Zero GPL dependencies, strict cargo-deny enforcement
3. **Reliability** — Group-by-default process tree management
4. **Observability** — Schema-driven outputs with observable fallbacks

## System Context

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           External Systems                               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌─────────────┐ │
│  │  CI/CD       │  │  Container   │  │  Enterprise  │  │  Developer  │ │
│  │  Pipelines   │  │  Runtimes    │  │  Apps        │  │  Tools      │ │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬──────┘ │
│         │                 │                 │                 │         │
│         └─────────────────┴────────┬────────┴─────────────────┘         │
│                                    │                                     │
│                                    ▼                                     │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                         Fulmen Ecosystem                         │   │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐       │   │
│  │  │ rsfulmen  │ │ gofulmen  │ │ pyfulmen  │ │ tsfulmen  │       │   │
│  │  └─────┬─────┘ └─────┬─────┘ └─────┬─────┘ └─────┬─────┘       │   │
│  │        │             │             │             │               │   │
│  │        └─────────────┴──────┬──────┴─────────────┘               │   │
│  └─────────────────────────────┼───────────────────────────────────┘   │
│                                │                                         │
│                                ▼                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                            sysprims                                   │   │
│  │                    (This System)                                 │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                │                                         │
│                                ▼                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                     Operating Systems                            │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │   │
│  │  │   Linux     │  │   macOS     │  │   Windows   │              │   │
│  │  │ (glibc/musl)│  │ (x64/arm64) │  │   (x64)     │              │   │
│  │  └─────────────┘  └─────────────┘  └─────────────┘              │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

## Component Architecture

### Core Crates

| Crate | Responsibility | Key Types |
|-------|----------------|-----------|
| `sysprims-core` | Shared types, errors, telemetry trait | `SysprimsError`, `Signal`, `TelemetryEmitter` |
| `sysprims-timeout` | Process execution with deadlines | `TimeoutConfig`, `TimeoutOutcome`, `GroupingMode` |
| `sysprims-signal` | Signal dispatch and process groups | `send_signal()`, `terminate()`, `force_kill()` |
| `sysprims-proc` | Process inspection and enumeration | `ProcessSnapshot`, `ProcessInfo`, `ProcessFilter` |
| `sysprims-cli` | CLI binaries (thin wrappers) | Binary entry points |
| `sysprims-ffi` | C-ABI exports | `sysprims_timeout_run()`, `sysprims_proc_list()`, etc. |

### Component Interaction

```
┌─────────────────────────────────────────────────────────────────┐
│                          sysprims-cli                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │ sysprims-timeout │  │  sysprims-kill   │  │  sysprims-pstat  │             │
│  │    (bin)    │  │    (bin)    │  │    (bin)    │             │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘             │
└─────────┼────────────────┼────────────────┼─────────────────────┘
          │                │                │
          ▼                ▼                ▼
┌─────────────────────────────────────────────────────────────────┐
│                       Library Crates                             │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │   sysprims-timeout   │  │   sysprims-signal    │  │    sysprims-proc     │ │
│  │                 │  │                 │  │                 │ │
│  │ • TimeoutConfig │  │ • send_signal() │  │ • snapshot()    │ │
│  │ • run_with_     │  │ • terminate()   │  │ • get_process() │ │
│  │   timeout()     │  │ • force_kill()  │  │ • ProcessFilter │ │
│  │ • GroupingMode  │  │ • signal_group()│  │                 │ │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘ │
│           │                    │                    │           │
│           └────────────────────┼────────────────────┘           │
│                                ▼                                 │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                        sysprims-core                              ││
│  │  • SysprimsError (thiserror)    • Signal enum                    ││
│  │  • TelemetryEmitter trait  • Platform abstractions          ││
│  │  • GroupingMode enum       • JSON schema types              ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Platform Abstraction                        │
│  ┌─────────────────────────────┐  ┌────────────────────────────┐│
│  │      Unix (libc)            │  │    Windows (windows-sys)   ││
│  │  • setpgid / killpg         │  │  • Job Objects             ││
│  │  • /proc filesystem         │  │  • Toolhelp32 snapshots    ││
│  │  • POSIX signals            │  │  • TerminateProcess        ││
│  └─────────────────────────────┘  └────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

## Key Mechanisms

### 1. Group-by-Default Process Control

The core reliability differentiator. See [ADR-0003](./adr/0003-group-by-default.md) for full details.

**Unix Flow**:
```
spawn() ──► setpgid(0, 0) ──► child becomes group leader
                │
                ▼
timeout ──► killpg(pgid, SIGTERM) ──► wait ──► killpg(pgid, SIGKILL)
```

**Windows Flow**:
```
CreateJobObject() ──► SetInformationJobObject(KILL_ON_CLOSE)
        │
        ▼
AssignProcessToJobObject() ──► all descendants tracked
        │
        ▼
timeout ──► CloseHandle(job) ──► all processes terminated
```

**Fallback Observable**:
```json
{
  "grouping_requested": "group_by_default",
  "grouping_effective": "group_by_default",
  "tree_kill_reliability": "best_effort"  // ← Job Object failed
}
```

### 2. FFI Architecture

The FFI layer uses the **Opaque Pointer + JSON String** pattern for maximum cross-language compatibility.

```
┌─────────────────────────────────────────────────────────────────┐
│                        Language Bindings                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
│  │   sysprims-go     │  │   sysprims-py     │  │   sysprims-ts     │          │
│  │   (CGo)      │  │   (PyO3)     │  │  (NAPI-RS)   │          │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘          │
│         │                 │                 │                    │
│         └─────────────────┼─────────────────┘                    │
│                           │                                      │
│                           ▼                                      │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                      C-ABI (sysprims.h)                           ││
│  │                   Generated via cbindgen                     ││
│  │                                                              ││
│  │  Memory Contract:                                            ││
│  │  • All returned char* owned by caller                        ││
│  │  • MUST free via sysprims_free_string()                          ││
│  │  • NEVER use C free() (Rust allocator)                       ││
│  │                                                              ││
│  │  Data Contract:                                              ││
│  │  • All strings UTF-8                                         ││
│  │  • Complex data as JSON strings                              ││
│  │  • Schema ID embedded in all outputs                         ││
│  └─────────────────────────────────────────────────────────────┘│
│                           │                                      │
│                           ▼                                      │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                       sysprims-ffi                                ││
│  │                   (Rust implementation)                      ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

### 3. Schema-Driven Contracts

All JSON outputs embed schema identifiers for runtime version detection and migration safety.

```
┌─────────────────────────────────────────────────────────────────┐
│                schemas.3leaps.dev (sysprims SSOT)              │
│  /sysprims/                                                     │
│    ├── timeout/v1.0.0/timeout-result.schema.json                │
│    └── process/v1.0.0/{process-info,process-filter}.schema.json │
└────────────────────────────────┬────────────────────────────────┘
                                 │
                                 │ CI validates outputs
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                         sysprims outputs                              │
│                                                                  │
│  {                                                              │
│    "schema_id": "https://schemas.3leaps.dev/sysprims/...",         │
│    "timestamp": "2025-12-31T12:00:00Z",                        │
│    "processes": [...]                                           │
│  }                                                              │
└─────────────────────────────────────────────────────────────────┘
```

## Data Flow

### Timeout Operation

```
User Request                Processing                     Output
─────────────────────────────────────────────────────────────────────

sysprims-timeout 30s             ┌──────────────┐
    --json                  │              │
    -- ./build.sh  ──────►  │ Parse args   │
                            │              │
                            └──────┬───────┘
                                   │
                                   ▼
                            ┌──────────────┐
                            │ Create       │
                            │ process      │◄──── Unix: setpgid()
                            │ group/job    │◄──── Windows: Job Object
                            └──────┬───────┘
                                   │
                                   ▼
                            ┌──────────────┐
                            │ Spawn child  │
                            │ in group     │
                            └──────┬───────┘
                                   │
                    ┌──────────────┼──────────────┐
                    │              │              │
                    ▼              ▼              ▼
              ┌──────────┐  ┌──────────┐  ┌──────────┐
              │ Child    │  │ Timeout  │  │ Child    │
              │ exits    │  │ fires    │  │ spawn    │
              │ normally │  │          │  │ fails    │
              └────┬─────┘  └────┬─────┘  └────┬─────┘
                   │             │             │
                   │             ▼             │
                   │      ┌──────────────┐     │
                   │      │ Signal group │     │
                   │      │ (TERM→KILL)  │     │
                   │      └──────┬───────┘     │
                   │             │             │
                   └──────────────┼─────────────┘
                                  │
                                  ▼
                           ┌──────────────┐      {
                           │ Build JSON   │        "schema_id": "...",
                           │ result       │──────► "status": "timeout",
                           └──────────────┘        "duration_ms": 30000,
                                                   "tree_kill_reliability":
                                                     "guaranteed"
                                                 }
```

### Process Enumeration

```
sysprims-pstat --json            ┌──────────────┐
    --filter                │              │
    name=nginx   ──────────►│ Parse filter │
                            │              │
                            └──────┬───────┘
                                   │
                   ┌───────────────┼───────────────┐
                   │               │               │
                   ▼               ▼               ▼
            ┌──────────┐   ┌──────────┐   ┌──────────┐
            │ Linux    │   │ macOS    │   │ Windows  │
            │ /proc    │   │ libproc  │   │ Toolhelp │
            └────┬─────┘   └────┬─────┘   └────┬─────┘
                 │              │              │
                 └──────────────┼──────────────┘
                                │
                                ▼
                         ┌──────────────┐
                         │ Apply filter │
                         │ (validated)  │
                         └──────┬───────┘
                                │
                                ▼
                         ┌──────────────┐      {
                         │ Build JSON   │        "schema_id": "...",
                         │ snapshot     │──────► "timestamp": "...",
                         └──────────────┘        "processes": [...]
                                               }
```

## Deployment Topology

### Static Binary Distribution

```
┌─────────────────────────────────────────────────────────────────┐
│                      Build Matrix (CI)                          │
│                                                                  │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │ Linux musl      │  │ Linux musl      │  │ Linux glibc     │ │
│  │ x86_64          │  │ aarch64         │  │ x86_64          │ │
│  │ (PRIMARY)       │  │                 │  │ (enterprise)    │ │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘ │
│           │                    │                    │           │
│  ┌────────┴────────┐  ┌────────┴────────┐  ┌────────┴────────┐ │
│  │ macOS x86_64    │  │ macOS aarch64   │  │ Windows x86_64  │ │
│  │                 │  │ (Apple Silicon) │  │ (MSVC)          │ │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘ │
│           │                    │                    │           │
└───────────┼────────────────────┼────────────────────┼───────────┘
            │                    │                    │
            ▼                    ▼                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Release Artifacts                          │
│                                                                  │
│  sysprims-${VERSION}-linux-x64-musl.tar.gz     (Distroless-ready)   │
│  sysprims-${VERSION}-linux-arm64-musl.tar.gz                        │
│  sysprims-${VERSION}-linux-x64-gnu.tar.gz                           │
│  (no macOS x64 artifacts)                                           │
│  sysprims-${VERSION}-darwin-arm64.tar.gz                            │
│  sysprims-${VERSION}-windows-x64.zip                                │
│  sbom-${VERSION}.spdx.json                                     │
│  THIRD_PARTY_NOTICES.md                                        │
└─────────────────────────────────────────────────────────────────┘
```

### Library Integration

```
┌─────────────────────────────────────────────────────────────────┐
│                       Rust Consumers                            │
│                                                                  │
│  [dependencies]                                                 │
│  sysprims-timeout = "0.1"     ◄──── Direct crate dependency         │
│  sysprims-proc = { version = "0.1", features = ["tracing"] }        │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                        Go Consumers                             │
│                                                                  │
│  import "github.com/3leaps/sysprims-go"                              │
│                        │                                         │
│                        ▼                                         │
│  Linked: libsysprims.a (static) or libsysprims.so (dynamic)              │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                      Python Consumers                           │
│                                                                  │
│  pip install sysprims        ◄──── PyPI wheel with bundled .so      │
│                                                                  │
│  from sysprims import timeout, proc, signal                         │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                    TypeScript Consumers                         │
│                                                                  │
│  npm install @3leaps/sysprims  ◄──── npm package with native addon  │
│                                                                  │
│  import { timeout, proc, signal } from '@3leaps/sysprims';          │
└─────────────────────────────────────────────────────────────────┘
```

## Quality Attributes

### Performance

| Operation | Target | Measurement |
|-----------|--------|-------------|
| Timeout spawn overhead | < 5ms | vs. raw `Command::spawn` |
| Process enumeration (1000 procs) | < 50ms | End-to-end snapshot |
| Signal delivery | < 1ms | PID to signal delivery |
| Memory footprint (CLI) | < 10MB | RSS at steady state |

### Reliability

| Metric | Target | Enforcement |
|--------|--------|-------------|
| Tree kill success rate | 100% (guaranteed mode) | Integration tests |
| Signal delivery success | 100% (valid PID) | Unit tests |
| JSON schema compliance | 100% | Golden tests |

### Security

| Concern | Mitigation |
|---------|------------|
| Privilege escalation | No implicit elevation; explicit errors |
| PID injection | Validate all external PID inputs |
| Memory safety | Rust ownership; careful FFI contracts |

### Maintainability

| Metric | Target |
|--------|--------|
| Test coverage | ≥ 80% |
| Documentation coverage | 100% public API |
| MSRV stability | 1.81.0 (policy: announce bumps) |

## Cross-References

- [ADR-0001: License Policy](./adr/0001-license-policy.md)
- [ADR-0002: Crate Structure](./adr/0002-crate-structure.md)
- [ADR-0003: Group-by-Default](./adr/0003-group-by-default.md)
- [ADR-0004: FFI Design](./adr/0004-ffi-design.md)
- [ADR-0005: Schema Contracts](./adr/0005-schema-contracts.md)
- [Stack Management](./stack/README.md)
- [Security Architecture](./security/README.md)
