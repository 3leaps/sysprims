# Release Notes

> **Note:** This file aggregates the latest 3 releases in reverse chronological order.
> For the complete release history, see `CHANGELOG.md`.
> For detailed release documentation, see `docs/releases/`.

---

## v0.1.13 - 2026-02-13

**Status:** macOS Command-Line Fidelity Fix & Binding Coverage

This release fixes a high-severity bug where `processList()` returned truncated `cmdline` on macOS (just the process name instead of the full argument vector), breaking downstream consumers that filter by command-line arguments. It also exports v0.1.12 process tree capabilities to the FFI layer and Go/TypeScript bindings.

### Highlights

- **macOS cmdline fix**: `cmdline` now returns the full argument vector (e.g. `["bun", "run", "scripts/dev.ts", "--root", "/path"]`) instead of `["bun"]`
- **FFI coverage**: `descendants` and `kill-descendants` now available through C-ABI FFI
- **Go binding**: `Descendants()` and `KillDescendants()` with option pattern
- **TypeScript binding**: `descendants()` and `killDescendants()` via N-API

### Bug Fix: macOS `cmdline` Truncation

**Before (v0.1.12):**
```json
{"pid": 12345, "name": "bun", "cmdline": ["bun"]}
```

**After (v0.1.13):**
```json
{"pid": 12345, "name": "bun", "cmdline": ["bun", "run", "scripts/dev.ts", "--root", "/some/path"]}
```

**Root cause:** The macOS implementation used `proc_name()` as a placeholder for `cmdline`, which only returns the process name (16 chars max). The fix uses `sysctl(CTL_KERN, KERN_PROCARGS2)` — the same kernel API that `ps` uses — to read the actual argv.

**Impact:** Any consumer filtering by `cmdline` arguments on macOS was affected. Known affected: kitfly `discoverOrphans()` which filters by `p.cmdline.some(arg => arg.includes("scripts/dev.ts"))`.

**Safety hardening (devrev):**
- PID 0 and overflow-range PIDs rejected before sysctl call
- `argc` capped at 4096 to prevent pathological allocation from malformed kernel data
- Empty argv entries filtered (consistent with Linux `/proc/[pid]/cmdline` behavior)

### FFI & Binding Coverage (Wave 1)

v0.1.12 added `descendants` and `kill-descendants` to the CLI and Rust crates. This release makes them available to language binding consumers:

| Function | FFI | Go | TypeScript |
|----------|:---:|:--:|:----------:|
| `descendants()` | New | New | New |
| `killDescendants()` | New | New | New |

**FFI functions:**
```c
int32_t sysprims_proc_descendants(const char *config_json, char **result_json_out);
int32_t sysprims_proc_kill_descendants(const char *config_json, char **result_json_out);
```

Safety enforcement happens in the FFI layer — bindings get PID 1 protection, self-exclusion, and parent protection for free.

### Upgrade Notes

- **No breaking changes** — all changes are additive
- macOS consumers will immediately see full `cmdline` data where previously truncated
- Consumers filtering by `cmdline` may see more matches than before (this is correct behavior)
- FFI shared library must be rebuilt for all platform targets to include new exports

---

## v0.1.12 - 2026-02-06

**Status:** Process Tree Operations & Enhanced Discovery Release

This release adds process tree traversal capabilities, ASCII tree visualization, and enhanced filtering for surgical process management. Operators can now inspect process hierarchies, identify runaway descendants, and terminate specific subtrees without affecting parent processes or critical system processes.

### Highlights

- **Process Tree Visibility**: New `descendants` command with ASCII art visualization shows instant, human-readable process trees
- **Targeted Cleanup**: `kill-descendants` enables surgical subtree termination with filter support (--cpu-above, --running-for, --name)
- **Age-Based Filtering**: `--running-for` option on all process commands helps distinguish long-running spinners from recent spikes
- **Parent PID Filtering**: `--ppid` option on `pstat` and `kill` for filtering by process parent
- **Safety by Design**: Filter-based kills always preview unless `--yes` provided; never targets self, PID 1, parent, or root without `--force`
- **Depth-Controlled Traversal**: `--max-levels N` limits tree depth (default 1 = direct children only, accepts "all" for full subtree)

### CLI: `sysprims descendants` Command

New subcommand to list child processes of a given root PID:

