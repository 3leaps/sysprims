#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
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

function fail(message) { throw new Error(message); }

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: "utf8", env: process.env, ...options });
  if (result.error) fail(`cannot run ${command}: ${result.error.message}`);
  return result;
}

function mustRun(command, args, options = {}) {
  const result = run(command, args, options);
  if (result.status !== 0) fail(`${command} ${args.join(" ")} failed:\n${result.stderr}${result.stdout}`);
  return result.stdout;
}

function digest(path, algorithm) { return createHash(algorithm).update(readFileSync(path)).digest(algorithm === "sha512" ? "base64" : "hex"); }
function json(path) { return JSON.parse(readFileSync(path, "utf8")); }

function pack(directory, outputDirectory) {
  const result = JSON.parse(mustRun("npm", ["pack", "--json", "--pack-destination", outputDirectory], { cwd: directory }));
  if (!Array.isArray(result) || result.length !== 1) fail(`npm pack returned an unexpected result for ${directory}`);
  const item = result[0];
  const path = join(outputDirectory, item.filename);
  if (!existsSync(path)) fail(`npm pack did not create ${path}`);
  return {
    name: item.name,
    version: item.version,
    filename: item.filename,
    sha256: digest(path, "sha256"),
    sri: `sha512-${digest(path, "sha512")}`,
    content: (item.files ?? []).map(({ path: name, size, mode }) => ({ path: name, size, mode })).sort((a, b) => a.path.localeCompare(b.path)),
  };
}

export function verifyStage(stageDirectory, expectedTag, expectedCommit) {
  const manifest = json(join(stageDirectory, "manifest.json"));
  if (manifest.schema !== "sysprims-npm-stage/v1") fail("unknown npm stage schema");
  if (manifest.tag !== expectedTag || manifest.commit !== expectedCommit) fail("npm stage tag/commit binding mismatch");
  const expectedNames = [...nativeOrder, "@3leaps/sysprims"];
  if (JSON.stringify(manifest.packages.map((entry) => entry.name)) !== JSON.stringify(expectedNames)) fail("npm stage package order/set mismatch");
  for (const entry of manifest.packages) {
    const path = join(stageDirectory, "packages", entry.filename);
    if (!existsSync(path)) fail(`staged tarball is missing: ${entry.filename}`);
    if (entry.sha256 !== digest(path, "sha256") || entry.sri !== `sha512-${digest(path, "sha512")}`) fail(`staged tarball digest mismatch: ${entry.name}`);
    if (entry.version !== expectedTag.replace(/^v/, "")) fail(`staged package version mismatch: ${entry.name}`);
  }
  return manifest;
}

export function parseRegistryMetadata(value, label = "npm package") {
  const tarball = value?.dist?.tarball ?? value?.["dist.tarball"];
  const integrity = value?.dist?.integrity ?? value?.["dist.integrity"];
  if (!tarball || !integrity) fail(`npm registry metadata is incomplete for ${label}`);
  return { tarball, integrity };
}

function registryState(entry, registryDirectory) {
  if (registryDirectory) {
    const path = join(registryDirectory, entry.filename);
    if (!existsSync(path)) return { state: "absent" };
    return { state: "present", sha256: digest(path, "sha256"), sri: `sha512-${digest(path, "sha512")}` };
  }
  const viewed = run("npm", ["view", `${entry.name}@${entry.version}`, "dist.tarball", "dist.integrity", "--json"]);
  if (viewed.status !== 0) {
    if (/E404|not found/i.test(`${viewed.stderr}${viewed.stdout}`)) return { state: "absent" };
    fail(`npm registry query failed for ${entry.name}@${entry.version}: ${viewed.stderr}${viewed.stdout}`);
  }
  const metadata = JSON.parse(viewed.stdout);
  const parsed = parseRegistryMetadata(metadata, `${entry.name}@${entry.version}`);
  return { state: "present", sri: parsed.integrity, tarball: parsed.tarball };
}

function assertPresentIdentical(entry, state, stageDirectory) {
  if (state.sri !== entry.sri) fail(`hostile same-version registry content for ${entry.name}@${entry.version}`);
  if (state.sha256 && state.sha256 !== entry.sha256) fail(`hostile same-version registry bytes for ${entry.name}@${entry.version}`);
  if (state.tarball) {
    const temporary = join(stageDirectory, `.registry-${entry.filename}`);
    try {
      mustRun("curl", ["-fsSL", "--retry", "3", "-o", temporary, state.tarball]);
      if (digest(temporary, "sha256") !== entry.sha256) fail(`hostile same-version registry bytes for ${entry.name}@${entry.version}`);
    } finally { rmSync(temporary, { force: true }); }
  }
}

export function publishResumably(stageDirectory, manifest, { registryDirectory, publish = true, publishPackage } = {}) {
  const completed = [];
  for (const entry of manifest.packages) {
    if (entry.name === "@3leaps/sysprims" && completed.length !== nativeOrder.length) fail("root package cannot publish before all native packages are verified");
    let state = registryState(entry, registryDirectory);
    if (state.state === "absent") {
      if (!publish) { completed.push({ name: entry.name, state: "absent" }); continue; }
      if (publishPackage) publishPackage(entry, join(stageDirectory, "packages", entry.filename));
      else mustRun("npm", ["publish", join(stageDirectory, "packages", entry.filename), "--access", "public", "--provenance"]);
      state = registryState(entry, registryDirectory);
      if (state.state !== "present") fail(`npm package remained absent after publish: ${entry.name}@${entry.version}`);
    }
    assertPresentIdentical(entry, state, stageDirectory);
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
    const nativeDirectory = resolve(argument(argv, "--native-dir", "npm-packages"));
    const rootDirectory = resolve(argument(argv, "--root-package", "bindings/typescript/sysprims"));
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
    for (const name of names) if (!byName.has(name)) fail(`stage is missing package ${name}`);
    writeFileSync(join(stageDirectory, "manifest.json"), `${JSON.stringify({ schema: "sysprims-npm-stage/v1", tag, commit, packages: names.map((name) => byName.get(name)) }, null, 2)}\n`);
    verifyStage(stageDirectory, tag, commit);
  } else if (command === "verify") {
    verifyStage(stageDirectory, tag, commit);
  } else if (command === "compare" || command === "publish") {
    const manifest = verifyStage(stageDirectory, tag, commit);
    publishResumably(stageDirectory, manifest, { publish: command === "publish" });
  } else fail("usage: npm-release.mjs <stage|verify|compare|publish> --stage DIR --tag vX.Y.Z --commit SHA");
  console.log(`[ok] npm ${command} completed for ${tag} at ${commit}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try { main(); } catch (error) { console.error(`[ERROR] ${error.message}`); process.exitCode = 1; }
}
