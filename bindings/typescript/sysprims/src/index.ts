import { SysprimsError, SysprimsErrorCode } from "./errors";
import { callJsonReturn, callU32Out, callVoid, loadSysprims } from "./ffi";
import type {
  PortBindingsSnapshot,
  PortFilter,
  ProcessFilter,
  ProcessInfo,
  ProcessSnapshot,
  WaitPidResult,
  TerminateTreeConfig,
  TerminateTreeResult,
  SpawnInGroupConfig,
  SpawnInGroupResult,
} from "./types";

export { SysprimsError, SysprimsErrorCode };
export type {
  PortBinding,
  PortBindingsSnapshot,
  PortFilter,
  ProcessFilter,
  ProcessInfo,
  ProcessSnapshot,
  ProcessState,
  Protocol,
  WaitPidResult,
  TerminateTreeConfig,
  TerminateTreeResult,
  SpawnInGroupConfig,
  SpawnInGroupResult,
} from "./types";

// -----------------------------------------------------------------------------
// Process Inspection
// -----------------------------------------------------------------------------

/**
 * Get information about a specific process by PID.
 *
 * @param pid - Process ID to query
 * @returns Process information including name, state, CPU, memory usage
 * @throws {SysprimsError} NotFound if process does not exist
 * @throws {SysprimsError} PermissionDenied if access is denied
 */
export function procGet(pid: number): ProcessInfo {
  const lib = loadSysprims();
  const result = callJsonReturn((out) => lib.sysprims_proc_get(pid >>> 0, out), lib);
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
export function processList(filter?: ProcessFilter): ProcessSnapshot {
  const lib = loadSysprims();
  const filterJson = filter ? JSON.stringify(filter) : "";
  const result = callJsonReturn((out) => lib.sysprims_proc_list(filterJson, out), lib);
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
  const result = callJsonReturn((out) => lib.sysprims_proc_listening_ports(filterJson, out), lib);
  return result as PortBindingsSnapshot;
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
  const result = callJsonReturn((out) => lib.sysprims_proc_wait_pid(pid >>> 0, timeoutMs, out), lib);
  return result as WaitPidResult;
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
  return callU32Out((out) => lib.sysprims_self_getpgid(out), lib);
}

/**
 * Get the session ID of the current process.
 *
 * @returns Session ID (SID)
 * @throws {SysprimsError} NotSupported on Windows (POSIX concept)
 */
export function selfSID(): number {
  const lib = loadSysprims();
  return callU32Out((out) => lib.sysprims_self_getsid(out), lib);
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
  callVoid(() => lib.sysprims_signal_send(pid >>> 0, signal | 0), lib);
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
  callVoid(() => lib.sysprims_signal_send_group(pgid >>> 0, signal | 0), lib);
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
  callVoid(() => lib.sysprims_terminate(pid >>> 0), lib);
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
  callVoid(() => lib.sysprims_force_kill(pid >>> 0), lib);
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
    return callJsonReturn((out) => lib.sysprims_terminate_tree(pid >>> 0, "", out), lib) as TerminateTreeResult;
  }

  const cfg: TerminateTreeConfig = {
    schema_id:
      config.schema_id ||
      "https://schemas.3leaps.dev/sysprims/process/v1.0.0/terminate-tree-config.schema.json",
    ...config,
  };

  return callJsonReturn(
    (out) => lib.sysprims_terminate_tree(pid >>> 0, JSON.stringify(cfg), out),
    lib,
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
  return callJsonReturn((out) => lib.sysprims_spawn_in_group(JSON.stringify(cfg), out), lib) as SpawnInGroupResult;
}
