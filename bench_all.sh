#!/bin/bash
# SoliDB Client Benchmark Suite
# Run this script from the root of the rust-db repository
# Results will be output in a format ready to copy-paste

# Note: not using set -e so script continues even if one benchmark fails

BENCH_PORT=9998
BENCH_PASSWORD="password"
BENCH_DATA_DIR="./bench_data_temp"
ITERATIONS=1000

echo "=============================================="
echo "  SoliDB Client Benchmark Suite"
echo "=============================================="
echo "Config: $ITERATIONS sequential inserts per client"
echo "Port: $BENCH_PORT"
echo ""

# Cleanup old data
rm -rf "$BENCH_DATA_DIR"
mkdir -p "$BENCH_DATA_DIR"

# Build everything in release mode
echo "[1/13] Building SoliDB server (release)..."
cargo build --release --quiet 2>/dev/null || cargo build --release

echo "[2/13] Building Rust benchmark..."
cargo build --release --bin benchmark --quiet 2>/dev/null || cargo build --release --bin benchmark

echo "[3/13] Installing client dependencies..."
# Go dependencies
echo "    Installing Go dependencies..."
cd clients/go-client && go mod download 2>/dev/null || go mod tidy 2>/dev/null
cd ../..

# Python dependencies
echo "    Installing Python dependencies..."
cd clients/PYTHON-client && pip install -q msgpack 2>/dev/null || pip3 install -q msgpack 2>/dev/null
cd ../..

# Node.js/Bun dependencies
echo "    Installing Node.js dependencies..."
cd clients/js-client && npm install --silent 2>/dev/null || bun install 2>/dev/null
cd ../..

# Ruby dependencies
echo "    Installing Ruby dependencies..."
cd clients/Ruby-client && bundle install --quiet 2>/dev/null || gem install msgpack 2>/dev/null
cd ../..

# PHP dependencies
echo "    Installing PHP dependencies..."
cd clients/PHP-client && composer install --quiet --no-interaction 2>/dev/null || true
cd ../..

# Start server
echo "[4/14] Starting SoliDB server..."
SOLIDB_ADMIN_PASSWORD="$BENCH_PASSWORD" ./target/release/solidb --port $BENCH_PORT --data-dir "$BENCH_DATA_DIR" > /dev/null 2>&1 &
SERVER_PID=$!
sleep 3

# Verify server is running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "ERROR: Server failed to start"
    exit 1
fi
echo "    Server running (PID: $SERVER_PID)"

# Function to extract result from output (macOS compatible)
extract_result() {
    local pattern=$1
    local output=$2
    echo "$output" | grep "$pattern" | sed "s/.*${pattern}//" | sed 's/[^0-9.]//g' | head -1
}

echo ""
echo "[5/14] Running Rust benchmark..."
cargo build --release --bin benchmark --quiet 2>/dev/null || cargo build --release --bin benchmark
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
RUST_OUTPUT=$(timeout 30s ./target/release/benchmark 2>&1 || echo "TIMEOUT_OR_ERROR")
RUST_RESULT=$(extract_result "RUST_BENCH_RESULT:" "$RUST_OUTPUT")
if [ -z "$RUST_RESULT" ] || [ "$RUST_RESULT" = "0" ]; then
    echo "    Rust: FAILED"
    echo "    Output: $(echo "$RUST_OUTPUT" | head -2)"
    RUST_RESULT="0"
else
    echo "    Rust: ${RUST_RESULT} ops/s"
fi

echo "[6/14] Running Go benchmark..."
cd clients/go-client
sed -i'' -e "s/127.0.0.1\", [0-9]*/127.0.0.1\", $BENCH_PORT/g" benchmark.go 2>/dev/null || true
sed -i'' -e "s/\"admin\", \"[^\"]*\"/\"admin\", \"$BENCH_PASSWORD\"/g" benchmark.go 2>/dev/null || true
GO_OUTPUT=$(go run benchmark.go 2>&1)
GO_RESULT=$(extract_result "GO_BENCH_RESULT:" "$GO_OUTPUT")
if [ -z "$GO_RESULT" ]; then
    echo "    Go: FAILED"
    GO_RESULT="0"
else
    echo "    Go: ${GO_RESULT} ops/s"
fi
cd ../..

echo "[6/11] Running Python benchmark..."
cd clients/PYTHON-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
PYTHON_OUTPUT=$(python3 benchmark.py 2>&1)
PYTHON_RESULT=$(extract_result "PYTHON_BENCH_RESULT:" "$PYTHON_OUTPUT")
if [ -z "$PYTHON_RESULT" ]; then
    echo "    Python: FAILED - $PYTHON_OUTPUT"
    PYTHON_RESULT="0"
