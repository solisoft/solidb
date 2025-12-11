#!/bin/bash
set -e

# Set consistent secret for authentication across all nodes
export JWT_SECRET="test-secret-for-3node-repl-500k"
export SOLIDB_ADMIN_PASSWORD="admin"

# Cleanup any previous run
pkill -f "solidb" || true
rm -rf tmp/repl3_n1 tmp/repl3_n2 tmp/repl3_n3
mkdir -p tmp/repl3_n1 tmp/repl3_n2 tmp/repl3_n3

BIN=./target/debug/solidb

# Start Node 1 (Bootstrap)
echo "Starting Node 1 (Bootstrap)..."
RUST_LOG=debug $BIN --port 8041 --replication-port 9041 --data-dir ./tmp/repl3_n1 > tmp/repl3_n1.log 2>&1 &
PID1=$!
sleep 5

# Start Node 2 (Peer 1)
echo "Starting Node 2 (Peer 1)..."
RUST_LOG=info $BIN --port 8042 --replication-port 9042 --peer 127.0.0.1:9041 --data-dir ./tmp/repl3_n2 > tmp/repl3_n2.log 2>&1 &
PID2=$!
sleep 5

# Start Node 3 (Peer 2)
echo "Starting Node 3 (Peer 2)..."
RUST_LOG=info $BIN --port 8043 --replication-port 9043 --peer 127.0.0.1:9041 --data-dir ./tmp/repl3_n3 > tmp/repl3_n3.log 2>&1 &
PID3=$!
sleep 5

echo "Cluster started. PIDs: $PID1, $PID2, $PID3"

# Create Collection on Node 3
echo "Creating collection on Node 3..."
curl -s -u admin:admin -X POST http://localhost:8043/_api/database/_system/collection \
  -H "Content-Type: application/json" \
  -d '{"name": "test_repl_500k", "type": "document"}'

sleep 2

# Insert 1000 documents using SDBQL loop
DOC_COUNT=1000
echo "Inserting $DOC_COUNT documents on Node 3..."
START_TIME=$(date +%s)
curl -s -u admin:admin -X POST http://localhost:8043/_api/database/_system/cursor \
  -H "Content-Type: application/json" \
  -d "{\"query\": \"FOR i IN 1..$DOC_COUNT INSERT { value: i, timestamp: DATE_NOW() } INTO test_repl_500k\"}"

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))
echo "Insertion took $DURATION seconds."

echo "Waiting for replication to catch up on Node 2 and Node 3..."

# Checks for Node 1 and Node 2
N1_DONE=false
N2_DONE=false

# Wait up to 300 seconds (5 minutes)
for i in {1..300}; do
    if [ "$N1_DONE" = false ]; then
        COUNT1=$(curl -s -u admin:admin -X POST http://localhost:8041/_api/database/_system/cursor \
          -H "Content-Type: application/json" \
          -d "{\"query\": \"RETURN COLLECTION_COUNT(\\\"test_repl_500k\\\")\"}" | grep -o '[0-9]*' | head -1)
        echo "Node 1 count: $COUNT1 / $DOC_COUNT"
        if [ "$COUNT1" == "$DOC_COUNT" ]; then
            echo "SUCCESS: Node 1 fully replicated!"
            N1_DONE=true
        fi
    fi

    if [ "$N2_DONE" = false ]; then
        COUNT2=$(curl -s -u admin:admin -X POST http://localhost:8042/_api/database/_system/cursor \
          -H "Content-Type: application/json" \
          -d "{\"query\": \"RETURN COLLECTION_COUNT(\\\"test_repl_500k\\\")\"}" | grep -o '[0-9]*' | head -1)
        echo "Node 2 count: $COUNT2 / $DOC_COUNT"
        if [ "$COUNT2" == "$DOC_COUNT" ]; then
            echo "SUCCESS: Node 2 fully replicated!"
            N2_DONE=true
        fi
    fi

    if [ "$N1_DONE" = true ] && [ "$N2_DONE" = true ]; then
        echo "ALL NODES SYNCED SUCCESSFULLY!"
        
        # Verify cluster peer discovery (optional but good sanity check)
        echo "Checking cluster status on Node 1..."
        STATUS=$(curl -s -u admin:admin http://localhost:8041/_api/cluster/status)
        echo "Cluster Status: $STATUS"
        
        break
    fi
    sleep 1
done

if [ "$N1_DONE" = false ] || [ "$N2_DONE" = false ]; then
    echo "FAILURE: Replication timed out."
    exit 1
fi

# Cleanup
kill $PID1 $PID2 $PID3 || true
echo "Test finished successfully."
