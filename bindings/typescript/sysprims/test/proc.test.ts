import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

import {
	type DescendantsResult,
	type ProcessFilter,
	type ProcessInfo,
	SysprimsError,
	SysprimsErrorCode,
	ancestors,
	descendants,
	guardStep,
	listeningPorts,
	procGet,
	processList,
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

function procVectorsPath(): string {
	const override = process.env.PROC_CONFORMANCE_VECTORS;
	if (override && override.length > 0) {
		return override;
	}
	return path.resolve(
		__dirname,
		"../../../../../testdata/conformance/proc/vectors.json",
	);
}

function matchesFilter(proc: ProcessInfo, filter?: ProcessFilter): boolean {
	if (!filter) return true;
	if (filter.name_contains !== undefined) {
		if (!proc.name.toLowerCase().includes(filter.name_contains.toLowerCase())) {
			return false;
		}
	}
	if (filter.name_equals !== undefined && proc.name !== filter.name_equals) {
		return false;
	}
	if (filter.user_equals !== undefined && proc.user !== filter.user_equals) {
		return false;
	}
	if (filter.pid_in !== undefined && filter.pid_in.length > 0) {
		if (!filter.pid_in.includes(proc.pid)) {
			return false;
		}
	}
	if (filter.ppid !== undefined && proc.ppid !== filter.ppid) {
		return false;
	}
	if (filter.state_in !== undefined && filter.state_in.length > 0) {
		if (!filter.state_in.includes(proc.state)) {
			return false;
		}
	}
	if (
		filter.cpu_above !== undefined &&
		!(proc.cpu_percent > filter.cpu_above)
	) {
		return false;
	}
	if (
		filter.memory_above_kb !== undefined &&
		!(proc.memory_kb > filter.memory_above_kb)
	) {
		return false;
	}
	if (
		filter.running_for_at_least_secs !== undefined &&
		!(proc.elapsed_seconds >= filter.running_for_at_least_secs)
	) {
		return false;
	}
	return true;
}

function selectDescendantTargetsTS(
	tree: DescendantsResult,
	filter: ProcessFilter | undefined,
	cascade: boolean,
): number[] {
	const matchedRoots: number[] = [];
	const childrenMap = new Map<number, number[]>();

	for (const lvl of tree.levels) {
		for (const p of lvl.processes) {
			const list = childrenMap.get(p.ppid) ?? [];
			list.push(p.pid);
			childrenMap.set(p.ppid, list);
			if (matchesFilter(p, filter)) {
				matchedRoots.push(p.pid);
			}
		}
	}

	const selected = new Set<number>();
	for (const pid of matchedRoots) {
		if (pid !== tree.root_pid) {
			selected.add(pid);
		}
	}

	if (cascade) {
		const stack = [...matchedRoots];
		const visited = new Set<number>();
		while (stack.length > 0) {
			const cur = stack.pop();
			if (cur === undefined || visited.has(cur)) continue;
			visited.add(cur);
			if (cur !== tree.root_pid) {
				selected.add(cur);
			}
			for (const child of childrenMap.get(cur) ?? []) {
				if (!visited.has(child)) {
					stack.push(child);
				}
			}
		}
	}

	return Array.from(selected).sort((a, b) => a - b);
}

const GUARD_PRESETS: Record<
	string,
	{ interval_ms: number; sample_duration_ms: number }
> = {
	interactive: { interval_ms: 3000, sample_duration_ms: 2000 },
	background: { interval_ms: 180000, sample_duration_ms: 3000 },
	watchdog: { interval_ms: 300000, sample_duration_ms: 5000 },
};

test("proc conformance vectors (4-language parity)", () => {
	const doc = JSON.parse(fs.readFileSync(procVectorsPath(), "utf8"));
	assert.equal(doc.suite, "sysprims-proc-conformance");

	const ndjsonLines: Array<Record<string, unknown>> = [];

	ndjsonLines.push({
		suite: "schemas",
		process_info_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info.schema.json",
		process_info_sampled_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info-sampled.schema.json",
		descendants_result_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.0.0/descendants-result.schema.json",
		descendants_result_sampled_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.1.0/descendants-result-sampled.schema.json",
		ancestors_result_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.0.0/ancestors-result.schema.json",
		wait_pid_result_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.0.0/wait-pid-result.schema.json",
		fd_snapshot_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json",
		port_bindings_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-bindings.schema.json",
		port_filter_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-filter.schema.json",
		guard_event_v1:
			"https://schemas.3leaps.dev/sysprims/process/v1.0.0/guard-event.schema.json",
	});

	const syntheticTree = doc.synthetic_tree as DescendantsResult;

	for (const v of doc.vectors) {
		if (v.kind === "validate_pid") {
			let slug = "ok";
			try {
				procGet(v.pid);
				assert.fail(`expected ${v.id} to throw`);
			} catch (err) {
				slug = errorCodeSlug(err);
			}
			assert.equal(slug, v.expected_error_category, v.id);
			ndjsonLines.push({
				id: v.id,
				kind: v.kind,
				outcome: slug,
			});
		} else if (v.kind === "validate_filter") {
			let slug = "ok";
			try {
				processList(v.filter);
				assert.fail(`expected ${v.id} to throw`);
			} catch (err) {
				slug = errorCodeSlug(err);
			}
			assert.equal(slug, v.expected_error_category, v.id);
			ndjsonLines.push({
				id: v.id,
				kind: v.kind,
				outcome: slug,
			});
		} else if (v.kind === "validate_port_filter") {
			let slug = "ok";
			try {
				listeningPorts({ protocol: "tcp", local_port: v.local_port });
				assert.fail(`expected ${v.id} to throw`);
			} catch (err) {
				slug = errorCodeSlug(err);
			}
			assert.equal(slug, v.expected_error_category, v.id);
			ndjsonLines.push({
				id: v.id,
				kind: v.kind,
				outcome: slug,
			});
		} else if (v.kind === "guard_preset") {
			const preset = GUARD_PRESETS[v.preset];
			assert.ok(preset, `unknown preset ${v.preset}`);
			assert.equal(preset.interval_ms, v.expected.interval_ms);
			assert.equal(preset.sample_duration_ms, v.expected.sample_duration_ms);
			ndjsonLines.push({
				id: v.id,
				kind: v.kind,
				preset: v.preset,
				interval_ms: preset.interval_ms,
				sample_duration_ms: preset.sample_duration_ms,
			});
		} else if (v.kind === "select_targets") {
			const got = selectDescendantTargetsTS(
				syntheticTree,
				v.filter,
				Boolean(v.cascade),
			);
			assert.deepEqual(got, v.expected_pids, v.id);
			ndjsonLines.push({
				id: v.id,
				kind: v.kind,
				cascade: Boolean(v.cascade),
				matched_pids: got,
			});
		} else if (v.kind === "live_probe_self") {
			const selfPid = process.pid;
			const selfInfo = procGet(selfPid);
			const anc = ancestors(selfPid, { maxDepth: 3 });
			const desc = descendants(selfPid, { maxLevels: 1 });
			const guard = guardStep({
				rule: { root_pid: selfPid, max_levels: 1, cpu_mode: "lifetime" },
				action: { kind: "kill_descendants", signal: 15, cascade: false },
				action_enabled: false,
				max_targets: 8,
			});
			ndjsonLines.push({
				id: v.id,
				kind: v.kind,
				ancestors_chain_non_empty:
					anc.chain.length > 0 && anc.chain[0].pid === selfPid,
				ancestors_schema_id: anc.schema_id,
				descendants_schema_id: desc.schema_id,
				fd_snapshot_schema_id:
					"https://schemas.3leaps.dev/sysprims/process/v1.0.0/fd-snapshot.schema.json",
				guard_event_schema_id: guard.schema_id,
				port_bindings_schema_id:
					"https://schemas.3leaps.dev/sysprims/process/v1.0.0/port-bindings.schema.json",
				process_info_schema_id:
					"https://schemas.3leaps.dev/sysprims/process/v1.1.0/process-info.schema.json",
				self_found: selfInfo.pid === selfPid && selfInfo.name.length > 0,
			});
		} else {
			assert.fail(`unknown vector kind: ${v.kind}`);
		}
	}

	const outPath = process.env.PROC_NDJSON_OUT;
	if (outPath && outPath.length > 0) {
		const content = `${ndjsonLines.map((line) => jcs(line)).join("\n")}\n`;
		fs.writeFileSync(outPath, content, "utf8");
	}
});