```bash
# Show direct children only (level 1)
sysprims descendants 7825 --table

# Show 2 levels deep (children + grandchildren)
sysprims descendants 7825 --max-levels 2 --table

# Show full subtree with ASCII art
sysprims descendants 7825 --max-levels all --tree

# Filter by CPU usage
sysprims descendants 7825 --cpu-above 80 --tree

# Filter by process age (long-running spinners)
sysprims descendants 7825 --running-for "1h" --tree
```

**Options:**

| Option | Description |
|--------|-------------|
| `--max-levels <N>` | Maximum traversal depth (1 = direct children, "all" = full subtree) |
| `--json` | Output as JSON (default) |
| `--table` | Human-readable table format (flat, grouped by level) |
| `--tree` | ASCII art tree with hierarchy visualization |
| `--name <NAME>` | Filter by process name (substring match) |
| `--user <USER>` | Filter by username |
| `--cpu-above <PERCENT>` | Filter by minimum CPU usage |
| `--memory-above <KB>` | Filter by minimum memory usage |
| `--running-for <DURATION>` | Filter by minimum process age (e.g., "5s", "1m", "2h") |

### CLI: `sysprims kill-descendants` Command

Send signals to descendants of a process without affecting parent or root:

```bash
# Preview what would be killed
sysprims kill-descendants 7825 --cpu-above 80 --dry-run

# Kill all high-CPU descendants (requires --yes for filter-based selection)
sysprims kill-descendants 7825 --cpu-above 80 --yes

# Kill direct children only (level 1)
sysprims kill-descendants 7825 --max-levels 1 --yes

# Use SIGKILL for hung processes
sysprims kill-descendants 7825 --cpu-above 90 --signal KILL --yes

# Kill full subtree (all descendants)
sysprims kill-descendants 7825 --max-levels all --yes
```

**Safety behaviors:**
- **Preview mode**: Filter-based selection defaults to `--dry-run` unless `--yes` is explicitly provided
- **Self exclusion**: Never targets CLI's own process
- **Parent protection**: Never targets parent process of selected descendants (unless `--force`)
- **PID 1 protection**: Never targets init/launchd (unless `--force`)
- **Root protection**: Never targets system root without `--force`

**Options:**

| Option | Description |
|--------|-------------|
| `--max-levels <N>` | Maximum traversal depth (same as `descendants`) |
| `-s, --signal <SIGNAL>` | Signal name or number (default: TERM) |
| `--name <NAME>` | Filter by process name |
| `--user <USER>` | Filter by username |
| `--cpu-above <PERCENT>` | Filter by minimum CPU usage |
| `--memory-above <KB>` | Filter by minimum memory usage |
| `--running-for <DURATION>` | Filter by minimum process age |
| `--dry-run` | Print matched targets but do not send signals |
| `--yes` | Proceed with kill (required for filter-based selection) |
| `--force` | Proceed even if CLI safety checks would normally refuse |
| `--json` | Output as JSON |

### CLI: Enhanced `sysprims pstat` Options

New filter options for process discovery:

```bash
# Filter by parent PID
sysprims pstat --ppid 7825 --table

# Filter by process age (long-running processes only)
sysprims pstat --cpu-above 80 --running-for "1h" --table

# Combine multiple filters
sysprims pstat --ppid 7825 --cpu-above 90 --running-for "10m" --table
```

**New filter options:**

| Option | Available on |
|--------|--------------|
| `--ppid <PID>` | `pstat`, `kill` |
| `--running-for <DURATION>` | `pstat`, `kill`, `descendants`, `kill-descendants` |

### CLI: Enhanced `sysprims kill` Options

The `kill` command now accepts all filter options in addition to explicit PIDs:

```bash
# Kill by parent PID filter
sysprims kill --ppid 7825 --signal TERM

# Kill by combined filters (requires --yes for filter-based selection)
sysprims kill --ppid 7825 --cpu-above 80 --yes

# Preview before killing (dry-run mode)
sysprims kill --ppid 7825 --cpu-above 80 --dry-run

# Force override for protected targets
sysprims kill --ppid 7825 --yes --force
```

### Validation

### Process Tree Operations

Tested on macOS arm64 (Darwin 25.2.0):

**Descendants command:**
```bash
$ sysprims descendants 7825 --max-levels 1 --table
--- Level 1 ---
    PID    PPID   CPU%    MEM(KB)    STATE USER             NAME
--------------------------------------------------------------------------------
   985    7825    0.0      62720        R davethompson     VSCodium Helper
   986    7825    0.0      83408        R davethompson     VSCodium Helper (Plugin)
   ... (40 total descendants)
```

