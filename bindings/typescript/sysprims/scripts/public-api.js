const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const ts = require("typescript");

const packageRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(packageRoot, "../../..");
const contract = require("./public-api-contract.json");
const outputPath = path.join(packageRoot, "docs", "public-api.md");

function fail(message) {
  throw new Error(message);
}

function sorted(value) {
  return [...value].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

function normalizeLineEndings(value) {
  return value.replace(/\r\n?/g, "\n");
}

function compareNames(label, actual, expected) {
  const actualNames = sorted(actual);
  const expectedNames = sorted(expected);
  const missing = expectedNames.filter((name) => !actualNames.includes(name));
  const unexpected = actualNames.filter((name) => !expectedNames.includes(name));
  if (missing.length || unexpected.length) {
    fail(
      `${label} drift${missing.length ? `\n  missing: ${missing.join(", ")}` : ""}${
        unexpected.length ? `\n  unexpected: ${unexpected.join(", ")}` : ""
      }`,
    );
  }
}

function section(markdown, heading) {
  const marker = `## ${heading}`;
  const start = markdown.indexOf(marker);
  if (start < 0) fail(`Capability intent matrix is missing section: ${heading}`);
  const contentStart = start + marker.length;
  const next = markdown.indexOf("\n## ", contentStart);
  return markdown.slice(contentStart, next < 0 ? markdown.length : next);
}

function tableRows(markdown, heading) {
  const lines = section(markdown, heading)
    .split("\n")
    .filter((line) => line.startsWith("|"));
  if (lines.length < 2) fail(`Capability intent matrix section has no table: ${heading}`);
  const cells = (line) =>
    line
      .slice(1, -1)
      .split("|")
      .map((cell) => cell.trim());
  const headers = cells(lines[0]);
  return lines.slice(2).map((line) => {
    const values = cells(line);
    return Object.fromEntries(headers.map((header, index) => [header, values[index] || ""]));
  });
}

function identifiers(markdown) {
  return [...markdown.matchAll(/`([^`]+)`/g)]
    .map((match) => match[1])
    .filter((name) => /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name));
}

function addCapability(capabilities, name, capability) {
  if (!capabilities.has(name)) capabilities.set(name, new Set());
  capabilities.get(name).add(capability);
}

function parseMatrix(markdown) {
  const typeNames = new Set(identifiers(section(markdown, "Public Type Export Inventory")));
  const otherValues = new Set();
  for (const row of tableRows(markdown, "Baseline Summary")) {
    if (row.Surface === "Other public runtime values") {
      for (const name of identifiers(row.Policy)) otherValues.add(name);
    }
  }

  const capabilities = new Map();
  for (const row of tableRows(markdown, "Public Capability Matrix")) {
    if (!row.Disposition.startsWith("exposed")) continue;
    for (const name of identifiers(row["Public TypeScript values and types"])) {
      addCapability(capabilities, name, row["Logical capability"]);
    }
  }
  const valueNames = new Set(
    [...capabilities.keys()].filter((name) => !typeNames.has(name)).concat([...otherValues]),
  );

  const napi = {};
  for (const row of tableRows(markdown, "N-API Runtime Export Classification")) {
    const [name] = identifiers(row["N-API runtime export"]);
    if (!name) fail("N-API matrix row has no runtime export identifier");
    if (napi[name]) fail(`N-API matrix contains duplicate symbol: ${name}`);
    let disposition;
    if (row.Disposition.includes("internal-only")) disposition = "internal-only";
    else if (row.Disposition.startsWith("exposed")) disposition = "exposed";
    else if (row.Disposition.startsWith("excluded")) disposition = "excluded";
    else fail(`N-API matrix has unknown disposition for ${name}: ${row.Disposition}`);
    napi[name] = {
      disposition,
      public:
        disposition === "exposed"
          ? identifiers(row["Public mapping"]).filter((candidate) => valueNames.has(candidate))
          : [],
    };
  }

  const cAbi = {};
  for (const row of tableRows(markdown, "C-ABI Runtime Export Classification")) {
    const [name] = identifiers(row["C-ABI symbol"]);
    if (!name) fail("C-ABI matrix row has no symbol identifier");
    if (cAbi[name]) fail(`C-ABI matrix contains duplicate symbol: ${name}`);
    let disposition;
    if (row["TypeScript disposition"].startsWith("exposed")) disposition = "exposed";
    else if (row["TypeScript disposition"].startsWith("excluded")) disposition = "excluded";
    else {
      fail(`C-ABI matrix has unknown disposition for ${name}: ${row["TypeScript disposition"]}`);
    }
    cAbi[name] = {
      disposition,
      public:
        disposition === "exposed"
          ? identifiers(row["TypeScript disposition"]).filter((candidate) =>
              valueNames.has(candidate),
            )
          : [],
    };
  }
  return { cAbi, capabilities, napi, typeNames, valueNames };
}

function compareClassifiedSurface(label, actual, expected) {
  compareNames(label, Object.keys(actual), Object.keys(expected));
  for (const name of Object.keys(expected)) {
    if (actual[name].disposition !== expected[name].disposition) {
      fail(`${label} ${name} disposition differs from the approved matrix`);
    }
    compareNames(`${label} ${name} public mapping`, actual[name].public, expected[name].public);
  }
}

function verifyMatrix() {
  const matrixPath = path.join(packageRoot, "docs", "capability-intent-matrix.md");
  const markdown = normalizeLineEndings(fs.readFileSync(matrixPath, "utf8"));
  const digest = crypto.createHash("sha256").update(markdown).digest("hex");
  if (digest !== contract.matrixSha256) {
    fail("Capability intent matrix changed; an approved contract manifest update is required");
  }
  const matrix = parseMatrix(markdown);
  compareNames("manifest public values", Object.keys(contract.publicValues), matrix.valueNames);
  compareNames("manifest public types", Object.keys(contract.publicTypes), matrix.typeNames);
  compareClassifiedSurface("manifest N-API inventory", contract.napi, matrix.napi);
  compareClassifiedSurface("manifest C ABI inventory", contract.cAbi, matrix.cAbi);
  for (const [name, capability] of [
    ...Object.entries(contract.publicValues),
    ...Object.entries(contract.publicTypes),
  ]) {
    if (!matrix.capabilities.get(name)?.has(capability)) {
      fail(`Manifest public symbol ${name} is not mapped to ${capability} by the approved matrix`);
    }
  }
}

function emitDeclarations(tempRoot) {
  const configPath = path.join(packageRoot, "tsconfig.build.json");
  const configFile = ts.readConfigFile(configPath, ts.sys.readFile);
  if (configFile.error) {
    fail(ts.flattenDiagnosticMessageText(configFile.error.messageText, "\n"));
  }
  const parsed = ts.parseJsonConfigFileContent(configFile.config, ts.sys, packageRoot, {
    declaration: true,
    declarationMap: false,
    emitDeclarationOnly: true,
    noEmit: false,
    outDir: tempRoot,
    sourceMap: false,
  });
  const program = ts.createProgram(parsed.fileNames, parsed.options);
  const diagnostics = ts.getPreEmitDiagnostics(program);
  if (diagnostics.length) {
    fail(
      ts.formatDiagnosticsWithColorAndContext(diagnostics, {
        getCanonicalFileName: (fileName) => fileName,
        getCurrentDirectory: () => packageRoot,
        getNewLine: () => "\n",
      }),
    );
  }
  const result = program.emit();
  if (result.emitSkipped) fail("TypeScript declaration emit was skipped");
}

function inspectPublicExports(tempRoot) {
  const indexPath = path.join(tempRoot, "index.d.ts");
  const program = ts.createProgram([indexPath], {
    module: ts.ModuleKind.CommonJS,
    moduleResolution: ts.ModuleResolutionKind.Node10,
    skipLibCheck: true,
    target: ts.ScriptTarget.ES2022,
  });
  const checker = program.getTypeChecker();
  const source = program.getSourceFile(indexPath);
  const moduleSymbol = source && checker.getSymbolAtLocation(source);
  if (!moduleSymbol) fail("Could not resolve emitted package declaration module");

  const values = new Map();
  const types = new Map();
  for (const exported of checker.getExportsOfModule(moduleSymbol)) {
    const target =
      exported.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported;
    const name = exported.getName();
    if (target.flags & ts.SymbolFlags.Value) values.set(name, describeValue(checker, target));
    if (target.flags & ts.SymbolFlags.Type) types.set(name, describeType(target));
  }
  return { values, types };
}

function describeValue(checker, symbol) {
  const declaration = symbol.valueDeclaration || symbol.declarations?.[0];
  if (symbol.flags & ts.SymbolFlags.Class) return "class";
  const type = checker.getTypeOfSymbolAtLocation(symbol, declaration);
  const signatures = checker.getSignaturesOfType(type, ts.SignatureKind.Call);
  if (signatures.length) {
    return signatures.map((signature) => checker.signatureToString(signature)).join("; ");
  }
  return checker.typeToString(type, declaration, ts.TypeFormatFlags.NoTruncation);
}

function describeType(symbol) {
  const declarations = (symbol.declarations || []).filter(
    (declaration) =>
      ts.isClassDeclaration(declaration) ||
      ts.isInterfaceDeclaration(declaration) ||
      ts.isTypeAliasDeclaration(declaration) ||
      ts.isEnumDeclaration(declaration),
  );
  if (declarations.length === 0) {
    fail(`Public type ${symbol.getName()} has no printable declaration`);
  }

  const printer = ts.createPrinter({
    newLine: ts.NewLineKind.LineFeed,
    removeComments: true,
  });
  return declarations
    .map((declaration) =>
      printer
        .printNode(ts.EmitHint.Unspecified, declaration, declaration.getSourceFile())
        .replace(/\s+/g, " ")
        .trim(),
    )
    .join(" ");
}

function tokenizeHeader(source) {
  const withoutComments = source
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/\/\/[^\n]*/g, " ")
    .replace(/^\s*#.*$/gm, " ");
  return withoutComments.match(/[A-Za-z_]\w*|\.\.\.|[^\s]/g) || [];
}

function parseCFunctions(headerPath) {
  const tokens = tokenizeHeader(fs.readFileSync(headerPath, "utf8"));
  const functions = [];
  let braces = 0;
  let statement = [];
  for (const token of tokens) {
    if (token === "{") braces += 1;
    if (token === "}") braces -= 1;
    statement.push(token);
    if (token !== ";" || braces !== 0) continue;

    if (!statement.includes("typedef")) {
      let parens = 0;
      for (let index = 0; index < statement.length; index += 1) {
        const current = statement[index];
        if (current === "(" && parens === 0) {
          const name = statement[index - 1];
          if (/^sysprims_[A-Za-z0-9_]+$/.test(name)) functions.push(name);
        }
        if (current === "(") parens += 1;
        if (current === ")") parens -= 1;
      }
    }
    statement = [];
  }
  return sorted(new Set(functions));
}

function validateMappings(publicValues) {
  for (const [surface, entries] of [
    ["N-API", contract.napi],
    ["C ABI", contract.cAbi],
  ]) {
    for (const [name, entry] of Object.entries(entries)) {
      if (!entry.disposition || !Array.isArray(entry.public)) {
        fail(`${surface} symbol ${name} is not classified and mapped`);
      }
      if (entry.disposition === "exposed" && entry.public.length === 0) {
        fail(`${surface} symbol ${name} is exposed but has no public mapping`);
      }
      for (const publicName of entry.public) {
        if (!publicValues.has(publicName)) {
          fail(`${surface} symbol ${name} maps to missing public value ${publicName}`);
        }
      }
    }
  }
}

function nativeCandidates() {
  const platform = { darwin: "darwin", linux: "linux", win32: "win32" }[process.platform];
  const arch = { arm64: "arm64", x64: "x64" }[process.arch];
  if (!platform || !arch) return [];
  let suffix = `${platform}-${arch}`;
  if (platform === "win32") suffix += "-msvc";
  if (platform === "linux") {
    const glibc = process.report?.getReport?.().header?.glibcVersionRuntime;
    suffix += glibc ? "-gnu" : "-musl";
  }
  const fileName = `sysprims.${suffix}.node`;
  const discovered = [];
  for (const directory of [
    packageRoot,
    path.join(packageRoot, "prebuilds"),
    path.join(packageRoot, "dist", "native"),
  ]) {
    if (!fs.existsSync(directory)) continue;
    for (const entry of fs.readdirSync(directory)) {
      if (entry.startsWith(`sysprims.${platform}-${arch}`) && entry.endsWith(".node")) {
        discovered.push(path.join(directory, entry));
      }
    }
  }
  return [
    ...new Set(
      [
        process.env.SYSPRIMS_NAPI_PATH,
        path.join(packageRoot, fileName),
        path.join(packageRoot, "prebuilds", fileName),
        path.join(packageRoot, "dist", "native", fileName),
        ...sorted(discovered),
      ].filter(Boolean),
    ),
  ];
}

function inspectNative(requireNative) {
  const candidates = nativeCandidates().filter((candidate) => fs.existsSync(candidate));
  if (candidates.length === 0) {
    if (requireNative) fail("No native addon found for runtime N-API inventory");
    return null;
  }
  const addon = require(candidates[0]);
  const callable = Object.getOwnPropertyNames(addon).filter(
    (name) => typeof addon[name] === "function",
  );
  compareNames("runtime N-API callable inventory", callable, Object.keys(contract.napi));
  return path.relative(packageRoot, candidates[0]);
}

function markdownCell(value) {
  return String(value).replaceAll("|", "\\|").replaceAll("\n", " ");
}

function renderDocument(exports, cFunctions) {
  const lines = [
    "# TypeScript Public API",
    "",
    "<!-- Generated by npm run api:generate. Do not edit directly. -->",
    "",
    "This document is generated from emitted package declarations and the reviewed native surface contract. See [Capability Intent Matrix](capability-intent-matrix.md) for policy and lifecycle rationale.",
    "",
    `Summary: ${exports.values.size} public values, ${exports.types.size} public types, ${Object.keys(contract.napi).length} N-API callables, and ${cFunctions.length} C-ABI functions.`,
    "",
    "## Public Values",
    "",
  ];
  for (const name of sorted(exports.values.keys())) {
    lines.push(
      `- \`${name}\`: \`${markdownCell(exports.values.get(name))}\` (${contract.publicValues[name]})`,
    );
  }
  lines.push("", "## Public Types", "");
  for (const name of sorted(exports.types.keys())) {
    lines.push(`- \`${name}\`: ${exports.types.get(name)} (${contract.publicTypes[name]})`);
  }
  lines.push("", "## N-API Runtime Callables", "");
  for (const name of sorted(Object.keys(contract.napi))) {
    const entry = contract.napi[name];
    lines.push(
      `- \`${name}\`: ${entry.disposition}; public mapping: ${entry.public.map((item) => `\`${item}\``).join(", ") || "none"}`,
    );
  }
  lines.push("", "## C-ABI Comparison Functions", "");
  for (const name of cFunctions) {
    const entry = contract.cAbi[name];
    lines.push(
      `- \`${name}\`: ${entry.disposition}; public mapping: ${entry.public.map((item) => `\`${item}\``).join(", ") || "none"}`,
    );
  }
  return `${lines.join("\n")}\n`;
}

function main() {
  const mode = process.argv[2];
  if (mode !== "generate" && mode !== "check") {
    fail(
      "Usage: node scripts/public-api.js <generate|check> [--require-native] [--c-header <path>]",
    );
  }
  verifyMatrix();
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sysprims-public-api-"));
  try {
    emitDeclarations(tempRoot);
    const exports = inspectPublicExports(tempRoot);
    compareNames("public value exports", exports.values.keys(), Object.keys(contract.publicValues));
    compareNames("public type exports", exports.types.keys(), Object.keys(contract.publicTypes));
    validateMappings(exports.values);

    const headerOption = process.argv.indexOf("--c-header");
    if (headerOption >= 0 && !process.argv[headerOption + 1]) {
      fail("--c-header requires a path");
    }
    const headerPath = path.resolve(
      headerOption >= 0
        ? process.argv[headerOption + 1]
        : process.env.SYSPRIMS_C_HEADER ||
            path.join(repoRoot, "bindings", "go", "sysprims", "include", "sysprims.h"),
    );
    const cFunctions = parseCFunctions(headerPath);
    compareNames("C ABI function inventory", cFunctions, Object.keys(contract.cAbi));
    const nativePath = inspectNative(process.argv.includes("--require-native"));
    const generated = renderDocument(exports, cFunctions);

    if (mode === "generate") {
      fs.writeFileSync(outputPath, generated);
      console.log(`Generated ${path.relative(packageRoot, outputPath)}`);
    } else {
      const current = fs.existsSync(outputPath) ? fs.readFileSync(outputPath, "utf8") : "";
      if (current !== generated) {
        fail("Generated public API documentation is out of date; run npm run api:generate");
      }
      console.log(
        `Public API contract is current${nativePath ? `; inspected ${nativePath}` : "; static N-API manifest checked"}`,
      );
    }
  } finally {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}

module.exports = { emitDeclarations, inspectPublicExports, normalizeLineEndings, renderDocument };
