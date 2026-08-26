const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const packageRoot = path.resolve(__dirname, "..");
const rootPackage = require(path.join(packageRoot, "package.json"));
const packagePrefix = "@3leaps/sysprims-";
const requested = new Set(process.argv.slice(2));
const packagePlatforms = {
  "linux-x64-gnu": { os: "linux", cpu: "x64", libc: "glibc" },
  "linux-x64-musl": { os: "linux", cpu: "x64", libc: "musl" },
  "linux-arm64-gnu": { os: "linux", cpu: "arm64", libc: "glibc" },
  "linux-arm64-musl": { os: "linux", cpu: "arm64", libc: "musl" },
  "darwin-arm64": { os: "darwin", cpu: "arm64" },
  "win32-x64-msvc": { os: "win32", cpu: "x64" },
  "win32-arm64-msvc": { os: "win32", cpu: "arm64" },
};
const supportedPackages = Object.keys(packagePlatforms);
const supportedSet = new Set(supportedPackages);
const platformDependencies = Object.entries(rootPackage.optionalDependencies ?? {}).filter(
  ([name]) => name.startsWith(packagePrefix),
);

const declaredPackages = new Set(
  platformDependencies.map(([name]) => name.slice(packagePrefix.length)),
);
if (rootPackage.name !== "@3leaps/sysprims") {
  throw new Error(`root package name must be @3leaps/sysprims, got ${rootPackage.name}`);
}
if (
  declaredPackages.size !== supportedSet.size ||
  [...supportedSet].some((packageDir) => !declaredPackages.has(packageDir))
) {
  throw new Error(`platform optionalDependencies must be exactly: ${supportedPackages.join(", ")}`);
}

const generatedPackages = fs
  .readdirSync(path.join(packageRoot, "npm"), { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name);
if (
  generatedPackages.length !== supportedSet.size ||
  generatedPackages.some((packageDir) => !supportedSet.has(packageDir))
) {
  throw new Error(`generated platform packages must be exactly: ${supportedPackages.join(", ")}`);
}

for (const [name, dependencyVersion] of platformDependencies) {
  const packageDir = name.slice(packagePrefix.length);

  const packagePath = path.join(packageRoot, "npm", packageDir, "package.json");
  if (!fs.existsSync(packagePath)) {
    throw new Error(`missing generated platform manifest: ${packagePath}`);
  }
  const platformPackage = JSON.parse(fs.readFileSync(packagePath, "utf8"));
  if (
    dependencyVersion !== rootPackage.version ||
    platformPackage.version !== rootPackage.version
  ) {
    throw new Error(
      `${name} version mismatch: root=${rootPackage.version} dependency=${dependencyVersion} platform=${platformPackage.version}`,
    );
  }
  if (platformPackage.name !== name) {
    throw new Error(`${packageDir} name mismatch: expected ${name}, got ${platformPackage.name}`);
  }
  const expectedMain = `sysprims.${packageDir}.node`;
  if (platformPackage.main !== expectedMain) {
    throw new Error(
      `${packageDir} main mismatch: expected ${expectedMain}, got ${platformPackage.main}`,
    );
  }
  if (JSON.stringify(platformPackage.files) !== JSON.stringify([expectedMain])) {
    throw new Error(`${packageDir} files must contain only ${expectedMain}`);
  }
  for (const [field, expected] of Object.entries(packagePlatforms[packageDir])) {
    if (JSON.stringify(platformPackage[field]) !== JSON.stringify([expected])) {
      throw new Error(`${packageDir} ${field} must be exactly ${expected}`);
    }
  }
  if (packagePlatforms[packageDir].libc === undefined && platformPackage.libc !== undefined) {
    throw new Error(`${packageDir} must not declare libc`);
  }
  if (requested.size > 0 && !requested.has(packageDir)) continue;

  const packed = spawnSync("npm", ["pack", "--dry-run", "--json", `./npm/${packageDir}`], {
    cwd: packageRoot,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  if (packed.status !== 0) {
    throw new Error(`npm pack failed for ${name}: ${packed.stderr || packed.stdout}`);
  }
  const metadata = JSON.parse(packed.stdout)[0];
  if (metadata.name !== name || metadata.version !== rootPackage.version) {
    throw new Error(
      `${packageDir} packed metadata mismatch: expected ${name}@${rootPackage.version}, got ${metadata.name}@${metadata.version}`,
    );
  }
  const nativeFiles = metadata.files
    .map((file) => file.path)
    .filter((filePath) => filePath.endsWith(".node"));
  if (JSON.stringify(nativeFiles) !== JSON.stringify([expectedMain])) {
    throw new Error(
      `${packageDir} packed metadata must contain only ${expectedMain} as native code`,
    );
  }
  console.log(`verified ${metadata.id}`);
  requested.delete(packageDir);
}

if (requested.size > 0) {
  throw new Error(`unknown platform package directories: ${[...requested].join(", ")}`);
}