**ASCII tree visualization:**
```bash
$ sysprims descendants 7825 --tree | head -15
7825 Electron [0.1% CPU, 160M, 7d18h]
├── 985 VSCodium Helper [0.0% CPU, 63M, 1d13h]
├── 986 VSCodium Helper (Plugin) [0.0% CPU, 84M, 1d13h]
├── 5495 VSCodium Helper (Renderer) [0.0% CPU, 119M, 16h47m]
└── ...
```

**Kill-descendants safety:**
```bash
# Parent excluded by default
$ sysprims kill-descendants 7825 --dry-run
# Output: 40 descendants (no parent PID)

# With --force, parent included
$ sysprims kill-descendants 7825 --dry-run --force
# Output: 41 processes (includes parent PID)
```

### Filter Validation

**Parent PID filter:**
```bash
$ sysprims pstat --ppid 7825 --table
# Shows 32 direct children of VSCodium Electron process
```

**Age-based filtering:**
```bash
# Find processes >90% CPU running >1 hour
$ sysprims pstat --cpu-above 90 --running-for "1h" --table
# Distinguishes long-running spinners from brief spikes
```

### Real-World Use Cases

**Scenario: Identify and terminate runaway Electron helper processes**

```bash
# 1. Find high CPU processes in tree
$ sysprims descendants 7825 --cpu-above 80 --tree

# 2. Preview what would be killed
$ sysprims kill-descendants 7825 --cpu-above 80 --dry-run --json

# 3. Terminate runaway descendants (parent VSCodium survives)
$ sysprims kill-descendants 7825 --cpu-above 80 --yes
```

**Scenario: Chrome renderer runaway**

```bash
# Chrome has many helper processes; find spinning one
$ sysprims descendants 67566 --name "Helper (Renderer)" --cpu-above 100 --tree

# Kill just the runaway renderer (not entire browser)
$ sysprims kill-descendants 67566 --name "Helper (Renderer)" --cpu-above 100 --max-levels 2 --yes
```

## Platform Notes

### macOS

**Process tree traversal** works correctly with `libproc`:
- Uses `proc_pidinfo(PROC_PIDTBSDINFO)` for parent-child relationships
- BFS traversal respects `--max-levels` depth limit
- Parent process exclusion enforced for `kill-descendants`

**Age filtering availability:**
- Process start time available via `proc_pidinfo()`
- `start_time_unix_ms` field populated for all processes
- `--running-for` filters work on macOS

**ASCII tree visualization:**
- Requires terminal supporting box-drawing characters (UTF-8)
- Falls back gracefully on terminals without tree line support
- Uses `├──`, `│   `, `└──` for tree structure

### Linux

**Full visibility**: `/proc/[pid]/stat` provides complete process tree without restrictions.
All features work identically to macOS.

### Windows

**Process tree traversal**: Supported via `CreateToolhelp32Snapshot`.
Depth limiting and filtering work as on Unix.

**ASCII tree visualization**: Box-drawing characters may not render correctly on some terminals.
Consider using `--table` or `--json` on Windows for reliability.

## Schema Additions

### `process-filter.schema.json`

New filter fields:

```json
{
  "properties": {
    "ppid": {
      "description": "Filter by parent process ID",
      "type": "integer",
      "minimum": 1
    },
    "running_for_at_least_secs": {
      "description": "Filter by minimum process age in seconds",
      "type": "number",
      "minimum": 0
    }
  }
}
```

### `descendants-result.schema.json` (NEW)

New schema for `descendants` command output:

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/process/v1.0.0/descendants-result.schema.json",
  "root_pid": "integer",
  "max_levels": "integer",
  "levels": [
    {
      "level": "integer",
      "processes": [{ /* Process objects */ }]
    }
  ],
  "total_found": "integer",
  "matched_by_filter": "integer"
}
```

## Safety Considerations

### PID Validation (ADR-0011)

All tree traversal and kill operations enforce ADR-0011 PID safety:

- **PID 0** never targeted (signals caller's process group)
- **PID 1** never targeted (init/launchd)
- **Parent processes** protected by default in `kill-descendants`
- **Self-exclusion** enforced for all kill operations
- **Root process** protection with `--force` override only

### Filter-Based Kill Preview

To prevent accidental bulk terminations:

```bash
# Without --yes, shows what would be killed (preview mode)
$ sysprims kill-descendants 7825 --cpu-above 80
# Output: Lists PIDs but does not send signals

