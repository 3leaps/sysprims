#!/bin/sh
# spawn-workers.sh - Orchestrator that spawns CPU worker processes
#
# Usage: ./spawn-workers.sh [count]
#
# Spawns N worker processes (default: 3) that burn CPU.
# The orchestrator stays alive, allowing you to demonstrate:
#   1. Multi-PID kill (kill workers, orchestrator survives)
#   2. Tree termination (kill orchestrator, workers die too)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COUNT="${1:-3}"

echo "Orchestrator PID: $$"
echo "Spawning $COUNT workers..."

PIDS=""
for i in $(seq 1 "$COUNT"); do
	"$SCRIPT_DIR/cpu-spinner.sh" "worker-$i" &
	PIDS="$PIDS $!"
done

echo "Worker PIDs:$PIDS"
echo ""
echo "--- Orchestrator ready ---"
echo "To kill workers only (orchestrator survives):"
echo "  sysprims kill$PIDS -s TERM --json"
echo ""
echo "To kill entire tree (orchestrator + workers):"
echo "  sysprims kill $$ -s TERM"
echo ""
echo "Press Ctrl+C to stop orchestrator (workers become orphans)"
echo "---"

# Wait for all workers (or until we're killed)
wait
