import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";
import { synchronize } from "./version-pack.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const versionScript = join(scriptDir, "version-pack.mjs");
const guardScript = join(scriptDir, "release-guard-tag-version.sh");
const roots = [];
const platformDirectories = [
  "darwin-arm64",
  "linux-arm64-gnu",
  "linux-arm64-musl",
  "linux-x64-gnu",
  "linux-x64-musl",
  "win32-arm64-msvc",
  "win32-x64-msvc",
];
const platformNames = platformDirectories.map(
  (directory) => `@3leaps/sysprims-${directory}`,
);

after(() => {
  for (const root of roots) {
    rmSync(root, { recursive: true, force: true });
  }
});

function command(root, executable, args, options = {}) {
  return spawnSync(executable, args, {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, ...options.env },
  });
}

function mustRun(root, executable, args, options = {}) {
  const result = command(root, executable, args, options);
  assert.equal(
    result.status,
    0,
    `${executable} ${args.join(" ")}\n${result.stderr}${result.stdout}`,
  );
  return result;
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function createFixture() {
  const root = mkdtempSync(join(tmpdir(), "sysprims-version-pack-test-"));
  roots.push(root);
  mkdirSync(join(root, "crate", "src"), { recursive: true });
  writeFileSync(join(root, "VERSION"), "0.2.1\n");
  writeFileSync(
    join(root, "Cargo.toml"),
    `[workspace]
members = ["crate"]
resolver = "2"

[workspace.package]
version = "0.2.1"
edition = "2021"

[workspace.dependencies]
sysprims-fixture = { version = "0.2.1", path = "crate" }
serde = "0.2.1"
`,
  );
  writeFileSync(
    join(root, "crate", "Cargo.toml"),
    `[package]
name = "sysprims-fixture"
version.workspace = true
edition.workspace = true

[lib]
path = "src/lib.rs"
`,
  );
  writeFileSync(join(root, "crate", "src", "lib.rs"), "pub fn fixture() {}\n");
  mustRun(root, "cargo", ["generate-lockfile"]);
  mkdirSync(join(root, "bindings/go/sysprims"), { recursive: true });
  writeFileSync(
    join(root, "bindings/go/sysprims/README.md"),
    `# sysprims Go bindings

\`\`\`bash
go get github.com/3leaps/sysprims/bindings/go/sysprims@v0.2.1
\`\`\`

The Go module resolves \`v0.2.1\` through the repository's path-prefixed
\`bindings/go/sysprims/v0.2.1\` tag. That tag and the canonical \`v0.2.1\` tag
identify the same reviewed commit.

v0.1.14 remains part of the historical API notes.
`,
  );

  const optionalDependencies = Object.fromEntries(
    platformNames.map((name) => [name, "0.2.1"]),
  );
  writeJson(join(root, "bindings/typescript/sysprims/package.json"), {
    name: "@3leaps/sysprims",
    version: "0.2.1",
    optionalDependencies,
    devDependencies: {
      typescript: "0.2.1",
    },
  });
  for (let index = 0; index < platformDirectories.length; index += 1) {
    writeJson(
      join(
        root,
        "bindings/typescript/sysprims/npm",
        platformDirectories[index],
        "package.json",
      ),
      {
        name: platformNames[index],
        version: "0.2.1",
        dependencies: {
          "unrelated-same-semver": "0.2.1",
        },
      },
    );
  }
  const packages = {
    "": {
      name: "@3leaps/sysprims",
      version: "0.2.1",
      optionalDependencies,
      devDependencies: {
        typescript: "0.2.1",
      },
    },
  };
  for (const name of platformNames) {
    const bareName = name.replace("@3leaps/", "");
    packages[`node_modules/${name}`] = {
      version: "0.2.1",
      resolved: `https://registry.npmjs.org/${name}/-/${bareName}-0.2.1.tgz`,
      integrity: "sha512-real-existing-evidence",
    };
  }
  writeJson(join(root, "bindings/typescript/sysprims/package-lock.json"), {
    name: "@3leaps/sysprims",
    version: "0.2.1",
    lockfileVersion: 3,
    requires: true,
    packages,
  });
  return root;
}

function version(root, ...args) {
  return command(root, process.execPath, [
    versionScript,
    ...args,
    "--root",
    root,
  ]);
}

function initGit(root) {
  mustRun(root, "git", ["init", "-b", "main"]);
  mustRun(root, "git", ["config", "user.name", "Version Pack Test"]);
  mustRun(root, "git", ["config", "user.email", "version-pack@example.invalid"]);
  mustRun(root, "git", ["add", "."]);
  mustRun(root, "git", ["commit", "-m", "fixture"]);
}

function guard(root, mode, extraEnv = {}) {
  return command(root, "bash", [guardScript], {
    env: {
      SYSPRIMS_REPO_ROOT: root,
      SYSPRIMS_TAG_GUARD_MODE: mode,
      ...extraEnv,
    },
  });
}

test("check accepts a coherent pack and ignores unrelated semvers", () => {
  const root = createFixture();
  const result = version(root, "check");
  assert.equal(result.status, 0, result.stderr);
});

test("check fails precisely when one native package is stale", () => {
  const root = createFixture();
  const path = join(
    root,
    "bindings/typescript/sysprims/npm/linux-x64-gnu/package.json",
  );
  const pkg = readJson(path);
  pkg.version = "0.2.0";
  writeJson(path, pkg);

  const result = version(root, "check");
  assert.notEqual(result.status, 0);
  assert.match(
    result.stderr,
    /TypeScript native package @3leaps\/sysprims-linux-x64-gnu is 0\.2\.0/,
  );
});

test("sync updates owned fields and removes only stale resolution evidence", () => {
  const root = createFixture();
  writeFileSync(join(root, "VERSION"), "0.2.2\n");

  const result = version(root, "sync");
  assert.equal(result.status, 0, `${result.stderr}${result.stdout}`);

  const rootPackage = readJson(
    join(root, "bindings/typescript/sysprims/package.json"),
  );
  assert.equal(rootPackage.version, "0.2.2");
  assert.equal(rootPackage.devDependencies.typescript, "0.2.1");
  for (const value of Object.values(rootPackage.optionalDependencies)) {
    assert.equal(value, "0.2.2");
  }

  const nativePackage = readJson(
    join(
      root,
      "bindings/typescript/sysprims/npm/linux-x64-gnu/package.json",
    ),
  );
  assert.equal(nativePackage.version, "0.2.2");
  assert.equal(
    nativePackage.dependencies["unrelated-same-semver"],
    "0.2.1",
  );

  const lock = readJson(
    join(root, "bindings/typescript/sysprims/package-lock.json"),
  );
  assert.equal(lock.version, "0.2.2");
  assert.equal(lock.packages[""].version, "0.2.2");
  assert.equal(
    lock.packages[""].devDependencies.typescript,
    "0.2.1",
  );
  for (const name of platformNames) {
    assert.equal(lock.packages[`node_modules/${name}`], undefined);
  }
  assert.match(
    readFileSync(join(root, "Cargo.lock"), "utf8"),
    /name = "sysprims-fixture"\nversion = "0\.2\.2"/,
  );
  assert.match(
    readFileSync(join(root, "Cargo.toml"), "utf8"),
    /sysprims-fixture = \{ version = "0\.2\.2", path = "crate" \}/,
  );
  const goReadme = readFileSync(
    join(root, "bindings/go/sysprims/README.md"),
    "utf8",
  );
  assert.match(
    goReadme,
    /go get github\.com\/3leaps\/sysprims\/bindings\/go\/sysprims@v0\.2\.2/,
  );
  assert.match(goReadme, /The Go module resolves `v0\.2\.2`/);
  assert.match(goReadme, /`bindings\/go\/sysprims\/v0\.2\.2` tag/);
  assert.match(goReadme, /canonical `v0\.2\.2` tag/);
  assert.match(goReadme, /v0\.1\.14 remains/);
  assert.equal(version(root, "check").status, 0);
});

test("check fails precisely when the Go README install version is stale", () => {
  const root = createFixture();
  const readmePath = join(root, "bindings/go/sysprims/README.md");
  writeFileSync(
    readmePath,
    readFileSync(readmePath, "utf8").replace(
      /sysprims@v0\.2\.1/,
      "sysprims@v0.2.0",
    ),
  );

  const result = version(root, "check");
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Go README install version is 0\.2\.0/);
});

