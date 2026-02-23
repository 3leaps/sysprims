export type JsonObject = Record<string, unknown>;

// Process types

export type ProcessState = "running" | "sleeping" | "stopped" | "zombie" | "unknown";
export type CpuMode = "lifetime" | "monitor";

/**
 * Information about a single process.
 * Matches schema: process-info.schema.json#/definitions/process_info
 */
export interface ProcessInfo {
  pid: number;
  ppid: number;
  name: string;
  user?: string | null;
  cpu_percent: number;
  memory_kb: number;
  elapsed_seconds: number;
  start_time_unix_ms?: number | null;
  exe_path?: string | null;
  state: ProcessState;
  cmdline: string[];
  env?: Record<string, string> | null;
  thread_count?: number | null;
}

/**
 * Filter criteria for process listing.
 * All fields use snake_case to match FFI expectations directly.
 */
export interface ProcessFilter {
  name_contains?: string;
  name_equals?: string;
  user_equals?: string;
  pid_in?: number[];
  ppid?: number;
  state_in?: ProcessState[];
  cpu_above?: number;
  memory_above_kb?: number;
  running_for_at_least_secs?: number;
}

/**
 * Optional process detail collection controls.
 *
 * Uses camelCase at the TypeScript API boundary; converted to snake_case for FFI.
 */
export interface ProcessOptions {
  includeEnv?: boolean;
  includeThreads?: boolean;
}

/**
 * Snapshot of running processes.
 * Matches schema: process-info.schema.json
 */
export interface ProcessSnapshot {
  schema_id: string;
  timestamp: string;
  processes: ProcessInfo[];
}

// Wait PID

/**
 * Result of waiting for a PID to exit.
 * Matches schema: wait-pid-result.schema.json
 */
export interface WaitPidResult {
  schema_id: string;
  timestamp: string;
  platform: string;
  pid: number;
  exited: boolean;
  timed_out: boolean;
  exit_code?: number | null;
  warnings: string[];
}

// Terminate tree

export interface TerminateTreeConfig {
  schema_id?: string;
  grace_timeout_ms?: number | null;
  kill_timeout_ms?: number | null;
  signal?: number | null;
  kill_signal?: number | null;
}

export interface TerminateTreeResult {
  schema_id: string;
  timestamp: string;
  platform: string;
  pid: number;
  pgid?: number | null;
  signal_sent: number;
  kill_signal?: number | null;
  escalated: boolean;
  exited: boolean;
  timed_out: boolean;
  tree_kill_reliability: "guaranteed" | "best_effort";
  warnings: string[];
}

// Spawn in group

export interface SpawnInGroupConfig {
  schema_id?: string;
  argv: string[];
  cwd?: string | null;
  env?: Record<string, string> | null;
}

export interface SpawnInGroupResult {
  schema_id: string;
  timestamp: string;
  platform: string;
  pid: number;
  pgid?: number | null;
  tree_kill_reliability: "guaranteed" | "best_effort";
  warnings: string[];
}

// Port types

export type Protocol = "tcp" | "udp";

/**
 * Information about a listening socket binding.
 * Matches schema: port-bindings.schema.json#/definitions/port_binding
 */
export interface PortBinding {
  protocol: Protocol;
  local_addr?: string | null;
  local_port: number;
  state?: string | null;
  pid?: number | null;
  process?: ProcessInfo;
}

/**
 * Filter criteria for port listing.
 * All fields use snake_case to match FFI expectations directly.
 */
export interface PortFilter {
  protocol?: Protocol;
  local_port?: number;
}

/**
 * Snapshot of listening ports.
 * Matches schema: port-bindings.schema.json
 */
export interface PortBindingsSnapshot {
  schema_id: string;
  timestamp: string;
  platform: string;
  bindings: PortBinding[];
  warnings: string[];
}

// File descriptors

export type FdKind = "file" | "socket" | "pipe" | "unknown";

export interface FdInfo {
  fd: number;
  kind: FdKind;
  path?: string | null;
}

export interface FdFilter {
  kind?: FdKind;
}

export interface FdSnapshot {
  schema_id: string;
  timestamp: string;
  platform: string;
  pid: number;
  fds: FdInfo[];
  warnings: string[];
}

// Signal operations

export interface BatchKillFailure {
  pid: number;
  error: string;
}

export interface BatchKillResult {
  succeeded: number[];
  failed: BatchKillFailure[];
}

// Descendants types

/**
 * A single level in a descendants traversal result.
 */
export interface DescendantsLevel {
  level: number;
  processes: ProcessInfo[];
}

/**
 * Result of a descendants traversal.
 * Matches schema: descendants-result.schema.json
 */
export interface DescendantsResult {
  schema_id: string;
  root_pid: number;
  max_levels: number;
  levels: DescendantsLevel[];
  total_found: number;
  matched_by_filter: number;
  timestamp: string;
  platform: string;
}

/**
 * Options for descendants traversal.
 */
export interface DescendantsOptions {
  /** Maximum depth (1 = children only). Omit or use Infinity for all levels. */
  maxLevels?: number;
  /** Optional filter applied to descendant processes. */
  filter?: ProcessFilter;
  /** CPU measurement mode used for cpu_above evaluation. */
  cpuMode?: CpuMode;
  /** Sampling interval in milliseconds (used with monitor mode). */
  sampleDurationMs?: number;
}

/**
 * Options for kill-descendants operation.
 */
export interface KillDescendantsOptions {
  /** Maximum depth. Omit or use Infinity for all levels. */
  maxLevels?: number;
  /** Optional filter applied to descendant processes. */
  filter?: ProcessFilter;
  /** CPU measurement mode used for cpu_above evaluation. */
  cpuMode?: CpuMode;
  /** Sampling interval in milliseconds (used with monitor mode). */
  sampleDurationMs?: number;
}

/**
 * A single failure in a kill-descendants operation.
 */
export interface KillDescendantsFailure {
  pid: number;
  error: string;
}

/**
 * Result of a kill-descendants operation.
 */
export interface KillDescendantsResult {
  schema_id: string;
  signal_sent: number;
  root_pid: number;
  succeeded: number[];
  failed: KillDescendantsFailure[];
  skipped_safety: number;
}
