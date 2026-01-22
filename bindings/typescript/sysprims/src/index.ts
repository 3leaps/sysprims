import { SysprimsError, SysprimsErrorCode } from "./errors";
import { callJsonReturn, callU32Out, loadSysprims } from "./ffi";
import type { ProcessInfo } from "./types";

export { SysprimsError, SysprimsErrorCode };

export function procGet(pid: number): ProcessInfo {
  const lib = loadSysprims();
  const result = callJsonReturn((out) => lib.sysprims_proc_get(pid >>> 0, out), lib);
  return result as ProcessInfo;
}

export function selfPGID(): number {
  const lib = loadSysprims();
  return callU32Out((out) => lib.sysprims_self_getpgid(out), lib);
}

export function selfSID(): number {
  const lib = loadSysprims();
  return callU32Out((out) => lib.sysprims_self_getsid(out), lib);
}
