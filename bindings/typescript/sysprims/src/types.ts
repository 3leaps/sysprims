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
