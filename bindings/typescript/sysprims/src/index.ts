import { SysprimsError, SysprimsErrorCode } from "./errors";
import { callJsonReturn, callU32Out, callVoid, loadSysprims } from "./ffi";
import type {
  BatchKillFailure,
  BatchKillResult,
  CpuMode,
  DescendantsOptions,
  DescendantsResult,
  FdFilter,
  FdSnapshot,
  KillDescendantsOptions,
  KillDescendantsResult,
  PortBindingsSnapshot,
  PortFilter,
  ProcessFilter,
  ProcessInfo,
  ProcessOptions,
  ProcessSnapshot,
  SpawnInGroupConfig,
  SpawnInGroupResult,
  TerminateTreeConfig,
  TerminateTreeResult,
  WaitPidResult,
} from "./types";

export { SysprimsError, SysprimsErrorCode };
export type {
  BatchKillFailure,
  BatchKillResult,
  CpuMode,
  DescendantsLevel,
  DescendantsOptions,
  DescendantsResult,
  FdFilter,
  FdSnapshot,
  KillDescendantsFailure,
  KillDescendantsOptions,
  KillDescendantsResult,
  PortBinding,
  PortBindingsSnapshot,
  PortFilter,
  ProcessFilter,
  ProcessInfo,
  ProcessOptions,
  ProcessSnapshot,
  ProcessState,
  Protocol,
  SpawnInGroupConfig,
  SpawnInGroupResult,
  TerminateTreeConfig,
  TerminateTreeResult,
  WaitPidResult,
} from "./types";

// -----------------------------------------------------------------------------
// Process Inspection
// -----------------------------------------------------------------------------

function serializeProcessOptions(options?: ProcessOptions): string {
  if (!options) {
    return "";
  }

  const wire: { include_env?: boolean; include_threads?: boolean } = {};
  if (options.includeEnv === true) {
    wire.include_env = true;
  }
  if (options.includeThreads === true) {
    wire.include_threads = true;
  }

  if (!wire.include_env && !wire.include_threads) {
    return "";
  }

  return JSON.stringify(wire);
}

function serializeDescendantsConfig(options?: {
  filter?: ProcessFilter;
  cpuMode?: CpuMode;
  sampleDurationMs?: number;
}): string {
  if (!options) {
    return "";
  }

  const wire: Record<string, unknown> = {};

  if (options.filter) {
    Object.assign(wire, options.filter);
  }

  if (options.cpuMode !== undefined) {
    if (options.cpuMode !== "lifetime" && options.cpuMode !== "monitor") {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        `invalid cpuMode: ${String(options.cpuMode)}`,
      );
    }
    wire.cpu_mode = options.cpuMode;
  }

  if (options.sampleDurationMs !== undefined) {
    const sample = options.sampleDurationMs;
    if (!Number.isFinite(sample) || sample < 0) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        "sampleDurationMs must be a finite number >= 0",
      );
    }
    wire.sample_duration_ms = Math.trunc(sample);
  }

  if (Object.keys(wire).length === 0) {
    return "";
  }

  return JSON.stringify(wire);
}

/**
 * Get information about a specific process by PID.
 *
 * @param pid - Process ID to query
 * @returns Process information including name, state, CPU, memory usage
 * @throws {SysprimsError} NotFound if process does not exist
 * @throws {SysprimsError} PermissionDenied if access is denied
 */
export function procGet(pid: number, options?: ProcessOptions): ProcessInfo {
  const lib = loadSysprims();
  const optionsJson = serializeProcessOptions(options);
  const result = callJsonReturn(() => lib.sysprimsProcGetEx(pid >>> 0, optionsJson));
  return result as ProcessInfo;
}

/**
 * List running processes with optional filtering.
 *
 * Filter fields use snake_case to match FFI/schema conventions:
 * - `name_contains`: substring match (case-insensitive)
 * - `name_equals`: exact name match
 * - `user_equals`: exact username match
 * - `pid_in`: array of PIDs to include
 * - `state_in`: array of states to include
 * - `cpu_above`: minimum CPU percentage (0-100)
 * - `memory_above_kb`: minimum memory in KB
 *
 * @param filter - Optional filter criteria (all fields are AND-ed)
 * @returns Snapshot of matching processes
 *
 * @example
 * // List all processes
 * const all = processList();
 *
 * @example
 * // Filter by name substring
 * const nginx = processList({ name_contains: "nginx" });
 *
 * @example
 * // Filter by multiple criteria
 * const heavy = processList({ cpu_above: 50, memory_above_kb: 100000 });
 */
