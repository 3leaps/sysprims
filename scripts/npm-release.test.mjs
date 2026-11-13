import assert from "node:assert/strict";
import {
	chmodSync,
	copyFileSync,
	mkdtempSync,
	mkdirSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { after, test } from "node:test";
import { spawnSync } from "node:child_process";
import {
	parseRegistryMetadata,
	publishResumably,
	stagePublished,
	verifyPublishedPypi,
	verifyStage,
} from "./npm-release.mjs";

const roots = [];
after(() => {
	for (const root of roots) rmSync(root, { recursive: true, force: true });
});
const names = [
	"darwin-arm64",
	"linux-arm64-gnu",
	"linux-arm64-musl",
	"linux-x64-gnu",
	"linux-x64-musl",
	"win32-arm64-msvc",
	"win32-x64-msvc",
]
	.map((name) => `@3leaps/sysprims-${name}`)
	.concat("@3leaps/sysprims");

function fixture() {
	const root = mkdtempSync(join(tmpdir(), "sysprims-npm-stage-test-"));
	roots.push(root);
	const stage = join(root, "stage");
	const packages = join(stage, "packages");
	const registry = join(root, "registry");
	mkdirSync(packages, { recursive: true });
	mkdirSync(registry);
	const entries = names.map((name, index) => {
		const filename = `package-${index}.tgz`;
		const bytes = Buffer.from(`${name}@0.2.3\n`);
		writeFileSync(join(packages, filename), bytes);
		return {
			name,
			version: "0.2.3",
			filename,
			sha256: createHash("sha256").update(bytes).digest("hex"),
			sri: `sha512-${createHash("sha512").update(bytes).digest("base64")}`,
			content: [
				{ path: "package/package.json", size: bytes.length, mode: 420 },
			],
		};
	});
	const manifest = {
		schema: "sysprims-npm-stage/v1",
		tag: "v0.2.3",
		commit: "a".repeat(40),
		packages: entries,
	};
	writeFileSync(
		join(stage, "manifest.json"),
		`${JSON.stringify(manifest, null, 2)}\n`,
	);
	return { stage, registry, manifest };
}

for (let completed = 1; completed <= 6; completed += 1) {
	test(`npm publish resumes safely after ${completed} native packages`, () => {
		const { stage, registry, manifest } = fixture();
		for (const entry of manifest.packages.slice(0, completed))
			copyFileSync(
				join(stage, "packages", entry.filename),
				join(registry, entry.filename),
			);
		const published = [];
		const result = publishResumably(stage, manifest, {
			registryDirectory: registry,
			publishPackage(entry, path) {
				published.push(entry.name);
				copyFileSync(path, join(registry, entry.filename));
			},
		});
		assert.equal(result.length, 8);
		assert.deepEqual(
			published,
			manifest.packages.slice(completed).map((entry) => entry.name),
		);
		assert.equal(published.at(-1), "@3leaps/sysprims");
	});
}

test("npm publish replay accepts an already complete identical registry", () => {
	const { stage, registry, manifest } = fixture();
	for (const entry of manifest.packages)
		copyFileSync(
			join(stage, "packages", entry.filename),
			join(registry, entry.filename),
		);
	let calls = 0;
	publishResumably(stage, manifest, {
		registryDirectory: registry,
		publishPackage() {
			calls += 1;
		},
	});
	assert.equal(calls, 0);
});

test("npm dry-run comparison never publishes", () => {
	const { stage, registry, manifest } = fixture();
	const result = publishResumably(stage, manifest, {
		registryDirectory: registry,
		publish: false,
	});
	assert.equal(
		result.every((entry) => entry.state === "absent"),
		true,
	);
});

test("npm publish rejects hostile same-version content", () => {
	const { stage, registry, manifest } = fixture();
	for (const entry of manifest.packages.slice(0, 2))
		copyFileSync(
			join(stage, "packages", entry.filename),
			join(registry, entry.filename),
		);
	writeFileSync(
		join(registry, manifest.packages[2].filename),
		"different bytes\n",
	);
	assert.throws(
		() => publishResumably(stage, manifest, { registryDirectory: registry }),
		/hostile same-version/,
	);
});

test("npm stage is bound to the exact tag and commit", () => {
	const { stage } = fixture();
	assert.equal(verifyStage(stage, "v0.2.3", "a".repeat(40)).packages.length, 8);
	assert.throws(
		() => verifyStage(stage, "v0.2.3", "b".repeat(40)),
		/tag\/commit binding mismatch/,
	);
});

test("npm registry parser accepts the dotted keys returned by npm view", () => {
	assert.deepEqual(
		parseRegistryMetadata(
			{
				"dist.tarball": "https://registry.example/package.tgz",
				"dist.integrity": "sha512-example",
			},
			"fixture",
		),
		{
			tarball: "https://registry.example/package.tgz",
			integrity: "sha512-example",
		},
	);
	assert.throws(
		() => parseRegistryMetadata({ "dist.tarball": "only-one" }, "fixture"),
		/metadata is incomplete/,
	);
});

test("npm live registry path consumes dotted npm-view metadata", () => {
	const { stage, manifest } = fixture();
	const bin = join(dirname(stage), "bin");
	mkdirSync(bin);
	const mapping = Object.fromEntries(
		manifest.packages.map((entry) => [
			`${entry.name}@${entry.version}`,
			{ path: join(stage, "packages", entry.filename), integrity: entry.sri },
		]),
	);
	const mappingPath = join(dirname(stage), "mapping.json");
	writeFileSync(mappingPath, JSON.stringify(mapping));
	writeFileSync(
		join(bin, "npm"),
		`#!/usr/bin/env node
const map = require(process.env.FAKE_NPM_MAPPING);
const item = map[process.argv[3]];
if (!item) process.exit(44);
console.log(JSON.stringify({"dist.tarball": "file://" + item.path, "dist.integrity": item.integrity}));
`,
	);
	writeFileSync(
		join(bin, "curl"),
		`#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
const out = args[args.indexOf("-o") + 1];
const source = args.at(-1).slice("file://".length);
fs.copyFileSync(source, out);
`,
	);
	chmodSync(join(bin, "npm"), 0o755);
	chmodSync(join(bin, "curl"), 0o755);
	const previous = {
		path: process.env.PATH,
		mapping: process.env.FAKE_NPM_MAPPING,
	};
	process.env.PATH = `${bin}:${previous.path}`;
	process.env.FAKE_NPM_MAPPING = mappingPath;
	try {
		const result = publishResumably(stage, manifest, { publish: false });
		assert.equal(
			result.every((entry) => entry.state === "identical"),
			true,
		);
	} finally {
		process.env.PATH = previous.path;
		if (previous.mapping === undefined) delete process.env.FAKE_NPM_MAPPING;
		else process.env.FAKE_NPM_MAPPING = previous.mapping;
	}
});

test("npm publish polls past eventual-consistency lag before verifying bytes", () => {
	const { stage, registry, manifest } = fixture();
	for (const entry of manifest.packages.slice(1))
		copyFileSync(
			join(stage, "packages", entry.filename),
			join(registry, entry.filename),
		);
	const sleeps = [];
	let postPublishReads = 0;
	let publishedFirst = false;
	const result = publishResumably(stage, manifest, {
		registryDirectory: registry,
		pollAttempts: 5,
		pollSleepMs: 250,
		sleepFn(ms) {
			sleeps.push(ms);
		},
		publishPackage(entry, path) {
			assert.equal(entry.name, manifest.packages[0].name);
			publishedFirst = true;
			copyFileSync(path, join(registry, entry.filename));
		},
		queryState(entry) {
			if (entry.name === manifest.packages[0].name) {
				if (!publishedFirst) return { state: "absent" };
				postPublishReads += 1;
				if (postPublishReads < 3) return { state: "absent" };
			}
			return { state: "present", sha256: entry.sha256, sri: entry.sri };
		},
	});
	assert.equal(result.length, 8);
	assert.equal(postPublishReads, 3);
	assert.deepEqual(sleeps, [250, 250]);
});

test("npm publish fails after exhausting bounded post-publish poll attempts", () => {
	const { stage, manifest } = fixture();
	const sleeps = [];
	assert.throws(
		() =>
			publishResumably(stage, manifest, {
				pollAttempts: 3,
				pollSleepMs: 100,
				sleepFn(ms) {
					sleeps.push(ms);
				},
				publishPackage() {},
				queryState() {
					return { state: "absent" };
				},
			}),
		/remained absent after publish \(3 poll attempts\)/,
	);
	assert.deepEqual(sleeps, [100, 100]);
});

test("npm publish fails when post-publish polling exceeds overall deadline", () => {
	const { stage, manifest } = fixture();
	let clock = 1000;
	assert.throws(
		() =>
			publishResumably(stage, manifest, {
				pollAttempts: 50,
				pollSleepMs: 100,
				pollDeadlineMs: 150,
				nowFn() {
					return clock;
				},
				sleepFn(ms) {
					clock += ms;
				},
				publishPackage() {},
				queryState() {
					clock += 60;
					return { state: "absent" };
				},
			}),
		/remained absent past 150ms deadline/,
	);
});

test("npm publish rejects late present query that overruns post-publish deadline", () => {
	const { stage, manifest } = fixture();
	let clock = 1000;
	let publishedFirst = false;
	const seenTimeouts = [];
	assert.throws(
		() =>
			publishResumably(stage, manifest, {
				pollAttempts: 5,
				pollSleepMs: 10,
				pollDeadlineMs: 20,
				nowFn() {
					return clock;
				},
				sleepFn(ms) {
					clock += ms;
				},
				publishPackage() {
					publishedFirst = true;
				},
				queryState(entry, _registryDir, options) {
					seenTimeouts.push(options?.timeoutMs);
					if (!publishedFirst) return { state: "absent" };
					clock += 150;
					return {
						state: "present",
						sha256: entry.sha256,
						sri: entry.sri,
					};
				},
			}),
		/exceeded 20ms deadline/,
	);
	assert.deepEqual(seenTimeouts, [20, 20]);
});

test("npm live registry query kills hung npm view subprocess at deadline", () => {
	const { stage, manifest } = fixture();
	const bin = join(dirname(stage), "hung-bin");
	mkdirSync(bin);
	writeFileSync(
		join(bin, "npm"),
		`#!/usr/bin/env node
if (process.argv[2] === "view") {
	Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 2000);
}
process.exit(44);
`,
	);
	chmodSync(join(bin, "npm"), 0o755);
	const previousPath = process.env.PATH;
	process.env.PATH = `${bin}:${previousPath}`;
	const started = Date.now();
	try {
		assert.throws(
			() =>
				publishResumably(stage, manifest, {
					publish: false,
					pollDeadlineMs: 120,
				}),
			/timed out|ETIMEDOUT/,
		);
		assert.equal(Date.now() - started < 1000, true);
	} finally {
		process.env.PATH = previousPath;
	}
});

test("npm default pollAttempts allows up to 36 attempts (180s CDN propagation)", () => {
	const { stage, manifest } = fixture();
	let queries = 0;
	let clock = 0;
	const completed = publishResumably(stage, manifest, {
		publishPackage: () => {},
		sleepFn: () => {
			clock += 5000;
		},
		nowFn: () => clock,
		queryState: (entry) => {
			if (entry.name !== "@3leaps/sysprims-darwin-arm64") {
				return { state: "present", sha256: entry.sha256, sri: entry.sri };
			}
			queries += 1;
			// Initial pre-publish check (1) + 20 post-publish poll attempts (> legacy 12 limit)
			if (queries <= 15) return { state: "absent" };
			return { state: "present", sha256: entry.sha256, sri: entry.sri };
		},
	});
	assert.equal(completed.length, 8);
});

function formatAnchor(tag, object, commit, files) {
	return [
		`tag=${tag}`,
		`object=${object}`,
		`commit=${commit}`,
		...files.map(({ name, sha256 }) => `file=${sha256} ${name}`),
		"",
	].join("\n");
}

test("stagePublished builds verified zero-rebuild stage from signed npm tarballs", () => {
	const root = mkdtempSync(join(tmpdir(), "sysprims-npm-published-"));
	roots.push(root);
	const published = join(root, "published-npm");
	const stage = join(root, "npm-stage");
	mkdirSync(published, { recursive: true });
	const tag = "v1.2.3";
	const object = "abcdef0123456789abcdef0123456789abcdef01";
	const commit = "0123456789abcdef0123456789abcdef01234567";
	const files = [];
	const createTgz = [
		"import io, json, sys, tarfile",
		"out_path, pkg_name, pkg_version = sys.argv[1], sys.argv[2], sys.argv[3]",
		"payload = json.dumps({'name': pkg_name, 'version': pkg_version}).encode('utf-8')",
		"with tarfile.open(out_path, 'w:gz') as tf:",
		"    info = tarfile.TarInfo('package/package.json')",
		"    info.size = len(payload)",
		"    info.mode = 0o644",
		"    tf.addfile(info, io.BytesIO(payload))",
	].join("\n");
	for (const name of names) {
		const filename = `${name.replace("@", "").replace("/", "-")}-1.2.3.tgz`;
		const tgzPath = join(published, filename);
		const res = spawnSync(
			"python3",
			["-I", "-c", createTgz, tgzPath, name, "1.2.3"],
			{ encoding: "utf8" },
		);
		assert.equal(res.status, 0, res.stderr);
		const sha256 = createHash("sha256")
			.update(readFileSync(tgzPath))
			.digest("hex");
		files.push({ name: filename, sha256 });
	}
	const anchorPath = join(root, "published-npm.anchor");
	writeFileSync(anchorPath, formatAnchor(tag, object, commit, files));
	const manifest = stagePublished(published, anchorPath, stage, tag, commit);
	assert.equal(manifest.packages.length, 8);
	assert.equal(manifest.packages[7].name, "@3leaps/sysprims");

	// Path traversal in manifest filename is rejected by verifyStage
	const tamperedManifest = structuredClone(manifest);
	tamperedManifest.packages[0].filename = "../outside.tgz";
	writeFileSync(
		join(stage, "manifest.json"),
		`${JSON.stringify(tamperedManifest, null, 2)}\n`,
	);
	assert.throws(
		() => verifyStage(stage, tag, commit),
		/invalid staged tarball filename/,
	);

	// Mutating a published tarball fails closed against the anchor
	writeFileSync(join(published, files[0].name), "corrupted\n");
	assert.throws(
		() => stagePublished(published, anchorPath, stage, tag, commit),
		/digest mismatch/,
	);
});

test("verifyPublishedPypi enforces exact 5 PYTHON_WHEEL_TAGS + 1 sdist and anchor digests", () => {
	const root = mkdtempSync(join(tmpdir(), "sysprims-pypi-published-"));
	roots.push(root);
	const published = join(root, "published-pypi");
	mkdirSync(published, { recursive: true });
	const tag = "v1.2.3";
	const object = "abcdef0123456789abcdef0123456789abcdef01";
	const commit = "0123456789abcdef0123456789abcdef01234567";
	const distNames = [
		"sysprims-1.2.3-cp310-abi3-macosx_11_0_arm64.whl",
		"sysprims-1.2.3-cp310-abi3-macosx_11_0_x86_64.whl",
		"sysprims-1.2.3-cp310-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
		"sysprims-1.2.3-cp310-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
		"sysprims-1.2.3-cp310-abi3-win_amd64.whl",
		"sysprims-1.2.3.tar.gz",
	];
	const files = distNames.map((name) => {
		const p = join(published, name);
		writeFileSync(p, `payload-${name}\n`);
		const sha256 = createHash("sha256").update(readFileSync(p)).digest("hex");
		return { name, sha256 };
	});
	const anchorPath = join(root, "published-pypi.anchor");
	writeFileSync(anchorPath, formatAnchor(tag, object, commit, files));
	assert.doesNotThrow(() =>
		verifyPublishedPypi(published, anchorPath, tag, commit),
	);
	writeFileSync(join(published, distNames[0]), "tampered\n");
	assert.throws(
		() => verifyPublishedPypi(published, anchorPath, tag, commit),
		/digest mismatch/,
	);
});
