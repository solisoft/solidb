#!/bin/bash
# Fast parallel insertion test for sharding disk usage

set -e
export JWT_SECRET="test-secret-for-sharding-verification-12345"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup
pkill -f "solidb" || true
sleep 1
rm -rf tmp/n1 tmp/n2 tmp/n3 || true
mkdir -p tmp/n1 tmp/n2 tmp/n3

echo "Compiling..."
cargo build --quiet 2>/dev/null

BIN=./target/debug/solidb

# Start 3-node cluster
echo "Starting 3-node cluster..."
$BIN --port 8001 --replication-port 9001 --data-dir ./tmp/n1 > tmp/n1.log 2>&1 &
PID1=$!
sleep 2
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir ./tmp/n2 > tmp/n2.log 2>&1 &
PID2=$!
sleep 2
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir ./tmp/n3 > tmp/n3.log 2>&1 &
PID3=$!
sleep 10

echo "Cluster PIDs: $PID1 $PID2 $PID3"

# Create collections
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" -d '{"name": "users"}' > /dev/null
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" -d '{"name": "users2", "numShards": 3, "replicationFactor": 2}' > /dev/null

sleep 2

# Generate payload (100 bytes - shorter for cmdline)
PAYLOAD=$(printf 'x%.0s' {1..100})

# Insert 100 docs using parallel background jobs
echo "Inserting 100 documents into 'users' (unsharded)..."
for i in $(seq 1 100); do
  curl -s -u admin:admin -X POST \
    "http://localhost:8001/_api/database/_system/document/users" \
    -H "Content-Type: application/json" \
    -d "{\"_key\": \"doc$i\", \"value\": \"$PAYLOAD\"}" > /dev/null &
  # Limit parallelism
  if (( i % 20 == 0 )); then wait; fi
done
wait

echo "Inserting 100 documents into 'users2' (sharded RF=2)..."
for i in $(seq 1 100); do
  curl -s -u admin:admin -X POST \
    "http://localhost:8001/_api/database/_system/document/users2" \
    -H "Content-Type: application/json" \
    -d "{\"_key\": \"doc$i\", \"value\": \"$PAYLOAD\"}" > /dev/null &
  if (( i % 20 == 0 )); then wait; fi
done
wait

sleep 10  # Wait for replication

echo ""
echo "============================================="
echo "DISK USAGE PER NODE"
echo "============================================="
echo ""
echo "Node 1 (coordinator):"
du -sh tmp/n1
echo ""
echo "Node 2:"
du -sh tmp/n2
echo ""
echo "Node 3:"
du -sh tmp/n3

# Cleanup
kill $PID1 $PID2 $PID3 2>/dev/null || true

echo ""
echo "============================================="
echo "EXPECTED BEHAVIOR:"
echo "============================================="
echo "With 100 docs each, ~500 bytes payload each:"
echo ""
echo "users (unsharded, replicated to ALL nodes):"
echo "  - 100 docs on EACH node = 100 copies total on each node"
echo "  - Per-node data: ~50KB + overhead"
echo ""
echo "users2 (sharded 3 shards, RF=2):"
echo "  - Each doc on 2 of 3 nodes = ~66 docs per node"
echo "  - Per-node data: ~33KB + overhead"
echo ""
echo "CONCLUSION: If sharding works correctly,"
echo "users2 should use LESS space per node than users"