test("check calls out stale npm resolution evidence without rewriting it", () => {
  const root = createFixture();
  const lockPath = join(
    root,
    "bindings/typescript/sysprims/package-lock.json",
  );
  const lock = readJson(lockPath);
  lock.packages[
    "node_modules/@3leaps/sysprims-linux-x64-gnu"
  ].version = "0.2.0";
  writeJson(lockPath, lock);

  const before = readFileSync(lockPath, "utf8");
  const result = version(root, "check");
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /stale npm platform resolution evidence/);
  assert.equal(readFileSync(lockPath, "utf8"), before);
});

test("set canonicalizes VERSION and synchronizes in one command", () => {
  const root = createFixture();
  writeFileSync(join(root, "VERSION"), "0.2.1\r\n");
  const result = version(root, "set", "0.2.2");
  assert.equal(result.status, 0, `${result.stderr}${result.stdout}`);
  assert.equal(readFileSync(join(root, "VERSION"), "utf8"), "0.2.2\n");
  assert.equal(version(root, "check").status, 0);
});

test("set synchronizes prerelease versions through Go README coordinates", () => {
  const root = createFixture();
  const result = version(root, "set", "0.2.2-rc.1");
  assert.equal(result.status, 0, `${result.stderr}${result.stdout}`);

  const goReadme = readFileSync(
    join(root, "bindings/go/sysprims/README.md"),
    "utf8",
  );
  assert.match(
    goReadme,
    /go get github\.com\/3leaps\/sysprims\/bindings\/go\/sysprims@v0\.2\.2-rc\.1/,
  );
  assert.match(goReadme, /The Go module resolves `v0\.2\.2-rc\.1`/);
  assert.match(goReadme, /`bindings\/go\/sysprims\/v0\.2\.2-rc\.1` tag/);
  assert.match(goReadme, /canonical `v0\.2\.2-rc\.1` tag/);
  assert.equal(version(root, "check").status, 0);
});

