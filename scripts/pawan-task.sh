#!/bin/bash
# Convenience wrapper for running pawan tasks via systemd
# Usage: pawan-task.sh 'task description'

# Generate a unique unit name using current timestamp
UNIT_NAME="pawan-task-$(date +%s)"

# Run the task via systemd-run with the generated unit name
echo "Running pawan task: $*"
echo "Unit name: $UNIT_NAME"

systemd-run --unit="$UNIT_NAME" --wait /opt/pawan/target/release/pawan task "$@"

# Get the exit status of the systemd-run command
EXIT_STATUS=$?

echo "Task completed with exit status: $EXIT_STATUS"
exit $EXIT_STATUS