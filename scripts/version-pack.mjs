#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultRoot = resolve(scriptDir, "..");
const platformNames = [
  "@3leaps/sysprims-darwin-arm64",
  "@3leaps/sysprims-linux-arm64-gnu",
  "@3leaps/sysprims-linux-arm64-musl",
  "@3leaps/sysprims-linux-x64-gnu",
  "@3leaps/sysprims-linux-x64-musl",
  "@3leaps/sysprims-win32-arm64-msvc",
  "@3leaps/sysprims-win32-x64-msvc",
];
const nativeDirectories = [
  "darwin-arm64",
  "linux-arm64-gnu",
  "linux-arm64-musl",
  "linux-x64-gnu",
  "linux-x64-musl",
  "win32-arm64-msvc",
  "win32-x64-msvc",
];
const baseOwnedPaths = [
  "VERSION",
  "Cargo.toml",
  "Cargo.lock",
  "bindings/go/sysprims/README.md",
  "bindings/go/sysprims/go.mod",
  "bindings/go/sysprims/prebuilt-manifest.json",
  "bindings/typescript/sysprims/package.json",
  "bindings/typescript/sysprims/package-lock.json",
  ...nativeDirectories.map(
    (directory) =>
      `bindings/typescript/sysprims/npm/${directory}/package.json`,
  ),
];
const goModulePath = "github.com/3leaps/sysprims/bindings/go/sysprims";
const goPlatforms = [
  "darwin-amd64",
  "darwin-arm64",
  "linux-amd64",
  "linux-amd64-musl",
  "linux-arm64",
  "linux-arm64-musl",
  "windows-amd64",
  "windows-arm64",
];
const semverSource =
  "(?:0|[1-9]\\d*)\\.(?:0|[1-9]\\d*)\\.(?:0|[1-9]\\d*)(?:-(?:(?:0|[1-9]\\d*|\\d*[A-Za-z-][0-9A-Za-z-]*)(?:\\.(?:0|[1-9]\\d*|\\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\\+(?:[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*))?";
const semverPattern = new RegExp(`^${semverSource}$`);

function fail(message) {
  throw new Error(message);
}

function parseArguments(argv) {
  const values = [...argv];
  const command = values.shift();
  let root = defaultRoot;

  for (let index = 0; index < values.length; index += 1) {
    if (values[index] === "--root") {
      if (!values[index + 1]) {
        fail("--root requires a path");
      }
      root = resolve(values[index + 1]);
      values.splice(index, 2);
      index -= 1;
    }
  }

  return { command, root, values };
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    fail(`cannot parse JSON ${path}: ${error.message}`);
  }
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function readReleasePlan(root, version = readCanonicalVersion(root)) {
  const relativePath = `docs/releases/v${version}.json`;
  const path = join(root, relativePath);
  if (!existsSync(path)) fail(`release plan is missing: ${relativePath}`);
  const plan = readJson(path);
  const errors = [];
  if (plan.schema !== "sysprims-release-plan/v1") errors.push("unknown release plan schema");
  if (plan.version !== version) errors.push(`release plan version is ${JSON.stringify(plan.version)}, expected ${version}`);
  if (plan.provenance_policy !== "commit_footer") errors.push("release plan provenance_policy must be commit_footer");
  const surfaces = plan.surfaces;
  if (!surfaces || typeof surfaces !== "object" || Array.isArray(surfaces)) {
    errors.push("release plan surfaces object is missing");
  } else {
    const expectedSurfaceNames = ["cli", "ffi", "go", "rust", "typescript"];
    const actualSurfaceNames = Object.keys(surfaces).sort();
    if (JSON.stringify(actualSurfaceNames) !== JSON.stringify(expectedSurfaceNames)) {
      errors.push(`release plan surface set must be exactly ${expectedSurfaceNames.join(", ")}`);
    }
    for (const name of ["rust", "cli", "ffi"]) {
      if (surfaces[name]?.disposition !== "publish") errors.push(`${name} disposition must be publish`);
    }
    for (const name of ["go", "typescript"]) {
      const surface = surfaces[name];
      if (!surface || !["publish", "skip"].includes(surface.disposition)) {
        errors.push(`${name} disposition must be publish or skip`);
        continue;
      }
      if (surface.disposition === "skip") {
        try { validateSemver(surface.retained_version, `${name} retained_version`); }
        catch (error) { errors.push(error.message); }
        if (surface.retained_version === version) errors.push(`${name} skip retained_version must differ from the release version`);
      } else if ("retained_version" in surface) {
        errors.push(`${name} retained_version is only valid for skip`);
      }
    }
    if (surfaces.go && !["pre_build", "resolved"].includes(surfaces.go.lock_phase)) {
      errors.push("go lock_phase must be pre_build or resolved");
    }
    if (surfaces.go?.disposition === "skip" && surfaces.go.lock_phase !== "resolved") {
      errors.push("skipped go surface must use resolved lock_phase");
    }
    if (surfaces.typescript && !["resolved", "pre_registry"].includes(surfaces.typescript.lock_phase)) {
      errors.push("typescript lock_phase must be resolved or pre_registry");
    }
    if (surfaces.typescript?.disposition === "skip" && surfaces.typescript.lock_phase !== "resolved") {
      errors.push("skipped typescript surface must use resolved lock_phase");
    }
  }
  if (errors.length) fail(errors.join("; "));
  return { plan, path, relativePath };
}

