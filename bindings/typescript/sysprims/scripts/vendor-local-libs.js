/* eslint-disable no-console */
const fs = require("node:fs");
const path = require("node:path");

function mkdirp(dir) {
  fs.mkdirSync(dir, { recursive: true });
}

function copyFile(src, dest) {
  mkdirp(path.dirname(dest));
  fs.copyFileSync(src, dest);
}

function listSharedLibs(sharedDir) {
  if (!fs.existsSync(sharedDir)) return [];
  return fs
    .readdirSync(sharedDir)
    .filter((name) => name.endsWith(".so") || name.endsWith(".dylib") || name.endsWith(".dll"))
    .map((name) => path.join(sharedDir, name));
}

function main() {
  const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
  const packageRoot = path.resolve(__dirname, "..");
  const localRelease = path.join(repoRoot, "dist", "local", "release", "sysprims-ffi", "lib");

  const libRoot = path.join(packageRoot, "_lib");
  mkdirp(libRoot);

  const alreadyVendored = fs
    .readdirSync(libRoot, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .some((d) => {
      const dir = path.join(libRoot, d.name);
      try {
        return fs
          .readdirSync(dir)
          .some((f) => f.endsWith(".so") || f.endsWith(".dylib") || f.endsWith(".dll"));
      } catch {
        return false;
      }
    });

  if (!fs.existsSync(localRelease)) {
    if (alreadyVendored) {
      console.log(
        `[ok] _lib already populated; skipping local vendoring (missing ${localRelease})`,
      );
      return;
    }
    console.error(`Missing local sysprims-ffi staging dir: ${localRelease}`);
    console.error(
      "Either populate _lib/<platform>/ with shared libs, or run: make build-local-ffi-shared",
    );
    process.exit(1);
  }

  const platforms = fs
    .readdirSync(localRelease, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name);

  let copied = 0;
  for (const platform of platforms) {
    const sharedDir = path.join(localRelease, platform, "shared");
    const libs = listSharedLibs(sharedDir);
    for (const libPath of libs) {
      const dest = path.join(packageRoot, "_lib", platform, path.basename(libPath));
      copyFile(libPath, dest);
      copied++;
    }
  }

  if (copied === 0) {
    console.error(`No shared libraries found under: ${localRelease}/*/shared/`);
    console.error("Run: make build-local-ffi-shared");
    process.exit(1);
  }

  console.log(`[ok] Vendored ${copied} shared library file(s) into ${libRoot}`);
}

main();
