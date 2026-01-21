import test from "node:test";
import assert from "node:assert/strict";

import {
	procGet,
	selfPGID,
	selfSID,
	SysprimsErrorCode,
	SysprimsError,
} from "../src/index";

test("procGet(process.pid) returns matching pid", () => {
	const info = procGet(process.pid);
	assert.equal(info.pid, process.pid);
});

test("selfPGID/selfSID are > 0 on Unix or NotSupported on Windows", () => {
	if (process.platform === "win32") {
		assert.throws(
			() => selfPGID(),
			(e: unknown) =>
				e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
		);
		assert.throws(
			() => selfSID(),
			(e: unknown) =>
				e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
		);
		return;
	}

	assert.ok(selfPGID() > 0);
	assert.ok(selfSID() > 0);
});
