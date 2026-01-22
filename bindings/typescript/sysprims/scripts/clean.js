const fs = require("node:fs");
const path = require("node:path");

function rmrf(p) {
  try {
    fs.rmSync(p, { recursive: true, force: true });
  } catch {
    // Best-effort; ignore.
  }
}

function main() {
  const packageRoot = path.resolve(__dirname, "..");
  rmrf(path.join(packageRoot, "dist"));
  rmrf(path.join(packageRoot, "dist-test"));
}

main();
