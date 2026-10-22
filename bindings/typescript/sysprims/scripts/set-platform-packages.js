// Point the root package at the platform packages published with it.
//
// The committed package.json lists no platform packages, so the lock file
// stays installable with `npm ci`. Before publishing, this script checks the
// platform package directory against `napi.targets` and writes
// optionalDependencies for exactly those packages, at the root version.
//
// Usage: node scripts/set-platform-packages.js [--check] <npm-dir> [package.json]
//
// <npm-dir> must hold one directory per napi target, named by napi's platform
// suffix (for example linux-x64-gnu), each with a package.json for
// <root name>-<suffix> at the root version and its <binaryName>.<suffix>.node.
// --check verifies without writing.

const fs = require("node:fs");
const path = require("node:path");
const { parseTriple } = require("@napi-rs/cli");

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function expectedPlatforms(root) {
  const targets = root.napi?.targets ?? [];
  if (targets.length === 0) fail("package.json has no napi.targets");
  return targets.map((triple) => parseTriple(triple).platformArchABI).sort();
}

function main() {
  const args = process.argv.slice(2);
  const check = args[0] === "--check";
  if (check) args.shift();
  const [dir, rootArg] = args;
  if (!dir || args.length > 2) {
    fail("usage: set-platform-packages.js [--check] <npm-dir> [package.json]");
  }

  const rootPath = rootArg ?? path.resolve(__dirname, "..", "package.json");
  const root = JSON.parse(fs.readFileSync(rootPath, "utf8"));
  if (root.optionalDependencies) {
    fail("package.json already lists optionalDependencies");
  }

  const expected = expectedPlatforms(root);
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const unexpected = entries.filter((e) => !e.isDirectory()).map((e) => e.name);
  if (unexpected.length > 0) fail(`unexpected entries: ${unexpected.join(", ")}`);
  const actual = entries.map((e) => e.name).sort();
  if (actual.join(",") !== expected.join(",")) {
    fail(`platform directories [${actual}] != napi.targets [${expected}]`);
  }

  const binaryName = root.napi?.binaryName;
  if (!binaryName) fail("package.json has no napi.binaryName");
  const names = [];
  for (const platform of actual) {
    const manifest = JSON.parse(fs.readFileSync(path.join(dir, platform, "package.json"), "utf8"));
    if (manifest.name !== `${root.name}-${platform}`) {
      fail(`${platform}: package name ${manifest.name}`);
    }
    if (manifest.version !== root.version) {
      fail(`${manifest.name}: version ${manifest.version} != ${root.version}`);
    }
    const addon = path.join(dir, platform, `${binaryName}.${platform}.node`);
    if (!fs.existsSync(addon) || fs.statSync(addon).size === 0) {
      fail(`${manifest.name}: missing ${path.basename(addon)}`);
    }
    names.push(manifest.name);
  }

  if (check) {
    console.log(`[ok] ${names.length} platform packages match napi.targets`);
    return;
  }
  root.optionalDependencies = Object.fromEntries(names.map((name) => [name, root.version]));
  fs.writeFileSync(rootPath, `${JSON.stringify(root, null, 2)}\n`);
  console.log(`[ok] ${names.length} platform packages at ${root.version}`);
}

main();