function plannedVersion(surface, releaseVersion) {
  return surface.disposition === "publish" ? releaseVersion : surface.retained_version;
}

function writeJsonAtomic(path, value) {
  writeTextAtomic(path, `${JSON.stringify(value, null, 2)}\n`);
}

function writeTextAtomic(path, value) {
  const temporary = `${path}.version-pack-${process.pid}`;
  writeFileSync(temporary, value);
  renameSync(temporary, path);
}

function validateSemver(value, label = "version") {
  if (!semverPattern.test(value)) {
    fail(`${label} is not canonical SemVer: ${JSON.stringify(value)}`);
  }
  return value;
}

function readCanonicalVersion(root) {
  const contents = readFileSync(join(root, "VERSION"), "utf8");
  const match = contents.match(/^([^\r\n]+)\n$/);
  if (!match) {
    fail("VERSION must contain one canonical SemVer followed by LF");
  }
  return validateSemver(match[1], "VERSION");
}

function run(root, command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    ...options,
  });
  if (result.error) {
    fail(`cannot run ${command}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const detail = `${result.stderr || ""}${result.stdout || ""}`.trim();
    fail(
      `${command} ${args.join(" ")} failed${detail ? `:\n${detail}` : ""}`,
    );
  }
  return result.stdout;
}

function preflight(root) {
  for (const relativePath of baseOwnedPaths) {
    const path = join(root, relativePath);
    if (!existsSync(path)) {
      fail(`required version-pack path is missing: ${relativePath}`);
    }
  }

  for (const relativePath of baseOwnedPaths.filter((path) =>
    path.endsWith(".json"),
  )) {
    readJson(join(root, relativePath));
  }

  run(root, "cargo", ["set-version", "-V"]);
  const metadata = JSON.parse(
    run(root, "cargo", ["metadata", "--no-deps", "--format-version", "1"]),
  );
  const paths = new Set(baseOwnedPaths);
  const canonicalRoot = realpathSync(root);
  for (const pkg of metadata.packages) {
    const manifestPath = relative(
      canonicalRoot,
      realpathSync(pkg.manifest_path),
    );
    if (manifestPath.startsWith("..")) {
      fail(`workspace manifest escapes repository root: ${pkg.manifest_path}`);
    }
    paths.add(manifestPath);
  }
  return [...paths];
}

function cargoMetadata(root, locked = true) {
  const args = ["metadata", "--no-deps", "--format-version", "1"];
  if (locked) {
    args.push("--locked");
  }
  return JSON.parse(run(root, "cargo", args));
}

function checkCargo(root, expected, errors) {
  let metadata;
  try {
    metadata = cargoMetadata(root);
  } catch (error) {
    errors.push(`Cargo workspace/lock metadata is stale: ${error.message}`);
    return;
  }

  const workspaceMembers = new Set(metadata.workspace_members);
  const workspacePackages = metadata.packages.filter((pkg) =>
    workspaceMembers.has(pkg.id),
  );
  if (workspacePackages.length === 0) {
    errors.push("Cargo metadata returned no workspace packages");
    return;
  }

  for (const pkg of workspacePackages) {
    if (pkg.version !== expected) {
      errors.push(
        `Cargo workspace package ${pkg.name} is ${pkg.version}, expected ${expected}`,
      );
    }
    for (const dependency of pkg.dependencies) {
      if (
        dependency.name.startsWith("sysprims-") &&
        dependency.source === null &&
        dependency.req !== "*" &&
        dependency.req !== expected &&
        dependency.req !== `=${expected}` &&
        dependency.req !== `^${expected}`
      ) {
        errors.push(
          `Cargo internal dependency ${pkg.name} -> ${dependency.name} pins ${dependency.req}, expected ${expected}`,
        );
      }
    }
  }
}

function checkOptionalPins(value, expected, label, errors) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    errors.push(`${label} is missing`);
    return;
  }
  for (const packageName of platformNames) {
    if (value[packageName] !== expected) {
      errors.push(
        `${label}[${packageName}] is ${JSON.stringify(value[packageName])}, expected ${expected}`,
      );
    }
  }
}

function validSha512Sri(value) {
  const match = typeof value === "string" && value.match(/^sha512-([A-Za-z0-9+/]+={0,2})$/);
  if (!match) return false;
  try { return Buffer.from(match[1], "base64").length === 64; }
  catch { return false; }
}

function checkJson(root, expected, plan, errors) {
  const typescriptVersion = plannedVersion(plan.surfaces.typescript, expected);
  const rootPackage = readJson(
    join(root, "bindings/typescript/sysprims/package.json"),
  );
  if (rootPackage.name !== "@3leaps/sysprims") {
    errors.push("TypeScript root package name is not @3leaps/sysprims");
  }
  if (rootPackage.version !== typescriptVersion) {
    errors.push(
      `TypeScript root package is ${rootPackage.version}, expected ${typescriptVersion}`,
    );
  }
  checkOptionalPins(
    rootPackage.optionalDependencies,
    typescriptVersion,
    "TypeScript root optionalDependencies",
    errors,
  );

  for (let index = 0; index < nativeDirectories.length; index += 1) {
    const directory = nativeDirectories[index];
    const expectedName = platformNames[index];
    const nativePackage = readJson(
      join(
        root,
        "bindings/typescript/sysprims/npm",
        directory,
        "package.json",
      ),
    );
    if (nativePackage.name !== expectedName) {
      errors.push(
        `TypeScript native package ${directory} is named ${JSON.stringify(nativePackage.name)}, expected ${expectedName}`,
      );
    }
    if (nativePackage.version !== typescriptVersion) {
      errors.push(
        `TypeScript native package ${expectedName} is ${nativePackage.version}, expected ${typescriptVersion}`,
      );
    }
  }

  const lock = readJson(
    join(root, "bindings/typescript/sysprims/package-lock.json"),
  );
  if (lock.name !== "@3leaps/sysprims") {
    errors.push("package-lock root name is not @3leaps/sysprims");
  }
  if (lock.version !== typescriptVersion) {
    errors.push(
      `package-lock authored root version is ${lock.version}, expected ${typescriptVersion}`,
    );
  }
  const lockRoot = lock.packages?.[""];
  if (!lockRoot) {
    errors.push('package-lock authored packages[""] entry is missing');
  } else {
    if (lockRoot.name !== "@3leaps/sysprims") {
      errors.push('package-lock packages[""] name is not @3leaps/sysprims');
    }
    if (lockRoot.version !== typescriptVersion) {
      errors.push(
        `package-lock packages[""] version is ${lockRoot.version}, expected ${typescriptVersion}`,
      );
    }
    checkOptionalPins(
      lockRoot.optionalDependencies,
      typescriptVersion,
      'package-lock packages[""].optionalDependencies',
      errors,
    );
  }

  for (const packageName of platformNames) {
    const resolution = lock.packages?.[`node_modules/${packageName}`];
    if (!resolution) {
      if (plan.surfaces.typescript.lock_phase === "resolved") errors.push(`resolved npm platform node is missing for ${packageName}`);
      continue;
    }
    if (resolution.version !== typescriptVersion) {
      errors.push(
        `stale npm platform resolution evidence for ${packageName}: resolved version ${resolution.version}, authored version ${typescriptVersion}`,
      );
      continue;
    }
    const encodedName = packageName.replace("@3leaps/", "");
    const expectedSuffix = `/${encodedName}-${typescriptVersion}.tgz`;
    if (
      typeof resolution.resolved !== "string" ||
      !resolution.resolved.endsWith(expectedSuffix) ||
      !validSha512Sri(resolution.integrity)
    ) {
      errors.push(
        `invalid npm platform resolution evidence for ${packageName}@${typescriptVersion}`,
      );
    }
    if (plan.surfaces.typescript.lock_phase === "pre_registry") {
      errors.push(`pre_registry plan must not contain registry resolution evidence for ${packageName}`);
    }
  }
}

function checkGo(root, expected, plan, errors) {
  const goVersion = plannedVersion(plan.surfaces.go, expected);
  const goMod = readFileSync(join(root, "bindings/go/sysprims/go.mod"), "utf8");
  const moduleLines = goMod.match(/^module\s+(.+)$/gm) ?? [];
  if (moduleLines.length !== 1 || moduleLines[0] !== `module ${goModulePath}`) {
    errors.push(`Go module path must be exactly ${goModulePath}`);
  }
  checkGoReadme(root, goVersion, errors);
  const path = join(root, "bindings/go/sysprims/prebuilt-manifest.json");
  const manifest = readJson(path);
  if (manifest.schema !== "sysprims-go-prebuilt-manifest/v1") errors.push("unknown Go prebuilt manifest schema");
  if (manifest.module !== goModulePath) errors.push("Go prebuilt manifest module path mismatch");
  const preBuild = plan.surfaces.go.disposition === "publish" && plan.surfaces.go.lock_phase === "pre_build";
  if (!preBuild && manifest.release_version !== goVersion) errors.push(`Go prebuilt manifest release_version is ${manifest.release_version}, expected ${goVersion}`);
  if (preBuild && manifest.release_version === expected) errors.push("Go pre_build manifest must describe the prior committed native version, not claim VERSION");
  try { validateSemver(manifest.release_version, "Go prebuilt manifest release_version"); }
  catch (error) { errors.push(error.message); }
  if (!Number.isInteger(manifest.ffi_abi_version) || manifest.ffi_abi_version < 1) errors.push("Go prebuilt manifest ffi_abi_version must be a positive integer");
  if (JSON.stringify(manifest.required_platforms) !== JSON.stringify(goPlatforms)) errors.push("Go prebuilt manifest required_platforms is incomplete or out of order");
  if (manifest.header?.path !== "include/sysprims.h") errors.push("Go prebuilt manifest header path mismatch");
  else if (manifest.header.sha256 !== sha256(join(root, "bindings/go/sysprims", manifest.header.path))) errors.push("Go prebuilt header hash mismatch");
  const builds = Array.isArray(manifest.platforms) ? manifest.platforms : [];
  if (builds.length !== goPlatforms.length || builds.some((entry) => !goPlatforms.includes(entry.platform))) {
    errors.push("Go prebuilt manifest platform entries must equal the exact required platform set");
  }
  for (const platform of goPlatforms) {
    const entries = builds.filter((entry) => entry.platform === platform);
    if (entries.length !== 1) { errors.push(`Go prebuilt manifest requires one ${platform} entry`); continue; }
    const entry = entries[0];
    const wantedPath = `lib/${platform}/libsysprims_ffi.a`;
    if (entry.path !== wantedPath) errors.push(`Go prebuilt ${platform} path is ${entry.path}, expected ${wantedPath}`);
    else if (entry.sha256 !== sha256(join(root, "bindings/go/sysprims", entry.path))) errors.push(`Go prebuilt ${platform} hash mismatch`);
    const nativeVersion = preBuild ? manifest.release_version : goVersion;
    if (entry.reported_version !== nativeVersion) errors.push(`Go prebuilt ${platform} reported_version is ${entry.reported_version}, expected ${nativeVersion}`);
  }
}

function extractUniqueRequiredVersion(text, pattern, label, expected, errors) {
  const matches = [...text.matchAll(pattern)];
  if (matches.length === 0) {
    errors.push(`${label} is missing`);
    return;
  }
  if (matches.length !== 1) {
    const versions = matches.map((match) => match[1]).join(", ");
    errors.push(`${label} appears ${matches.length} times: ${versions}`);
    return;
  }
  const match = matches[0];
  if (match[1] !== expected) {
    errors.push(`${label} is ${match[1]}, expected ${expected}`);
  }
}

function checkGoReadme(root, expected, errors) {
  const readme = readFileSync(
    join(root, "bindings/go/sysprims/README.md"),
    "utf8",
  );
  extractUniqueRequiredVersion(
    readme,
    new RegExp(
      `go get github\\.com\\/3leaps\\/sysprims\\/bindings\\/go\\/sysprims@v(${semverSource})`,
      "g",
    ),
    "Go README install version",
    expected,
    errors,
  );
  extractUniqueRequiredVersion(
    readme,
    new RegExp(`The Go module resolves \`v(${semverSource})\``, "g"),
    "Go README module version",
    expected,
    errors,
  );
  extractUniqueRequiredVersion(
    readme,
    new RegExp(
      `\`bindings\\/go\\/sysprims\\/v(${semverSource})\` tag`,
      "g",
    ),
    "Go README path-prefixed tag version",
    expected,
    errors,
  );
  extractUniqueRequiredVersion(
    readme,
    new RegExp(`canonical \`v(${semverSource})\` tag`, "g"),
    "Go README canonical tag version",
    expected,
    errors,
  );
}

