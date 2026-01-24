import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import test from "node:test";

import {
  forceKill,
  listeningPorts,
  processList,
  procGet,
  SysprimsError,
  SysprimsErrorCode,
  selfPGID,
  selfSID,
  terminate,
} from "../src/index";

// -----------------------------------------------------------------------------
// Test Helpers
// -----------------------------------------------------------------------------

/**
 * Spawn a short-lived child process and return its PID after it exits.
 * This provides a deterministic "dead PID" for NotFound assertions.
 */
async function getDeadPid(): Promise<number> {
  const child = spawn(process.execPath, ["-e", "process.exit(0)"], {
    stdio: "ignore",
  });
  const pid = child.pid;
  if (pid === undefined) {
    throw new Error("Failed to spawn child process");
  }
  await once(child, "exit");
  return pid;
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

test("terminate throws NotFound for exited process", async () => {
  const deadPid = await getDeadPid();

  assert.throws(
    () => terminate(deadPid),
    (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotFound,
    "terminate should throw NotFound for exited process",
  );
});

test("forceKill throws NotFound for exited process", async () => {
  const deadPid = await getDeadPid();

  assert.throws(
    () => forceKill(deadPid),
    (e: unknown) => e instanceof SysprimsError && e.code === SysprimsErrorCode.NotFound,
    "forceKill should throw NotFound for exited process",
  );
});
