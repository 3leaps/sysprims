import fs from "node:fs";
import path from "node:path";

import koffi from "koffi";

import { SysprimsError, SysprimsErrorCode } from "./errors";
import { resolvePlatformId, sharedLibFilename } from "./platform";

const EXPECTED_ABI_VERSION = 1;

// biome-ignore lint/suspicious/noExplicitAny: koffi C-ABI pointers are untyped
type KoffiPtr = any;
// biome-ignore lint/suspicious/noExplicitAny: koffi out-params use mutable arrays
type KoffiOutArray = any[];

type SysprimsLib = {
	sysprims_abi_version: () => number;
	sysprims_last_error_code: () => number;
	sysprims_last_error: () => string;
	sysprims_clear_error: () => void;
	sysprims_free_string: (ptr: KoffiPtr) => void;

	sysprims_proc_get: (pid: number, out: KoffiOutArray) => number;
	sysprims_self_getpgid: (out: KoffiOutArray) => number;
	sysprims_self_getsid: (out: KoffiOutArray) => number;
};

function packageRoot(): string {
	let current = __dirname;
	for (let i = 0; i < 6; i++) {
		const candidate = path.join(current, "package.json");
		if (fs.existsSync(candidate)) return current;
		const parent = path.dirname(current);
		if (parent === current) break;
		current = parent;
	}
	throw new Error("Could not locate package root (package.json not found)");
}

function resolveLibraryPath(): string {
	const override = process.env.SYSPRIMS_TS_LIB_PATH;
	if (override && override.length > 0) return override;

	const platformId = resolvePlatformId();
	const filename = sharedLibFilename(platformId);
	const p = path.join(packageRoot(), "_lib", platformId, filename);
	if (!fs.existsSync(p)) {
		throw new Error(
			`Missing sysprims shared library for ${platformId}: ${p}\n` +
				`Populate _lib/<platform>/ with the shared library (CI vendoring), or run:\n` +
				`  make build-local-ffi-shared\n` +
				`  npm run vendor:local`,
		);
	}
	return p;
}

function raiseSysprimsError(lib: SysprimsLib, code: number): never {
	const msg = lib.sysprims_last_error();
	const codeNameSuffix = ` (code=${code})`;
	throw new SysprimsError(
		code as SysprimsErrorCode,
		msg && msg.length > 0 ? msg : `sysprims error${codeNameSuffix}`,
	);
}

let cached: SysprimsLib | null = null;

export function loadSysprims(): SysprimsLib {
	if (cached) return cached;

	const libPath = resolveLibraryPath();
	const lib = koffi.load(libPath);

	const sysprims_free_string = lib.func("void sysprims_free_string(void *s)");
	const SysprimsOwnedStr = koffi.disposable(
		"SysprimsOwnedStr",
		"str",
		sysprims_free_string,
	);
	const SysprimsOwnedStrOut = koffi.out(koffi.pointer(SysprimsOwnedStr)); // SysprimsOwnedStr* == char**
	const u32Out = koffi.out(koffi.pointer("uint32")); // uint32_t*

	const api: SysprimsLib = {
		sysprims_abi_version: lib.func("uint32_t sysprims_abi_version(void)"),
		sysprims_last_error_code: lib.func(
			"int32_t sysprims_last_error_code(void)",
		),
		sysprims_last_error: lib.func("SysprimsOwnedStr sysprims_last_error(void)"),
		sysprims_clear_error: lib.func("void sysprims_clear_error(void)"),
		sysprims_free_string,

		sysprims_proc_get: lib.func("sysprims_proc_get", "int32", [
			"uint32",
			SysprimsOwnedStrOut,
		]),
		sysprims_self_getpgid: lib.func("sysprims_self_getpgid", "int32", [u32Out]),
		sysprims_self_getsid: lib.func("sysprims_self_getsid", "int32", [u32Out]),
	};

	const abi = api.sysprims_abi_version();
	if (abi !== EXPECTED_ABI_VERSION) {
		throw new Error(
			`ABI version mismatch: expected ${EXPECTED_ABI_VERSION}, got ${abi}. ` +
				`Ensure the bundled sysprims shared library matches this package.`,
		);
	}

	cached = api;
	return api;
}

export function callJsonReturn(
	fn: (outPtr: KoffiOutArray) => number,
	lib: SysprimsLib,
): unknown {
	lib.sysprims_clear_error();
	const out: KoffiOutArray = [null];
	const code = fn(out);
	if (code !== SysprimsErrorCode.Ok) {
		raiseSysprimsError(lib, code);
	}

	return JSON.parse(out[0] as string);
}

export function callU32Out(
	fn: (outPtr: KoffiOutArray) => number,
	lib: SysprimsLib,
): number {
	lib.sysprims_clear_error();
	const out: KoffiOutArray = [0];
	const code = fn(out);
	if (code !== SysprimsErrorCode.Ok) {
		raiseSysprimsError(lib, code);
	}

	return out[0] >>> 0;
}
