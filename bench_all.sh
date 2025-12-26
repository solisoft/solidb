#!/bin/bash
# SoliDB Client Benchmark Suite
# Run this script from the root of the rust-db repository
# Results will be output in a format ready to copy-paste

set -e

BENCH_PORT=9998
BENCH_PASSWORD="benchmark123"
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
echo "[1/11] Building SoliDB server (release)..."
cargo build --release --quiet 2>/dev/null || cargo build --release

echo "[2/11] Building Rust benchmark..."
cargo build --release --bin benchmark --quiet 2>/dev/null || cargo build --release --bin benchmark

# Start server
echo "[3/11] Starting SoliDB server..."
SOLIDB_ADMIN_PASSWORD="$BENCH_PASSWORD" ./target/release/solidb --port $BENCH_PORT --data-dir "$BENCH_DATA_DIR" > /dev/null 2>&1 &
SERVER_PID=$!
sleep 3

# Verify server is running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "ERROR: Server failed to start"
    exit 1
fi
echo "    Server running (PID: $SERVER_PID)"

# Function to run benchmark and extract result
run_bench() {
    local name=$1
    local cmd=$2
    echo -n "    $name: "
    result=$(eval "$cmd" 2>/dev/null | grep -oP '\d+\.\d+' | tail -1)
    if [ -n "$result" ]; then
        echo "$result ops/s"
        echo "$result"
    else
        echo "FAILED"
        echo "0"
    fi
}

echo ""
echo "[4/11] Running Rust benchmark..."
# Update Rust benchmark config
sed -i "s/127.0.0.1:[0-9]*/127.0.0.1:$BENCH_PORT/g" src/bin/benchmark.rs 2>/dev/null || true
sed -i "s/\"admin\", \"[^\"]*\"/\"admin\", \"$BENCH_PASSWORD\"/g" src/bin/benchmark.rs 2>/dev/null || true
cargo build --release --bin benchmark --quiet 2>/dev/null || true
RUST_RESULT=$(./target/release/benchmark 2>&1 | grep -oP 'RUST_BENCH_RESULT:\K[\d.]+' || echo "0")
echo "    Rust: ${RUST_RESULT:-0} ops/s"

echo "[5/11] Running Go benchmark..."
cd clients/go-client
# Update Go benchmark config
sed -i "s/127.0.0.1\", [0-9]*/127.0.0.1\", $BENCH_PORT/g" benchmark.go 2>/dev/null || true
sed -i "s/\"admin\", \"[^\"]*\"/\"admin\", \"$BENCH_PASSWORD\"/g" benchmark.go 2>/dev/null || true
GO_RESULT=$(go run benchmark.go 2>&1 | grep -oP 'GO_BENCH_RESULT:\K[\d.]+' || echo "0")
echo "    Go: ${GO_RESULT:-0} ops/s"
cd ../..

echo "[6/11] Running Python benchmark..."
cd clients/PYTHON-client
# Update Python benchmark config
sed -i "s/127.0.0.1\", [0-9]*/127.0.0.1\", $BENCH_PORT/g" benchmark.py 2>/dev/null || true
sed -i "s/\"admin\", \"[^\"]*\"/\"admin\", \"$BENCH_PASSWORD\"/g" benchmark.py 2>/dev/null || true
PYTHON_RESULT=$(python3 benchmark.py 2>&1 | grep -oP 'PYTHON_BENCH_RESULT:\K[\d.]+' || echo "0")
echo "    Python: ${PYTHON_RESULT:-0} ops/s"
cd ../..

echo "[7/11] Running Bun/JS benchmark..."
cd clients/js-client
# Update JS benchmark config
sed -i "s/127.0.0.1', [0-9]*/127.0.0.1', $BENCH_PORT/g" benchmark.ts 2>/dev/null || true
sed -i "s/'admin', '[^']*'/'admin', '$BENCH_PASSWORD'/g" benchmark.ts 2>/dev/null || true
if command -v bun &> /dev/null; then
    JS_RESULT=$(bun run benchmark.ts 2>&1 | grep -oP 'JS_BENCH_RESULT:\K[\d.]+' || echo "0")
else
    JS_RESULT=$(npx ts-node benchmark.ts 2>&1 | grep -oP 'JS_BENCH_RESULT:\K[\d.]+' || echo "0")
fi
echo "    Bun/JS: ${JS_RESULT:-0} ops/s"
cd ../..

echo "[8/11] Running Ruby benchmark..."
cd clients/Ruby-client
# Update Ruby benchmark config
sed -i "s/127.0.0.1', [0-9]*/127.0.0.1', $BENCH_PORT/g" benchmark.rb 2>/dev/null || true
sed -i "s/'_system', 'admin', 'admin'/'_system', 'admin', '$BENCH_PASSWORD'/g" benchmark.rb 2>/dev/null || true
RUBY_RESULT=$(ruby benchmark.rb 2>&1 | grep -oP 'RUBY_BENCH_RESULT:\K[\d.]+' || echo "0")
echo "    Ruby: ${RUBY_RESULT:-0} ops/s"
cd ../..

echo "[9/11] Running PHP benchmark..."
cd clients/PHP-client
# Update PHP benchmark config
sed -i "s/127.0.0.1', [0-9]*/127.0.0.1', $BENCH_PORT/g" benchmark.php 2>/dev/null || true
sed -i "s/'_system', 'admin', 'admin'/'_system', 'admin', '$BENCH_PASSWORD'/g" benchmark.php 2>/dev/null || true
PHP_RESULT=$(php benchmark.php 2>&1 | grep -oP 'PHP_BENCH_RESULT:\K[\d.]+' || echo "0")
echo "    PHP: ${PHP_RESULT:-0} ops/s"
cd ../..

echo "[10/11] Running Elixir benchmark..."
cd clients/elixir_client
# Update Elixir benchmark config
sed -i "s/127.0.0.1\", [0-9]*/127.0.0.1\", $BENCH_PORT/g" benchmark.exs 2>/dev/null || true
sed -i "s/\"admin\", \"password\"/\"admin\", \"$BENCH_PASSWORD\"/g" benchmark.exs 2>/dev/null || true
ELIXIR_RESULT=$(mix run benchmark.exs 2>&1 | grep -oP 'ELIXIR_BENCH_RESULT:\K[\d.]+' || echo "0")
echo "    Elixir: ${ELIXIR_RESULT:-0} ops/s"
cd ../..

echo "[11/11] Stopping server..."
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

# Cleanup
rm -rf "$BENCH_DATA_DIR"

echo ""
echo "=============================================="
echo "  BENCHMARK RESULTS (copy this)"
echo "=============================================="
echo "RUST=$RUST_RESULT"
echo "GO=$GO_RESULT"
echo "PYTHON=$PYTHON_RESULT"
echo "JS=$JS_RESULT"
echo "RUBY=$RUBY_RESULT"
echo "PHP=$PHP_RESULT"
echo "ELIXIR=$ELIXIR_RESULT"
echo ""
echo "Machine: $(uname -m) / $(cat /etc/os-release 2>/dev/null | grep PRETTY_NAME | cut -d'"' -f2 || uname -s)"
echo "Cores: $(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 'unknown')"
echo "=============================================="