export function processList(filter?: ProcessFilter, options?: ProcessOptions): ProcessSnapshot {
  const lib = loadSysprims();
  const filterJson = filter ? JSON.stringify(filter) : "";
  const optionsJson = serializeProcessOptions(options);
  const result = callJsonReturn(() => lib.sysprimsProcListEx(filterJson, optionsJson));
  return result as ProcessSnapshot;
}

/**
 * List listening network ports with optional filtering.
 *
 * Results are best-effort: some platforms may not provide full process attribution,
 * and elevated privileges may be required for complete visibility. Check the
 * `warnings` array in the result for any limitations encountered.
 *
 * Filter fields use snake_case to match FFI/schema conventions:
 * - `protocol`: "tcp" or "udp"
 * - `local_port`: specific port number
 *
 * @param filter - Optional filter criteria
 * @returns Snapshot of listening ports (may be empty if no ports are listening)
 *
 * @example
 * // List all listening ports
 * const ports = listeningPorts();
 *
 * @example
 * // Filter by protocol
 * const tcpPorts = listeningPorts({ protocol: "tcp" });
 *
 * @example
 * // Find specific port
 * const http = listeningPorts({ local_port: 8080 });
 */
export function listeningPorts(filter?: PortFilter): PortBindingsSnapshot {
  const lib = loadSysprims();
  const filterJson = filter ? JSON.stringify(filter) : "";
  const result = callJsonReturn(() => lib.sysprimsProcListeningPorts(filterJson));
  return result as PortBindingsSnapshot;
}

// -----------------------------------------------------------------------------
// Descendants
// -----------------------------------------------------------------------------

const MAX_LEVELS_ALL = 0xFFFFFFFF; // u32::MAX

/**
 * Get descendants of a process.
 *
 * Performs a BFS traversal of the process tree starting at `pid` and returns
 * processes grouped by depth level.
 *
 * @param pid - Root process ID to traverse from
 * @param options - Optional traversal configuration
 * @returns Descendants grouped by level with metadata
 * @throws {SysprimsError} NotFound if root process does not exist
 * @throws {SysprimsError} InvalidArgument if pid is 0 or filter is invalid
 *
 * @example
 * // Get all descendants
 * const result = descendants(1234);
 *
 * @example
 * // Get direct children only
 * const result = descendants(1234, { maxLevels: 1 });
 *
 * @example
 * // Filter descendants by name
 * const result = descendants(1234, { filter: { name_contains: "worker" } });
 */
export function descendants(pid: number, options?: DescendantsOptions): DescendantsResult {
  const lib = loadSysprims();
  const maxLevels = options?.maxLevels != null && isFinite(options.maxLevels)
    ? options.maxLevels >>> 0
    : MAX_LEVELS_ALL;
  const configJson = serializeDescendantsConfig(options);
  return callJsonReturn(() =>
    lib.sysprimsProcDescendants(pid >>> 0, maxLevels, configJson),
  ) as DescendantsResult;
}

/**
 * Kill descendants of a process.
 *
 * Traverses the process tree from `pid`, collects descendant PIDs, and sends
 * the specified signal. Safety rules are enforced: root PID, self, PID 1, and
 * parent are automatically excluded from the kill list.
 *
 * @param pid - Root process ID (will NOT be killed)
 * @param signal - Signal number to send (default: 15 = SIGTERM)
 * @param options - Optional configuration
 * @returns Result with succeeded/failed PIDs and safety skip count
 * @throws {SysprimsError} NotFound if root process does not exist
 * @throws {SysprimsError} InvalidArgument if pid is 0 or filter is invalid
 *
 * @example
 * // Kill all descendants with SIGTERM
 * const result = killDescendants(1234);
 *
 * @example
 * // Force kill descendants matching a filter
 * const result = killDescendants(1234, 9, {
 *   filter: { cpu_above: 90 },
 * });
 */
export function killDescendants(
  pid: number,
  signal = 15,
  options?: KillDescendantsOptions,
): KillDescendantsResult {
  const lib = loadSysprims();
  const maxLevels = options?.maxLevels != null && isFinite(options.maxLevels)
    ? options.maxLevels >>> 0
    : MAX_LEVELS_ALL;
  const configJson = serializeDescendantsConfig(options);
  return callJsonReturn(() =>
    lib.sysprimsProcKillDescendants(pid >>> 0, maxLevels, signal | 0, configJson),
  ) as KillDescendantsResult;
}

