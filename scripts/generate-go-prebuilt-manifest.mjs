#!/usr/bin/env node
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, renameSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const platforms = [
  "darwin-amd64",
  "darwin-arm64",
  "linux-amd64",
  "linux-amd64-musl",
  "linux-arm64",
  "linux-arm64-musl",
  "windows-amd64",
  "windows-arm64",
];

function fail(message) { throw new Error(message); }
function hash(path) { return createHash("sha256").update(readFileSync(path)).digest("hex"); }
function writeJsonAtomic(path, value) {
  const temporary = `${path}.${process.pid}.tmp`;
  writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`);
  renameSync(temporary, path);
}
function filesBelow(path) {
  const files = [];
  for (const name of readdirSync(path)) {
    const child = join(path, name);
    if (statSync(child).isDirectory()) files.push(...filesBelow(child));
    else files.push(child);
  }
  return files;
}

export function generate(root, evidenceDirectory) {
  const version = readFileSync(join(root, "VERSION"), "utf8").trim();
  const planPath = join(root, `docs/releases/v${version}.json`);
  const plan = JSON.parse(readFileSync(planPath, "utf8"));
  if (plan.surfaces?.go?.disposition !== "publish") fail("Go prebuilt generation requires a publish plan");
  if (plan.surfaces.go.lock_phase !== "pre_build") fail("Go prebuilt generation requires go.lock_phase=pre_build");
  if (!existsSync(evidenceDirectory)) fail(`Go smoke evidence directory is missing: ${evidenceDirectory}`);

  const evidenceByPlatform = new Map();
  for (const path of filesBelow(evidenceDirectory).filter((value) => value.endsWith(".json"))) {
    const evidence = JSON.parse(readFileSync(path, "utf8").replace(/^\uFEFF/, ""));
    if (evidence.schema !== "sysprims-go-smoke-evidence/v1") fail(`unknown Go smoke evidence schema in ${path}`);
    if (!platforms.includes(evidence.platform)) fail(`unexpected Go smoke platform ${evidence.platform}`);
    if (evidenceByPlatform.has(evidence.platform)) fail(`duplicate Go smoke evidence for ${evidence.platform}`);
    if (evidence.version !== version) fail(`Go smoke ${evidence.platform} reported ${evidence.version}, expected ${version}`);
    if (!Number.isInteger(evidence.ffi_abi_version) || evidence.ffi_abi_version < 1) fail(`Go smoke ${evidence.platform} reported invalid FFI ABI version`);
    evidenceByPlatform.set(evidence.platform, evidence);
  }
  for (const platform of platforms) {
    if (!evidenceByPlatform.has(platform)) fail(`Go smoke evidence is missing for ${platform}`);
  }
  if (evidenceByPlatform.size !== platforms.length) fail("Go smoke evidence set is not exact");
  const abiVersions = new Set([...evidenceByPlatform.values()].map((value) => value.ffi_abi_version));
  if (abiVersions.size !== 1) fail("Go smoke evidence disagrees on FFI ABI version");

  const base = join(root, "bindings/go/sysprims");
  const headerPath = join(base, "include/sysprims.h");
  if (!existsSync(headerPath)) fail("generated Go header is missing");
  const manifest = {
    schema: "sysprims-go-prebuilt-manifest/v1",
    module: "github.com/3leaps/sysprims/bindings/go/sysprims",
    release_version: version,
    ffi_abi_version: [...abiVersions][0],
    required_platforms: platforms,
    header: { path: "include/sysprims.h", sha256: hash(headerPath) },
    platforms: platforms.map((platform) => {
      const relativePath = `lib/${platform}/libsysprims_ffi.a`;
      const path = join(base, relativePath);
      if (!existsSync(path)) fail(`Go prebuilt is missing: ${relativePath}`);
      return {
        platform,
        path: relativePath,
        sha256: hash(path),
        reported_version: evidenceByPlatform.get(platform).version,
      };
    }),
  };
  plan.surfaces.go.lock_phase = "resolved";
  writeJsonAtomic(join(base, "prebuilt-manifest.json"), manifest);
  writeJsonAtomic(planPath, plan);
  return manifest;
}

function argument(argv, name, fallback) {
  const index = argv.indexOf(name);
  return index === -1 ? fallback : argv[index + 1];
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const root = resolve(argument(process.argv.slice(2), "--root", "."));
    const evidence = argument(process.argv.slice(2), "--evidence-dir");
    if (!evidence) fail("--evidence-dir is required");
    const manifest = generate(root, resolve(evidence));
    console.log(`[ok] Go prebuilt manifest generated from ${manifest.platforms.length} smoke receipts`);
  } catch (error) {
    console.error(`[ERROR] ${error.message}`);
    process.exitCode = 1;
  }
}
