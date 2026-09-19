import assert from "node:assert/strict";
import { copyFileSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { after, test } from "node:test";
import { publishResumably, verifyStage } from "./npm-release.mjs";

const roots = [];
after(() => roots.forEach((root) => rmSync(root, { recursive: true, force: true })));
const names = ["darwin-arm64", "linux-arm64-gnu", "linux-arm64-musl", "linux-x64-gnu", "linux-x64-musl", "win32-arm64-msvc", "win32-x64-msvc"].map((name) => `@3leaps/sysprims-${name}`).concat("@3leaps/sysprims");

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
      content: [{ path: "package/package.json", size: bytes.length, mode: 420 }],
    };
  });
  const manifest = { schema: "sysprims-npm-stage/v1", tag: "v0.2.3", commit: "a".repeat(40), packages: entries };
  writeFileSync(join(stage, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return { stage, registry, manifest };
}

for (let completed = 1; completed <= 6; completed += 1) {
  test(`npm publish resumes safely after ${completed} native packages`, () => {
    const { stage, registry, manifest } = fixture();
    for (const entry of manifest.packages.slice(0, completed)) copyFileSync(join(stage, "packages", entry.filename), join(registry, entry.filename));
    const published = [];
    const result = publishResumably(stage, manifest, {
      registryDirectory: registry,
      publishPackage(entry, path) { published.push(entry.name); copyFileSync(path, join(registry, entry.filename)); },
    });
    assert.equal(result.length, 8);
    assert.deepEqual(published, manifest.packages.slice(completed).map((entry) => entry.name));
    assert.equal(published.at(-1), "@3leaps/sysprims");
  });
}

test("npm publish replay accepts an already complete identical registry", () => {
  const { stage, registry, manifest } = fixture();
  for (const entry of manifest.packages) copyFileSync(join(stage, "packages", entry.filename), join(registry, entry.filename));
  let calls = 0;
  publishResumably(stage, manifest, { registryDirectory: registry, publishPackage() { calls += 1; } });
  assert.equal(calls, 0);
});

test("npm dry-run comparison never publishes", () => {
  const { stage, registry, manifest } = fixture();
  const result = publishResumably(stage, manifest, { registryDirectory: registry, publish: false });
  assert.equal(result.every((entry) => entry.state === "absent"), true);
});

test("npm publish rejects hostile same-version content", () => {
  const { stage, registry, manifest } = fixture();
  for (const entry of manifest.packages.slice(0, 2)) copyFileSync(join(stage, "packages", entry.filename), join(registry, entry.filename));
  writeFileSync(join(registry, manifest.packages[2].filename), "different bytes\n");
  assert.throws(() => publishResumably(stage, manifest, { registryDirectory: registry }), /hostile same-version/);
});

test("npm stage is bound to the exact tag and commit", () => {
  const { stage } = fixture();
  assert.equal(verifyStage(stage, "v0.2.3", "a".repeat(40)).packages.length, 8);
  assert.throws(() => verifyStage(stage, "v0.2.3", "b".repeat(40)), /tag\/commit binding mismatch/);
});
