#!/bin/bash
set -e

# Set consistent secret for authentication across all nodes
export JWT_SECRET="test-secret-incremental"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
rm -rf tmp/inc_n1 tmp/inc_n2
mkdir -p tmp/inc_n1 tmp/inc_n2

BIN=./target/debug/solidb

# Start Node 1 (Bootstrap)
echo "Starting Node 1..."
RUST_LOG=debug $BIN --port 8051 --replication-port 9051 --data-dir ./tmp/inc_n1 > tmp/inc_n1.log 2>&1 &
PID1=$!
sleep 5

# Start Node 2 (Peer)
echo "Starting Node 2..."
RUST_LOG=debug $BIN --port 8052 --replication-port 9052 --peer 127.0.0.1:9051 --data-dir ./tmp/inc_n2 > tmp/inc_n2.log 2>&1 &
PID2=$!
sleep 5

echo "Cluster started. PIDs: $PID1, $PID2"

# Create Collection on Node 1
echo "Creating collection on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8051/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "test_inc", "type": "document"}'

sleep 2

# Batch 1: 10,000 documents
echo "Batch 1: Inserting 10,000 documents on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8051/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR i IN 1..10000 INSERT { value: i, batch: 1, timestamp: DATE_NOW() } INTO test_inc"}'

echo "Waiting for Batch 1 to replicate..."
for i in {1..60}; do
    COUNT=$(curl -s -u admin:admin -X POST http://localhost:8052/_api/database/_system/cursor \
      -H "Content-Type: application/json" \
      -d '{"query": "RETURN COLLECTION_COUNT(\"test_inc\")"}' | grep -o '[0-9]*' | head -1)
    
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

# Batch 2: Another 10,000 documents (Total 20,000)
echo "Batch 2: Inserting 10,000 MORE documents on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8051/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR i IN 10001..20000 INSERT { value: i, batch: 2, timestamp: DATE_NOW() } INTO test_inc"}'

echo "Waiting for Batch 2 to replicate..."
for i in {1..60}; do
    COUNT=$(curl -s -u admin:admin -X POST http://localhost:8052/_api/database/_system/cursor \
      -H "Content-Type: application/json" \
      -d '{"query": "RETURN COLLECTION_COUNT(\"test_inc\")"}' | grep -o '[0-9]*' | head -1)
    
    echo "Node 2 count: $COUNT / 20000"
    if [ "$COUNT" == "20000" ]; then
        echo "SUCCESS: Batch 2 Replicated Successfully!"
        break
    fi
    sleep 1
done

if [ "$COUNT" != "20000" ]; then
    echo "FAILURE: Batch 2 replication timed out."
    exit 1
fi

echo "Cleaning up..."
kill $PID1 $PID2 || true
