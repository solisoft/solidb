#!/bin/bash
set -e

# Set consistent secret for authentication across all nodes
export JWT_SECRET="test-secret-incremental-3node"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
rm -rf tmp/inc3_n1 tmp/inc3_n2 tmp/inc3_n3
mkdir -p tmp/inc3_n1 tmp/inc3_n2 tmp/inc3_n3

BIN=./target/debug/solidb

# Start Node 1 (Bootstrap)
echo "Starting Node 1..."
RUST_LOG=debug $BIN --port 8061 --replication-port 9061 --data-dir ./tmp/inc3_n1 > tmp/inc3_n1.log 2>&1 &
PID1=$!
sleep 5

# Start Node 2 (Peer 1)
echo "Starting Node 2..."
RUST_LOG=debug $BIN --port 8062 --replication-port 9062 --peer 127.0.0.1:9061 --data-dir ./tmp/inc3_n2 > tmp/inc3_n2.log 2>&1 &
PID2=$!
sleep 5

# Start Node 3 (Peer 2)
echo "Starting Node 3..."
RUST_LOG=debug $BIN --port 8063 --replication-port 9063 --peer 127.0.0.1:9061 --data-dir ./tmp/inc3_n3 > tmp/inc3_n3.log 2>&1 &
PID3=$!
sleep 5

echo "Cluster started. PIDs: $PID1, $PID2, $PID3"

# Create Collection on Node 1
echo "Creating collection on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8061/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "test_inc_3node", "type": "document"}'

sleep 2

# Batch 1: 10,000 documents
echo "Batch 1: Inserting 10,000 documents on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8061/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR i IN 1..10000 INSERT { value: i, batch: 1, timestamp: DATE_NOW() } INTO test_inc_3node"}'

echo "Waiting for Batch 1 to replicate to Node 2 and Node 3..."
N2_DONE=false
N3_DONE=false

for i in {1..60}; do
    if [ "$N2_DONE" = false ]; then
        COUNT2=$(curl -s -u admin:admin -X POST http://localhost:8062/_api/database/_system/cursor \
          -H "Content-Type: application/json" \
          -d '{"query": "RETURN COLLECTION_COUNT(\"test_inc_3node\")"}' | grep -o '[0-9]*' | head -1)
        echo "Node 2 count: $COUNT2 / 10000"
        if [ "$COUNT2" == "10000" ]; then
            echo "Node 2: Batch 1 Replicated!"
            N2_DONE=true
        fi
    fi

    if [ "$N3_DONE" = false ]; then
        COUNT3=$(curl -s -u admin:admin -X POST http://localhost:8063/_api/database/_system/cursor \
          -H "Content-Type: application/json" \
          -d '{"query": "RETURN COLLECTION_COUNT(\"test_inc_3node\")"}' | grep -o '[0-9]*' | head -1)
        echo "Node 3 count: $COUNT3 / 10000"
        if [ "$COUNT3" == "10000" ]; then
            echo "Node 3: Batch 1 Replicated!"
            N3_DONE=true
        fi
    fi

    if [ "$N2_DONE" = true ] && [ "$N3_DONE" = true ]; then
        echo "Batch 1 Replicated Successfully to ALL nodes!"
        break
    fi
    sleep 1
done

if [ "$N2_DONE" = false ] || [ "$N3_DONE" = false ]; then
    echo "FAILURE: Batch 1 replication timed out."
    kill $PID1 $PID2 $PID3 || true
    exit 1
fi

sleep 2

# Batch 2: Another 10,000 documents (Total 20,000)
echo "Batch 2: Inserting 10,000 MORE documents on Node 1..."
curl -s -u admin:admin -X POST http://localhost:8061/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d '{"query": "FOR i IN 10001..20000 INSERT { value: i, batch: 2, timestamp: DATE_NOW() } INTO test_inc_3node"}'

echo "Waiting for Batch 2 to replicate..."
N2_DONE=false
N3_DONE=false

for i in {1..60}; do
    if [ "$N2_DONE" = false ]; then
        COUNT2=$(curl -s -u admin:admin -X POST http://localhost:8062/_api/database/_system/cursor \
          -H "Content-Type: application/json" \
          -d '{"query": "RETURN COLLECTION_COUNT(\"test_inc_3node\")"}' | grep -o '[0-9]*' | head -1)
        echo "Node 2 count: $COUNT2 / 20000"
        if [ "$COUNT2" == "20000" ]; then
            echo "Node 2: Batch 2 Replicated!"
            N2_DONE=true
        fi
    fi

    if [ "$N3_DONE" = false ]; then
        COUNT3=$(curl -s -u admin:admin -X POST http://localhost:8063/_api/database/_system/cursor \
          -H "Content-Type: application/json" \
          -d '{"query": "RETURN COLLECTION_COUNT(\"test_inc_3node\")"}' | grep -o '[0-9]*' | head -1)
        echo "Node 3 count: $COUNT3 / 20000"
        if [ "$COUNT3" == "20000" ]; then
            echo "Node 3: Batch 2 Replicated!"
            N3_DONE=true
        fi
    fi
    
    if [ "$N2_DONE" = true ] && [ "$N3_DONE" = true ]; then
        echo "SUCCESS: Batch 2 Replicated Successfully to ALL nodes!"
        break
    fi
    sleep 1
done

if [ "$N2_DONE" = false ] || [ "$N3_DONE" = false ]; then
    echo "FAILURE: Batch 2 replication timed out."
    exit 1
fi

echo "Cleaning up..."
kill $PID1 $PID2 $PID3 || true
