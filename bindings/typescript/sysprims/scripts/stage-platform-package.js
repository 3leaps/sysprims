const fs = require("node:fs");
const path = require("node:path");

const expectedFile = process.env.EXPECTED_FILE;
const packageDir = process.env.PACKAGE_DIR;

if (!expectedFile || !packageDir) {
  throw new Error("EXPECTED_FILE and PACKAGE_DIR are required");
}
if (!fs.existsSync(expectedFile)) {
  throw new Error(`expected target native addon was not produced: ${expectedFile}`);
}

const destination = path.join("npm", packageDir, expectedFile);
if (!fs.existsSync(path.dirname(destination))) {
  throw new Error(`napi package directory was not produced: ${path.dirname(destination)}`);
}
fs.copyFileSync(expectedFile, destination);
