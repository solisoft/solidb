#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
HURL_DIR="$SCRIPT_DIR"

cd "$PROJECT_ROOT"

echo "=== SDBQL Hurl Test Runner ==="

ADMIN_PASSWORD="admin123"

# Check for debug binary, build if needed
if [ ! -f "./target/debug/solidb" ]; then
    echo "Building debug binary..."
    cargo build
else
    echo "Using existing debug binary."
fi

# Create temp data directory
TEMP_DIR=$(mktemp -d)
echo "Using temp data dir: $TEMP_DIR"

# Find available port
PORT=$(comm -23 <(seq 49152 65535 | sort) <(ss -tan | awk '{print $4}' | grep -oP '[0-9]+$' | sort -u) | shuf | head -1)
echo "Using port: $PORT"

# Cleanup function
cleanup() {
    echo "Cleaning up..."
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

# Start server with admin password
echo "Starting solidb on port $PORT..."
export SOLIDB_ADMIN_PASSWORD="$ADMIN_PASSWORD"
./target/debug/solidb --port "$PORT" --data-dir "$TEMP_DIR" > /tmp/solidb.log 2>&1 &
SERVER_PID=$!

# Wait for server to be ready
echo "Waiting for server to be ready..."
MAX_WAIT=30
WAITED=0
while [ $WAITED -lt $MAX_WAIT ]; do
    if curl -s "http://localhost:$PORT/" > /dev/null 2>&1; then
        echo "Server is ready!"
        break
    fi
    sleep 1
    WAITED=$((WAITED + 1))
done

if [ $WAITED -ge $MAX_WAIT ]; then
    echo "ERROR: Server failed to start within $MAX_WAIT seconds"
    cat /tmp/solidb.log
    exit 1
fi

# Run hurl tests
echo "Running hurl tests..."
cd "$HURL_DIR"
hurl --test --variable port="$PORT" sdbql_tests.hurl

echo "=== Tests completed ==="