function collectErrors(root) {
  const errors = [];
  let expected;
  try {
    expected = readCanonicalVersion(root);
  } catch (error) {
    return [error.message];
  }

  try {
    const { plan } = readReleasePlan(root, expected);
    checkCargo(root, expected, errors);
    checkJson(root, expected, plan, errors);
    checkGo(root, expected, plan, errors);
  } catch (error) {
    errors.push(error.message);
  }
  return errors;
}

function check(root, quiet = false) {
  const errors = collectErrors(root);
  if (errors.length > 0) {
    for (const error of errors) {
      console.error(`[ERROR] ${error}`);
    }
    fail(`version pack has ${errors.length} error(s)`);
  }
  if (!quiet) {
    console.log(`[ok] Version pack is coherent at ${readCanonicalVersion(root)}`);
  }
}

function updateJsonSurfaces(root, version) {
  const { plan } = readReleasePlan(root, version);
  if (plan.surfaces.typescript.disposition === "skip") return;
  const rootPath = join(root, "bindings/typescript/sysprims/package.json");
  const rootPackage = readJson(rootPath);
  rootPackage.version = version;
  rootPackage.optionalDependencies ??= {};
  for (const packageName of platformNames) {
    rootPackage.optionalDependencies[packageName] = version;
  }
  writeJsonAtomic(rootPath, rootPackage);

  for (let index = 0; index < nativeDirectories.length; index += 1) {
    const path = join(
      root,
      "bindings/typescript/sysprims/npm",
      nativeDirectories[index],
      "package.json",
    );
    const nativePackage = readJson(path);
    if (nativePackage.name !== platformNames[index]) {
      fail(
        `refusing to rewrite unexpected native package ${path}: ${JSON.stringify(nativePackage.name)}`,
      );
    }
    nativePackage.version = version;
    writeJsonAtomic(path, nativePackage);
  }

  const lockPath = join(
    root,
    "bindings/typescript/sysprims/package-lock.json",
  );
  const lock = readJson(lockPath);
  if (!lock.packages?.[""]) {
    fail('package-lock authored packages[""] entry is missing');
  }
  lock.version = version;
  lock.packages[""].version = version;
  lock.packages[""].optionalDependencies ??= {};
  for (const packageName of platformNames) {
    lock.packages[""].optionalDependencies[packageName] = version;
    const resolutionKey = `node_modules/${packageName}`;
    const resolution = lock.packages[resolutionKey];
    if (
      resolution &&
      (plan.surfaces.typescript.lock_phase === "pre_registry" ||
        resolution.version !== version)
    ) {
      delete lock.packages[resolutionKey];
    }
  }
  writeJsonAtomic(lockPath, lock);
}

