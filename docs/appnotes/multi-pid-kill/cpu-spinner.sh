#!/bin/sh
# cpu-spinner.sh - Simple CPU load generator
#
# Usage: ./cpu-spinner.sh [label]
#
# Spins in a tight loop until killed. Used to demonstrate
# sysprims multi-PID kill and process inspection.

LABEL="${1:-worker}"
echo "[$LABEL] PID $$ spinning..."

# Spin until killed
i=0
while true; do
	i=$((i + 1))
	# Do some busywork to burn CPU
	: $((i * i * i))
done
