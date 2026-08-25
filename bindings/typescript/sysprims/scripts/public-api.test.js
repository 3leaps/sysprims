const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const contract = require("./public-api-contract.json");
const {
  emitDeclarations,
  inspectPublicExports,
  normalizeLineEndings,
  renderDocument,
} = require("./public-api");

test("matrix content is stable across checkout line endings", () => {
  const markdown = "# Matrix\n\n| Surface | Policy |\n| --- | --- |\n";
  assert.equal(normalizeLineEndings(markdown.replaceAll("\n", "\r\n")), markdown);
});

test("generated API changes when an exported member or union changes", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sysprims-api-shape-"));
  try {
    emitDeclarations(tempRoot);
    const baseline = inspectPublicExports(tempRoot);
    const baselineDocument = renderDocument(baseline, Object.keys(contract.cAbi).sort());
    const typesPath = path.join(tempRoot, "types.d.ts");
    const declarations = fs.readFileSync(typesPath, "utf8");
    const changed = declarations
      .replace("includeEnv?: boolean;", "includeEnv?: string;")
      .replace(
        'export type FdKind = "file" | "socket" | "pipe" | "unknown";',
        'export type FdKind = "file" | "socket";',
      );
    assert.notEqual(changed, declarations, "test mutation must change emitted declarations");
    fs.writeFileSync(typesPath, changed);

    const mutated = inspectPublicExports(tempRoot);
    const mutatedDocument = renderDocument(mutated, Object.keys(contract.cAbi).sort());
    assert.notEqual(mutated.types.get("ProcessOptions"), baseline.types.get("ProcessOptions"));
    assert.notEqual(mutated.types.get("FdKind"), baseline.types.get("FdKind"));
    assert.notEqual(mutatedDocument, baselineDocument);
  } finally {
    fs.rmSync(tempRoot, { force: true, recursive: true });
  }
});
