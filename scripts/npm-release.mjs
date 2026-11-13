#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
	copyFileSync,
	existsSync,
	mkdirSync,
	readFileSync,
	readdirSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";

const nativeOrder = [
	"@3leaps/sysprims-darwin-arm64",
	"@3leaps/sysprims-linux-arm64-gnu",
	"@3leaps/sysprims-linux-arm64-musl",
	"@3leaps/sysprims-linux-x64-gnu",
	"@3leaps/sysprims-linux-x64-musl",
	"@3leaps/sysprims-win32-arm64-msvc",
	"@3leaps/sysprims-win32-x64-msvc",
];

function fail(message) {
	throw new Error(message);
}

function run(command, args, options = {}) {
	const result = spawnSync(command, args, {
		encoding: "utf8",
		env: process.env,
		...options,
	});
	if (result.error) fail(`cannot run ${command}: ${result.error.message}`);
	return result;
}

function mustRun(command, args, options = {}) {
	const result = run(command, args, options);
	if (result.status !== 0)
		fail(
			`${command} ${args.join(" ")} failed:\n${result.stderr}${result.stdout}`,
		);
	return result.stdout;
}

function digest(path, algorithm) {
	return createHash(algorithm)
		.update(readFileSync(path))
		.digest(algorithm === "sha512" ? "base64" : "hex");
}
function json(path) {
	return JSON.parse(readFileSync(path, "utf8"));
}

function pack(directory, outputDirectory) {
	const result = JSON.parse(
		mustRun(
			"npm",
			[
				"pack",
				"--json",
				"--ignore-scripts",
				"--pack-destination",
				outputDirectory,
			],
			{ cwd: directory },
		),
	);
	if (!Array.isArray(result) || result.length !== 1)
		fail(`npm pack returned an unexpected result for ${directory}`);
	const item = result[0];
	const path = join(outputDirectory, item.filename);
	if (!existsSync(path)) fail(`npm pack did not create ${path}`);
	return {
		name: item.name,
		version: item.version,
		filename: item.filename,
		sha256: digest(path, "sha256"),
		sri: `sha512-${digest(path, "sha512")}`,
		content: (item.files ?? [])
			.map(({ path: name, size, mode }) => ({ path: name, size, mode }))
			.sort((a, b) => a.path.localeCompare(b.path)),
	};
}

const SAFE_FILENAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

const PYTHON_WHEEL_TAGS = [
	"cp310-abi3-macosx_11_0_arm64",
	"cp310-abi3-macosx_11_0_x86_64",
	"cp310-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64",
	"cp310-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64",
	"cp310-abi3-win_amd64",
];

function parsePublishedAnchor(anchorPath) {
	if (!existsSync(anchorPath))
		fail(`published anchor is missing: ${anchorPath}`);
	const fields = new Map();
	const files = [];
	const seen = new Set();
	for (const line of readFileSync(anchorPath, "utf8").split("\n")) {
		if (!line) continue;
		const fileMatch = /^file=([0-9a-f]{64}) ([A-Za-z0-9][A-Za-z0-9._-]*)$/.exec(
			line,
		);
		if (fileMatch) {
			const [, sha256, name] = fileMatch;
			if (seen.has(name)) fail(`${anchorPath} lists ${name} twice`);
			seen.add(name);
			files.push({ name, sha256 });
			continue;
		}
		const fieldMatch = /^(tag|object|commit)=(.+)$/.exec(line);
		if (!fieldMatch || fields.has(fieldMatch[1]))
			fail(`${anchorPath} is malformed`);
		fields.set(fieldMatch[1], fieldMatch[2]);
	}
	if (!fields.get("tag") || !/^[0-9a-f]{40}$/.test(fields.get("object") ?? "") || !/^[0-9a-f]{40}$/.test(fields.get("commit") ?? ""))
		fail(`${anchorPath} is missing required tag/object/commit fields`);
	return {
		tag: fields.get("tag"),
		object: fields.get("object"),
		commit: fields.get("commit"),
		files,
	};
}

