#!/bin/bash
# Benchmark scatter-gather performance on sharded collection
# Tests the improvement from parallel shard queries

set -e

SCRIPT_DIR=$(dirname "$0")
cd "$SCRIPT_DIR/.."

BIN="./target/debug/solidb"
ITERATIONS=${ITERATIONS:-10}

# Build first
echo "Building..."
cargo build --quiet 2>/dev/null

# Clean up
rm -rf /tmp/scatter_bench_data
mkdir -p /tmp/scatter_bench_data/{n1,n2,n3}

# Kill existing
pkill -f "solidb.*scatter_bench" 2>/dev/null || true
sleep 1

echo "Starting 3-node cluster..."
export SOLIDB_ADMIN_PASSWORD="admin"

$BIN --port 8001 --replication-port 9001 --data-dir /tmp/scatter_bench_data/n1 > /tmp/scatter_bench_data/n1.log 2>&1 &
PID1=$!
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir /tmp/scatter_bench_data/n2 > /tmp/scatter_bench_data/n2.log 2>&1 &
PID2=$!
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir /tmp/scatter_bench_data/n3 > /tmp/scatter_bench_data/n3.log 2>&1 &
PID3=$!
sleep 5

echo "Creating database and collection..."
curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database" \
    -H "Content-Type: application/json" -d '{"name": "benchdb"}' > /dev/null

curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/benchdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "bench_coll", "numShards": 3, "replicationFactor": 2}' > /dev/null

# Replicate to other nodes workaround
sleep 2
curl -s -X POST "http://admin:admin@127.0.0.1:8002/_api/database/benchdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "bench_coll", "numShards": 3, "replicationFactor": 2}' > /dev/null
curl -s -X POST "http://admin:admin@127.0.0.1:8003/_api/database/benchdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "bench_coll", "numShards": 3, "replicationFactor": 2}' > /dev/null

echo "Generating documents..."
rm -f /tmp/scatter_bench_data/docs.jsonl
# Generate 3000 docs (1000 per shard)
for i in $(seq 1 3000); do
    echo "{\"_key\": \"doc$i\", \"value\": \"x$i\", \"num\": $i}" >> /tmp/scatter_bench_data/docs.jsonl
done

echo "Importing documents..."
curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/benchdb/collection/bench_coll/import" \
    -F "file=@/tmp/scatter_bench_data/docs.jsonl" > /dev/null

echo "Import done. Waiting 2s for replication..."
sleep 2

echo ""
echo "=============================================="
echo "  Scatter-Gather Benchmark Results"
echo "  ($ITERATIONS iterations per query)"
echo "=============================================="
echo ""

# Benchmark queries
echo "📊 Query: LIMIT 20 (first page)"
total_time=0
for i in $(seq 1 $ITERATIONS); do
    start=$(date +%s%N)
    curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/benchdb/cursor" \
        -H "Content-Type: application/json" \
        -d '{"query": "FOR doc IN bench_coll LIMIT 20 RETURN doc"}' > /dev/null
    end=$(date +%s%N)
    ((total_time+=($end - $start)/1000000))
done
avg=$((total_time / ITERATIONS))
echo "   Average time: ${avg}ms"

echo ""
echo "📊 Query: LIMIT 20 OFFSET 1000"
total_time=0
for i in $(seq 1 $ITERATIONS); do
    start=$(date +%s%N)
    curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/benchdb/cursor" \
        -H "Content-Type: application/json" \
        -d '{"query": "FOR doc IN bench_coll LIMIT 20 OFFSET 1000 RETURN doc"}' > /dev/null
    end=$(date +%s%N)
    ((total_time+=($end - $start)/1000000))
done
avg=$((total_time / ITERATIONS))
echo "   Average time: ${avg}ms"

echo ""
echo "📊 Query: LIMIT 20 OFFSET 2000"
total_time=0
for i in $(seq 1 $ITERATIONS); do
    start=$(date +%s%N)
    curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/benchdb/cursor" \
        -H "Content-Type: application/json" \
        -d '{"query": "FOR doc IN bench_coll LIMIT 20 OFFSET 2000 RETURN doc"}' > /dev/null
    end=$(date +%s%N)
    ((total_time+=($end - $start)/1000000))
done
avg=$((total_time / ITERATIONS))
echo "   Average time: ${avg}ms"

echo ""
echo "📊 Query: RETURN all (no LIMIT)"
total_time=0
for i in $(seq 1 $ITERATIONS); do
    start=$(date +%s%N)
    curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/benchdb/cursor" \
        -H "Content-Type: application/json" \
        -d '{"query": "FOR doc IN bench_coll RETURN doc"}' > /dev/null
    end=$(date +%s%N)
    ((total_time+=($end - $start)/1000000))
done
avg=$((total_time / ITERATIONS))
echo "   Average time: ${avg}ms"

echo ""
echo "=============================================="
echo "  Cleanup..."
# Cleanup
kill $PID1 $PID2 $PID3 2>/dev/null || true
rm -rf /tmp/scatter_bench_data
echo "Done!"