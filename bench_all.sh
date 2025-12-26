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
echo "[5/18] Running Rust benchmark..."
cargo build --release --bin benchmark --quiet 2>/dev/null || cargo build --release --bin benchmark
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
RUST_OUTPUT=$(timeout 60s ./target/release/benchmark 2>&1 || echo "TIMEOUT_OR_ERROR")
RUST_RESULT=$(extract_result "RUST_BENCH_RESULT:" "$RUST_OUTPUT")
RUST_READ_RESULT=$(extract_result "RUST_READ_BENCH_RESULT:" "$RUST_OUTPUT")
if [ -z "$RUST_RESULT" ] || [ "$RUST_RESULT" = "0" ]; then
    echo "    Rust: FAILED"
    echo "    Output: $(echo "$RUST_OUTPUT" | head -2)"
    RUST_RESULT="0"
    RUST_READ_RESULT="0"
else
    echo "    Rust Insert: ${RUST_RESULT} ops/s"
    echo "    Rust Read:   ${RUST_READ_RESULT} ops/s"
fi

echo "[6/18] Running Go benchmark..."
cd clients/go-client
sed -i'' -e "s/127.0.0.1\", [0-9]*/127.0.0.1\", $BENCH_PORT/g" benchmark.go 2>/dev/null || true
sed -i'' -e "s/\"admin\", \"[^\"]*\"/\"admin\", \"$BENCH_PASSWORD\"/g" benchmark.go 2>/dev/null || true
GO_OUTPUT=$(go run benchmark.go 2>&1)
GO_RESULT=$(extract_result "GO_BENCH_RESULT:" "$GO_OUTPUT")
GO_READ_RESULT=$(extract_result "GO_READ_BENCH_RESULT:" "$GO_OUTPUT")
if [ -z "$GO_RESULT" ]; then
    echo "    Go: FAILED"
    GO_RESULT="0"
    GO_READ_RESULT="0"
else
    echo "    Go Insert: ${GO_RESULT} ops/s"
    echo "    Go Read:   ${GO_READ_RESULT} ops/s"
fi
cd ../..

echo "[7/18] Running Python benchmark..."
cd clients/PYTHON-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
PYTHON_OUTPUT=$(python3 benchmark.py 2>&1)
PYTHON_RESULT=$(extract_result "PYTHON_BENCH_RESULT:" "$PYTHON_OUTPUT")
PYTHON_READ_RESULT=$(extract_result "PYTHON_READ_BENCH_RESULT:" "$PYTHON_OUTPUT")
if [ -z "$PYTHON_RESULT" ]; then
    echo "    Python: FAILED - $PYTHON_OUTPUT"
    PYTHON_RESULT="0"
    PYTHON_READ_RESULT="0"
else
    echo "    Python Insert: ${PYTHON_RESULT} ops/s"
    echo "    Python Read:   ${PYTHON_READ_RESULT} ops/s"
fi
cd ../..

echo "[8/18] Running Bun/JS benchmark..."
cd clients/js-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
if command -v bun &> /dev/null; then
    JS_OUTPUT=$(bun run benchmark.ts 2>&1)
else
    JS_OUTPUT=$(npx ts-node benchmark.ts 2>&1)
fi
JS_RESULT=$(extract_result "JS_BENCH_RESULT:" "$JS_OUTPUT")
JS_READ_RESULT=$(extract_result "JS_READ_BENCH_RESULT:" "$JS_OUTPUT")
if [ -z "$JS_RESULT" ]; then
    echo "    Bun/JS: FAILED"
    JS_RESULT="0"
    JS_READ_RESULT="0"
else
    echo "    Bun/JS Insert: ${JS_RESULT} ops/s"
    echo "    Bun/JS Read:   ${JS_READ_RESULT} ops/s"
fi
cd ../..

