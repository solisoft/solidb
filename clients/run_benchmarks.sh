#!/bin/bash
# Simple benchmark runner that uses an already-running SoliDB server
# Make sure SoliDB is running on port 6745 with password "password"

PORT="${SOLIDB_PORT:-6745}"
PASS="${SOLIDB_ADMIN_PASSWORD:-password}"

echo "=============================================="
echo "SoliDB Client Benchmarks (1000 inserts each)"
echo "=============================================="
echo "Server: 127.0.0.1:$PORT"
echo ""

cd "$(dirname "$0")/.."

# Results storage
declare -A RESULTS

# 1. Rust (need to have it built first)
echo -n "Rust........... "
if cargo build --bin benchmark --release 2>/dev/null; then
    RUST_RES=$(./target/release/benchmark 2>/dev/null | grep "RUST_BENCH_RESULT" | cut -d':' -f2)
    if [ -n "$RUST_RES" ]; then
        echo "$RUST_RES ops/s"
        RESULTS[rust]=$RUST_RES
    else
        echo "FAILED"
        RESULTS[rust]=0
    fi
else
    echo "BUILD FAILED"
    RESULTS[rust]=0
fi

# 2. Go
echo -n "Go............. "
cd clients/go-client
go build -o benchmark_bin benchmark.go 2>/dev/null
GO_RES=$(./benchmark_bin 2>/dev/null | grep "GO_BENCH_RESULT" | cut -d':' -f2)
cd ../..
if [ -n "$GO_RES" ]; then
    echo "$GO_RES ops/s"
    RESULTS[go]=$GO_RES
else
    echo "FAILED"
    RESULTS[go]=0
fi

# 3. Bun/JS
echo -n "Bun/JS......... "
cd clients/js-client
if command -v bun &> /dev/null; then
    JS_RES=$(bun run benchmark.ts 2>/dev/null | grep "JS_BENCH_RESULT" | cut -d':' -f2)
else
    JS_RES=$(npx ts-node benchmark.ts 2>/dev/null | grep "JS_BENCH_RESULT" | cut -d':' -f2)
fi
cd ../..
if [ -n "$JS_RES" ]; then
    echo "$JS_RES ops/s"
    RESULTS[js]=$JS_RES
else
    echo "FAILED"
    RESULTS[js]=0
fi

# 4. Python
echo -n "Python......... "
cd clients/python-client
PYTHON_RES=$(python3 benchmark.py 2>/dev/null | grep "PYTHON_BENCH_RESULT" | cut -d':' -f2)
cd ../..
if [ -n "$PYTHON_RES" ]; then
    echo "$PYTHON_RES ops/s"
    RESULTS[python]=$PYTHON_RES
else
    echo "FAILED"
    RESULTS[python]=0
fi

# 5. PHP
echo -n "PHP............ "
cd clients/PHP-client
PHP_RES=$(php benchmark.php 2>/dev/null | grep "PHP_BENCH_RESULT" | cut -d':' -f2)
cd ../..
if [ -n "$PHP_RES" ]; then
    echo "$PHP_RES ops/s"
    RESULTS[php]=$PHP_RES
else
    echo "FAILED"
    RESULTS[php]=0
fi

# 6. Ruby
echo -n "Ruby........... "
cd clients/ruby-client
RUBY_RES=$(ruby -Ilib benchmark.rb 2>/dev/null | grep "RUBY_BENCH_RESULT" | cut -d':' -f2)
cd ../..
if [ -n "$RUBY_RES" ]; then
    echo "$RUBY_RES ops/s"
    RESULTS[ruby]=$RUBY_RES
else
    echo "FAILED"
    RESULTS[ruby]=0
fi

echo ""
echo "=============================================="
echo "Summary (Operations per Second)"
echo "=============================================="
echo "{"
echo "  \"rust\": ${RESULTS[rust]:-0},"
echo "  \"go\": ${RESULTS[go]:-0},"
echo "  \"js\": ${RESULTS[js]:-0},"
echo "  \"python\": ${RESULTS[python]:-0},"
echo "  \"php\": ${RESULTS[php]:-0},"
echo "  \"ruby\": ${RESULTS[ruby]:-0}"
echo "}"
