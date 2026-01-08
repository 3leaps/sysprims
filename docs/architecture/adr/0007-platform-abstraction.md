# ADR-0007: Platform Abstraction Strategy

> **Status**: Accepted  
> **Date**: 2025-12-31  
> **Authors**: Architecture Council

## Context

sysprims must support Linux, macOS, and Windows with consistent behavior where possible, while acknowledging platform differences when necessary.

Key challenges:
1. Signal semantics differ significantly (Unix vs Windows)
2. Process enumeration uses different APIs per platform
3. Process grouping mechanisms vary (Unix process groups vs Windows Job Objects)
4. Some features are platform-specific (e.g., Unix signals HUP, USR1)

We need a strategy that:
- Maximizes code reuse
- Makes platform differences explicit
- Doesn't hide important behavioral differences

## Decision

### Abstraction Layers

We use a **thin abstraction** strategy with platform-specific modules:

```
sysprims-core/
├── src/
│   ├── lib.rs           # Public API
│   ├── error.rs         # Unified error types
│   ├── signal.rs        # Signal enum (cross-platform)
│   └── platform/
│       ├── mod.rs       # Platform selection
│       ├── unix.rs      # Unix-specific implementation
│       └── windows.rs   # Windows-specific implementation
```

### Platform Selection

Compile-time selection via `cfg`:

```rust
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
```

### Explicit Incompatibility

When a feature isn't available on a platform, return `NotSupported`:

```rust
// Unix-only feature
#[cfg(unix)]
pub fn send_signal_group(pgid: u32, signal: Signal) -> Result<(), SysprimsError> {
    // Implementation
}

#[cfg(windows)]
pub fn send_signal_group(_pgid: u32, _signal: Signal) -> Result<(), SysprimsError> {
    Err(SysprimsError::NotSupported {
        feature: "process group signals",
        platform: "windows",
    })
}
```

### Signal Mapping

Signals are mapped, not abstracted away:

```rust
pub enum Signal {
    Term,       // SIGTERM (Unix) / TerminateProcess (Windows)
    Kill,       // SIGKILL (Unix) / TerminateProcess (Windows)
    Int,        // SIGINT (Unix) / GenerateConsoleCtrlEvent (Windows, best-effort)
    Hup,        // SIGHUP (Unix only)
    Usr1,       // SIGUSR1 (Unix only)
    Usr2,       // SIGUSR2 (Unix only)
    Custom(i32), // Platform-specific signal number
}

impl Signal {
    pub fn is_supported(&self) -> bool {
        #[cfg(windows)]
        match self {
            Signal::Term | Signal::Kill => true,
            Signal::Int => true,  // Best-effort
            _ => false,
        }
        #[cfg(unix)]
        true
    }
}
```

### Process Enumeration

Each platform has its own enumeration strategy:

```rust
// sysprims-proc/src/platform/linux.rs
pub fn enumerate_processes() -> Result<Vec<RawProcessInfo>, SysprimsError> {
    // Read from /proc
}

// sysprims-proc/src/platform/macos.rs
pub fn enumerate_processes() -> Result<Vec<RawProcessInfo>, SysprimsError> {
    // Use libproc
}

// sysprims-proc/src/platform/windows.rs
pub fn enumerate_processes() -> Result<Vec<RawProcessInfo>, SysprimsError> {
    // Use Toolhelp32
}
```

### Observable Differences

Platform differences are observable in output:

```json
{
  "schema_id": "...",
  "platform": "windows",
  "processes": [...],
  "warnings": ["Some process details unavailable without elevation"]
}
```

## Consequences

### Positive

- Clear where platform differences exist
- No "lowest common denominator" limitations
- Easy to add platform-specific features
- Compile-time errors for unsupported calls

### Negative

- More code per platform
- Must test on all platforms
- Some features unavailable on some platforms

### Neutral

- Documentation must cover platform differences
- CI must run on all platforms
- Users must handle `NotSupported` errors

## Alternatives Considered

### Alternative 1: Full Abstraction

Hide all platform differences behind unified API.

**Rejected**: 
- Some features can't be abstracted (Unix signals)
- Hides important behavioral differences
- Leads to surprises when deploying cross-platform

### Alternative 2: Unix-First with Emulation

Target Unix, emulate on Windows.

**Rejected**:
- Windows emulation is imperfect
- Hides Windows-native solutions (Job Objects)
- Poor Windows performance/reliability

### Alternative 3: Separate Crates per Platform

`sysprims-linux`, `sysprims-macos`, `sysprims-windows`.

**Rejected**:
- Fragmented API surface
- Users must handle platform selection
- Duplicated shared code

## References

- [Rust Platform-Specific Dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#platform-specific-dependencies)
- [cfg attribute](https://doc.rust-lang.org/reference/conditional-compilation.html)
