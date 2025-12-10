#!/bin/bash
set -e

# Test that documents written on any node sync to all other nodes
export JWT_SECRET="test-secret-for-replication"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup
pkill -f "solidb" || true
rm -rf tmp/n1 tmp/n2 tmp/n3
mkdir -p tmp/n1 tmp/n2 tmp/n3

echo "Compiling..."
cargo build 2>&1 | tail -3

BIN=./target/debug/solidb

# Start 3-node cluster
echo "Starting 3-node cluster..."
$BIN --data-dir tmp/n1 --port 8001 --replication-port 9001 > tmp/n1.log 2>&1 &
sleep 2
$BIN --data-dir tmp/n2 --port 8002 --replication-port 9002 --peer 127.0.0.1:9001 > tmp/n2.log 2>&1 &
sleep 1
$BIN --data-dir tmp/n3 --port 8003 --replication-port 9003 --peer 127.0.0.1:9001 > tmp/n3.log 2>&1 &
sleep 5

echo "Waiting for cluster to stabilize (15 seconds)..."
sleep 15

# Create collection on Node 1
echo ""
echo "=== Creating collection on Node 1 ==="
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "sync_test", "type": "document"}'
echo ""
sleep 2

# Insert doc on Node 1
echo ""
echo "=== Inserting doc1 on Node 1 ==="
curl -s -u admin:admin -X POST http://localhost:8001/_api/database/_system/document/sync_test \
  -H "Content-Type: application/json" \
  -d '{"_key": "doc1", "source": "node1"}'
echo ""
sleep 2

# Insert doc on Node 2
echo ""
echo "=== Inserting doc2 on Node 2 ==="
curl -s -u admin:admin -X POST http://localhost:8002/_api/database/_system/document/sync_test \
  -H "Content-Type: application/json" \
  -d '{"_key": "doc2", "source": "node2"}'
echo ""
sleep 2

# Insert doc on Node 3
echo ""
echo "=== Inserting doc3 on Node 3 ==="
curl -s -u admin:admin -X POST http://localhost:8003/_api/database/_system/document/sync_test \
  -H "Content-Type: application/json" \
  -d '{"_key": "doc3", "source": "node3"}'
echo ""

# Wait for sync
echo ""
echo "Waiting for replication (10 seconds)..."
sleep 10

# Check all nodes
echo ""
echo "=== Document count per node ==="
FAILED=0
for port in 8001 8002 8003; do
  RESULT=$(curl -s -u admin:admin "http://localhost:$port/_api/database/_system/cursor" \
    -H "Content-Type: application/json" \
    -d '{"query": "FOR doc IN sync_test RETURN doc"}')
  COUNT=$(echo "$RESULT" | jq '.count')
  DOCS=$(echo "$RESULT" | jq -c '[.result[]._key]')
  echo "  Node $port: $COUNT documents - $DOCS"
  
  if [ "$COUNT" -ne 3 ]; then
    FAILED=1
  fi
done

echo ""
if [ "$FAILED" -eq 0 ]; then
  echo "SUCCESS: All 3 documents synced to all nodes!"
else
  echo "FAILURE: Not all documents synced to all nodes"
  echo ""
  echo "Checking logs for replication issues..."
  echo "--- Node 2 peer discovery ---"
  grep -i "Discovered\|new peer" tmp/n2.log | tail -5
  echo "--- Node 3 peer discovery ---"
  grep -i "Discovered\|new peer" tmp/n3.log | tail -5
fi

# Cleanup
pkill -f "solidb" || true
echo "Done!"
