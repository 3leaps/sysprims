# sysprims

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust: 1.81+](https://img.shields.io/badge/rust-1.81%2B-orange.svg)](https://www.rust-lang.org/)

**Reliable process control without license toxicity.**

sysprims provides GPL-free, cross-platform process utilities that can be statically or dynamically linked into your applications. When you need process control primitives but can't accept copyleft obligations, sysprims offers a straightforward solution.

**Lifecycle Phase**: `alpha` | See [RELEASE_NOTES.md](RELEASE_NOTES.md) for current version

## The Problem

You're building software that needs to spawn processes with timeouts, send signals, or inspect running processes. Your options:

1. **Shell out to GNU coreutils** — GPL licensed, can't statically link without license concerns
2. **Reimplement from scratch** — Time-consuming, platform-specific edge cases everywhere
3. **Accept incomplete behavior** — GNU `timeout` doesn't reliably kill process trees; orphaned children leak

## What sysprims Offers

- **GPL-free**: MIT/Apache-2.0 dual licensed. Link statically or dynamically without copyleft concerns.
- **Group-by-default**: When you timeout a process, the entire tree dies. No orphans, no leaked resources.
- **Cross-platform**: Linux (musl + glibc), macOS (x64 + arm64), Windows (x64) from a single API.
- **Library-first**: Embed directly in Rust, Go, Python, or TypeScript. CLIs are thin wrappers.

### Group-by-Default: The Core Difference

The fundamental reliability improvement over GNU alternatives isn't just license cleanliness—it's *correct behavior*.

**The problem with typical process spawning:**
```
Parent spawns Child
Child spawns Grandchildren
Parent times out, kills Child
Grandchildren continue running as orphans
→ CI jobs hang "after timing out"
→ Zombie processes in containers
→ Resource leaks in long-running services
```

**sysprims behavior:**
```
Parent spawns Child in new process group (Unix) / Job Object (Windows)
Child spawns Grandchildren (automatically in same group/job)
Parent times out, signals entire group/job
→ All processes terminate together
→ No orphans, no leaks
```

On Unix, this uses `setpgid`/`killpg`. On Windows, Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. When Job Object creation fails (nested jobs, privilege limits), sysprims proceeds with best-effort termination and exposes the degradation in JSON output so your automation can detect it.

## Who Should Use This

**Platform Engineers**: You need process utilities in your tooling but can't introduce GPL dependencies. sysprims gives you the primitives without the license overhead.

**Library Authors**: You're building something that spawns subprocesses and need reliable cleanup. Depend on sysprims instead of shelling out to `timeout` or reimplementing signal handling.

**Enterprise Teams**: Your legal department has opinions about copyleft licenses in your software supply chain. sysprims is designed for environments where license hygiene matters.

**Large OSS Projects**: You want to avoid license toxicity debates in your contributor community. MIT/Apache-2.0 is unambiguous.

## Quick Start

### As a Rust Library

```toml
[dependencies]
sysprims-timeout = "0.1"
sysprims-signal = "0.1"
sysprims-proc = "0.1"
```

```rust
use sysprims_timeout::{
    run_with_timeout, TimeoutConfig, TimeoutOutcome,
    GroupingMode, TreeKillReliability, SIGTERM,
};
use std::time::Duration;

let config = TimeoutConfig {
    signal: SIGTERM,
    kill_after: Duration::from_secs(10),
    grouping: GroupingMode::GroupByDefault,
    preserve_status: false,
};

match run_with_timeout("./build.sh", &[], Duration::from_secs(30), config)? {
    TimeoutOutcome::Completed { exit_status } => {
        println!("Build finished with status: {:?}", exit_status);
    }
    TimeoutOutcome::TimedOut { signal_sent, escalated, tree_kill_reliability } => {
        println!("Build timed out, sent signal {}, escalated: {}", signal_sent, escalated);
        if tree_kill_reliability == TreeKillReliability::BestEffort {
            eprintln!("Warning: tree kill may be incomplete");
        }
    }
}
```

### As a CLI

```bash
# Run command with 30 second timeout
sysprims timeout 30s -- ./long-build.sh

# Send signal to process
sysprims kill -s TERM 1234

# Run timeout with custom signal and escalation delay
sysprims timeout --signal TERM --kill-after 2s 5s -- ./long-build.sh

# List processes as JSON
sysprims pstat --json

# List processes with filters and table output
sysprims pstat --name nginx --cpu-above 5 --table
```

### Exit Codes

| Condition | Exit Code |
|-----------|-----------|
| Command completed (default) | 0 |
| Command completed (with `--preserve-status`) | Child's exit code |
| Command timed out | 124 |
| sysprims itself failed | 125 |
| Command not executable | 126 |
| Command not found | 127 |
| Killed by signal N | 128+N |

## Modules

### sysprims-timeout

Process execution with deadlines and reliable tree-kill.

```rust
use sysprims_timeout::{run_with_timeout, run_with_timeout_default, TimeoutConfig, GroupingMode};
use std::time::Duration;

// Quick usage with defaults (SIGTERM, 10s escalation, group-by-default)
let result = run_with_timeout_default("make", &["build"], Duration::from_secs(300))?;

// Or configure explicitly
let config = TimeoutConfig {
    grouping: GroupingMode::Foreground,  // Opt-out of tree-kill for legacy compat
    ..Default::default()
};
let result = run_with_timeout("make", &["build"], Duration::from_secs(300), config)?;
```

### sysprims-signal

Cross-platform signal dispatch.

```rust
use sysprims_signal::{kill, kill_by_name, killpg, match_signal_names, terminate, force_kill, SIGTERM};

// Send specific signal (using rsfulmen constants)
kill(pid, SIGTERM)?;

// Platform-agnostic helpers
terminate(pid)?;    // SIGTERM on Unix, TerminateProcess on Windows
force_kill(pid)?;   // SIGKILL on Unix, TerminateProcess on Windows

// Resolve by name (accepts "SIGTERM", "TERM", "term")
kill_by_name(pid, "TERM")?;

// List available signals matching a glob pattern
let matches = match_signal_names("SIGT*");

// Process group operations (Unix only)
#[cfg(unix)]
killpg(pgid, SIGTERM)?;
```

**Signal mapping:**

| Signal | Linux/macOS | Windows |
|--------|-------------|---------|
| TERM | SIGTERM | TerminateProcess |
| KILL | SIGKILL | TerminateProcess |
| INT | SIGINT | GenerateConsoleCtrlEvent (best-effort) |
| HUP, USR1, USR2 | Native | Not supported (returns error) |

Note: On Windows, `SIGINT` delivery is best-effort and depends on console
attachment and process group membership.

### sysprims-proc

Process inspection and enumeration.

```rust
use sysprims_proc::{snapshot, get_process, ProcessFilter};

// Get all processes
let snap = snapshot()?;
for proc in &snap.processes {
    println!("{}: {} ({}% CPU)", proc.pid, proc.name, proc.cpu_percent);
}

// Filter processes
let filter = ProcessFilter::builder()
    .name_contains("nginx")
    .cpu_above(10.0)
    .build();
let filtered = snapshot_filtered(&filter)?;
```

**CLI:**
```bash
sysprims pstat [OPTIONS]
sysprims pstat --json                          # JSON output (default)
sysprims pstat --table                         # Human-readable table
sysprims pstat --pid 1234                      # Single process by PID
sysprims pstat --name nginx --cpu-above 10    # Filter by name and CPU
sysprims pstat --sort cpu                      # Sort by CPU usage
```

**Filter options:**

| Option | Description |
|--------|-------------|
| `--pid <PID>` | Show only a specific process |
| `--name <NAME>` | Filter by name (substring, case-insensitive) |
| `--user <USER>` | Filter by username |
| `--cpu-above <PERCENT>` | Filter by minimum CPU usage (0-100) |
| `--memory-above <KB>` | Filter by minimum memory in KB |
| `--sort <FIELD>` | Sort by: pid, name, cpu, memory (default: pid) |

## Platform Support

See [Platform Support Matrix](docs/standards/platform-support.md) for the canonical reference.

| Platform | Target | Status |
|----------|--------|--------|
| Linux x64 (glibc) | `x86_64-unknown-linux-gnu` | Supported |
| Linux x64 (musl) | `x86_64-unknown-linux-musl` | Supported |
| Linux arm64 (glibc) | `aarch64-unknown-linux-gnu` | Supported |
| Linux arm64 (musl) | `aarch64-unknown-linux-musl` | Supported |
| macOS arm64 | `aarch64-apple-darwin` | Supported |
| Windows x64 | `x86_64-pc-windows-msvc` | Supported |

**Not supported**: macOS x64 (Intel Macs) - end-of-life hardware.

### Feature Parity

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| Process tree kill | setpgid/killpg | setpgid/killpg | Job Objects |
| Signal TERM/KILL | Native | Native | Mapped |
| Signal INT | Native | Native | Best-effort |
| Signal HUP/USR1/2 | Native | Native | Not supported |
| Process enumeration | /proc | libproc | Toolhelp32 |

## FFI and Language Bindings

sysprims exposes a C-ABI for integration with other languages:

```c
#include "sysprims.h"

char* result = NULL;
SysprimsErrorCode err = sysprims_proc_list(NULL, &result);
if (err == SYSPRIMS_OK) {
    printf("%s\n", result);
    sysprims_free_string(result);  // Always use sysprims allocator
}
```

**Language bindings:**

| Language | Status | Package |
|----------|--------|---------|
| Go | Available | `github.com/3leaps/sysprims/bindings/go/sysprims` |
| TypeScript | Available | `npm install @3leaps/sysprims` |
| Python | Planned (v0.2.x) | `pip install sysprims` |

### As a Go Library

```go
import "github.com/3leaps/sysprims/bindings/go/sysprims"

// Send signal to process
sysprims.Kill(pid, sysprims.SIGTERM)

// Run command with timeout
result, err := sysprims.RunWithTimeout(
    "make", []string{"build"},
    5*time.Minute,
    sysprims.DefaultTimeoutConfig(),
)

// Map listening port to owning process
proto := sysprims.ProtocolTCP
port := uint16(8080)
snap, _ := sysprims.ListeningPorts(&sysprims.PortFilter{
    Protocol: &proto,
    LocalPort: &port,
})
```

### As a TypeScript Library

```typescript
import { processList, listeningPorts, terminate, procGet } from '@3leaps/sysprims';

// Get process info by PID
const proc = procGet(process.pid);
console.log(`Process ${proc.pid}: ${proc.name}`);

// List processes matching a filter
const nginx = processList({ name_contains: "nginx" });

// Map listening port to owning process
const http = listeningPorts({ local_port: 8080 });

// Gracefully terminate a process
terminate(pid);
```

**Note:** TypeScript bindings support Linux (glibc and musl/Alpine), macOS (x64 or arm64), and Windows x64.

See [docs/guides/language-bindings.md](docs/guides/language-bindings.md) for build requirements and platform details.

## Ecosystem

sysprims integrates with the [Fulmen](https://github.com/fulmenhq) ecosystem:

- **[rsfulmen](https://github.com/fulmenhq/rsfulmen)**: sysprims uses rsfulmen's signal constants (`SIGTERM`, `SIGKILL`, etc.) for cross-platform consistency
- **Go bindings**: Available with consistent signal semantics
- **Python/TypeScript bindings**: Planned for v0.1.x series, will integrate with respective fulmen libraries

This ensures signal semantics and exit codes are consistent whether you're writing Rust, Go, Python, or TypeScript.

## Prior Art

sysprims builds on the work of others in this space:

- **[uutils/coreutils](https://github.com/uutils/coreutils)** — MIT-licensed Rust rewrite of GNU coreutils. Excellent CLI tools, though focused on POSIX compatibility rather than embeddable library use.
- **[subprocess](https://crates.io/crates/subprocess)** — Process spawning library. Great for basic spawning, though without timeout/tree-kill semantics.

We're not claiming to replace these projects. sysprims fills a specific niche: embeddable, license-clean process primitives with first-class bindings and group-by-default behavior.

## Development

```bash
# Build
cargo build

# Test
cargo test

# Full quality check
make check
```

### Quality Gates

- `cargo fmt --check` — zero diff
- `cargo clippy -- -Dwarnings` — zero warnings
- `cargo test` — all tests pass
- `cargo deny check` — no GPL dependencies

## Supply Chain

sysprims is designed for environments where dependency hygiene matters:

- **License-clean**: All dependencies use MIT, Apache-2.0, or compatible licenses
- **Auditable**: Run `cargo tree` to inspect the full dependency graph
- **SBOM-ready**: Compatible with `cargo sbom`
- **No runtime network calls**: All functionality is local

```bash
# Check dependencies
cargo deny check licenses

# Audit for vulnerabilities
cargo audit

# Generate SBOM
cargo sbom > sbom.json
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Apache-2.0 provides explicit patent grants, which may be valuable for enterprise adoption.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines and [MAINTAINERS.md](MAINTAINERS.md) for governance.

---

<div align="center">

**Built by the [3 Leaps](https://3leaps.net) team**

Part of the [Fulmen Ecosystem](https://github.com/fulmenhq)

</div>