else
    echo "    Python: ${PYTHON_RESULT} ops/s"
fi
cd ../..

echo "[7/11] Running Bun/JS benchmark..."
cd clients/js-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
if command -v bun &> /dev/null; then
    JS_OUTPUT=$(bun run benchmark.ts 2>&1)
else
    JS_OUTPUT=$(npx ts-node benchmark.ts 2>&1)
fi
JS_RESULT=$(extract_result "JS_BENCH_RESULT:" "$JS_OUTPUT")
if [ -z "$JS_RESULT" ]; then
    echo "    Bun/JS: FAILED"
    JS_RESULT="0"
else
    echo "    Bun/JS: ${JS_RESULT} ops/s"
fi
cd ../..

echo "[8/11] Running Ruby benchmark..."
cd clients/Ruby-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
RUBY_OUTPUT=$(ruby -Ilib benchmark.rb 2>&1)
RUBY_RESULT=$(extract_result "RUBY_BENCH_RESULT:" "$RUBY_OUTPUT")
if [ -z "$RUBY_RESULT" ]; then
    echo "    Ruby: FAILED"
    RUBY_RESULT="0"
else
    echo "    Ruby: ${RUBY_RESULT} ops/s"
fi
cd ../..

echo "[9/11] Running PHP benchmark..."
cd clients/PHP-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
PHP_OUTPUT=$(php benchmark.php 2>&1)
PHP_RESULT=$(extract_result "PHP_BENCH_RESULT:" "$PHP_OUTPUT")
if [ -z "$PHP_RESULT" ]; then
    echo "    PHP: FAILED"
    PHP_RESULT="0"
else
    echo "    PHP: ${PHP_RESULT} ops/s"
fi
cd ../..

echo "[10/13] Running Elixir benchmark..."
echo "    Elixir: SKIPPED (requires mix setup)"
ELIXIR_RESULT="0"

echo ""
echo "=== MULTI-CORE PARALLEL BENCHMARKS (8 workers, 10K inserts) ==="
echo ""

echo "[11/13] Running Go parallel benchmark..."
cd clients/go-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
GO_PARALLEL_OUTPUT=$(go run benchmark_parallel.go 2>&1)
GO_PARALLEL_RESULT=$(extract_result "GO_PARALLEL_BENCH_RESULT:" "$GO_PARALLEL_OUTPUT")
if [ -z "$GO_PARALLEL_RESULT" ]; then
    echo "    Go (parallel): FAILED"
    GO_PARALLEL_RESULT="0"
else
    echo "    Go (parallel): ${GO_PARALLEL_RESULT} ops/s"
fi
cd ../..

echo "[12/13] Running Python parallel benchmark..."
cd clients/PYTHON-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
PYTHON_PARALLEL_OUTPUT=$(python3 benchmark_parallel.py 2>&1)
PYTHON_PARALLEL_RESULT=$(extract_result "PYTHON_PARALLEL_BENCH_RESULT:" "$PYTHON_PARALLEL_OUTPUT")
if [ -z "$PYTHON_PARALLEL_RESULT" ]; then
    echo "    Python (parallel): FAILED"
    PYTHON_PARALLEL_RESULT="0"
else
    echo "    Python (parallel): ${PYTHON_PARALLEL_RESULT} ops/s"
fi
cd ../..

echo "[13/13] Stopping server..."
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

# Cleanup
rm -rf "$BENCH_DATA_DIR"

echo ""
echo "=============================================="
echo "  BENCHMARK RESULTS (copy this)"
echo "=============================================="
echo "=== Sequential (1K inserts, single connection) ==="
echo "RUST=$RUST_RESULT"
echo "GO=$GO_RESULT"
echo "PYTHON=$PYTHON_RESULT"
echo "JS=$JS_RESULT"
echo "RUBY=$RUBY_RESULT"
echo "PHP=$PHP_RESULT"
echo "ELIXIR=$ELIXIR_RESULT"
echo ""
echo "=== Parallel (10K inserts, 8 connections) ==="
echo "GO_PARALLEL=$GO_PARALLEL_RESULT"
echo "PYTHON_PARALLEL=$PYTHON_PARALLEL_RESULT"
echo ""
echo "Machine: $(uname -m) / $(cat /etc/os-release 2>/dev/null | grep PRETTY_NAME | cut -d'"' -f2 || uname -s)"
echo "Cores: $(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 'unknown')"
echo "=============================================="