# Requires --yes to proceed
$ sysprims kill-descendants 7825 --cpu-above 80 --yes
# Output: Sends signals to matched PIDs
```

### Process Tree Best Practices

**Depth control**: Always start with `--max-levels 1` (direct children) to avoid broad subtree operations. Use `--max-levels 2` or `--max-levels 3` only when necessary.

**Age filtering**: Use `--running-for "10m"` or `--running-for "1h"` to identify processes that are genuinely stuck vs. momentary spikes from recent workloads.

**Parent protection**: When using `kill-descendants`, remember that the parent process (e.g., Electron, Chrome main browser) is intentionally excluded. To restart the entire application, target the parent PID directly with `sysprims kill <PID>` or `sysprims terminate-tree <PID>`.

## Upgrade Notes

- **No breaking changes**
- All new CLI commands are additions, not replacements
- Existing `kill` and `pstat` commands remain fully backward compatible
- JSON output formats extended, not modified
- **Security Fix**: Updated `time` crate from 0.3.45 to 0.3.47 (fixes RUSTSEC-2026-0009 DoS via stack exhaustion in RFC 2822 parsing)

- **Process tree visibility**: `descendants --tree` provides instant hierarchy view without external tools
- **Targeted cleanup**: `kill-descendants` with filters enables surgical subtree termination
- **Age awareness**: `--running-for` helps distinguish persistent problems from transient spikes

## Files Changed

- `crates/sysprims-proc/src/lib.rs` - Added `ppid` to ProcessFilter, `running_for_at_least_secs` filter, descendants() traversal function
- `crates/sysprims-cli/src/main.rs` - Added `descendants` and `kill-descendants` commands, ASCII tree renderer, filter logic integration
- `schemas/process/v1.0.0/process-filter.schema.json` - Added `ppid` and `running_for_at_least_secs` fields
- `schemas/process/v1.0.0/descendants-result.schema.json` (NEW) - Schema for descendants output with level structure

## References

- **Feature brief**: `.plans/active/v0.1.12/feature-brief.md`
- **ADR-0011**: PID Validation Safety
- **Platform Support**: `docs/standards/platform-support.md`
- **Schema contracts**: `docs/standards/schema-contracts.md`

---

## [0.1.11] - 2026-02-04

**Status:** macOS Port Discovery & Bun Runtime Support Release

Fixes `listeningPorts()` returning empty results on macOS, adds a new `ports` CLI command, and enables Bun runtime support for TypeScript bindings.

### Highlights

- **macOS Port Discovery Fixed**: `listeningPorts()` now works on macOS (was returning empty)
- **New CLI Command**: `sysprims ports` for listing listening port bindings
- **Bun Runtime Support**: TypeScript bindings now work under Bun

### New CLI Command: `sysprims ports`

List listening port bindings with optional filtering:

```bash
# Table output for human inspection
sysprims ports --table

# Filter by protocol
sysprims ports --protocol tcp --table

# Filter by specific port (JSON output)
sysprims ports --protocol tcp --local-port 8080 --json
```

Output includes full process details (name, PID, exe_path, cmdline, user).

### macOS Port Discovery Fix

The `listeningPorts()` function was returning empty results on macOS due to SDK struct layout mismatches in socket fdinfo parsing. This release fixes the issue:

**Before (v0.1.10):**
```
Found 0 bindings
```

**After (v0.1.11):**
```
Found 68 bindings
  tcp *:9999 -> pid=54659 (bun)
  tcp 127.0.0.1:8080 -> pid=40672 (namelens)
  ...
```

**Technical changes:**
- UID filtering: scans current-user processes only (reduces SIP/TCC permission errors)
- Heuristic vinfo_stat size detection (136/144 bytes) for SDK compatibility
- Offset-based parsing instead of fixed struct layout
- Strict TCP listener filtering (`TSI_S_LISTEN` state only)

### Bun Runtime Support

TypeScript bindings now work under Bun. The explicit Bun block has been removed:

```typescript
// REMOVED in v0.1.11:
if (process.versions?.bun) {
  throw new Error("sysprims TypeScript bindings are not yet validated on Bun...");
}
```

Validated functionality under Bun:

| Feature | Status |
|---------|--------|
| Module loading | Works |
| `procGet()` | Works |
| `terminate()` | Works |
| `listeningPorts()` | Works (with macOS fix) |

### Upgrade Notes

- No breaking changes
- macOS users will now see port bindings that were previously invisible
- Bun users can use sysprims directly without workarounds