function updateCargoWorkspaceDependencyPins(root, version) {
  const cargoPath = join(root, "Cargo.toml");
  let cargoToml = readFileSync(cargoPath, "utf8");
  let replaced = 0;
  cargoToml = cargoToml.replace(
    /^(sysprims-[A-Za-z0-9_-]+\s*=\s*\{[^\n]*\})$/gm,
    (line) => {
      if (!/\bversion\s*=/.test(line)) {
        fail(`workspace dependency is missing version pin: ${line}`);
      }
      replaced += 1;
      return line.replace(/\bversion\s*=\s*"[^"]+"/, `version = "${version}"`);
    }
  );
  if (replaced === 0) {
    fail("cannot find versioned sysprims workspace dependency pins");
  }
  writeTextAtomic(cargoPath, cargoToml);
}

function replaceExactlyOnce(text, pattern, replacement, label) {
  const matches = [...text.matchAll(pattern)];
  if (matches.length !== 1) {
    fail(`expected exactly one ${label} replacement, found ${matches.length}`);
  }
  return text.replace(pattern, replacement);
}

function updateGoReadme(root, version) {
  const { plan } = readReleasePlan(root, version);
  if (plan.surfaces.go.disposition === "skip") return;
  const readmePath = join(root, "bindings/go/sysprims/README.md");
  let readme = readFileSync(readmePath, "utf8");
  readme = replaceExactlyOnce(
    readme,
    new RegExp(
      `(go get github\\.com\\/3leaps\\/sysprims\\/bindings\\/go\\/sysprims@)v${semverSource}`,
      "g",
    ),
    (_match, prefix) => `${prefix}v${version}`,
    "Go README install version",
  );
  readme = replaceExactlyOnce(
    readme,
    new RegExp(`(The Go module resolves \`)v${semverSource}(\`)`, "g"),
    (_match, prefix, suffix) => `${prefix}v${version}${suffix}`,
    "Go README module version",
  );
  readme = replaceExactlyOnce(
    readme,
    new RegExp(`(\`bindings\\/go\\/sysprims\\/)v${semverSource}(\` tag)`, "g"),
    (_match, prefix, suffix) => `${prefix}v${version}${suffix}`,
    "Go README path-prefixed tag version",
  );
  readme = replaceExactlyOnce(
    readme,
    new RegExp(`(canonical \`)v${semverSource}(\` tag)`, "g"),
    (_match, prefix, suffix) => `${prefix}v${version}${suffix}`,
    "Go README canonical tag version",
  );
  writeTextAtomic(readmePath, readme);
}

