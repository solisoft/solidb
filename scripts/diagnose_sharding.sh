#!/bin/bash
# Diagnostic script to compare document counts on each node
# for sharded vs unsharded collections

set -e

export JWT_SECRET="test-secret-for-sharding-verification-12345"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup
pkill -f "solidb" || true
sleep 1
rm -rf tmp/n1 tmp/n2 tmp/n3 || true
mkdir -p tmp/n1 tmp/n2 tmp/n3

echo "Compiling..."
cargo build --quiet

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
sleep 10  # Wait for cluster formation

echo "Cluster PIDs: $PID1 $PID2 $PID3"

# Create collections
echo "Creating unsharded collection 'users'..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "users", "type": "document"}' | jq .

echo "Creating sharded collection 'users2' (3 shards, RF=2)..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "users2", "type": "document", "numShards": 3, "replicationFactor": 2}' | jq .

sleep 2

# Insert 9 documents to each collection (3 per shard)
echo "Inserting 9 documents into 'users' (unsharded)..."
for i in {1..9}; do
  curl -s -u admin:admin -X POST "http://localhost:8001/_api/database/_system/document/users" \
    -H "Content-Type: application/json" \
    -d "{\"_key\": \"doc$i\", \"value\": \"test\"}" > /dev/null
done

echo "Inserting 9 documents into 'users2' (sharded)..."
for i in {1..9}; do
  curl -s -u admin:admin -X POST "http://localhost:8001/_api/database/_system/document/users2" \
    -H "Content-Type: application/json" \
    -d "{\"_key\": \"doc$i\", \"value\": \"test\"}" > /dev/null
done

sleep 5  # Wait for replication

echo ""
echo "============================================="
echo "DOCUMENT COUNTS PER NODE (checking local storage)"
echo "============================================="

for PORT in 8001 8002 8003; do
  echo ""
  echo "--- Node on port $PORT ---"
  echo "users (unsharded):"
  curl -s -u admin:admin "http://localhost:$PORT/_api/database/_system/collections" | jq '.collections[] | select(.name == "users") | {name, count}'
  echo "users2 (sharded RF=2):"
  curl -s -u admin:admin "http://localhost:$PORT/_api/database/_system/collections" | jq '.collections[] | select(.name == "users2") | {name, count}'
done

echo ""
echo "============================================="
echo "DISK USAGE PER NODE"
echo "============================================="
du -sh tmp/n1 tmp/n2 tmp/n3

# Cleanup
kill $PID1 $PID2 $PID3 2>/dev/null || true

echo ""
echo "Expected behavior:"
echo "- users (unsharded, replicated to all): 30 docs on EACH node"
echo "- users2 (sharded, RF=2): ~20 docs per node (30*2/3)"
echo "- Therefore: users should use MORE space per node than users2"
