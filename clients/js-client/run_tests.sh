#!/bin/bash
set -e

cd "$(dirname "$0")"

# Configuration
PORT=9999
ADMIN_USER="admin"
ADMIN_PASSWORD="admin"
DATA_DIR="./test_data_js"
PROJECT_ROOT="../.."

echo "Preparing Node.js test environment..."

if ! command -v node &> /dev/null; then
    echo "Error: node is not installed."
    exit 1
fi

# Install Dependencies
echo "Installing dependencies..."
npm install --quiet

# 1. Clean up old test data
rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

# 2. Build the server
echo "Building SoliDB..."
cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" --quiet

# 3. Start the server
SERVER_BIN="$PROJECT_ROOT/target/debug/solidb"
echo "Starting SoliDB on port $PORT..."
ENV_VARS="SOLIDB_PORT=$PORT SOLIDB_DATA_DIR=$DATA_DIR SOLIDB_ADMIN_PASSWORD=$ADMIN_PASSWORD RUST_LOG=info"
eval "$ENV_VARS $SERVER_BIN" > /dev/null 2>&1 &
SERVER_PID=$!

cleanup() {
    echo "stopping server..."
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
    rm -rf "$DATA_DIR"
}
trap cleanup EXIT

# Wait for server
echo "Waiting for server to start (PID: $SERVER_PID)..."
for i in {1..30}; do
    if nc -z localhost $PORT 2>/dev/null; then
        echo "Server is ready on port $PORT!"
        break
    fi
    sleep 0.1
done

if ! nc -z localhost $PORT 2>/dev/null; then
    echo "Error: Server failed to start on port $PORT"
    exit 1
fi

# 4. Run Tests
echo "Running JS Tests..."
export SOLIDB_PORT=$PORT
npm test

echo "Tests COMPLETED!"