export function verifyStage(stageDirectory, expectedTag, expectedCommit) {
	const manifest = json(join(stageDirectory, "manifest.json"));
	if (manifest.schema !== "sysprims-npm-stage/v1")
		fail("unknown npm stage schema");
	if (manifest.tag !== expectedTag || manifest.commit !== expectedCommit)
		fail("npm stage tag/commit binding mismatch");
	const expectedNames = [...nativeOrder, "@3leaps/sysprims"];
	if (
		JSON.stringify(manifest.packages.map((entry) => entry.name)) !==
		JSON.stringify(expectedNames)
	)
		fail("npm stage package order/set mismatch");
	for (const entry of manifest.packages) {
		if (!SAFE_FILENAME.test(entry.filename ?? ""))
			fail(`invalid staged tarball filename: ${entry.filename}`);
		const path = join(stageDirectory, "packages", entry.filename);
		if (!existsSync(path)) fail(`staged tarball is missing: ${entry.filename}`);
		if (
			entry.sha256 !== digest(path, "sha256") ||
			entry.sri !== `sha512-${digest(path, "sha512")}`
		)
			fail(`staged tarball digest mismatch: ${entry.name}`);
		if (entry.version !== expectedTag.replace(/^v/, ""))
			fail(`staged package version mismatch: ${entry.name}`);
	}
	return manifest;
}

function inspectTarball(tarballPath) {
	const script = [
		"import json, sys, tarfile",
		"path = sys.argv[1]",
		"pkg = None",
		"files = []",
		"with tarfile.open(path, 'r:gz') as tf:",
		"    for m in tf.getmembers():",
		"        if not m.isfile():",
		"            continue",
		"        rel = m.name[8:] if m.name.startswith('package/') else m.name",
		"        files.append({'path': rel, 'size': m.size, 'mode': m.mode})",
		"        if m.name == 'package/package.json':",
		"            f = tf.extractfile(m)",
		"            pkg = json.load(f) if f is not None else None",
		"if not isinstance(pkg, dict) or not pkg.get('name') or not pkg.get('version'):",
		"    raise SystemExit('missing package/package.json metadata in ' + path)",
		"files.sort(key=lambda item: item['path'])",
		"print(json.dumps({'name': pkg['name'], 'version': pkg['version'], 'files': files}))",
	].join("\n");
	return JSON.parse(mustRun("python3", ["-I", "-c", script, tarballPath]));
}

export function stagePublished(
	publishedDirectory,
	anchorPath,
	stageDirectory,
	expectedTag,
	expectedCommit,
) {
	const anchor = parsePublishedAnchor(anchorPath);
	if (anchor.tag !== expectedTag || anchor.commit !== expectedCommit)
		fail("published npm anchor tag/commit mismatch");
	const expectedNames = [...nativeOrder, "@3leaps/sysprims"];
	if (anchor.files.length !== expectedNames.length)
		fail(`published npm anchor must list ${expectedNames.length} tarballs`);
	const actualFiles = readdirSync(publishedDirectory)
		.filter((name) => !name.startsWith("."))
		.sort();
	const anchoredFiles = anchor.files.map((item) => item.name).sort();
	if (JSON.stringify(actualFiles) !== JSON.stringify(anchoredFiles))
		fail("published npm directory inventory does not match anchor");
	rmSync(stageDirectory, { recursive: true, force: true });
	const packagesDirectory = join(stageDirectory, "packages");
	mkdirSync(packagesDirectory, { recursive: true });
	const byName = new Map();
	for (const item of anchor.files) {
		if (!SAFE_FILENAME.test(item?.name ?? "") || !item?.sha256)
			fail("published npm anchor entry is incomplete");
		const sourcePath = join(publishedDirectory, item.name);
		if (!existsSync(sourcePath))
			fail(`published npm tarball is missing: ${item.name}`);
		const actualSha256 = digest(sourcePath, "sha256");
		if (actualSha256 !== item.sha256)
			fail(`published npm tarball digest mismatch: ${item.name}`);
		const destinationPath = join(packagesDirectory, item.name);
		copyFileSync(sourcePath, destinationPath);
		const inspected = inspectTarball(destinationPath);
		byName.set(inspected.name, {
			name: inspected.name,
			version: inspected.version,
			filename: item.name,
			sha256: actualSha256,
			sri: `sha512-${digest(destinationPath, "sha512")}`,
			content: inspected.files,
		});
	}
	for (const name of expectedNames)
		if (!byName.has(name)) fail(`published stage is missing package ${name}`);
	writeFileSync(
		join(stageDirectory, "manifest.json"),
		`${JSON.stringify(
			{
				schema: "sysprims-npm-stage/v1",
				tag: expectedTag,
				commit: expectedCommit,
				packages: expectedNames.map((name) => byName.get(name)),
			},
			null,
			2,
		)}\n`,
	);
	return verifyStage(stageDirectory, expectedTag, expectedCommit);
}

