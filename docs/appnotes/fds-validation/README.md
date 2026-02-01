# App Note: FD Validation with Synthetic Cases

Demonstrates `sysprims fds` for inspecting open file descriptors with controlled, synthetic test processes.

## The Scenario

You need to diagnose what files, sockets, or pipes a process has open. Unlike `lsof` (GPL-licensed), sysprims provides a library-first, license-clean alternative.

This appnote creates synthetic processes with known FDs so you can validate sysprims output against expected results.

## Prerequisites

```bash
# From repository root
make build
```

## Synthetic Test Script

The `synthetic-fd-holder.sh` script intentionally opens:
- A temp file (to test `kind: file` with path resolution)
- A TCP socket (to test `kind: socket`)
- A pipe (to test `kind: pipe`)

It holds these open until signaled, allowing you to inspect them.

## Demo 1: Single File Descriptor

Start the synthetic process:

```bash
cd docs/appnotes/fds-validation
chmod +x *.sh
./synthetic-fd-holder.sh
```

Output:
```
[synthetic-fd-holder] PID 12345 holding FDs...
  - temp file: /tmp/synthetic-fd-holder-12345-*.txt
  - TCP socket: 127.0.0.1:54321
  - pipe: read=3 write=4
Press Ctrl+C or send SIGTERM to exit.
```

In another terminal, inspect its FDs:

```bash
# Replace 12345 with the actual PID
./target/debug/sysprims fds --pid 12345 --table
```

Expected output:
```
   FD KIND     PATH/INFO
--------------------------------------------------------------------------------
    0 file     /dev/null
    1 file     /dev/null
    2 file     /dev/null
    3 file     /tmp/synthetic-fd-holder-12345-*.txt
    4 socket   127.0.0.1:54321
    5 pipe     -
```

## Demo 2: JSON Output and Filtering

Get structured output for scripting:

```bash
./target/debug/sysprims fds --pid 12345 --json | jq '.fds[] | select(.kind == "file")'
```

Filter by kind:

```bash
# Only sockets
./target/debug/sysprims fds --pid 12345 --kind socket --table

# Only files
./target/debug/sysprims fds --pid 12345 --kind file --json
```

## Demo 3: Platform Behavior Differences

### Linux
File paths are resolved via `/proc/<pid>/fd/` symlinks. You should see full paths:
```json
{
  "fd": 3,
  "kind": "file",
  "path": "/tmp/synthetic-fd-holder-12345-*.txt"
}
```

### macOS
Uses `proc_pidinfo()` with best-effort path recovery. Paths may be `-` when unavailable:
```json
{
  "fd": 3,
  "kind": "file",
  "path": null
}
```

### Windows
Returns `NotSupported` - handle enumeration requires elevated privileges:
```bash
./target/debug/sysprims fds --pid 12345
# Error: open file descriptor enumeration is not supported on windows
```

## Demo 4: Validation Script

Automated validation that sysprims detects expected FDs:

```bash
./validate-fds.sh 12345
```

This script:
1. Runs `sysprims fds --json`
2. Checks for at least one file FD
3. Checks for at least one socket FD
4. Checks for at least one pipe FD (if supported)
5. Reports PASS/FAIL

## Demo 5: Inspecting Self

Inspect the current shell's FDs:

```bash
./target/debug/sysprims fds --pid $$ --table
```

Compare with:
```bash
# Traditional (GPL) approach
lsof -p $$
```

## Cleanup

Kill the synthetic process:

```bash
# From the terminal running synthetic-fd-holder.sh, press Ctrl+C
# Or from another terminal:
./target/debug/sysprims terminate 12345
```

## When to Use fds

| Scenario | Command |
|----------|---------|
| Debug runaway processes | `sysprims fds --pid <PID> --table` |
| Find which process has a file open | `sysprims pstat --name <name>` then `sysprims fds --pid <PID>` |
| Scripting/automation | `sysprims fds --pid <PID> --json` |
| Filter by resource type | `sysprims fds --pid <PID> --kind socket` |

## See Also

- [Runaway Process Diagnosis Guide](../../guides/runaway-process-diagnosis.md)
- [Multi-PID Kill App Note](../multi-pid-kill/)
- FD Snapshot Schema: `schemas/process/v1.0.0/fd-snapshot.schema.json`
