#!/bin/bash
set -e

# Set consistent secret for authentication across all nodes
export JWT_SECRET="test-secret-for-sharding-verification-12345"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
rm -rf tmp/n1 tmp/n2 tmp/n3
mkdir -p tmp/n1 tmp/n2 tmp/n3

# Compile first
echo "Compiling..."
cargo build

BIN=./target/debug/solidb

# Start Node 1 (Bootstrap)
echo "Starting Node 1..."
$BIN --port 8001 --replication-port 9001 --data-dir ./tmp/n1 > tmp/n1.log 2>&1 &
PID1=$!
sleep 2

# Start Node 2
echo "Starting Node 2..."
$BIN --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 --data-dir ./tmp/n2 > tmp/n2.log 2>&1 &
PID2=$!
sleep 2

# Start Node 3
echo "Starting Node 3..."
$BIN --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 --data-dir ./tmp/n3 > tmp/n3.log 2>&1 &
PID3=$!
sleep 15 # Wait for cluster sync

echo "Cluster started. PIDs: $PID1, $PID2, $PID3"

# Create Sharded Collection on Node 1
echo "Creating sharded collection on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "test_sharded", "type": "document", "numShards": 3, "replicationFactor": 2, "shardKey": "_key"}'

sleep 2

# Insert Document on Node 1
echo "Inserting document into Node 1..."
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/document/test_sharded \
  -H "Content-Type: application/json" \
  -d '{"_key": "doc1", "value": "check_visibility"}'

sleep 2

# Query on Node 3 using SDBQL (this triggers scatter-gather)
echo "Querying document on Node 3 (expecting success)..."
RESPONSE=$(curl -s -u admin:admin -X POST http://localhost:8003/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN test_sharded RETURN doc"}')
echo "Response from Node 3: $RESPONSE"

# Cleanup
kill $PID1 $PID2 $PID3 || true

# Verification
if [[ "$RESPONSE" == *"doc1"* ]]; then
  echo "SUCCESS: Document inserted on Node 1 is visible on Node 3!"
  exit 0
else
  echo "FAILURE: Document not found on Node 3."
  exit 1
fi

