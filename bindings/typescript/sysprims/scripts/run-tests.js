const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

function walk(dir, out) {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const e of entries) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      walk(p, out);
      continue;
    }
    if (e.isFile() && e.name.endsWith('.test.js')) {
      out.push(p);
    }
  }
}

function main() {
  const packageRoot = path.resolve(__dirname, '..');
  const distTestRoot = path.join(packageRoot, 'dist-test');

  if (!fs.existsSync(distTestRoot)) {
    console.error(`Missing dist-test directory: ${distTestRoot}`);
    process.exit(1);
  }

  const candidates = [];
  const preferred = path.join(distTestRoot, 'test');
  if (fs.existsSync(preferred)) {
    walk(preferred, candidates);
  } else {
    walk(distTestRoot, candidates);
  }

  candidates.sort();

  if (candidates.length === 0) {
    console.error(`No .test.js files found under: ${distTestRoot}`);
    process.exit(1);
  }

  const args = ['--test', ...candidates];
  const res = spawnSync(process.execPath, args, {
    stdio: 'inherit',
    cwd: packageRoot,
    env: process.env,
  });

  process.exit(res.status ?? 1);
}

main();