echo "[9/18] Running Ruby benchmark..."
if command -v ruby &> /dev/null; then
    cd clients/Ruby-client
    export SOLIDB_PORT=$BENCH_PORT
    export SOLIDB_PASSWORD=$BENCH_PASSWORD
    RUBY_OUTPUT=$(ruby -Ilib benchmark.rb 2>&1)
    RUBY_RESULT=$(extract_result "RUBY_BENCH_RESULT:" "$RUBY_OUTPUT")
    RUBY_READ_RESULT=$(extract_result "RUBY_READ_BENCH_RESULT:" "$RUBY_OUTPUT")
    if [ -z "$RUBY_RESULT" ]; then
        echo "    Ruby: FAILED"
        RUBY_RESULT="0"
        RUBY_READ_RESULT="0"
    else
        echo "    Ruby Insert: ${RUBY_RESULT} ops/s"
        echo "    Ruby Read:   ${RUBY_READ_RESULT} ops/s"
    fi
    cd ../..
else
    echo "    Ruby: SKIPPED (ruby not installed)"
    RUBY_RESULT="0"
    RUBY_READ_RESULT="0"
fi

echo "[10/18] Running PHP benchmark..."
if command -v php &> /dev/null; then
    cd clients/PHP-client
    export SOLIDB_PORT=$BENCH_PORT
    export SOLIDB_PASSWORD=$BENCH_PASSWORD
    PHP_OUTPUT=$(php benchmark.php 2>&1)
    PHP_RESULT=$(extract_result "PHP_BENCH_RESULT:" "$PHP_OUTPUT")
    PHP_READ_RESULT=$(extract_result "PHP_READ_BENCH_RESULT:" "$PHP_OUTPUT")
    if [ -z "$PHP_RESULT" ]; then
        echo "    PHP: FAILED"
        PHP_RESULT="0"
        PHP_READ_RESULT="0"
    else
        echo "    PHP Insert: ${PHP_RESULT} ops/s"
        echo "    PHP Read:   ${PHP_READ_RESULT} ops/s"
    fi
    cd ../..
else
    echo "    PHP: SKIPPED (php not installed)"
    PHP_RESULT="0"
    PHP_READ_RESULT="0"
fi

echo "[11/18] Running Elixir benchmark..."
echo "    Elixir: SKIPPED (requires mix setup)"
ELIXIR_RESULT="0"

echo ""
echo "=== MULTI-CORE PARALLEL BENCHMARKS (16 workers, 10K inserts) ==="
echo ""

echo "[12/18] Running Rust parallel benchmark..."
cargo build --release --bin benchmark_parallel --quiet 2>/dev/null || cargo build --release --bin benchmark_parallel
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
RUST_PARALLEL_OUTPUT=$(timeout 60s ./target/release/benchmark_parallel 2>&1 || echo "TIMEOUT_OR_ERROR")
RUST_PARALLEL_RESULT=$(extract_result "RUST_PARALLEL_BENCH_RESULT:" "$RUST_PARALLEL_OUTPUT")
if [ -z "$RUST_PARALLEL_RESULT" ] || [ "$RUST_PARALLEL_RESULT" = "0" ]; then
    echo "    Rust (parallel): FAILED"
    RUST_PARALLEL_RESULT="0"
else
    echo "    Rust (parallel): ${RUST_PARALLEL_RESULT} ops/s"
fi

echo "[13/18] Running Go parallel benchmark..."
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

echo "[14/18] Running Python parallel benchmark..."
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

echo "[15/18] Running JS parallel benchmark..."
cd clients/js-client
export SOLIDB_PORT=$BENCH_PORT
export SOLIDB_PASSWORD=$BENCH_PASSWORD
if command -v bun &> /dev/null; then
    JS_PARALLEL_OUTPUT=$(bun run benchmark_parallel.ts 2>&1)
else
    JS_PARALLEL_OUTPUT=$(npx ts-node benchmark_parallel.ts 2>&1)
fi
JS_PARALLEL_RESULT=$(extract_result "JS_PARALLEL_BENCH_RESULT:" "$JS_PARALLEL_OUTPUT")
if [ -z "$JS_PARALLEL_RESULT" ]; then
    echo "    JS (parallel): FAILED"
    JS_PARALLEL_RESULT="0"
