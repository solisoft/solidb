#!/bin/bash
# Reproduce query slowness on sharded collection

set -e

SCRIPT_DIR=$(dirname "$0")
cd "$SCRIPT_DIR/.."

BIN="./target/debug/solidb"

# Build first
echo "Building..."
cargo build --quiet

# Clean up
rm -rf /tmp/slow_test_data
mkdir -p /tmp/slow_test_data/{n1,n2,n3}

# Kill existing
pkill -f "solidb.*slow_test" 2>/dev/null || true
sleep 1

echo "Starting 3-node cluster..."
export SOLIDB_ADMIN_PASSWORD="admin"

$BIN --port 8001 --replication-port 9001 --data-dir /tmp/slow_test_data/n1 > /tmp/slow_test_data/n1.log 2>&1 &
PID1=$!
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir /tmp/slow_test_data/n2 > /tmp/slow_test_data/n2.log 2>&1 &
PID2=$!
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir /tmp/slow_test_data/n3 > /tmp/slow_test_data/n3.log 2>&1 &
PID3=$!
sleep 5

echo "Creating database and collection..."
curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database" \
    -H "Content-Type: application/json" -d '{"name": "perfdb"}' > /dev/null

curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/perfdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "large_coll", "numShards": 3, "replicationFactor": 2}' > /dev/null

# Replicate to other nodes workaround
sleep 2
curl -s -X POST "http://admin:admin@127.0.0.1:8002/_api/database/perfdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "large_coll", "numShards": 3, "replicationFactor": 2}' > /dev/null
curl -s -X POST "http://admin:admin@127.0.0.1:8003/_api/database/perfdb/collection" \
    -H "Content-Type: application/json" \
    -d '{"name": "large_coll", "numShards": 3, "replicationFactor": 2}' > /dev/null

echo "Generating 10,000 documents..."
rm -f /tmp/slow_test_data/docs.jsonl
# Generate 10k docs
ruby -e 'require "json"; puts (1..1000).map { |i| { _key: "doc#{i}", value: "x" * 100, num: i } }.to_json' > /tmp/slow_test_data/docs.jsonl

echo "Importing documents..."
curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/perfdb/collection/large_coll/import" \
    -F "file=@/tmp/slow_test_data/docs.jsonl" > /dev/null

echo "Import done. Sleeping 2s..."
sleep 2

echo "Running query LIMIT 20 (Page 1)..."
start_time=$(date +%s%N)
curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/perfdb/cursor" \
    -H "Content-Type: application/json" \
    -d '{"query": "FOR doc IN large_coll LIMIT 0, 20 RETURN doc"}' > /dev/null
end_time=$(date +%s%N)
duration=$(( (end_time - start_time) / 1000000 ))
echo "Query time (LIMIT 0, 20): ${duration}ms"

echo "Running query LIMIT 20 OFFSET 5000..."
start_time=$(date +%s%N)
curl -s -X POST "http://admin:admin@127.0.0.1:8001/_api/database/perfdb/cursor" \
    -H "Content-Type: application/json" \
    -d '{"query": "FOR doc IN large_coll LIMIT 5000, 20 RETURN doc"}' > /dev/null
end_time=$(date +%s%N)
duration=$(( (end_time - start_time) / 1000000 ))
echo "Query time (LIMIT 5000, 20): ${duration}ms"

# Cleanup
echo "Cleaning up..."
kill $PID1 $PID2 $PID3 2>/dev/null || true