export function verifyPublishedPypi(
	publishedDirectory,
	anchorPath,
	expectedTag,
	expectedCommit,
) {
	const anchor = parsePublishedAnchor(anchorPath);
	if (anchor.tag !== expectedTag || anchor.commit !== expectedCommit)
		fail("published pypi anchor tag/commit mismatch");
	const expectedVersion = expectedTag.replace(/^v/, "");
	const expectedSet = [
		...PYTHON_WHEEL_TAGS.map((tag) => `sysprims-${expectedVersion}-${tag}.whl`),
		`sysprims-${expectedVersion}.tar.gz`,
	].sort();
	const actualFiles = readdirSync(publishedDirectory)
		.filter((name) => !name.startsWith("."))
		.sort();
	const anchoredFiles = anchor.files.map((item) => item.name).sort();
	if (JSON.stringify(actualFiles) !== JSON.stringify(anchoredFiles))
		fail("published pypi directory inventory does not match anchor");
	if (JSON.stringify(anchoredFiles) !== JSON.stringify(expectedSet))
		fail(
			`published pypi stage does not match expected distribution set for ${expectedTag}`,
		);
	for (const item of anchor.files) {
		if (!SAFE_FILENAME.test(item?.name ?? "") || !item?.sha256)
			fail("published pypi anchor entry is incomplete");
		const sourcePath = join(publishedDirectory, item.name);
		if (!existsSync(sourcePath))
			fail(`published pypi distribution is missing: ${item.name}`);
		if (digest(sourcePath, "sha256") !== item.sha256)
			fail(`published pypi distribution digest mismatch: ${item.name}`);
	}
	return anchor;
}

export function parseRegistryMetadata(value, label = "npm package") {
	const tarball = value?.dist?.tarball ?? value?.["dist.tarball"];
	const integrity = value?.dist?.integrity ?? value?.["dist.integrity"];
	if (!tarball || !integrity)
		fail(`npm registry metadata is incomplete for ${label}`);
	return { tarball, integrity };
}

function registryState(entry, registryDirectory, { timeoutMs } = {}) {
	if (registryDirectory) {
		const path = join(registryDirectory, entry.filename);
		if (!existsSync(path)) return { state: "absent" };
		return {
			state: "present",
			sha256: digest(path, "sha256"),
			sri: `sha512-${digest(path, "sha512")}`,
		};
	}
	const runOptions =
		Number.isFinite(timeoutMs) && timeoutMs > 0
			? { timeout: Math.max(1, Math.floor(timeoutMs)) }
			: {};
	const viewed = run(
		"npm",
		[
			"view",
			`${entry.name}@${entry.version}`,
			"dist.tarball",
			"dist.integrity",
			"--json",
		],
		runOptions,
	);
	if (viewed.signal)
		fail(
			`npm registry query timed out or was terminated (${viewed.signal}) for ${entry.name}@${entry.version}`,
		);
	if (viewed.status !== 0) {
		if (/E404|not found/i.test(`${viewed.stderr}${viewed.stdout}`))
			return { state: "absent" };
		fail(
			`npm registry query failed for ${entry.name}@${entry.version}: ${viewed.stderr}${viewed.stdout}`,
		);
	}
	const metadata = JSON.parse(viewed.stdout);
	const parsed = parseRegistryMetadata(
		metadata,
		`${entry.name}@${entry.version}`,
	);
	return { state: "present", sri: parsed.integrity, tarball: parsed.tarball };
}

const VERIFY_USER_AGENT =
	"sysprims-release-verify/1.0 (+https://github.com/3leaps/sysprims)";

