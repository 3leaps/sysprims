export const SysprimsErrorCode = {
	Ok: 0,
	InvalidArgument: 1,
	SpawnFailed: 2,
	Timeout: 3,
	PermissionDenied: 4,
	NotFound: 5,
	NotSupported: 6,
	GroupCreationFailed: 7,
	System: 8,
	Internal: 99,
} as const;

export type SysprimsErrorCode =
	(typeof SysprimsErrorCode)[keyof typeof SysprimsErrorCode];

const errorCodeNames: Record<number, string> = Object.fromEntries(
	Object.entries(SysprimsErrorCode).map(([k, v]) => [v, k]),
);

export class SysprimsError extends Error {
	public readonly code: SysprimsErrorCode;
	public readonly codeName: string;

	constructor(code: SysprimsErrorCode, message: string) {
		super(message);
		this.name = "SysprimsError";
		this.code = code;
		this.codeName = errorCodeNames[code] ?? `Unknown(${code})`;
	}
}
