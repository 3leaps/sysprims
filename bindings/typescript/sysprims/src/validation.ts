import { SysprimsError, SysprimsErrorCode } from "./errors";
import type { ProcessFilter } from "./types";

export const MAX_SAFE_PID = 0x7fffffff;
export const U32_MAX = 0xffffffff;
const I32_MIN = -0x80000000;
const I32_MAX = 0x7fffffff;

function invalid(message: string): never {
  throw new SysprimsError(SysprimsErrorCode.InvalidArgument, message);
}

function validateInteger(value: number, name: string, min: number, max: number): number {
  if (!Number.isFinite(value) || !Number.isInteger(value)) {
    invalid(`${name} must be a finite integer`);
  }
  if (value < min || value > max) {
    invalid(`${name} must be between ${min} and ${max}`);
  }
  return value;
}

export function validatePid(value: number, name = "pid"): number {
  return validateInteger(value, name, 1, MAX_SAFE_PID);
}

export function validateSignal(value: number, name = "signal"): number {
  return validateInteger(value, name, I32_MIN, I32_MAX);
}

export function validateU32(value: number, name: string): number {
  return validateInteger(value, name, 0, U32_MAX);
}

export function validateMaxLevels(value: number | null | undefined): number {
  if (value == null || value === Infinity) {
    return U32_MAX;
  }
  return validateU32(value, "maxLevels");
}

export function validateDuration(
  value: number,
  name: string,
  max = Number.MAX_SAFE_INTEGER,
): number {
  return validateInteger(value, name, 0, max);
}

export function validatePort(value: number, name = "local_port"): number {
  return validateInteger(value, name, 1, 65535);
}

export function validateProcessFilter(filter?: ProcessFilter, name = "filter"): void {
  if (!filter) return;

  if (filter.pid_in !== undefined) {
    if (!Array.isArray(filter.pid_in)) {
      invalid(`${name}.pid_in must be an array`);
    }
    for (const pid of filter.pid_in) {
      validatePid(pid, `${name}.pid_in entry`);
    }
  }
  if (filter.ppid !== undefined) validatePid(filter.ppid, `${name}.ppid`);
  if (filter.cpu_above !== undefined) {
    const cpu = filter.cpu_above;
    if (!Number.isFinite(cpu) || cpu < 0 || cpu > 100) {
      invalid(`${name}.cpu_above must be a finite number between 0 and 100`);
    }
  }
  if (filter.memory_above_kb !== undefined) {
    validateDuration(filter.memory_above_kb, `${name}.memory_above_kb`);
  }
  if (filter.running_for_at_least_secs !== undefined) {
    validateDuration(filter.running_for_at_least_secs, `${name}.running_for_at_least_secs`);
  }
}