function defaultSleep(ms) {
	if (ms > 0) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function assertPresentIdentical(
	entry,
	state,
	stageDirectory,
	{ timeoutMs } = {},
) {
	if (state.sri !== entry.sri)
		fail(
			`hostile same-version registry content for ${entry.name}@${entry.version}`,
		);
	if (state.sha256 && state.sha256 !== entry.sha256)
		fail(
			`hostile same-version registry bytes for ${entry.name}@${entry.version}`,
		);
	if (state.tarball) {
		const hasTimeout = Number.isFinite(timeoutMs) && timeoutMs > 0;
		const curlMaxSec = hasTimeout
			? Math.max(1, Math.ceil(timeoutMs / 1000))
			: 60;
		const runOptions = hasTimeout
			? { timeout: Math.max(1, Math.floor(timeoutMs)) }
			: {};
		const temporary = join(stageDirectory, `.registry-${entry.filename}`);
		try {
			mustRun(
				"curl",
				[
					"--proto",
					"=https",
					"--tlsv1.2",
					"--max-time",
					String(curlMaxSec),
					"-fsSL",
					"--retry",
					hasTimeout ? "0" : "3",
					"-A",
					VERIFY_USER_AGENT,
					"-o",
					temporary,
					state.tarball,
				],
				runOptions,
			);
			if (digest(temporary, "sha256") !== entry.sha256)
				fail(
					`hostile same-version registry bytes for ${entry.name}@${entry.version}`,
				);
		} finally {
			rmSync(temporary, { force: true });
		}
	}
}

export function publishResumably(
	stageDirectory,
	manifest,
	{
		registryDirectory,
		publish = true,
		publishPackage,
		queryState = registryState,
		sleepFn = defaultSleep,
		nowFn = Date.now,
		pollAttempts = Number.parseInt(
			process.env.SYSPRIMS_NPM_POLL_ATTEMPTS ?? "36",
			10,
		),
		pollSleepMs = Number.parseInt(
			process.env.SYSPRIMS_NPM_POLL_SLEEP_MS ?? "5000",
			10,
		),
		pollDeadlineMs = Number.parseInt(
			process.env.SYSPRIMS_NPM_POLL_DEADLINE_MS ?? "180000",
			10,
		),
	} = {},
) {
	if (!Number.isInteger(pollAttempts) || pollAttempts < 1)
		fail("SYSPRIMS_NPM_POLL_ATTEMPTS must be a positive integer");
	if (!Number.isInteger(pollSleepMs) || pollSleepMs < 0)
		fail("SYSPRIMS_NPM_POLL_SLEEP_MS must be a non-negative integer");
	if (!Number.isInteger(pollDeadlineMs) || pollDeadlineMs < 1)
		fail("SYSPRIMS_NPM_POLL_DEADLINE_MS must be a positive integer");
	const completed = [];
	for (const entry of manifest.packages) {
		if (
			entry.name === "@3leaps/sysprims" &&
			completed.length !== nativeOrder.length
		)
			fail(
				"root package cannot publish before all native packages are verified",
			);
		let state = queryState(entry, registryDirectory, {
			timeoutMs: pollDeadlineMs,
		});
		let verifyDeadline = null;
		if (state.state === "absent") {
			if (!publish) {
				completed.push({ name: entry.name, state: "absent" });
				continue;
			}
			if (publishPackage)
				publishPackage(entry, join(stageDirectory, "packages", entry.filename));
			else
				mustRun("npm", [
					"publish",
					join(stageDirectory, "packages", entry.filename),
					"--access",
					"public",
					"--provenance",
				]);
			const deadline = nowFn() + pollDeadlineMs;
			verifyDeadline = deadline;
			for (let attempt = 1; attempt <= pollAttempts; attempt += 1) {
				const remainingBeforeQuery = deadline - nowFn();
				if (remainingBeforeQuery <= 0)
					fail(
						`npm package remained absent past ${pollDeadlineMs}ms deadline: ${entry.name}@${entry.version}`,
					);
				state = queryState(entry, registryDirectory, {
					timeoutMs: remainingBeforeQuery,
				});
				if (nowFn() > deadline)
					fail(
						`npm package verification exceeded ${pollDeadlineMs}ms deadline: ${entry.name}@${entry.version}`,
					);
				if (state.state === "present") break;
				if (nowFn() >= deadline)
					fail(
						`npm package remained absent past ${pollDeadlineMs}ms deadline: ${entry.name}@${entry.version}`,
					);
				if (attempt < pollAttempts)
					sleepFn(Math.min(pollSleepMs, Math.max(0, deadline - nowFn())));
			}
			if (state.state !== "present")
				fail(
					`npm package remained absent after publish (${pollAttempts} poll attempts): ${entry.name}@${entry.version}`,
				);
		}
		const verifyOptions =
			verifyDeadline !== null
				? { timeoutMs: Math.max(1, verifyDeadline - nowFn()) }
				: { timeoutMs: pollDeadlineMs };
		if (verifyDeadline !== null && nowFn() > verifyDeadline)
			fail(
				`npm package verification exceeded ${pollDeadlineMs}ms deadline: ${entry.name}@${entry.version}`,
			);
		assertPresentIdentical(entry, state, stageDirectory, verifyOptions);
		if (verifyDeadline !== null && nowFn() > verifyDeadline)
			fail(
				`npm package verification exceeded ${pollDeadlineMs}ms deadline: ${entry.name}@${entry.version}`,
			);
		completed.push({ name: entry.name, state: "identical" });
	}
	return completed;
}

function argument(argv, name, fallback) {
	const index = argv.indexOf(name);
	return index === -1 ? fallback : argv[index + 1];
}

function main() {
	const argv = process.argv.slice(2);
	const command = argv[0];
	const stageDirectory = resolve(argument(argv, "--stage", "dist/npm-stage"));
	const tag = argument(argv, "--tag", process.env.GITHUB_REF_NAME);
	const commit = argument(argv, "--commit", process.env.GITHUB_SHA);
	if (!tag || !commit) fail("--tag and --commit are required");
	if (command === "stage") {
		const nativeDirectory = resolve(
			argument(argv, "--native-dir", "npm-packages"),
		);
		const rootDirectory = resolve(
			argument(argv, "--root-package", "bindings/typescript/sysprims"),
		);
		rmSync(stageDirectory, { recursive: true, force: true });
		const packagesDirectory = join(stageDirectory, "packages");
		mkdirSync(packagesDirectory, { recursive: true });
		const byName = new Map();
		for (const child of readdirSync(nativeDirectory)) {
			const directory = join(nativeDirectory, child);
			if (!existsSync(join(directory, "package.json"))) continue;
			const entry = pack(directory, packagesDirectory);
			byName.set(entry.name, entry);
		}
		const root = pack(rootDirectory, packagesDirectory);
		byName.set(root.name, root);
		const names = [...nativeOrder, "@3leaps/sysprims"];
		for (const name of names)
			if (!byName.has(name)) fail(`stage is missing package ${name}`);
		writeFileSync(
			join(stageDirectory, "manifest.json"),
			`${JSON.stringify({ schema: "sysprims-npm-stage/v1", tag, commit, packages: names.map((name) => byName.get(name)) }, null, 2)}\n`,
		);
		verifyStage(stageDirectory, tag, commit);
	} else if (command === "stage-published") {
		const publishedDirectory = resolve(
			argument(argv, "--published-dir", "dist/published-npm"),
		);
		const anchorPath = resolve(
			argument(argv, "--anchor", "dist/published-npm.anchor"),
		);
		stagePublished(publishedDirectory, anchorPath, stageDirectory, tag, commit);
	} else if (command === "verify-pypi") {
		const publishedDirectory = resolve(
			argument(argv, "--published-dir", "dist/published-pypi"),
		);
		const anchorPath = resolve(
			argument(argv, "--anchor", "dist/published-pypi.anchor"),
		);
		verifyPublishedPypi(publishedDirectory, anchorPath, tag, commit);
	} else if (command === "verify") {
		verifyStage(stageDirectory, tag, commit);
	} else if (command === "compare" || command === "publish") {
		const manifest = verifyStage(stageDirectory, tag, commit);
		publishResumably(stageDirectory, manifest, {
			publish: command === "publish",
		});
	} else
		fail(
			"usage: npm-release.mjs <stage|stage-published|verify|verify-pypi|compare|publish> --stage DIR --tag vX.Y.Z --commit SHA",
		);
	console.log(`[ok] npm ${command} completed for ${tag} at ${commit}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
	try {
		main();
	} catch (error) {
		console.error(`[ERROR] ${error.message}`);
		process.exitCode = 1;
	}
}
