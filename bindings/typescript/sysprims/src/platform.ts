import fs from "node:fs";

export type SysprimsPlatformId =
  | "darwin-arm64"
  | "darwin-amd64"
  | "linux-arm64"
  | "linux-amd64"
  | "windows-amd64";

function isMuslLinux(): boolean {
  if (process.platform !== "linux") return false;

  // biome-ignore lint/suspicious/noExplicitAny: process.report is experimental Node.js API
  const report = (process as any).report?.getReport?.();
  const glibcVersion = report?.header?.glibcVersionRuntime;
  if (typeof glibcVersion === "string" && glibcVersion.length > 0) return false;

  try {
    const entries = fs.readdirSync("/lib");
    return entries.some((e) => e.startsWith("ld-musl"));
  } catch {
    return true;
  }
}

export function resolvePlatformId(): SysprimsPlatformId {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "darwin") {
    if (arch === "arm64") return "darwin-arm64";
    if (arch === "x64") return "darwin-amd64";
    throw new Error(`Not supported: darwin/${arch}`);
  }

  if (platform === "linux") {
    if (isMuslLinux()) {
      throw new Error("Not supported: linux musl (Alpine). Use a glibc-based distro/image.");
    }
    if (arch === "arm64") return "linux-arm64";
    if (arch === "x64") return "linux-amd64";
    throw new Error(`Not supported: linux/${arch}`);
  }

  if (platform === "win32") {
    if (arch === "x64") return "windows-amd64";
    throw new Error(`Not supported: win32/${arch}`);
  }

  throw new Error(`Not supported: ${platform}/${arch}`);
}

export function sharedLibFilename(platformId: SysprimsPlatformId): string {
  if (platformId.startsWith("windows-")) return "sysprims_ffi.dll";
  if (platformId.startsWith("darwin-")) return "libsysprims_ffi.dylib";
  return "libsysprims_ffi.so";
}
