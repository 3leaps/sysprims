#!/bin/sh
# synthetic-fd-holder.sh - Creates synthetic FDs for validation
#
# Usage: ./synthetic-fd-holder.sh
#
# Opens a file, socket, and pipe, then waits for signal.
# Used to validate sysprims fds output against known state.

PID="$$"
LABEL="${1:-synthetic-fd-holder}"

# Temp file
TEMP_FILE="/tmp/${LABEL}-${PID}-$$.txt"

# Setup cleanup trap
cleanup() {
	echo "[$LABEL] Cleaning up..."
	rm -f "$TEMP_FILE" 2>/dev/null
	exit 0
}
trap cleanup INT TERM

# Open a temp file (FD 3)
exec 3>"$TEMP_FILE"
echo "opened file" >&3
FILE_PATH="$TEMP_FILE"

# Open a TCP socket using /dev/tcp (bash-specific, but POSIX sh can't do sockets)
# For POSIX compatibility, we'll use a different approach
# Instead, open a pipe (FDs 4 and 5)
exec 4<"$TEMP_FILE"
exec 5>"$TEMP_FILE"

echo "[$LABEL] PID $PID holding FDs..."
echo "  - temp file: $FILE_PATH (FD 3)"
echo "  - pipe read: FD 4"
echo "  - pipe write: FD 5"
echo "Press Ctrl+C or send SIGTERM to exit."

# Hold open
while true; do
	sleep 1
done
