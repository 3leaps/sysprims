import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { after, test } from "node:test";
import { generate } from "./generate-go-prebuilt-manifest.mjs";

const roots = [];
const platforms = ["darwin-amd64", "darwin-arm64", "linux-amd64", "linux-amd64-musl", "linux-arm64", "linux-arm64-musl", "windows-amd64", "windows-arm64"];
after(() => roots.forEach((root) => rmSync(root, { recursive: true, force: true })));

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "sysprims-go-manifest-test-"));
  roots.push(root);
  const evidence = join(root, "evidence");
  mkdirSync(join(root, "docs/releases"), { recursive: true });
  mkdirSync(join(root, "bindings/go/sysprims/include"), { recursive: true });
  mkdirSync(evidence);
  writeFileSync(join(root, "VERSION"), "0.2.4\n");
  writeFileSync(join(root, "docs/releases/v0.2.4.json"), `${JSON.stringify({
    schema: "sysprims-release-plan/v1", version: "0.2.4", provenance_policy: "commit_footer",
    surfaces: { go: { disposition: "publish", lock_phase: "pre_build" } },
  }, null, 2)}\n`);
  writeFileSync(join(root, "bindings/go/sysprims/include/sysprims.h"), "header\n");
  for (const platform of platforms) {
    const library = join(root, `bindings/go/sysprims/lib/${platform}/libsysprims_ffi.a`);
    mkdirSync(dirname(library), { recursive: true });
    writeFileSync(library, `${platform}\n`);
    writeFileSync(join(evidence, `${platform}.json`), `${JSON.stringify({
      schema: "sysprims-go-smoke-evidence/v1", platform, version: "0.2.4", ffi_abi_version: 1,
    })}\n`);
  }
  return { root, evidence };
}

test("Go manifest generator consumes the exact observed smoke set", () => {
  const { root, evidence } = fixture();
  const manifest = generate(root, evidence);
  assert.equal(manifest.ffi_abi_version, 1);
  assert.deepEqual(manifest.platforms.map((entry) => entry.reported_version), Array(8).fill("0.2.4"));
  const plan = JSON.parse(readFileSync(join(root, "docs/releases/v0.2.4.json"), "utf8"));
  assert.equal(plan.surfaces.go.lock_phase, "resolved");
});

test("Go manifest generator rejects missing and mismatched smoke evidence", () => {
  const missing = fixture();
  rmSync(join(missing.evidence, "windows-arm64.json"));
  assert.throws(() => generate(missing.root, missing.evidence), /missing for windows-arm64/);

  const mismatch = fixture();
  writeFileSync(join(mismatch.evidence, "linux-amd64.json"), `${JSON.stringify({
    schema: "sysprims-go-smoke-evidence/v1", platform: "linux-amd64", version: "0.2.3", ffi_abi_version: 1,
  })}\n`);
  assert.throws(() => generate(mismatch.root, mismatch.evidence), /reported 0.2.3, expected 0.2.4/);
});

test("Go manifest generator rejects inconsistent observed ABI versions", () => {
  const { root, evidence } = fixture();
  writeFileSync(join(evidence, "darwin-arm64.json"), `${JSON.stringify({
    schema: "sysprims-go-smoke-evidence/v1", platform: "darwin-arm64", version: "0.2.4", ffi_abi_version: 2,
  })}\n`);
  assert.throws(() => generate(root, evidence), /disagrees on FFI ABI version/);
});