// -----------------------------------------------------------------------------
// Wait
// -----------------------------------------------------------------------------

/**
 * Wait for a PID to exit up to the provided timeout (milliseconds).
 *
 * Best-effort behavior:
 * - Unix: polling strategy (we are not necessarily the parent)
 * - Windows: process wait APIs when available
 */
export function waitPID(pid: number, timeoutMs: number): WaitPidResult {
  const lib = loadSysprims();
  const result = callJsonReturn(() => lib.sysprimsProcWaitPid(pid >>> 0, timeoutMs));
  return result as WaitPidResult;
}

// -----------------------------------------------------------------------------
// File Descriptors
// -----------------------------------------------------------------------------

/**
 * List open file descriptors for a PID.
 *
 * Best-effort behavior: fields may be missing and warnings may be present.
 */
export function listFds(pid: number, filter?: FdFilter): FdSnapshot {
  const lib = loadSysprims();
  const filterJson = filter ? JSON.stringify(filter) : "";
  const result = callJsonReturn(() => lib.sysprimsProcListFds(pid >>> 0, filterJson));
  return result as FdSnapshot;
}

// -----------------------------------------------------------------------------
// Self Introspection
// -----------------------------------------------------------------------------

/**
 * Get the process group ID of the current process.
 *
 * @returns Process group ID (PGID)
 * @throws {SysprimsError} NotSupported on Windows (POSIX concept)
 */
export function selfPGID(): number {
  const lib = loadSysprims();
  return callU32Out(() => lib.sysprimsSelfGetpgid());
}

/**
 * Get the session ID of the current process.
 *
 * @returns Session ID (SID)
 * @throws {SysprimsError} NotSupported on Windows (POSIX concept)
 */
export function selfSID(): number {
  const lib = loadSysprims();
  return callU32Out(() => lib.sysprimsSelfGetsid());
}

// -----------------------------------------------------------------------------
// Signal Operations
// -----------------------------------------------------------------------------

/**
 * Send a signal to a process.
 *
 * On Unix: sends the specified POSIX signal (e.g., 15=SIGTERM, 9=SIGKILL).
 * On Windows: only signal 0 (existence check) and termination signals are
 * meaningful; other signals may return NotSupported.
 *
 * @param pid - Target process ID
 * @param signal - Signal number (Unix semantics)
 * @throws {SysprimsError} NotFound if process does not exist
 * @throws {SysprimsError} PermissionDenied if access is denied
 * @throws {SysprimsError} NotSupported if signal is not supported on this platform
 *
 * @example
 * // Check if process exists (signal 0)
 * signalSend(1234, 0);
 *
 * @example
 * // Send SIGTERM
 * signalSend(1234, 15);
 */
export function signalSend(pid: number, signal: number): void {
  const lib = loadSysprims();
  callVoid(() => lib.sysprimsSignalSend(pid >>> 0, signal | 0));
}

/**
 * Send a signal to a process group.
 *
 * On Unix: sends the signal to all processes in the group.
 * On Windows: NotSupported (no process group concept).
 *
 * @param pgid - Target process group ID
 * @param signal - Signal number (Unix semantics)
 * @throws {SysprimsError} NotFound if process group does not exist
 * @throws {SysprimsError} PermissionDenied if access is denied
 * @throws {SysprimsError} NotSupported on Windows
 */
export function signalSendGroup(pgid: number, signal: number): void {
  const lib = loadSysprims();
  callVoid(() => lib.sysprimsSignalSendGroup(pgid >>> 0, signal | 0));
}

/**
 * Terminate a process gracefully.
 *
 * On Unix: sends SIGTERM.
 * On Windows: calls TerminateProcess.
 *
 * @param pid - Target process ID
 * @throws {SysprimsError} NotFound if process does not exist
 * @throws {SysprimsError} PermissionDenied if access is denied
 */
export function terminate(pid: number): void {
  const lib = loadSysprims();
  callVoid(() => lib.sysprimsTerminate(pid >>> 0));
}

/**
 * Force kill a process immediately.
 *
 * On Unix: sends SIGKILL (cannot be caught or ignored).
 * On Windows: calls TerminateProcess.
 *
 * @param pid - Target process ID
 * @throws {SysprimsError} NotFound if process does not exist
 * @throws {SysprimsError} PermissionDenied if access is denied
 */
