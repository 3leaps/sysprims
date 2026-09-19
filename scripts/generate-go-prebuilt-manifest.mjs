#!/usr/bin/env node
import { createHash } from "node:crypto";
import { existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(process.argv[2] ?? ".");
const version = readFileSync(join(root, "VERSION"), "utf8").trim();
const platforms = ["darwin-amd64", "darwin-arm64", "linux-amd64", "linux-amd64-musl", "linux-arm64", "linux-arm64-musl", "windows-amd64", "windows-arm64"];
const base = join(root, "bindings/go/sysprims");
const hash = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const headerPath = join(base, "include/sysprims.h");
if (!existsSync(headerPath)) throw new Error("generated Go header is missing");
const manifest = {
  schema: "sysprims-go-prebuilt-manifest/v1",
  module: "github.com/3leaps/sysprims/bindings/go/sysprims",
  release_version: version,
  ffi_abi_version: 1,
  required_platforms: platforms,
  header: { path: "include/sysprims.h", sha256: hash(headerPath) },
  platforms: platforms.map((platform) => {
    const relative = `lib/${platform}/libsysprims_ffi.a`;
    const path = join(base, relative);
    if (!existsSync(path)) throw new Error(`Go prebuilt is missing: ${relative}`);
    return { platform, path: relative, sha256: hash(path), reported_version: version };
  }),
};
const path = join(base, "prebuilt-manifest.json");
const temporary = `${path}.${process.pid}.tmp`;
writeFileSync(temporary, `${JSON.stringify(manifest, null, 2)}\n`);
renameSync(temporary, path);
const planPath = join(root, `docs/releases/v${version}.json`);
const plan = JSON.parse(readFileSync(planPath, "utf8"));
if (plan.surfaces?.go?.disposition !== "publish") throw new Error("Go prebuilt generation requires a publish plan");
plan.surfaces.go.lock_phase = "resolved";
const planTemporary = `${planPath}.${process.pid}.tmp`;
writeFileSync(planTemporary, `${JSON.stringify(plan, null, 2)}\n`);
renameSync(planTemporary, planPath);
console.log(`[ok] Go prebuilt manifest generated for ${version}`);
