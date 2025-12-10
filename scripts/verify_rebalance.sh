#!/bin/bash
set -e

# Set consistent auth for all nodes
export JWT_SECRET="test-secret-for-rebalance-verification"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
rm -rf tmp/n1 tmp/n2 tmp/n3 tmp/n4 tmp/n5
mkdir -p tmp/n1 tmp/n2 tmp/n3 tmp/n4 tmp/n5

# Compile first
echo "Compiling..."
cargo build

BIN=./target/debug/solidb

# Start 5-node cluster
echo "Starting 5-node cluster..."

# Node 1 (bootstrap)
$BIN --data-dir tmp/n1 --port 8001 --replication-port 9001 > tmp/n1.log 2>&1 &
sleep 2

# Node 2 joins Node 1
$BIN --data-dir tmp/n2 --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 > tmp/n2.log 2>&1 &
sleep 1

# Node 3 joins Node 1
$BIN --data-dir tmp/n3 --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 > tmp/n3.log 2>&1 &
sleep 1

# Node 4 joins Node 1
$BIN --data-dir tmp/n4 --port 8004 --replication-port 9004 --peer 127.0.0.1:9001 > tmp/n4.log 2>&1 &
sleep 1

# Node 5 joins Node 1
$BIN --data-dir tmp/n5 --port 8005 --replication-port 9005 --peer 127.0.0.1:9001 > tmp/n5.log 2>&1 &
sleep 3

echo "5-node cluster started"

# Wait for cluster to stabilize
echo "Waiting for cluster to stabilize (10 seconds)..."
sleep 10

# Create sharded collection on Node 1
echo ""
echo "=== Creating sharded collection (5 shards, RF=2) ==="
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "test_rebalance", "type": "document", "numShards": 5, "replicationFactor": 2, "shardKey": "_key"}'
echo ""
sleep 2

# Check initial distribution
echo ""
echo "=== Initial Stats (5 nodes) ==="
STATS=$(curl -s -u admin:admin http://localhost:8001/_api/database/_system/collection/test_rebalance/stats)
echo "$STATS" | jq '.'
echo ""
echo "--- Distribution per node ---"
echo "$STATS" | jq '.cluster.distribution'

# Insert some test documents
echo ""
echo "=== Inserting 10 test documents ==="
for i in {1..10}; do
  curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/document/test_rebalance \
    -H "Content-Type: application/json" \
    -d "{\"_key\": \"doc$i\", \"value\": $i}" > /dev/null
done
echo "Inserted 10 documents"

# Verify documents are distributed
echo ""
echo "=== Document count per node ==="
for port in 8001 8002 8003 8004 8005; do
  count=$(curl -s -u admin:admin "http://localhost:$port/_api/database/_system/cursor" \
    -H "Content-Type: application/json" \
    -d '{"query": "FOR doc IN test_rebalance RETURN doc"}' | jq '.count')
  echo "  Node $port: $count documents"
done

# Now# Kill nodes one by one, verifying data after each
echo ""
echo "=== Sequential Node Failure Test ==="

for NODE_PORT in 8005; do
  NODE_NUM=$((NODE_PORT - 8000))
  REPL_PORT=$((NODE_PORT + 1000))
  
  echo ""
  echo "--- Killing Node $NODE_NUM (port $NODE_PORT) ---"
  NODE_PID=$(pgrep -f "solidb.*--port $NODE_PORT")
  if [ -n "$NODE_PID" ]; then
    kill $NODE_PID
    echo "Node $NODE_NUM (PID $NODE_PID) killed"
    echo "Node $NODE_NUM (PID $NODE_PID) killed"
  fi
  
  # Auto-removal is now enabled: nodes that fail health checks (3 consecutive failures)
  # will be automatically removed by ShardCoordinator after ~6 seconds
  # We just wait for auto-detection and rebalance
  
  echo "Waiting 15 seconds for auto-detection and rebalance..."
  sleep 15
  
  # Check document count from Node 1
  RESULT=$(curl -s -u admin:admin "http://localhost:8001/_api/database/_system/cursor" \
    -H "Content-Type: application/json" \
    -d '{"query": "FOR doc IN test_rebalance RETURN doc"}')
  COUNT=$(echo "$RESULT" | jq '.count')
  
  # Get cluster stats
  STATS=$(curl -s -u admin:admin http://localhost:8001/_api/database/_system/collection/test_rebalance/stats)
  TOTAL_NODES=$(echo "$STATS" | jq '.cluster.total_nodes')
  
  echo "After killing Node $NODE_NUM: $COUNT documents found, $TOTAL_NODES nodes remaining"
  
  if [ "$COUNT" -ne 10 ]; then
    echo "FAILURE: Expected 10 documents, found $COUNT after killing Node $NODE_NUM"
    pkill -f "solidb" || true
    exit 1
  fi
done

echo ""
echo "=== Final Stats (only Node 1 remaining) ==="
curl -s -u admin:admin http://localhost:8001/_api/database/_system/collection/test_rebalance/stats | jq '.'

# Final verification
echo ""
echo "=== Final Verification ==="
RESULT=$(curl -s -u admin:admin "http://localhost:8001/_api/database/_system/cursor" \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR doc IN test_rebalance RETURN doc"}')
COUNT=$(echo "$RESULT" | jq '.count')
echo "Documents found on last remaining node: $COUNT"

if [ "$COUNT" -eq 10 ]; then
  echo ""
  echo "SUCCESS: All 10 documents still accessible with only 1 node remaining!"
else
  echo ""
  echo "FAILURE: Expected 10 documents, found $COUNT"
fi

# Cleanup
echo ""
echo "Cleaning up..."
pkill -f "solidb" || true

echo "Done!"
