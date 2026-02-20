import fs from "node:fs";
import path from "node:path";
import { SysprimsError, SysprimsErrorCode } from "./errors";

import { loadNativeBinding } from "./native";

let cached: SysprimsLib | null = null;

export function loadSysprims(): SysprimsLib {
  if (cached) return cached;

  const packageRoot = findPackageRoot(__dirname);

  const api: SysprimsLib = loadNativeBinding(packageRoot);
  cached = api;
  return cached;
}

function findPackageRoot(startDir: string): string {
  let current = startDir;
  for (let i = 0; i < 8; i++) {
    const candidate = path.join(current, "package.json");
    if (fs.existsSync(candidate)) return current;
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
  throw new Error("Could not locate package root (package.json not found)");
}

export type SysprimsCallJsonResult = { code: number; json?: string; message?: string };
export type SysprimsCallU32Result = { code: number; value?: number; message?: string };
export type SysprimsCallVoidResult = { code: number; message?: string };

export type SysprimsLib = {
  sysprimsAbiVersion: () => number;

  // Process inspection
  sysprimsProcGet: (pid: number) => SysprimsCallJsonResult;
  sysprimsProcGetEx: (pid: number, optionsJson: string) => SysprimsCallJsonResult;
  sysprimsProcList: (filterJson: string) => SysprimsCallJsonResult;
  sysprimsProcListEx: (filterJson: string, optionsJson: string) => SysprimsCallJsonResult;
  sysprimsProcListeningPorts: (filterJson: string) => SysprimsCallJsonResult;
  sysprimsProcWaitPid: (pid: number, timeoutMs: number) => SysprimsCallJsonResult;
  sysprimsProcListFds: (pid: number, filterJson: string) => SysprimsCallJsonResult;

  // Descendants
  sysprimsProcDescendants: (
    rootPid: number,
    maxLevels: number,
    filterJson: string,
  ) => SysprimsCallJsonResult;
  sysprimsProcKillDescendants: (
    rootPid: number,
    maxLevels: number,
    signal: number,
    filterJson: string,
  ) => SysprimsCallJsonResult;

  // Self introspection
  sysprimsSelfGetpgid: () => SysprimsCallU32Result;
  sysprimsSelfGetsid: () => SysprimsCallU32Result;

  // Signals
  sysprimsSignalSend: (pid: number, signal: number) => SysprimsCallVoidResult;
  sysprimsSignalSendGroup: (pgid: number, signal: number) => SysprimsCallVoidResult;
  sysprimsTerminate: (pid: number) => SysprimsCallVoidResult;
  sysprimsForceKill: (pid: number) => SysprimsCallVoidResult;

  // Terminate tree
  sysprimsTerminateTree: (pid: number, configJson: string) => SysprimsCallJsonResult;

  // Spawn in group
  sysprimsSpawnInGroup: (configJson: string) => SysprimsCallJsonResult;
};

function raiseSysprimsError(code: number, message?: string): never {
  const codeNameSuffix = ` (code=${code})`;
  throw new SysprimsError(
    code as SysprimsErrorCode,
    message && message.length > 0 ? message : `sysprims error${codeNameSuffix}`,
  );
}

export function callJsonReturn(fn: () => SysprimsCallJsonResult): unknown {
  const r = fn();
  if (r.code !== SysprimsErrorCode.Ok) {
    raiseSysprimsError(r.code, r.message);
  }
  return JSON.parse(r.json as string);
}

export function callU32Out(fn: () => SysprimsCallU32Result): number {
  const r = fn();
  if (r.code !== SysprimsErrorCode.Ok) {
    raiseSysprimsError(r.code, r.message);
  }
  return (r.value as number) >>> 0;
}

export function callVoid(fn: () => SysprimsCallVoidResult): void {
  const r = fn();
  if (r.code !== SysprimsErrorCode.Ok) {
    raiseSysprimsError(r.code, r.message);
  }
}
