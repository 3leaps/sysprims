#!/usr/bin/env node
// The one source for release-tag decisions. Everything is read as git blobs at
// a commit, never from the working tree:
//   VERSION, docs/releases/v<version>.json (Go publish/skip),
//   config/release/release-tags.txt, and the presence of go/recurrence/go.mod.
// Also the shared reader for an annotated tag's message bytes with any OpenPGP
// signature armor removed.
//
// CLI (exit 0 with output, or exit 1 with a message on stderr):
//   release-tag-set.mjs tags [--commit <rev>] [--tag vX.Y.Z]   one tag per line
//   release-tag-set.mjs body <tag-ref>                          message bytes
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const canonicalPrefixes = ["v", "bindings/go/sysprims/v", "go/recurrence/v"];
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function fail(message) {
  throw new Error(message);
}

function git(root, args, { binary = false, allowFailure = false } = {}) {
  const result = spawnSync("git", args, { cwd: root, encoding: binary ? "buffer" : "utf8" });
  if (result.error) fail(`cannot run git: ${result.error.message}`);
  if (result.status !== 0) {
    if (allowFailure) return null;
    fail(`git ${args.join(" ")} failed: ${String(result.stderr).trim()}`);
  }
  return result.stdout;
}

export function resolveCommit(root, rev) {
  const commit = git(root, ["rev-parse", "--verify", "--quiet", `${rev}^{commit}`], { allowFailure: true });
  if (!commit) fail(`cannot resolve ${rev} to a commit`);
  return commit.trim();
}

// Mode and content of a path at a commit; null when absent. Only regular
// files count, so a symlink or a directory never stands in for a blob.
function blob(root, commit, path) {
  const entry = git(root, ["ls-tree", commit, "--", path]).trim();
  if (!entry) return null;
  const [mode, type] = entry.split(/\s+/);
  if (type !== "blob" || (mode !== "100644" && mode !== "100755")) fail(`${path} at ${commit} is not a regular file`);
  return git(root, ["cat-file", "blob", `${commit}:${path}`]);
}

// The committed prefix list must be exactly the canonical list, in order. A
// shorter list would silently drop a companion tag from every verifier, so
// membership comes only from the plan and the tree, never from editing this
// file. Changing the list is a change to this resolver and its tests.
export function parsePrefixes(text) {
  const expected = `${canonicalPrefixes.join("\n")}\n`;
  if (text !== expected) {
    fail(`config/release/release-tags.txt must be exactly ${JSON.stringify(expected)}`);
  }
  return [...canonicalPrefixes];
}

// The required tag set for the release at `commit`. When `releaseTag` is
// given it must be the canonical tag for that commit's VERSION.
export function releaseTagSet(root = defaultRoot, commit = "HEAD", releaseTag = undefined) {
  const resolved = resolveCommit(root, commit);
  const versionText = blob(root, resolved, "VERSION");
  if (versionText === null) fail(`VERSION is missing at ${resolved}`);
  const version = versionText.replace(/\n$/, "");
  if (!semver.test(version)) fail(`VERSION at ${resolved} is not canonical X.Y.Z`);
  if (releaseTag !== undefined && releaseTag !== `v${version}`) {
    fail(`release tag ${releaseTag} does not match VERSION ${version} at ${resolved}`);
  }
  const planText = blob(root, resolved, `docs/releases/v${version}.json`);
  if (planText === null) fail(`release plan docs/releases/v${version}.json is missing at ${resolved}`);
  const disposition = JSON.parse(planText)?.surfaces?.go?.disposition;
  if (disposition !== "publish" && disposition !== "skip") fail(`unknown Go disposition ${JSON.stringify(disposition)} at ${resolved}`);
  const prefixesText = blob(root, resolved, "config/release/release-tags.txt");
  if (prefixesText === null) fail(`config/release/release-tags.txt is missing at ${resolved}`);
  const recurrence = blob(root, resolved, "go/recurrence/go.mod") !== null;
  const tags = [];
  for (const prefix of parsePrefixes(prefixesText)) {
    if (prefix === "bindings/go/sysprims/v" && disposition !== "publish") continue;
    if (prefix === "go/recurrence/v" && !recurrence) continue;
    tags.push(`${prefix}${version}`);
  }
  return { commit: resolved, version, goDisposition: disposition, recurrence, tags };
}

// Message bytes of an annotated tag: everything after the header, without a
// trailing OpenPGP signature block. Fails for a lightweight tag.
export function annotationBody(root, tagRef) {
  const ref = tagRef.startsWith("refs/") ? tagRef : `refs/tags/${tagRef}`;
  const type = git(root, ["cat-file", "-t", ref], { allowFailure: true });
  if (type === null || type.trim() !== "tag") fail(`${tagRef} is not an annotated tag`);
  const raw = git(root, ["cat-file", "tag", ref], { binary: true });
  return stripSignature(raw, tagRef);
}

export function stripSignature(raw, label = "tag") {
  const separator = raw.indexOf("\n\n");
  if (separator === -1) fail(`annotated tag object is malformed for ${label}`);
  let body = raw.subarray(separator + 2);
  const marker = Buffer.from("-----BEGIN PGP SIGNATURE-----\n");
  if (body.subarray(0, marker.length).equals(marker)) return Buffer.alloc(0);
  const at = body.indexOf(Buffer.concat([Buffer.from("\n"), marker]));
  if (at >= 0) body = body.subarray(0, at + 1);
  return body;
}

function main(argv) {
  const [command, ...rest] = argv;
  if (command === "tags") {
    let commit = "HEAD";
    let tag;
    for (let i = 0; i < rest.length; i += 2) {
      if (rest[i] === "--commit" && rest[i + 1]) commit = rest[i + 1];
      else if (rest[i] === "--tag" && rest[i + 1]) tag = rest[i + 1];
      else fail("usage: release-tag-set.mjs tags [--commit <rev>] [--tag vX.Y.Z]");
    }
    const set = releaseTagSet(process.cwd(), commit, tag);
    if (set.tags.length === 0) fail("empty release tag set");
    process.stdout.write(`${set.tags.join("\n")}\n`);
  } else if (command === "body" && rest.length === 1) {
    process.stdout.write(annotationBody(process.cwd(), rest[0]));
  } else {
    fail("usage: release-tag-set.mjs tags [--commit <rev>] [--tag vX.Y.Z] | body <tag>");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`error: ${error.message}\n`);
    process.exit(1);
  }
}
