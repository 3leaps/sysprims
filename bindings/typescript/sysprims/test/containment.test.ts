import assert from "node:assert/strict";
import os from "node:os";
import test from "node:test";

import {
  ContainedProcess,
  spawnContained,
  SysprimsError,
  SysprimsErrorCode,
} from "../src/index";

function isInvalid(error: unknown): boolean {
  return error instanceof SysprimsError && error.code === SysprimsErrorCode.InvalidArgument;
}

test("spawnContained rejects empty argv and signal 0", async () => {
  await assert.rejects(() => spawnContained([]), isInvalid);
  await assert.rejects(() => spawnContained(["true"], { signal: 0 }), isInvalid);
});

test("spawnContained happy path, dispose twice, and stale reuse", async (t) => {
  if (os.platform() === "win32") {
    t.skip("managed spawn is unix-only until Job assignment exists");
    return;
  }

  const first = await spawnContained(["true"], { graceTimeoutMs: 50, killTimeoutMs: 200 });
  const snap = await first.wait({ timeoutMs: 5000 });
  assert.equal(snap.leader_status, "completed");
  assert.equal(snap.tree_kill_reliability, "guaranteed");
  assert.equal(snap.boundary_strength, "cooperative_group");
  assert.ok(snap.completion?.status === "empty" || snap.completion?.status === "unknown");
  await first.close();
  await first.close();
  await assert.rejects(() => first.poll(), isInvalid);

  const second = await spawnContained(["true"], { graceTimeoutMs: 50, killTimeoutMs: 200 });
  await assert.rejects(() => first.identity(), isInvalid);
  await second.close();
});

test("terminate keeps spawn reliability and second call is inert", async (t) => {
  if (os.platform() === "win32") {
    t.skip("managed spawn is unix-only until Job assignment exists");
    return;
  }

  const handle = await spawnContained(["sleep", "30"], {
    graceTimeoutMs: 50,
    killTimeoutMs: 500,
  });
  const before = await handle.identity();
  const snap = await handle.terminate();
  assert.equal(snap.leader_status, "terminated");
  assert.equal(snap.tree_kill_reliability, before.tree_kill_reliability);
  const again = await handle.terminate();
  assert.equal(again.handle_state, "inert");
  await handle.close();
});

test("cancel wait during terminate leaves native owned or inert", async (t) => {
  if (os.platform() === "win32") {
    t.skip("managed spawn is unix-only until Job assignment exists");
    return;
  }

  const handle = await spawnContained(["sleep", "30"], {
    graceTimeoutMs: 80,
    killTimeoutMs: 500,
  });
  const controller = new AbortController();
  const waitPromise = handle.wait({ timeoutMs: 5000, signal: controller.signal });
  const terminatePromise = handle.terminate();
  controller.abort();
  await assert.rejects(() => waitPromise, (error: unknown) => {
    return error instanceof SysprimsError && error.code === SysprimsErrorCode.Timeout;
  });
  const terminated = await terminatePromise;
  assert.ok(terminated.handle_state === "inert" || terminated.leader_status === "terminated");
  const poll = await handle.poll();
  assert.equal(poll.handle_state, "inert");
  await handle.close();
});

test("close races wait and terminate", async (t) => {
  if (os.platform() === "win32") {
    t.skip("managed spawn is unix-only until Job assignment exists");
    return;
  }

  const handle = await spawnContained(["sleep", "30"], {
    graceTimeoutMs: 50,
    killTimeoutMs: 500,
  });
  await Promise.allSettled([
    handle.wait({ timeoutMs: 2000 }),
    handle.terminate(),
    handle.close(),
  ]);
  await handle.close();
});

test("windows managed spawn fails before spawn", async (t) => {
  if (os.platform() !== "win32") {
    t.skip("windows-only rejection fixture");
    return;
  }
  await assert.rejects(
    () => spawnContained(["cmd", "/C", "echo should-not-run"]),
    (error: unknown) =>
      error instanceof SysprimsError && error.code === SysprimsErrorCode.NotSupported,
  );
});

test("ContainedProcess is a class", () => {
  assert.equal(typeof ContainedProcess, "function");
});
