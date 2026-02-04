import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import test from "node:test";

import {
  forceKill,
  listeningPorts,
  listFds,
  processList,
  procGet,
  SysprimsError,
  SysprimsErrorCode,
  selfPGID,
  selfSID,
  spawnInGroup,
  terminate,
  terminateTree,
  waitPID,
} from "../src/index";

// -----------------------------------------------------------------------------
// Test Helpers
// -----------------------------------------------------------------------------

async function waitForExit(child: ReturnType<typeof spawn>, timeoutMs: number): Promise<void> {
  await Promise.race([
    once(child, "exit"),
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error("child did not exit in time")), timeoutMs),
    ),
  ]);
}

function spawnLongRunningChild(): ReturnType<typeof spawn> {
  // Long-running process we fully control.
  // Using setInterval keeps the process alive until terminated.
  return spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
}

// -----------------------------------------------------------------------------
// Process Inspection Tests
// -----------------------------------------------------------------------------

test("procGet(process.pid) returns matching pid", () => {
  const info = procGet(process.pid);
  assert.equal(info.pid, process.pid);
});

test("processList() returns processes including current process", () => {
  const snapshot = processList();

  assert.ok(snapshot.schema_id, "snapshot should have schema_id");
  assert.ok(snapshot.timestamp, "snapshot should have timestamp");
  assert.ok(Array.isArray(snapshot.processes), "processes should be an array");
  assert.ok(snapshot.processes.length > 0, "should have at least one process");

  // Current process should be in the list
  const self = snapshot.processes.find((p) => p.pid === process.pid);
  assert.ok(self, "current process should be in the list");
});

test("processList({ pid_in: [...] }) filters correctly", () => {
  const snapshot = processList({ pid_in: [process.pid] });

  assert.equal(snapshot.processes.length, 1, "should return exactly one process");
  assert.equal(snapshot.processes[0].pid, process.pid);
});

test("processList({ name_contains }) filters correctly", () => {
  // Get current process name first
  const current = procGet(process.pid);
  const snapshot = processList({ name_contains: current.name });

  assert.ok(snapshot.processes.length >= 1, "should find at least current process");
  const found = snapshot.processes.find((p) => p.pid === process.pid);
  assert.ok(found, "current process should match its own name filter");
});

test("listFds(process.pid) returns a snapshot", () => {
  if (process.platform === "win32") {
    assert.throws(
      () => listFds(process.pid),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    return;
  }

  const snap = listFds(process.pid);
  assert.ok(snap.schema_id);
  assert.ok(snap.timestamp);
  assert.ok(snap.platform);
  assert.equal(snap.pid, process.pid);
  assert.ok(Array.isArray(snap.fds));
  assert.ok(Array.isArray(snap.warnings));
});

test("listeningPorts() returns a snapshot with required fields", () => {
  const snapshot = listeningPorts();

  assert.ok(snapshot.schema_id, "snapshot should have schema_id");
  assert.ok(snapshot.timestamp, "snapshot should have timestamp");
  assert.ok(snapshot.platform, "snapshot should have platform");
  assert.ok(Array.isArray(snapshot.bindings), "bindings should be an array");
  assert.ok(Array.isArray(snapshot.warnings), "warnings should be an array");
});

test("listeningPorts({ protocol: 'tcp' }) filters correctly", () => {
  const snapshot = listeningPorts({ protocol: "tcp" });

  for (const binding of snapshot.bindings) {
    assert.equal(binding.protocol, "tcp", "all bindings should be TCP");
  }
});

// -----------------------------------------------------------------------------
// Self Introspection Tests
// -----------------------------------------------------------------------------

test("selfPGID/selfSID are > 0 on Unix or NotSupported on Windows", () => {
  if (process.platform === "win32") {
    assert.throws(
      () => selfPGID(),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    assert.throws(
      () => selfSID(),
      (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotSupported,
    );
    return;
  }

  assert.ok(selfPGID() > 0);
  assert.ok(selfSID() > 0);
});

// -----------------------------------------------------------------------------
// Signal Tests (error-path only, using spawn-and-exit for safe PIDs)
// -----------------------------------------------------------------------------

test("terminate rejects pid 0", () => {
  assert.throws(
    () => terminate(0),
    (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.InvalidArgument,
  );
});

test("forceKill rejects pid 0", () => {
  assert.throws(
    () => forceKill(0),
    (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.InvalidArgument,
  );
});

test("waitPID(process.pid, 1ms) returns timed_out", () => {
  const pid = process.pid;
  const r = waitPID(pid, 1);
  assert.equal(r.pid, pid);
  assert.equal(r.timed_out, true);
});

test("terminate kills a spawned child process", async () => {
  const child = spawnLongRunningChild();
  const pid = child.pid;
  if (pid === undefined) {
    throw new Error("Failed to spawn child process");
  }

  // Give it a moment to start.
  await new Promise((r) => setTimeout(r, 50));

  terminate(pid);
  await waitForExit(child, 5000);
});

test("forceKill kills a spawned child process", async () => {
  const child = spawnLongRunningChild();
  const pid = child.pid;
  if (pid === undefined) {
    throw new Error("Failed to spawn child process");
  }

  await new Promise((r) => setTimeout(r, 50));

  forceKill(pid);
  await waitForExit(child, 5000);
});

test("terminateTree kills a spawned child process", async () => {
  const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
  const pid = child.pid;
  if (pid === undefined) {
    throw new Error("Failed to spawn child process");
  }

  await new Promise((r) => setTimeout(r, 50));

  const r = terminateTree(pid, { grace_timeout_ms: 100, kill_timeout_ms: 5000 });
  assert.equal(r.pid, pid);

  await Promise.race([
    once(child, "exit"),
    new Promise((_, reject) => setTimeout(() => reject(new Error("child did not exit")), 5000)),
  ]);
});

test("spawnInGroup returns a pid", () => {
  // This is a smoke test; we terminate via terminateTree to avoid leaking processes.
  const argv =
    process.platform === "win32" ? ["cmd", "/C", "ping -n 60 127.0.0.1 >NUL"] : ["sleep", "60"];
  const r = spawnInGroup({ argv });
  assert.ok(r.pid > 0);
  terminateTree(r.pid, { grace_timeout_ms: 100, kill_timeout_ms: 1000 });
});
