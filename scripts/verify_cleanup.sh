#!/bin/bash
set -e

# Configuration
BIN="./target/debug/solidb"
DATA_DIR="./tmp/verify_cleanup"
PEERS="--peer 127.0.0.1:9001"

# Cleanup
rm -rf $DATA_DIR
mkdir -p $DATA_DIR/n1 $DATA_DIR/n2 $DATA_DIR/n3
pkill -f "$BIN" || true

echo "Starting 3-node cluster..."

# Start Node 1 (Bootstrap)
$BIN --port 8001 --replication-port 9001 --data-dir $DATA_DIR/n1 > $DATA_DIR/n1.log 2>&1 &
N1_PID=$!
sleep 2

# Start Node 2
$BIN --port 8002 --replication-port 9002 $PEERS --data-dir $DATA_DIR/n2 > $DATA_DIR/n2.log 2>&1 &
N2_PID=$!
sleep 2

# Start Node 3
$BIN --port 8003 --replication-port 9003 $PEERS --data-dir $DATA_DIR/n3 > $DATA_DIR/n3.log 2>&1 &
N3_PID=$!
sleep 5

echo "Cluster started. PIDs: $N1_PID, $N2_PID, $N3_PID"

# Create Sharded Collection (RF=2, Shards=3)
echo "Creating sharded collection..."
RESPONSE=$(curl -s -X POST http://localhost:8001/api/db/_system/collections \
  -H "Content-Type: application/json" \
  -d '{"name":"test_cleanup","type":"document","numShards":3,"replicationFactor":2,"shardKey":"_key"}')
echo "Create response: $RESPONSE"

# Check if collection exists
RESPONSE=$(curl -s http://localhost:8001/api/db/_system/collections/test_cleanup)
echo "Collection info: $RESPONSE"

echo ""
sleep 2

# Insert Documents (all to Node 1)
echo "Inserting 99 documents to Node 1..."
for i in {1..10}; do # Reduce to 10 for quick debug
  curl -s -X POST http://localhost:8001/api/db/_system/collections/test_cleanup/documents \
    -H "Content-Type: application/json" \
    -d "{\"_key\":\"doc$i\", \"value\": $i}" > /dev/null
done
echo "Insert complete (partial batch for debug)."

# Verify Initial State (Expect 99 on N1 due to "write buffer" behavior)
RESPONSE=$(curl -s http://localhost:8001/api/db/_system/collections/test_cleanup)
COUNT=$(echo "$RESPONSE" | jq .count)
echo "Initial count on N1: $COUNT"

if [ "$COUNT" == "99" ]; then
    echo "Confirmed N1 has all 99 documents initially."
else
    echo "Unexpected initial count: $COUNT (expected 99)"
    echo "Raw response: $RESPONSE"
    pkill -P $$ || true
    exit 1
fi

echo "Waiting 65s for cleanup cycle (default interval=60s)..."
sleep 65

# Verify Post-Cleanup State (Expect ~66 on N1)
RESPONSE_AFTER=$(curl -s http://localhost:8001/api/db/_system/collections/test_cleanup)
COUNT_AFTER=$(echo "$RESPONSE_AFTER" | jq .count)
echo "Count on N1 after cleanup: $COUNT_AFTER"

# With 3 nodes, 3 shards, RF=2:
# Node responsible for (Primary + Replica) / TotalCopies
# Per document: 2 copies stored in cluster.
# Probability node is responsible = 2/3.
# Expected docs = 99 * (2/3) = 66
# Allow variance (60-72)

if [ "$COUNT_AFTER" -ge 60 ] && [ "$COUNT_AFTER" -le 72 ]; then
    echo "SUCCESS: Cleanup removed foreign documents. Count is within expected range (approx 66)."
    pkill -P $$ || true
    exit 0
else
    echo "FAILURE: Cleanup didn't work as expected. Count: $COUNT_AFTER (expected 60-72)."
    pkill -P $$ || true
    exit 1
fi
