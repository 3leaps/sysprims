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
  "bindings/typescript/sysprims/package.json",
  "bindings/typescript/sysprims/package-lock.json",
  ...nativeDirectories.map(
    (directory) =>
      `bindings/typescript/sysprims/npm/${directory}/package.json`,
  ),
];
const semverPattern =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

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
        dependency.req !== `=${expected}`
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

function checkJson(root, expected, errors) {
  const rootPackage = readJson(
    join(root, "bindings/typescript/sysprims/package.json"),
  );
  if (rootPackage.name !== "@3leaps/sysprims") {
    errors.push("TypeScript root package name is not @3leaps/sysprims");
  }
  if (rootPackage.version !== expected) {
    errors.push(
      `TypeScript root package is ${rootPackage.version}, expected ${expected}`,
    );
  }
  checkOptionalPins(
    rootPackage.optionalDependencies,
    expected,
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
    if (nativePackage.version !== expected) {
      errors.push(
        `TypeScript native package ${expectedName} is ${nativePackage.version}, expected ${expected}`,
      );
    }
  }

  const lock = readJson(
    join(root, "bindings/typescript/sysprims/package-lock.json"),
  );
  if (lock.name !== "@3leaps/sysprims") {
    errors.push("package-lock root name is not @3leaps/sysprims");
  }
  if (lock.version !== expected) {
    errors.push(
      `package-lock authored root version is ${lock.version}, expected ${expected}`,
    );
  }
  const lockRoot = lock.packages?.[""];
  if (!lockRoot) {
    errors.push('package-lock authored packages[""] entry is missing');
  } else {
    if (lockRoot.name !== "@3leaps/sysprims") {
      errors.push('package-lock packages[""] name is not @3leaps/sysprims');
    }
    if (lockRoot.version !== expected) {
      errors.push(
        `package-lock packages[""] version is ${lockRoot.version}, expected ${expected}`,
      );
    }
    checkOptionalPins(
      lockRoot.optionalDependencies,
      expected,
      'package-lock packages[""].optionalDependencies',
      errors,
    );
  }

  for (const packageName of platformNames) {
    const resolution = lock.packages?.[`node_modules/${packageName}`];
    if (!resolution) {
      continue;
    }
    if (resolution.version !== expected) {
      errors.push(
        `stale npm platform resolution evidence for ${packageName}: resolved version ${resolution.version}, authored version ${expected}; refresh from real staged tarballs or remove the stale node`,
      );
      continue;
    }
    const encodedName = packageName.replace("@3leaps/", "");
    const expectedSuffix = `/${encodedName}-${expected}.tgz`;
    if (
      typeof resolution.resolved !== "string" ||
      !resolution.resolved.endsWith(expectedSuffix) ||
      typeof resolution.integrity !== "string" ||
      resolution.integrity.length === 0
    ) {
      errors.push(
        `invalid npm platform resolution evidence for ${packageName}@${expected}; regenerate it from the real published tarball`,
      );
    }
  }
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
    checkCargo(root, expected, errors);
  } catch (error) {
    errors.push(error.message);
  }
  try {
    checkJson(root, expected, errors);
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
    if (resolution && resolution.version !== version) {
      delete lock.packages[resolutionKey];
    }
  }
  writeJsonAtomic(lockPath, lock);
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
    afterCargoSetVersion?.();
    updateJsonSurfaces(root, version);
    check(root, true);
  });

  console.log(`[ok] Version pack synchronized at ${version}`);
}

function bump(root, component) {
  const current = readCanonicalVersion(root);
  const match = current.match(semverPattern);
  if (match[4] || match[5]) {
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
        "usage: version-pack.mjs <check|sync|set VERSION|bump patch|minor|major|owned-paths> [--root PATH]",
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
