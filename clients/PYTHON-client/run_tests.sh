#!/bin/bash
set -e

cd "$(dirname "$0")"

# Configuration
PORT=9999
ADMIN_USER="admin"
ADMIN_PASSWORD="admin"
DATA_DIR="./test_data_python"
PROJECT_ROOT="../.."
VENV_DIR=".venv"

echo "Preparing Python test environment..."

if ! command -v python3 &> /dev/null; then
    echo "Error: python3 is not installed."
    exit 1
fi

# Setup Virtualenv
if [ ! -d "$VENV_DIR" ]; then
    echo "Creating virtualenv..."
    python3 -m venv "$VENV_DIR"
fi

source "$VENV_DIR/bin/activate"

# Install Dependencies
echo "Installing dependencies..."
pip install -r requirements.txt --quiet
pip install -e . --quiet # Install local package in editable mode

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
echo "Running Python Tests..."
export SOLIDB_PORT=$PORT
pytest -v tests/test_client.py

echo "Tests COMPLETED!"
