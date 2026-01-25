export type JsonObject = Record<string, unknown>;

// Process types

export type ProcessState = "running" | "sleeping" | "stopped" | "zombie" | "unknown";

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
  state_in?: ProcessState[];
  cpu_above?: number;
  memory_above_kb?: number;
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
