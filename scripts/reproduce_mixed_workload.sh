#!/bin/bash
set -e

# Set consistent secret for authentication across all nodes
export JWT_SECRET="test-secret-mixed-workload"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
rm -rf tmp/mix_n1 tmp/mix_n2
mkdir -p tmp/mix_n1 tmp/mix_n2

BIN=./target/debug/solidb

# Start Node 1 (Bootstrap)
echo "Starting Node 1..."
RUST_LOG=debug $BIN --port 8071 --replication-port 9071 --data-dir ./tmp/mix_n1 > tmp/mix_n1.log 2>&1 &
PID1=$!
sleep 5

# Start Node 2 (Peer)
echo "Starting Node 2..."
RUST_LOG=debug $BIN --port 8072 --replication-port 9072 --peer 127.0.0.1:9071 --data-dir ./tmp/mix_n2 > tmp/mix_n2.log 2>&1 &
PID2=$!
sleep 5

echo "Cluster started. PIDs: $PID1, $PID2"

# Create Collection on Node 1
echo "Creating collection on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8071/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "test_mixed", "type": "document"}'

sleep 2

# Batch 1: 10,000 documents (Batch Mode)
echo "Batch 1: Inserting 10,000 documents on Node 1 (Batch Mode)..."
curl -s -u admin:admin -X POST http://localhost:8071/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR i IN 1..10000 INSERT { value: i, batch: 1 } INTO test_mixed"}'

echo "Waiting for Batch 1 to replicate..."
for i in {1..60}; do
    COUNT=$(curl -s -u admin:admin -X POST http://localhost:8072/_api/database/_system/cursor \
      -H "Content-Type: application/json" \
      -d '{"query": "RETURN COLLECTION_COUNT(\"test_mixed\")"}' | grep -o '[0-9]*' | head -1)
    
    echo "Node 2 count: $COUNT / 10000"
    if [ "$COUNT" == "10000" ]; then
        echo "Batch 1 Replicated Successfully!"
        break
    fi
    sleep 1
done

if [ "$COUNT" != "10000" ]; then
    echo "FAILURE: Batch 1 replication timed out."
    kill $PID1 $PID2 || true
    exit 1
fi

sleep 2

# Single Insert: 1 document (Single Mode)
echo "Single Insert: Inserting 1 document on Node 1 (Single Mode)..."
# Using SDBQL Single INSERT (not loop) which might use different path,
# OR we can use the document API directly. Let's use document API to be sure it's "Single".
curl -s -u admin:admin -X POST http://localhost:8071/_api/database/_system/document/test_mixed \
  -H "Content-Type: application/json" \
  -d '{"value": 10001, "batch": "single"}'

echo "Waiting for Single Insert to replicate..."
for i in {1..60}; do
    COUNT=$(curl -s -u admin:admin -X POST http://localhost:8072/_api/database/_system/cursor \
      -H "Content-Type: application/json" \
      -d '{"query": "RETURN COLLECTION_COUNT(\"test_mixed\")"}' | grep -o '[0-9]*' | head -1)
    
    echo "Node 2 count: $COUNT / 10001"
    if [ "$COUNT" == "10001" ]; then
        echo "SUCCESS: Single Insert Replicated Successfully!"
        break
    fi
    sleep 1
done

if [ "$COUNT" != "10001" ]; then
    echo "FAILURE: Single Insert replication timed out."
    exit 1
fi

echo "Cleaning up..."
kill $PID1 $PID2 || true
