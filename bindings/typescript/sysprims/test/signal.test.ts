import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
	MAX_SAFE_PID,
	ProcessHandle,
	SIGALRM,
	SIGHUP,
	SIGINT,
	SIGKILL,
	SIGPIPE,
	SIGQUIT,
	SIGTERM,
	SIGUSR1,
	SIGUSR2,
	SysprimsError,
	SysprimsErrorCode,
	killMany,
	matchSignalNames,
	openProcessHandle,
	openProcessHandleVerified,
	resolveSignalName,
	validatePgid,
	validatePid,
} from "../src/index";
import type { SignalBinding } from "../src/index";

function jcs(value: unknown): string {
	if (Array.isArray(value)) {
		return `[${value.map((item) => jcs(item)).join(",")}]`;
	}
	if (value !== null && typeof value === "object") {
		const obj = value as Record<string, unknown>;
		const keys = Object.keys(obj).sort();
		return `{${keys.map((k) => `${JSON.stringify(k)}:${jcs(obj[k])}`).join(",")}}`;
	}
	return JSON.stringify(value);
}

function errorCodeSlug(err: unknown): string {
	assert.ok(err instanceof SysprimsError, `expected SysprimsError, got ${err}`);
	switch (err.code) {
		case SysprimsErrorCode.Ok:
			return "ok";
		case SysprimsErrorCode.InvalidArgument:
			return "invalid_argument";
		case SysprimsErrorCode.SpawnFailed:
			return "spawn_failed";
		case SysprimsErrorCode.Timeout:
			return "timeout";
		case SysprimsErrorCode.PermissionDenied:
			return "permission_denied";
		case SysprimsErrorCode.NotFound:
			return "not_found";
		case SysprimsErrorCode.NotSupported:
			return "not_supported";
		case SysprimsErrorCode.GroupCreationFailed:
			return "group_creation_failed";
		case SysprimsErrorCode.System:
			return "system";
		case SysprimsErrorCode.Internal:
			return "internal";
		default:
			throw new Error(`unknown error code: ${err.code}`);
	}
}

const SIGNAL_BY_NAME: Record<string, number> = {
	SIGHUP,
	SIGINT,
	SIGQUIT,
	SIGKILL,
	SIGPIPE,
	SIGALRM,
	SIGTERM,
	SIGUSR1,
	SIGUSR2,
};

function signalVectorsPath(): string {
	const override = process.env.SIGNAL_CONFORMANCE_VECTORS;
	if (override && override.length > 0) {
		return override;
	}
	// dist-test/test/signal.test.js -> repo root is 5 dirs up
	return path.resolve(
		__dirname,
		"../../../../../testdata/conformance/signal/vectors.json",
	);
}

test("ProcessHandle lifecycle, verification, non-serializability, and child exitlatch", async () => {
	assert.throws(
		() => new (ProcessHandle as unknown as new () => ProcessHandle)(),
		(err: unknown) =>
			err instanceof SysprimsError &&
			err.code === SysprimsErrorCode.InvalidArgument,
	);

	const selfPid = process.pid;
	const selfHandle = openProcessHandle(selfPid);
	try {
		assert.equal(selfHandle.pid, selfPid);
		assert.ok(selfHandle.startIdentity.length > 0);
		const validBindings: readonly SignalBinding[] = [
			"bound",
			"checked_numeric",
		];
		assert.ok(validBindings.includes(selfHandle.binding));
		assert.equal(selfHandle.exited(), false);
		assert.throws(
			() => JSON.stringify(selfHandle),
			(err: unknown) =>
				err instanceof SysprimsError &&
				err.code === SysprimsErrorCode.InvalidArgument,
		);

		const verified = openProcessHandleVerified(
			selfPid,
			selfHandle.startIdentity,
		);
		try {
			assert.equal(verified.startIdentity, selfHandle.startIdentity);
		} finally {
			verified.close();
		}

		assert.throws(
			() => openProcessHandleVerified(selfPid, "0000000000.999999"),
			(err: unknown) =>
				err instanceof SysprimsError && err.code === SysprimsErrorCode.NotFound,
		);
	} finally {
		selfHandle.close();
	}

	assert.equal(selfHandle.closed, true);
	selfHandle.close(); // idempotent
	assert.throws(
		() => selfHandle.exited(),
		(err: unknown) =>
			err instanceof SysprimsError &&
			err.code === SysprimsErrorCode.InvalidArgument,
	);

	const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 30000)"], {
		stdio: "ignore",
	});
	assert.ok(typeof child.pid === "number" && child.pid > 0);
	const childExit = new Promise<void>((resolve) => {
		child.once("exit", () => resolve());
	});
	const ch = openProcessHandle(child.pid);
	try {
		assert.equal(ch.exited(), false);
		const usedBinding = ch.forceKill();
		assert.equal(usedBinding, ch.binding);
		await childExit;
		assert.equal(ch.exited(), true);
		assert.equal(ch.exited(), true); // latched
		assert.throws(
			() => ch.forceKill(),
			(err: unknown) =>
				err instanceof SysprimsError && err.code === SysprimsErrorCode.NotFound,
		);
	} finally {
		ch.close();
		if (child.exitCode === null && child.signalCode === null) {
			child.kill("SIGKILL");
		}
	}
});

