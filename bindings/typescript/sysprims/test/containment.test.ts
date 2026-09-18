import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { loadSysprims } from "../src/ffi";
import { ContainedProcess, spawnContained, SysprimsError, SysprimsErrorCode } from "../src/index";

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
  await assert.rejects(
    () => waitPromise,
    (error: unknown) => {
      return error instanceof SysprimsError && error.code === SysprimsErrorCode.Timeout;
    },
  );
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
  const [waitResult, terminateResult, closeResult] = await Promise.allSettled([
    handle.wait({ timeoutMs: 2000 }),
    handle.terminate(),
    handle.close(),
  ]);
  if (closeResult.status === "rejected") {
    await handle.identity();
    await handle.close();
  } else {
    await handle.close();
    await assert.rejects(() => handle.identity(), isInvalid);
  }
  for (const result of [waitResult, terminateResult]) {
    if (result.status === "rejected") {
      assert.ok(isInvalid(result.reason), `unexpected race error: ${result.reason}`);
    } else {
      assert.equal(result.value.handle_state, "inert");
      assert.equal(result.value.leader_status, "terminated");
    }
  }
  if (waitResult.status === "fulfilled" && terminateResult.status === "fulfilled") {
    assert.deepEqual(waitResult.value.identity, terminateResult.value.identity);
    assert.deepEqual(waitResult.value.completion, terminateResult.value.completion);
  }
});

test("failed close keeps the token retryable", async (t) => {
  if (os.platform() === "win32") {
    t.skip("managed spawn is unix-only until Job assignment exists");
    return;
  }

  const handle = await spawnContained(["sleep", "30"], {
    graceTimeoutMs: 50,
    killTimeoutMs: 500,
  });
  const native = loadSysprims();
  const original = native.sysprimsContainmentClose;
  native.sysprimsContainmentClose = async () => ({
    code: SysprimsErrorCode.InvalidArgument,
    message: "forced close failure",
  });
  try {
    await assert.rejects(() => handle.close(), isInvalid);
    await handle.identity();
  } finally {
    native.sysprimsContainmentClose = original;
  }
  await handle.close();
  await assert.rejects(() => handle.identity(), isInvalid);
});

test("already-aborted wait does not launch after abort", async (t) => {
  if (os.platform() === "win32") {
    t.skip("managed spawn is unix-only until Job assignment exists");
    return;
  }

  const handle = await spawnContained(["true"], { graceTimeoutMs: 50, killTimeoutMs: 200 });
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(
    () => handle.wait({ timeoutMs: 5000, signal: controller.signal }),
    (error: unknown) => error instanceof SysprimsError && error.code === SysprimsErrorCode.Timeout,
  );
  await handle.close();
});

test("invalid high signal does not spawn", async () => {
  const marker = path.join(os.tmpdir(), `sysprims-ts-high-signal-${process.pid}.marker`);
  try {
    fs.rmSync(marker, { force: true });
  } catch {
    // absent is fine
  }
  await assert.rejects(() => spawnContained(["touch", marker], { signal: 99 }), isInvalid);
  assert.equal(fs.existsSync(marker), false);
});

test("windows managed spawn fails before spawn", async (t) => {
  if (os.platform() !== "win32") {
    t.skip("windows-only rejection fixture");
    return;
  }
  const marker = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "sysprims-no-spawn-")), "marker");
  t.after(() => fs.rmSync(path.dirname(marker), { recursive: true, force: true }));
  await assert.rejects(
    () => spawnContained(["cmd", "/C", `echo spawned>"${marker}"`]),
    (error: unknown) =>
      error instanceof SysprimsError && error.code === SysprimsErrorCode.NotSupported,
  );
  assert.equal(fs.existsSync(marker), false);
});

test("ContainedProcess is a class", () => {
  assert.equal(typeof ContainedProcess, "function");
});

for (const [name, argv, expected] of [
  ["long-lived", ["sleep", "30"], "timed_out"],
  ["fast", ["true"], "completed"],
] as const) {
  test(`native deadline with delayed first observation: ${name}`, async (t) => {
    if (os.platform() === "win32") {
      t.skip("Unix containment");
      return;
    }
    const handle = await spawnContained([...argv], {
      executionTimeoutMs: 200,
      graceTimeoutMs: 20,
      killTimeoutMs: 200,
    });
    try {
      await delay(450);
      const deadline = performance.now() + 3000;
      let observed = await handle.identity();
      while (observed.handle_state !== "inert" && performance.now() < deadline) {
        await delay(10);
        observed = await handle.identity();
      }
      assert.equal(observed.handle_state, "inert");
      const result = await handle.wait({ timeoutMs: 1000 });
      assert.equal(result.leader_status, expected);
      if (expected === "completed") assert.equal(result.timed_out, false);
    } finally {
      await handle.close();
    }
  });
}

test("bounded wait returns while another operation finalizes", async (t) => {
  if (os.platform() === "win32") {
    t.skip("Unix containment");
    return;
  }
  const handle = await spawnContained(["sleep", "30"], { graceTimeoutMs: 800, killTimeoutMs: 200 });
  const terminating = handle.terminate();
  try {
    await delay(100);
    const started = performance.now();
    const result = await handle.wait({ timeoutMs: 10 });
    const elapsed = performance.now() - started;
    assert.equal(result.handle_state, "active");
    assert.equal(result.leader_status, "running");
    assert.ok(elapsed < 300, `10ms wait took ${elapsed}ms`);
    assert.equal((await terminating).leader_status, "terminated");
  } finally {
    await terminating;
    await handle.close();
  }
});

test("explicit close unregisters finalizer and never closes twice", (t) => {
  if (os.platform() === "win32") {
    t.skip("Unix containment");
    return;
  }
  // Isolate a deterministic FinalizationRegistry scheduler from the suite.
  // Flushing it represents a GC finalization turn after explicit close.
  execFileSync(
    process.execPath,
    [
      "-e",
      `
    const assert = require('node:assert/strict');
    const registries = [];
    global.FinalizationRegistry = class {
      constructor(callback) { this.callback = callback; this.entries = new Map(); registries.push(this); }
      register(target, value, token) { this.entries.set(token, value); }
      unregister(token) { return this.entries.delete(token); }
      flush() { for (const value of this.entries.values()) this.callback(value); this.entries.clear(); }
    };
    const {spawnContained} = require(${JSON.stringify(path.resolve(__dirname, "../src/index.js"))});
    const {loadSysprims} = require(${JSON.stringify(path.resolve(__dirname, "../src/ffi.js"))});
    (async () => {
      const native = loadSysprims();
      const close = native.sysprimsContainmentClose;
      let calls = 0;
      native.sysprimsContainmentClose = (...args) => { calls++; return close(...args); };
      const handle = await spawnContained(['true'], {graceTimeoutMs:20, killTimeoutMs:200});
      assert.equal(registries.length, 1);
      assert.equal(registries[0].entries.size, 1);
      await handle.close();
      await handle.close();
      assert.equal(registries[0].entries.size, 0);
      registries[0].flush();
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(calls, 1);
    })().catch(error => { console.error(error); process.exitCode = 1; });
  `,
    ],
    { timeout: 10000, stdio: "pipe" },
  );
});
