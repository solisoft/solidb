#!/bin/bash

# Path to your PID file
PID_FILE="./luaonbeans.pid"
PORT=9003

# Read the PID from the file (assuming it's a single integer on the first line)
PID=$(cat "$PID_FILE")

# Kill the process gracefully (SIGTERM)
if kill -TERM "$PID" 2>/dev/null; then
    echo "Process $PID killed successfully."
    # Optionally, remove the PID file
    rm -f "$PID_FILE"
else
    echo "Failed to kill process $PID (may already be dead or invalid)."
fi

BEANS_ENV=production ./luaonbeans.org -D . -sX -d -p $PORT -P luaonbeans.pid -L luaonbeans.log > /dev/null 2>&1 &

exit 0
