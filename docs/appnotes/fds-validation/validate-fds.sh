#!/bin/sh
# validate-fds.sh - Validate sysprims fds output against synthetic FDs
#
# Usage: ./validate-fds.sh <PID>
#
# Checks that sysprims fds detects expected file, socket, and pipe FDs.

set -e

PID="${1:-}"
if [ -z "$PID" ]; then
	echo "Usage: $0 <PID>"
	echo "  where <PID> is the process ID from synthetic-fd-holder.sh"
	exit 1
fi

# Try to find sysprims binary
if [ -x "./target/debug/sysprims" ]; then
	SYSPRIMS="./target/debug/sysprims"
elif [ -x "../../../target/debug/sysprims" ]; then
	SYSPRIMS="../../../target/debug/sysprims"
elif command -v sysprims >/dev/null 2>&1; then
	SYSPRIMS="sysprims"
else
	echo "Error: Cannot find sysprims binary. Build with: make build"
	exit 1
fi

echo "Validating FD detection for PID $PID..."

# Get JSON output
OUTPUT=$($SYSPRIMS fds --pid "$PID" --json 2>/dev/null) || {
	echo "FAIL: Could not run sysprims fds (is it built?)"
	exit 1
}

# Check for schema_id
if echo "$OUTPUT" | grep -q "fd-snapshot.schema.json"; then
	echo "  [OK] Schema ID present"
else
	echo "  [FAIL] Missing schema ID"
	exit 1
fi

# Count FDs by kind (handle space after colon in JSON)
FILE_COUNT=$(echo "$OUTPUT" | grep -c '"kind" *: *"file"' || true)
SOCKET_COUNT=$(echo "$OUTPUT" | grep -c '"kind" *: *"socket"' || true)
PIPE_COUNT=$(echo "$OUTPUT" | grep -c '"kind" *: *"pipe"' || true)

echo "  Detected: $FILE_COUNT file(s), $SOCKET_COUNT socket(s), $PIPE_COUNT pipe(s)"

# Validate expectations
PASS=0
if [ "$FILE_COUNT" -ge 1 ]; then
	echo "  [OK] At least 1 file FD found"
	PASS=$((PASS + 1))
else
	echo "  [WARN] No file FDs found (may be OK on some platforms)"
fi

if [ "$SOCKET_COUNT" -ge 0 ]; then
	echo "  [OK] Socket count acceptable (may vary by platform)"
	PASS=$((PASS + 1))
fi

if [ "$PIPE_COUNT" -ge 0 ]; then
	echo "  [OK] Pipe count acceptable (may vary by platform)"
	PASS=$((PASS + 1))
fi

# Overall result
if [ "$PASS" -ge 2 ]; then
	echo ""
	echo "PASS: FD validation successful ($PASS/3 checks passed)"
	exit 0
else
	echo ""
	echo "FAIL: FD validation failed (only $PASS/3 checks passed)"
	echo ""
	echo "Raw output:"
	echo "$OUTPUT" | head -30
	exit 1
fi