test("signal conformance vectors (4-language parity)", () => {
	const doc = JSON.parse(fs.readFileSync(signalVectorsPath(), "utf8"));
	assert.equal(doc.schema, "sysprims-signal-conformance/v1");

	const ndjsonLines: Array<Record<string, unknown>> = [];

	const constants = doc.constants;
	assert.equal(constants.max_safe_pid, MAX_SAFE_PID);
	assert.equal(constants.signals_common.SIGHUP, SIGHUP);
	assert.equal(constants.signals_common.SIGINT, SIGINT);
	assert.equal(constants.signals_common.SIGQUIT, SIGQUIT);
	assert.equal(constants.signals_common.SIGKILL, SIGKILL);
	assert.equal(constants.signals_common.SIGPIPE, SIGPIPE);
	assert.equal(constants.signals_common.SIGALRM, SIGALRM);
	assert.equal(constants.signals_common.SIGTERM, SIGTERM);
	const platKey = os.platform() === "darwin" ? "darwin" : "other";
	assert.equal(constants.signals_platform[platKey].SIGUSR1, SIGUSR1);
	assert.equal(constants.signals_platform[platKey].SIGUSR2, SIGUSR2);

	ndjsonLines.push({
		suite: "constants",
		id: "signal-constants",
		max_safe_pid: MAX_SAFE_PID,
		sighup: SIGHUP,
		sigint: SIGINT,
		sigquit: SIGQUIT,
		sigkill: SIGKILL,
		sigpipe: SIGPIPE,
		sigalrm: SIGALRM,
		sigterm: SIGTERM,
		sigusr1: SIGUSR1,
		sigusr2: SIGUSR2,
	});

	for (const c of doc.pid_validation_cases) {
		let got = "ok";
		if (c.expected === "ok") {
			if (c.kind === "pid") {
				assert.equal(validatePid(c.value), c.value, c.id);
			} else {
				assert.equal(validatePgid(c.value), c.value, c.id);
			}
		} else {
			try {
				if (c.kind === "pid") {
					validatePid(c.value);
				} else {
					validatePgid(c.value);
				}
				assert.fail(`expected ${c.id} to throw`);
			} catch (err) {
				got = errorCodeSlug(err);
			}
			assert.equal(got, c.expected, c.id);
		}
		ndjsonLines.push({
			suite: "pid-validation",
			id: c.id,
			kind: c.kind,
			outcome: got,
		});
	}

	for (const c of doc.batch_validation_cases) {
		let got = "ok";
		try {
			killMany(c.pids, SIGTERM);
			assert.fail(`expected ${c.id} to throw`);
		} catch (err) {
			got = errorCodeSlug(err);
		}
		assert.equal(got, c.expected, c.id);
		ndjsonLines.push({
			suite: "batch-validation",
			id: c.id,
			outcome: got,
		});
	}

	for (const c of doc.resolve_signal_cases) {
		if (c.expected === "ok") {
			const signum = resolveSignalName(c.input);
			assert.equal(signum, SIGNAL_BY_NAME[c.signal], c.id);
			ndjsonLines.push({
				suite: "resolve-signal",
				id: c.id,
				outcome: "ok",
				signum,
			});
		} else {
			let got = "ok";
			try {
				resolveSignalName(c.input);
				assert.fail(`expected ${c.id} to throw`);
			} catch (err) {
				got = errorCodeSlug(err);
			}
			assert.equal(got, c.expected, c.id);
			ndjsonLines.push({
				suite: "resolve-signal",
				id: c.id,
				outcome: got,
			});
		}
	}

	for (const c of doc.match_signal_cases) {
		const matches = matchSignalNames(c.pattern);
		assert.deepEqual(matches, c.expected, c.id);
		ndjsonLines.push({
			suite: "match-signal",
			id: c.id,
			matches,
		});
	}

	for (const c of doc.process_handle_validation_cases) {
		const pid = c.pid === "self" ? process.pid : c.pid;
		let got = "ok";
		try {
			const h =
				typeof c.expected_start_identity === "string"
					? openProcessHandleVerified(pid, c.expected_start_identity)
					: openProcessHandle(pid);
			h.close();
			assert.fail(`expected ${c.id} to throw`);
		} catch (err) {
			got = errorCodeSlug(err);
		}
		assert.equal(got, c.expected, c.id);
		ndjsonLines.push({
			suite: "process-handle-validation",
			id: c.id,
			outcome: got,
		});
	}

	const outPath = process.env.SIGNAL_NDJSON_OUT;
	if (outPath && outPath.length > 0) {
		fs.writeFileSync(
			outPath,
			ndjsonLines.map((row) => `${jcs(row)}\n`).join(""),
			"utf8",
		);
	}
});