test("check rejects duplicate Go README release coordinates", () => {
  const root = createFixture();
  const readmePath = join(root, "bindings/go/sysprims/README.md");
  writeFileSync(
    readmePath,
    `${readFileSync(readmePath, "utf8")}\n` +
      "go get github.com/3leaps/sysprims/bindings/go/sysprims@v0.2.0\n",
  );

  const result = version(root, "check");
  assert.notEqual(result.status, 0);
  assert.match(
    result.stderr,
    /Go README install version appears 2 times: 0\.2\.1, 0\.2\.0/,
  );
});

test("patch, minor, and major bumps each synchronize the full pack", () => {
  const root = createFixture();
  for (const [component, expected] of [
    ["patch", "0.2.2"],
    ["minor", "0.3.0"],
    ["major", "1.0.0"],
  ]) {
    const result = version(root, "bump", component);
    assert.equal(result.status, 0, `${result.stderr}${result.stdout}`);
    assert.equal(readFileSync(join(root, "VERSION"), "utf8"), `${expected}\n`);
    assert.equal(version(root, "check").status, 0);
  }
});

test("set preflights every owned path before writing", () => {
  const root = createFixture();
  rmSync(
    join(
      root,
      "bindings/typescript/sysprims/npm/win32-arm64-msvc/package.json",
    ),
  );
  const result = version(root, "set", "0.2.2");
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /required version-pack path is missing/);
  assert.equal(readFileSync(join(root, "VERSION"), "utf8"), "0.2.1\n");
  assert.match(
    readFileSync(join(root, "Cargo.toml"), "utf8"),
    /version = "0\.2\.1"/,
  );
});