else
    echo "    JS (parallel): ${JS_PARALLEL_RESULT} ops/s"
fi
cd ../..

echo "[16/18] Running Ruby parallel benchmark..."
if command -v ruby &> /dev/null; then
    cd clients/Ruby-client
    export SOLIDB_PORT=$BENCH_PORT
    export SOLIDB_PASSWORD=$BENCH_PASSWORD
    RUBY_PARALLEL_OUTPUT=$(ruby -Ilib benchmark_parallel.rb 2>&1)
    RUBY_PARALLEL_RESULT=$(extract_result "RUBY_PARALLEL_BENCH_RESULT:" "$RUBY_PARALLEL_OUTPUT")
    if [ -z "$RUBY_PARALLEL_RESULT" ]; then
        echo "    Ruby (parallel): FAILED"
        RUBY_PARALLEL_RESULT="0"
    else
        echo "    Ruby (parallel): ${RUBY_PARALLEL_RESULT} ops/s"
    fi
    cd ../..
else
    echo "    Ruby (parallel): SKIPPED (ruby not installed)"
    RUBY_PARALLEL_RESULT="0"
fi

echo "[17/18] Running PHP parallel benchmark..."
if command -v php &> /dev/null; then
    cd clients/PHP-client
    export SOLIDB_PORT=$BENCH_PORT
    export SOLIDB_PASSWORD=$BENCH_PASSWORD
    PHP_PARALLEL_OUTPUT=$(php benchmark_parallel.php 2>&1)
    PHP_PARALLEL_RESULT=$(extract_result "PHP_PARALLEL_BENCH_RESULT:" "$PHP_PARALLEL_OUTPUT")
    if [ -z "$PHP_PARALLEL_RESULT" ]; then
        echo "    PHP (parallel): FAILED"
        PHP_PARALLEL_RESULT="0"
    else
        echo "    PHP (parallel): ${PHP_PARALLEL_RESULT} ops/s"
    fi
    cd ../..
else
    echo "    PHP (parallel): SKIPPED (php not installed)"
    PHP_PARALLEL_RESULT="0"
fi

echo "[18/18] Stopping server..."
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

# Cleanup
rm -rf "$BENCH_DATA_DIR"

echo ""
echo "=============================================="
echo "  BENCHMARK RESULTS (copy this)"
echo "=============================================="
echo "=== Sequential INSERT (1K ops, single connection) ==="
echo "RUST_INSERT=$RUST_RESULT"
echo "GO_INSERT=$GO_RESULT"
echo "PYTHON_INSERT=$PYTHON_RESULT"
echo "JS_INSERT=$JS_RESULT"
echo "RUBY_INSERT=$RUBY_RESULT"
echo "PHP_INSERT=$PHP_RESULT"
echo "ELIXIR_INSERT=$ELIXIR_RESULT"
echo ""
echo "=== Sequential READ (1K ops, single connection) ==="
echo "RUST_READ=$RUST_READ_RESULT"
echo "GO_READ=$GO_READ_RESULT"
echo "PYTHON_READ=$PYTHON_READ_RESULT"
echo "JS_READ=$JS_READ_RESULT"
echo "RUBY_READ=$RUBY_READ_RESULT"
echo "PHP_READ=$PHP_READ_RESULT"
echo ""
echo "=== Parallel INSERT (10K ops, 16 connections) ==="
echo "RUST_PARALLEL=$RUST_PARALLEL_RESULT"
echo "GO_PARALLEL=$GO_PARALLEL_RESULT"
echo "PYTHON_PARALLEL=$PYTHON_PARALLEL_RESULT"
echo "JS_PARALLEL=$JS_PARALLEL_RESULT"
echo "RUBY_PARALLEL=$RUBY_PARALLEL_RESULT"
echo "PHP_PARALLEL=$PHP_PARALLEL_RESULT"
echo ""
echo "Machine: $(uname -m) / $(cat /etc/os-release 2>/dev/null | grep PRETTY_NAME | cut -d'"' -f2 || uname -s)"
echo "Cores: $(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 'unknown')"
echo "=============================================="