function withRollback(root, paths, operation) {
  const backupRoot = mkdtempSync(join(tmpdir(), "sysprims-version-pack-"));
  try {
    for (const relativePath of paths) {
      const backupPath = join(backupRoot, relativePath);
      mkdirSync(dirname(backupPath), { recursive: true });
      copyFileSync(join(root, relativePath), backupPath);
    }
    try {
      operation();
    } catch (error) {
      for (const relativePath of paths) {
        copyFileSync(join(backupRoot, relativePath), join(root, relativePath));
      }
      throw new Error(`version-pack update rolled back: ${error.message}`);
    }
  } finally {
    rmSync(backupRoot, { recursive: true, force: true });
  }
}

export function synchronize(
  root,
  requestedVersion,
  { afterCargoSetVersion } = {},
) {
  const paths = preflight(root);
  const version = requestedVersion ?? readCanonicalVersion(root);
  validateSemver(version);

  withRollback(root, paths, () => {
    if (requestedVersion !== undefined) {
      writeTextAtomic(join(root, "VERSION"), `${version}\n`);
    }
    run(root, "cargo", ["set-version", "--workspace", version], {
      stdio: "inherit",
    });
    updateCargoWorkspaceDependencyPins(root, version);
    afterCargoSetVersion?.();
    updateJsonSurfaces(root, version);
    updateGoReadme(root, version);
    check(root, true);
  });

  console.log(`[ok] Version pack synchronized at ${version}`);
}