test("set restores every owned byte after a mid-sync failure", () => {
  const root = createFixture();
  assert.equal(version(root, "check").status, 0);
  const ownedPathsResult = version(root, "owned-paths");
  assert.equal(
    ownedPathsResult.status,
    0,
    `${ownedPathsResult.stderr}${ownedPathsResult.stdout}`,
  );
  const ownedPaths = ownedPathsResult.stdout.trim().split("\n");
  const before = new Map(
    ownedPaths.map((relativePath) => [
      relativePath,
      readFileSync(join(root, relativePath)),
    ]),
  );

  assert.throws(
    () =>
      synchronize(root, "0.2.2", {
        afterCargoSetVersion() {
          throw new Error("injected failure after Cargo writes");
        },
      }),
    /version-pack update rolled back: injected failure after Cargo writes/,
  );
  for (const [relativePath, expected] of before) {
    assert.deepEqual(
      readFileSync(join(root, relativePath)),
      expected,
      `${relativePath} was not restored byte-for-byte`,
    );
  }
});

test("pre-tag guard ignores nearest older tag and rejects a dirty tree", () => {
  const root = createFixture();
  initGit(root);
  mustRun(root, "git", ["tag", "-a", "v0.2.0", "-m", "older"]);

  const clean = guard(root, "pre-tag");
  assert.equal(clean.status, 0, `${clean.stderr}${clean.stdout}`);

  const wrongIntended = guard(root, "pre-tag", {
    SYSPRIMS_RELEASE_TAG: "v9.9.9",
  });
  assert.notEqual(wrongIntended.status, 0);
  assert.match(wrongIntended.stderr, /does not equal v0\.2\.1/);

  const packagePath = join(
    root,
    "bindings/typescript/sysprims/npm/linux-x64-gnu/package.json",
  );
  writeFileSync(packagePath, `${readFileSync(packagePath, "utf8")}\n`);
  const dirty = guard(root, "pre-tag");
  assert.notEqual(dirty.status, 0);
  assert.match(dirty.stderr, /requires a clean working tree/);
});

test("post-tag guard requires exact annotated and co-peeled tags", () => {
  const noTagRoot = createFixture();
  initGit(noTagRoot);
  const noTag = guard(noTagRoot, "post-tag");
  assert.notEqual(noTag.status, 0);
  assert.match(noTag.stderr, /exact canonical tag v0\.2\.1 does not exist/);

  const lightweightRoot = createFixture();
  initGit(lightweightRoot);
  mustRun(lightweightRoot, "git", ["tag", "v0.2.1"]);
  mustRun(lightweightRoot, "git", [
    "tag",
    "-a",
    "bindings/go/sysprims/v0.2.1",
    "-m",
    "go",
  ]);
  const lightweight = guard(lightweightRoot, "post-tag");
  assert.notEqual(lightweight.status, 0);
  assert.match(lightweight.stderr, /canonical tag v0\.2\.1 must be annotated/);

  const divergentRoot = createFixture();
  initGit(divergentRoot);
  mustRun(divergentRoot, "git", [
    "tag",
    "-a",
    "bindings/go/sysprims/v0.2.1",
    "-m",
    "go old",
  ]);
  writeFileSync(join(divergentRoot, "README.md"), "next commit\n");
  mustRun(divergentRoot, "git", ["add", "README.md"]);
  mustRun(divergentRoot, "git", ["commit", "-m", "next"]);
  mustRun(divergentRoot, "git", ["tag", "-a", "v0.2.1", "-m", "canonical"]);
  const divergent = guard(divergentRoot, "post-tag");
  assert.notEqual(divergent.status, 0);
  assert.match(divergent.stderr, /canonical and Go tags peel to different commits/);

  const coherentRoot = createFixture();
  initGit(coherentRoot);
  mustRun(coherentRoot, "git", ["tag", "-a", "v0.2.1", "-m", "canonical"]);
  mustRun(coherentRoot, "git", [
    "tag",
    "-a",
    "bindings/go/sysprims/v0.2.1",
    "-m",
    "go",
  ]);
  const coherent = guard(coherentRoot, "post-tag");
  assert.equal(coherent.status, 0, `${coherent.stderr}${coherent.stdout}`);
});
