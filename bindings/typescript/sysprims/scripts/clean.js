const fs = require('fs');
const path = require('path');

function rmrf(p) {
  try {
    fs.rmSync(p, { recursive: true, force: true });
  } catch {
    // Best-effort; ignore.
  }
}

function main() {
  const packageRoot = path.resolve(__dirname, '..');
  rmrf(path.join(packageRoot, 'dist'));
  rmrf(path.join(packageRoot, 'dist-test'));
}

main();