export function forceKill(pid: number): void {
  const lib = loadSysprims();
  callVoid(() => lib.sysprimsForceKill(pid >>> 0));
}

const MAX_SAFE_PID = 2147483647;

function validatePidList(pids: number[]): void {
  if (!Array.isArray(pids) || pids.length === 0) {
    throw new SysprimsError(SysprimsErrorCode.InvalidArgument, "pids must not be empty");
  }
  for (const pid of pids) {
    if (!Number.isInteger(pid)) {
      throw new SysprimsError(SysprimsErrorCode.InvalidArgument, "pid must be an integer");
    }
    if (pid <= 0) {
      throw new SysprimsError(SysprimsErrorCode.InvalidArgument, "pid must be > 0");
    }
    if (pid > MAX_SAFE_PID) {
      throw new SysprimsError(
        SysprimsErrorCode.InvalidArgument,
        `pid ${pid} exceeds maximum safe value ${MAX_SAFE_PID}`,
      );
    }
  }
}

function validateSignal(signal: number): void {
  if (!Number.isInteger(signal)) {
    throw new SysprimsError(SysprimsErrorCode.InvalidArgument, "signal must be an integer");
  }
}

/**
 * Send a signal to multiple processes.
 *
 * PID validation happens for the entire slice before any signals are sent.
 * Individual failures are collected and returned.
 */
export function killMany(pids: number[], signal: number): BatchKillResult {
  validatePidList(pids);
  validateSignal(signal);

  const result: BatchKillResult = { succeeded: [], failed: [] };
  for (const pid of pids) {
    try {
      signalSend(pid, signal);
      result.succeeded.push(pid >>> 0);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      result.failed.push({ pid: pid >>> 0, error: msg } satisfies BatchKillFailure);
    }
  }
  return result;
}

/**
 * Terminate multiple processes gracefully.
 *
 * On Unix: sends SIGTERM.
 * On Windows: calls TerminateProcess.
 */
export function terminateMany(pids: number[]): BatchKillResult {
  validatePidList(pids);

  const result: BatchKillResult = { succeeded: [], failed: [] };
  for (const pid of pids) {
    try {
      terminate(pid);
      result.succeeded.push(pid >>> 0);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      result.failed.push({ pid: pid >>> 0, error: msg } satisfies BatchKillFailure);
    }
  }
  return result;
}

/**
 * Force kill multiple processes immediately.
 *
 * On Unix: sends SIGKILL.
 * On Windows: calls TerminateProcess.
 */
export function forceKillMany(pids: number[]): BatchKillResult {
  validatePidList(pids);

  const result: BatchKillResult = { succeeded: [], failed: [] };
  for (const pid of pids) {
    try {
      forceKill(pid);
      result.succeeded.push(pid >>> 0);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      result.failed.push({ pid: pid >>> 0, error: msg } satisfies BatchKillFailure);
    }
  }
  return result;
}

// -----------------------------------------------------------------------------
// Terminate Tree
// -----------------------------------------------------------------------------

/**
 * Terminate a process with escalation (TERM -> wait -> KILL).
 *
 * This is intended for supervisor stop flows.
 *
 * Note: this is a PID-only API. On Unix, if `pid` is a process group leader,
 * sysprims may use group kill for better coverage.
 */
export function terminateTree(pid: number, config?: TerminateTreeConfig): TerminateTreeResult {
  const lib = loadSysprims();

  if (!config) {
    return callJsonReturn(() => lib.sysprimsTerminateTree(pid >>> 0, "")) as TerminateTreeResult;
  }

  const cfg: TerminateTreeConfig = {
    schema_id:
      config.schema_id ||
      "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-config.schema.json",
    ...config,
  };

  return callJsonReturn(() =>
    lib.sysprimsTerminateTree(pid >>> 0, JSON.stringify(cfg)),
  ) as TerminateTreeResult;
}

// -----------------------------------------------------------------------------
// Spawn In Group
// -----------------------------------------------------------------------------

export function spawnInGroup(config: SpawnInGroupConfig): SpawnInGroupResult {
  const lib = loadSysprims();
  const cfg: SpawnInGroupConfig = {
    schema_id:
      config.schema_id ||
      "https://schemas.3leaps.dev/sysprims/process/v1.0.0/spawn-in-group-config.schema.json",
    ...config,
  };
  return callJsonReturn(() => lib.sysprimsSpawnInGroup(JSON.stringify(cfg))) as SpawnInGroupResult;
}
