import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
	SysprimsError,
	SysprimsErrorCode,
	runNohup,
	runSetsid,
	selfPGID,
	selfSID,
	spawnInGroup,
} from "../src/index";

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

function sessionVectorsPath(): string {
	const override = process.env.SESSION_CONFORMANCE_VECTORS;
	if (override && override.length > 0) {
		return override;
	}
	// dist-test/test/session.test.js -> repo root is 5 dirs up
	return path.resolve(
		__dirname,
		"../../../../../testdata/conformance/session/vectors.json",
	);
}

test("session conformance vectors (4-language parity)", () => {
	const doc = JSON.parse(fs.readFileSync(sessionVectorsPath(), "utf8"));
	assert.equal(doc.schema, "sysprims-session-conformance/v1");

	const ndjsonLines: Array<Record<string, unknown>> = [];
	const schemas = doc.schemas;

	ndjsonLines.push({
		suite: "schemas",
		run_setsid_config_v1: schemas.run_setsid_config_v1,
		run_nohup_config_v1: schemas.run_nohup_config_v1,
		session_spawn_result_v1: schemas.session_spawn_result_v1,
		spawn_in_group_config_v1: schemas.spawn_in_group_config_v1,
		spawn_in_group_result_v1: schemas.spawn_in_group_result_v1,
	});

	for (const c of doc.config_validation_cases) {
		let got = "ok";
		try {
			if (c.verb === "setsid") {
				runSetsid({ schema_id: c.schema_id, argv: c.argv, wait: true });
			} else if (c.verb === "nohup") {
				runNohup({ schema_id: c.schema_id, argv: c.argv, wait: true });
			} else {
				spawnInGroup({ schema_id: c.schema_id, argv: c.argv });
			}
			assert.fail(`expected ${c.id} to throw`);
		} catch (err) {
			got = errorCodeSlug(err);
		}
		assert.equal(got, c.expected, c.id);
		ndjsonLines.push({
			suite: "config-validation",
			id: c.id,
			verb: c.verb,
			outcome: got,
		});
	}

	for (const c of doc.self_introspection_cases) {
		const expected =
			os.platform() === "win32" ? c.expected_windows : c.expected_unix;
		let got = "ok";
		try {
			const val = c.op === "self_getpgid" ? selfPGID() : selfSID();
			if (val <= 0) {
				got = "invalid_value";
			}
		} catch (err) {
			got = errorCodeSlug(err);
		}
		assert.equal(got, expected, c.id);
		ndjsonLines.push({
			suite: "self-introspection",
			id: c.id,
			op: c.op,
			outcome: got,
		});
	}

	for (const c of doc.spawn_cases) {
		const expectedObj =
			os.platform() === "win32" ? c.expected_windows : c.expected_unix;
		const wait = Boolean(c.wait);
		const useTempOutput = Boolean(c.use_temp_output_file);
		const useSymlinkOutput = Boolean(c.use_symlink_output_file);

		if (c.verb === "setsid") {
			let row: Record<string, unknown>;
			try {
				const res = runSetsid({ argv: c.argv, wait });
				assert.equal(res.schema_id, schemas.session_spawn_result_v1);
				const structuralMatch =
					typeof res.pid === "number" &&
					res.pid > 0 &&
					res.pid === res.sid &&
					res.pid === res.pgid;
				row = {
					suite: "spawn",
					id: c.id,
					outcome: "ok",
					verb: res.verb,
					status: res.status,
					session_kind: res.session_kind,
					identifier_provenance: res.identifier_provenance,
					structural_ids_match: structuralMatch,
					exit_code: res.exit_code,
				};
			} catch (err) {
				row = {
					suite: "spawn",
					id: c.id,
					outcome: errorCodeSlug(err),
				};
			}
			for (const [k, v] of Object.entries(expectedObj)) {
				assert.equal(row[k], v, `${c.id}.${k}`);
			}
			ndjsonLines.push(row);
		} else if (c.verb === "nohup") {
			const tempDir = fs.mkdtempSync(
				path.join(os.tmpdir(), "sysprims-session-ts-"),
			);
			let outputFile: string | undefined;
			if (useSymlinkOutput) {
				const targetPath = path.join(tempDir, "target.log");
				const linkPath = path.join(tempDir, "link.log");
				fs.writeFileSync(targetPath, "existing", "utf8");
				if (os.platform() !== "win32") {
					fs.symlinkSync(targetPath, linkPath);
				}
				outputFile = linkPath;
			} else if (useTempOutput) {
				outputFile = path.join(tempDir, "nohup.out");
			}

			const callerSid = os.platform() === "win32" ? 0 : selfSID();
			const callerPgid = os.platform() === "win32" ? 0 : selfPGID();
			let row: Record<string, unknown>;
			try {
				const res = runNohup({ argv: c.argv, wait, output_file: outputFile });
				assert.equal(res.schema_id, schemas.session_spawn_result_v1);
				const inheritedMatch =
					typeof res.pid === "number" &&
					res.pid > 0 &&
					res.sid === callerSid &&
					res.pgid === callerPgid;
				row = {
					suite: "spawn",
					id: c.id,
					outcome: "ok",
					verb: res.verb,
					status: res.status,
					session_kind: res.session_kind,
					identifier_provenance: res.identifier_provenance,
					inherited_context_matches: inheritedMatch,
					exit_code: res.exit_code,
				};
			} catch (err) {
				row = {
					suite: "spawn",
					id: c.id,
					outcome: errorCodeSlug(err),
				};
			} finally {
				fs.rmSync(tempDir, { recursive: true, force: true });
			}
			for (const [k, v] of Object.entries(expectedObj)) {
				assert.equal(row[k], v, `${c.id}.${k}`);
			}
			ndjsonLines.push(row);
		} else if (c.verb === "spawn_in_group") {
			let row: Record<string, unknown>;
			try {
				const res = spawnInGroup({ argv: c.argv });
				assert.equal(res.schema_id, schemas.spawn_in_group_result_v1);
				const structuralMatch =
					typeof res.pid === "number" && res.pid > 0 && res.pgid === res.pid;
				row = {
					suite: "spawn",
					id: c.id,
					outcome: "ok",
					structural_ids_match: structuralMatch,
					tree_kill_reliability: res.tree_kill_reliability,
				};
			} catch (err) {
				row = {
					suite: "spawn",
					id: c.id,
					outcome: errorCodeSlug(err),
				};
			}
			for (const [k, v] of Object.entries(expectedObj)) {
				assert.equal(row[k], v, `${c.id}.${k}`);
			}
			ndjsonLines.push(row);
		}
	}

	const outPath = process.env.SESSION_NDJSON_OUT;
	if (outPath && outPath.length > 0) {
		fs.writeFileSync(
			outPath,
			ndjsonLines.map((row) => `${jcs(row)}\n`).join(""),
			"utf8",
		);
	}
});