function bump(root, component) {
  const current = readCanonicalVersion(root);
  const match = current.match(semverPattern);
  if (current.includes("-") || current.includes("+")) {
    fail(`cannot ${component}-bump prerelease/build version ${current}`);
  }
  let [major, minor, patch] = current.split(".").map(Number);
  if (component === "patch") {
    patch += 1;
  } else if (component === "minor") {
    minor += 1;
    patch = 0;
  } else if (component === "major") {
    major += 1;
    minor = 0;
    patch = 0;
  } else {
    fail(`unknown bump component: ${component}`);
  }
  synchronize(root, `${major}.${minor}.${patch}`);
}

function main() {
  const { command, root, values } = parseArguments(process.argv.slice(2));
  switch (command) {
    case "plan-check":
      if (values.length !== 0) fail("plan-check takes no positional arguments");
      readReleasePlan(root);
      console.log(`[ok] Release plan is valid at ${readCanonicalVersion(root)}`);
      break;
    case "check":
      if (values.length !== 0) fail("check takes no positional arguments");
      check(root);
      break;
    case "sync":
      if (values.length !== 0) fail("sync takes no positional arguments");
      synchronize(root);
      break;
    case "set":
      if (values.length !== 1) fail("set requires exactly one SemVer");
      synchronize(root, validateSemver(values[0]));
      break;
    case "bump":
      if (values.length !== 1) {
        fail("bump requires one of: patch, minor, major");
      }
      bump(root, values[0]);
      break;
    case "owned-paths":
      if (values.length !== 0) {
        fail("owned-paths takes no positional arguments");
      }
      console.log(preflight(root).join("\n"));
      break;
    default:
      fail(
        "usage: version-pack.mjs <plan-check|check|sync|set VERSION|bump patch|minor|major|owned-paths> [--root PATH]",
      );
  }
}

if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
) {
  try {
    main();
  } catch (error) {
    console.error(`[ERROR] ${error.message}`);
    process.exitCode = 1;
  }
}
