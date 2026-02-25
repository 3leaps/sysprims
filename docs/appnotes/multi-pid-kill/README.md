# App Note: Multi-PID Kill

Demonstrates batch signal delivery to multiple processes, contrasted with tree termination.

## The Scenario

You have runaway processes consuming CPU. You want to kill specific PIDs without:

- Killing parent/orchestrator processes
- Running multiple `kill` commands
- Losing track of which signals succeeded or failed

## Prerequisites

```bash
# From repository root
make build
```

## Setup: Spawn Test Processes

In one terminal, start the orchestrator:

```bash
cd docs/appnotes/multi-pid-kill
chmod +x *.sh
./spawn-workers.sh 3
```

Output:

```
Orchestrator PID: 12345
Spawning 3 workers...
[worker-1] PID 12346 spinning...
[worker-2] PID 12347 spinning...
[worker-3] PID 12348 spinning...
Worker PIDs: 12346 12347 12348
```

## Demo 1: Find High-CPU Processes

In another terminal, use sysprims to find the workers:

```bash
# Find processes by name (uses lifetime-average CPU)
./target/debug/sysprims pstat --name bash --cpu-above 50 --table

# For Activity Monitor-style instantaneous CPU, use --cpu-mode monitor
# (this samples over 1 second by default)
./target/debug/sysprims pstat --cpu-mode monitor --cpu-above 50 --sort cpu --table

# Get PIDs as JSON for scripting
./target/debug/sysprims pstat --cpu-mode monitor --cpu-above 50 --json | jq -r '.processes[].pid'
```

**Note**: In `--cpu-mode monitor`, CPU can exceed 100% when a process uses multiple cores.

## Demo 2: Multi-PID Kill (Workers Only)

Kill just the workers, leaving the orchestrator alive:

```bash
# Replace with actual PIDs from spawn-workers.sh output
./target/debug/sysprims kill 12346 12347 12348 -s TERM --json
```

Output:

```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/signal/v1.0.0/batch-kill-result.schema.json",
  "signal_sent": 15,
  "succeeded": [12346, 12347, 12348],
  "failed": []
}
```

**Key behavior**: Exit code 0 if all succeed, 1 if any fail.

The orchestrator (`spawn-workers.sh`) is still running - you can spawn more workers.

## Demo 3: Scripted Workflow

Combine pstat and kill for a complete workflow:

```bash
# Find and kill all cpu-spinner processes in one pipeline
PIDS=$(./target/debug/sysprims pstat --name cpu-spinner --json 2>/dev/null | jq -r '.processes[].pid' | tr '\n' ' ')
./target/debug/sysprims kill $PIDS -s TERM --json
```

## Demo 4: Tree Termination (Contrast)

Restart the orchestrator:

```bash
./spawn-workers.sh 3
```

Now kill the orchestrator - all workers die with it:

```bash
# Replace 12345 with the orchestrator PID
./target/debug/sysprims kill 12345 -s TERM
```

All processes terminate because they share a process group.

## Validation Semantics

Multi-PID kill validates all PIDs before sending any signals:

```bash
# Mix of valid and invalid PIDs
./target/debug/sysprims kill 12346 99999 12347 -s TERM --json
```

Output shows which succeeded and which failed:

```json
{
  "succeeded": [12346, 12347],
  "failed": [{ "pid": 99999, "error": "Process 99999 not found" }]
}
```

## Cleanup

Kill any remaining processes:

```bash
pkill -f cpu-spinner.sh
pkill -f spawn-workers.sh
```

## When to Use Each Approach

| Scenario                        | Command                               |
| ------------------------------- | ------------------------------------- |
| Kill specific runaway processes | `sysprims kill PID1 PID2 ... -s TERM` |
| Kill entire process tree        | `sysprims kill PARENT_PID -s TERM`    |
| Kill with timeout escalation    | `sysprims timeout 5s -- command`      |

## See Also

- [Runaway Process Diagnosis Guide](../../guides/runaway-process-diagnosis.md)
- [Signal Specification](../../design/sysprims-signal/signal-spec.md)
